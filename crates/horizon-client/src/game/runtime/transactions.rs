//! `host.transaction` (#41 §7): NeoForge 26.3's `Transaction` and `SnapshotJournal`, kept by
//! the game at the storage level, so a mod's handlers hold no state and write no journal.
//!
//! A write through a capability helper inside a transaction applies in place, so later steps
//! see it; the first write to a storage at each depth keeps that storage as it was. What a
//! write would tell the world (`setChanged` and comparators, a jukebox starting or stopping, a
//! cauldron's block update, an item dropped) waits for the root commit and then runs once,
//! as `SnapshotJournal.onRootCommit` does. A nested commit hands its snapshots and waiting
//! work to the transaction around it; an abort puts every storage back quietly and drops the
//! waiting work, which nothing outside had seen.
//!
//! Covered here: block entities (slots, saved compound, custom name, lock, loot roll, attachments), a hopper's cooldown and
//! cauldron and composter block states. The attachment map of every other holder (an entity, a player, a
//! chunk, the level) is kept on the first access to it through `attachment_map`. A block's tentative state is stored only (`World::set_stored`): the
//! one real write, with its remesh, relight and logs, is at the root commit. A jukebox is
//! told at the root commit from its record before and after, so a disc swapped in one
//! transaction changes the song. A player's inventory (slots, armour, off hand, cursor) is kept
//! whole on its first write, from wherever the player's state lives then: the local player's or
//! a guest's, swapped in by `native_as_player`. An item a write drops from a player waits for
//! the root commit too. An item entity's stack is kept on its first write; one emptied stays,
//! empty, until the root commit removes it. A guest's menu storage (ender chest, crafting grid,
//! a mod's chest menu) is kept whole on its first write through a menu slot; a crafting grid
//! changed by the transaction has its result made again at the root commit.
//! `LivingRef::set_health` keeps the player's synced float storage and unpublished update
//! events, or the mob's health and sync dirtiness. This does not journal other health,
//! damage, death or attribute operations.
//! An entity added inside one goes at an abort, one removed is only marked until the root
//! commit, and mods hear of joins and leaves at the root commit (`transactions/entities.rs`).
use super::{BlockEntity, BlockStateId, Game, machines::HopperState};
use horizon_client::{items::ItemStack, world::BlockEntityExtras};
use horizon_mod_runtime::attachments::{Attachments, Holder};
use mod_api::{Transaction, TransactionLifecycle, TransactionSeal};
use std::collections::HashMap;

#[path = "transactions/decision.rs"]
pub(in crate::game) mod decision;
#[path = "transactions/entities.rs"]
mod entities;
pub(in crate::game) use entities::Announcement;
#[path = "transactions/scratch.rs"]
mod scratch;

type Pos = (i32, i32, i32);
type ComposterRandoms =
    HashMap<(mod_api::DimensionKey, (i32, i32)), HashMap<Pos, horizon_core::random::LegacyRandom>>;

/// The open transactions, innermost last, and which close is running, if any: what
/// `transaction_lifecycle` reads.
#[derive(Default)]
pub(super) struct OpenTransactions {
    levels: Vec<Level>,
    closing: Option<TransactionLifecycle>,
    /// The root commits telling their joins and leaves, outermost first: a listener's command in
    /// another level can commit a transaction there while this level's commit is told.
    committing: Vec<entities::Committing>,
    composter_epoch: Option<mod_api::WorldEpoch>,
    /// Each composter port owns its transactional random stream, independently of Level.random.
    composter_randoms: ComposterRandoms,
}

impl std::ops::Deref for OpenTransactions {
    type Target = Vec<Level>;
    fn deref(&self) -> &Vec<Level> {
        &self.levels
    }
}

impl std::ops::DerefMut for OpenTransactions {
    fn deref_mut(&mut self) -> &mut Vec<Level> {
        &mut self.levels
    }
}

/// Puts the closing phase back when a close ends, however it ends.
struct ClosingPhase<'a, 'g> {
    game: &'a mut Game<'g>,
    outer: Option<TransactionLifecycle>,
}

impl Drop for ClosingPhase<'_, '_> {
    fn drop(&mut self) {
        self.game.transactions.closing = self.outer;
    }
}

/// One open transaction.
#[derive(Default)]
pub(super) struct Level {
    /// What names it to the mods that write in it (`Transaction`): unguessable in practice, so
    /// only the value the game handed out names it.
    token: u64,
    /// What ends it (`TransactionSeal`), drawn apart from `token`: only whoever opened it holds
    /// it, never a handler lent the `Transaction`.
    close_token: u64,
    dimension: Option<super::Dim>,
    /// Each block entity as it was before this transaction first wrote it.
    block_entities: HashMap<Pos, Kept>,
    /// The block entities this transaction removed (`journal_remove_block_entity`): an abort
    /// puts them back as `block_entities` keeps them.
    removed_block_entities: std::collections::HashSet<Pos>,
    /// Each item slot (block entity, slot) a slot-only write changed, as it was before this
    /// transaction first wrote it, for a block entity not kept whole in `block_entities`: a
    /// move through a container keeps the slot it touches, not the entity's whole saved data.
    slots: HashMap<(Pos, usize), KeptSlot>,
    /// Each block's state before this transaction first wrote it.
    blocks: HashMap<Pos, BlockStateId>,
    /// Composter-local random streams before this depth first evaluated their layers.
    composter_randoms: ComposterRandoms,
    /// Each quart cell before this transaction first wrote it.
    biomes: HashMap<Pos, u16>,
    /// Each player's inventory before this transaction first wrote it, by UUID.
    players: HashMap<u128, KeptPlayer>,
    /// Synced player health and its unpublished update events before set_health.
    player_health: HashMap<u128, KeptPlayerHealth>,
    /// Mob health and synchronization dirtiness before set_health.
    mob_health: HashMap<u32, KeptMobHealth>,
    /// Each entity's inventory (`entity_slots`) before this transaction first wrote it.
    entities: HashMap<u32, Vec<Option<ItemStack>>>,
    /// Each mob's or armor stand's equipment before this transaction first
    /// wrote it (#932), told apart for the way back.
    equipment: HashMap<u32, KeptEquipment>,
    /// Each chested mob's item list's length before `entity_slots` first grew it to its slots
    /// in this transaction.
    mob_lengths: HashMap<u32, usize>,
    /// Each item entity's stack before this transaction first wrote it.
    item_entities: HashMap<u32, ItemStack>,
    /// Each guest's menu storage before this transaction first wrote it, by connection.
    menus: HashMap<u32, KeptMenus>,
    /// Each entity's, player's, chunk's and the level's attachment map before this transaction
    /// first reached it (a block entity's is kept with it).
    attachments: HashMap<Holder, KeptAttachments>,
    /// Held once this transaction reaches an attachment map, until it closes: the save
    /// admission gate (`passes_settled`) refuses a save while a map may hold tentative
    /// values, which only the root commit makes saveable.
    save_gate: Option<horizon_client::join_seq::visibility::PassLease>,
    /// Each chunk's unsaved mark and save queue entry before this transaction first reached an
    /// attachment map in it: an abort puts them back, so a chunk only a tentative write marked
    /// is clean again.
    chunk_marks: HashMap<(i32, i32), horizon_client::world::ChunkMark>,
    /// What the root commit does, in order.
    after: Vec<After>,
    /// Internal decide/fire/apply state. No menu route uses this yet.
    actions: decision::Journal,
    action: Option<decision::ActiveAction>,
    /// The scratch stacks (`ItemPlace::Scratch`), live: kept by the outermost transaction only.
    scratch: HashMap<u64, scratch::Scratch>,
    /// Each scratch stack before this transaction first wrote it; none where it didn't exist.
    scratch_kept: HashMap<u64, Option<scratch::Scratch>>,
    /// The entities it added and removed, and the announcements it holds.
    lifetimes: entities::EntityJournal,
}

/// A slot's stack before a transaction first wrote it, and the generation of the block entity
/// it belongs to: an abort puts it back only into that same block entity, never into one that
/// replaced it.
struct KeptSlot {
    stack: Option<ItemStack>,
    generation: Option<u64>,
}

struct KeptAttachments {
    contents: Attachments,
    indexed: bool,
}

struct Kept {
    entity: Option<BlockEntity>,
    generation: Option<u64>,
    /// None where it had no map: an abort removes one a write made.
    attachments: Option<Attachments>,
    hopper: Option<HopperState>,
    /// The custom name Horizon keeps beside the model (`World::block_names`).
    name: Option<horizon_core::nbt::Tag>,
    /// The container lock and loot table roll kept beside the model.
    extras: BlockEntityExtras,
}

impl Kept {
    fn capture(
        world: &horizon_client::world::World,
        hoppers: &HashMap<Pos, HopperState>,
        pos: Pos,
    ) -> Self {
        Self {
            entity: world.block_entities.get(&pos).cloned(),
            generation: world.block_entities.generation(&pos),
            attachments: world.block_entities.attachments(pos).cloned(),
            hopper: hoppers.get(&pos).copied(),
            name: world.block_names.get(&pos).cloned(),
            extras: world.block_entity_extras(pos),
        }
    }
}

struct KeptPlayerHealth {
    data: horizon_world::synced_floats::SyncedFloats,
    events_len: usize,
    dead: bool,
    guest: Option<(u32, bool, u32)>,
}

struct KeptMobHealth {
    absorption: f32,
    health: f32,
    needs_sync: bool,
    death_time: i32,
    death_canceled: bool,
    processed: bool,
}

#[derive(Clone)]
struct KeptPlayer {
    ender_items: Vec<Option<ItemStack>>,
    player_extra: Option<horizon_core::nbt::Tag>,
    inventory: Vec<Option<ItemStack>>,
    armor: [Option<ItemStack>; 4],
    offhand: Option<ItemStack>,
    equipment: horizon_items::equipment::Equipment,
    carried: Option<ItemStack>,
}

/// A mob's or an armor stand's equipment as a transaction first wrote it
/// (#932): the same eight slots, restored their own way.
#[derive(Clone)]
enum KeptEquipment {
    Mob(Vec<Option<ItemStack>>),
    Stand(Vec<Option<ItemStack>>),
}

#[derive(Clone)]
struct KeptMenus {
    ender_items: Vec<Option<ItemStack>>,
    craft_grid: [Option<ItemStack>; 4],
    native_menu: Vec<Option<ItemStack>>,
}

/// What a write tells the world, at the root commit.
pub(super) enum After {
    /// Reserved raw bytes publish only when their connection incarnation is still live.
    Payload(super::payload_transport::Deferred),
    /// A mod's validated vanilla packet, bound to its physical protocol incarnation.
    Packet(horizon_client::net::PreparedPacket),
    Cauldron(super::cauldron_interactions::Effects),
    Composter(super::composter::Effects),
    /// A mod's world pickup or placement of fluid, its block already set: what is left of it
    /// (`Game::commit_fluid_write`).
    FluidWrite(Box<super::world_fluids::WrittenFluid>),
    /// A bucket's sound and fluid game event (`level_play_fluid_effects`).
    FluidEffects(Box<super::world_fluids::FluidEffects>),
    /// `BlockEntity.setChanged`.
    Changed(Pos),
    /// `BlockEntity.setChanged` and `Level.sendBlockUpdated` after its data was loaded.
    Updated(Pos),
    /// `Level.sendBlockUpdated` alone, without marking the save dirty.
    NotifyClients(Pos),
    /// A provider's answer changed here (`level_invalidate_capabilities`): the capability
    /// generation moves on.
    Capabilities(Pos),

    /// `ServerLevel.destroyBlockProgress`: breaker id, position, stage. Sent once, at the root commit.
    BreakProgress(i32, [i32; 3], i32),
    /// The block's new state, set with its updates (`Level.setBlock`, `UPDATE_ALL`).
    Block(Pos),
    /// Save, remesh and resend a changed biome column.
    Biome((i32, i32)),
    /// Deferred interaction effects, replayed in the same root-commit order.
    Effect(DeferredEffect),
    /// `Block.popResource`.
    Drop([i32; 3], ItemStack),
    /// `Player.drop(stack, false)` by the player with this UUID: what no longer fits.
    PlayerDrop(u128, ItemStack),
    GivePickup(u128),
    InventoryTrigger(u128),
    GiveRest(u128, ItemStack, Option<[i32; 4]>),
    PlayerEquip(u128, String, Option<ItemStack>),
    /// An item split off an item entity, dropped where it is (`Block.popResource`'s delay).
    Spawn([f64; 3], ItemStack),
    /// The item entity with this id was emptied: removed, unless it holds items again.
    Emptied(u32),
    /// `LivingEntity.onEquipItem`'s sound and equip/unequip game event, at the entity `who`
    /// is: what an equipment write through a transaction tells the world only at the root
    /// commit, so an abort stays quiet (#932).
    Equip {
        at: [f64; 3],
        sound: Option<String>,
        equips: bool,
        who: u32,
    },
}

/// Effects shared by transactional interaction contexts.
pub(super) enum DeferredEffect {
    CustomStat {
        player: u128,
        key: &'static str,
        amount: i32,
    },
    Sound {
        sound: &'static str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
    },
}

/// A fresh transaction token: a process-wide count through a randomly keyed 64-bit hash, so a
/// mod cannot guess, in practice, the token of a transaction it was not handed.
fn frame_token() -> u64 {
    use std::hash::BuildHasher;
    use std::sync::{OnceLock, atomic::AtomicU64, atomic::Ordering};
    static KEYS: OnceLock<std::hash::RandomState> = OnceLock::new();
    static COUNT: AtomicU64 = AtomicU64::new(0);
    KEYS.get_or_init(std::hash::RandomState::new)
        .hash_one(COUNT.fetch_add(1, Ordering::Relaxed))
}

impl Level {
    /// Keeps chested mob `id`'s item list's length, `len`, before `entity_slots` grows it, if
    /// this transaction hasn't yet.
    pub(super) fn keep_mob_length(&mut self, id: u32, len: usize) {
        self.mob_lengths.entry(id).or_insert(len);
    }
}

impl Game<'_> {
    /// Opens a transaction, nested in the open one if any (`Transaction.open`), and gives back
    /// the seal that ends it.
    pub(in crate::game) fn transaction_open(&mut self) -> TransactionSeal {
        let (token, close) = self.open_storage_transaction();
        mod_api::__private::transaction_seal(token, close)
    }
    /// Opens a transaction and gives back its write and close tokens.
    fn open_storage_transaction(&mut self) -> (u64, u64) {
        if let Some(scope) = &mut self.block_entity_tick_items {
            scope.open_transaction();
        }
        let (token, close_token) = (frame_token(), frame_token());
        if self.transactions.is_empty() {
            self.world.block_entities.defer_retirement();
        }
        let lifetimes = self.entity_journal_open();
        self.transactions.push(Level {
            token,
            close_token,
            dimension: Some(self.dim),
            lifetimes,
            ..Level::default()
        });
        if let Some(key) = &mut self.block_entity_tick_key {
            key.unbalanced = true;
        }
        (token, close_token)
    }
    /// Closes the transaction `seal` ends, first aborting every one opened inside it and left
    /// open, so a guard closing its own is never stranded behind a child a mod abandoned.
    /// Refused when `seal` names no open transaction: only whoever opened one ends it, never a
    /// handler lent its `Transaction` (a hopper's move lends the game's own).
    pub(in crate::game) fn transaction_close(&mut self, seal: &TransactionSeal, commit: bool) {
        let (token, close) = mod_api::__private::seal_tokens(seal);
        let Some(at) = self
            .transactions
            .iter()
            .position(|level| level.token == token && level.close_token == close)
        else {
            log::warn!("Horizon: refused to close a transaction the seal does not end");
            return;
        };
        if self.transactions.len() > at + 1 {
            log::warn!("Horizon: aborted a transaction left open inside the one being closed");
        }
        while self.transactions.len() > at + 1 {
            self.close_storage_transaction(false);
        }
        self.close_storage_transaction(commit);
    }
    /// Whether `tx` names an open transaction: a write given any other changes nothing, so none
    /// happens outside the journal.
    pub(in crate::game) fn transaction_holds(&self, tx: &Transaction) -> bool {
        let named = mod_api::__private::transaction_token(tx);
        self.transactions.iter().any(|level| level.token == named)
    }
    /// `transaction_lifecycle`: the close running, else whether one is open.
    pub(in crate::game) fn transaction_lifecycle(&self) -> TransactionLifecycle {
        match self.transactions.closing {
            Some(closing) => closing,
            None if self.transactions.is_empty() => TransactionLifecycle::None,
            None => TransactionLifecycle::Open,
        }
    }
    /// `transaction_current`: the innermost open transaction's write token, never its seal.
    pub(in crate::game) fn transaction_current(&self) -> Option<Transaction> {
        let level = self.transactions.last()?;
        Some(mod_api::__private::transaction_of(level.token))
    }
    /// `transaction_depth`: how deeply `tx` is nested, 0 for the outermost.
    pub(in crate::game) fn transaction_depth(&self, tx: &Transaction) -> Option<u32> {
        let named = mod_api::__private::transaction_token(tx);
        let at = self
            .transactions
            .iter()
            .position(|level| level.token == named)?;
        u32::try_from(at).ok()
    }
    /// Closes the innermost transaction: `commit` hands its changes to the one around it, or
    /// at the root makes them final and tells the world; otherwise every storage it wrote goes
    /// back to how it was. Nothing happens with none open.
    fn close_storage_transaction(&mut self, commit: bool) {
        let Some(level) = self.transactions.pop() else {
            return;
        };
        let closing = if self.transactions.is_empty() {
            TransactionLifecycle::RootClosing
        } else {
            TransactionLifecycle::Closing
        };
        // Kept and put back, so a transaction a root-commit listener opens and closes doesn't
        // end the root's phase early. The guard puts it back on a panic too, so a listener
        // that panics leaves the phase as it was.
        let outer = self.transactions.closing.replace(closing);
        // The root's own close, not one a root-commit listener opened: what the transactions
        // removed is final now, so it retires (`defer_retirement`).
        let root = self.transactions.is_empty() && outer.is_none();
        let guard = ClosingPhase { game: self, outer };
        guard.game.close_level(level, commit);
        if root {
            guard.game.world.block_entities.publish_retirement();
        }
    }
    /// What closing `level` does, with the closing phase already set.
    fn close_level(&mut self, mut level: Level, commit: bool) {
        if let Some(scope) = &mut self.block_entity_tick_items {
            scope.close_transaction(commit);
        }
        if let Some(key) = &mut self.block_entity_tick_key {
            key.unbalanced = self.transactions.len() != key.transaction_depth;
        }
        debug_assert_eq!(
            level.dimension,
            Some(self.dim),
            "transaction closed in a different level"
        );
        let lifetimes = std::mem::take(&mut level.lifetimes);
        match (commit, self.transactions.last_mut()) {
            (true, Some(outer)) => {
                outer.actions.merge(level.actions);
                // Slots before whole entities: a slot the outer transaction already kept the
                // whole entity of is older there, but one kept whole only by this child is not,
                // so the child's slot entries go up beside its capture.
                for ((pos, slot), kept) in level.slots {
                    if !outer.block_entities.contains_key(&pos) {
                        outer.slots.entry((pos, slot)).or_insert(kept);
                    }
                }
                for (pos, kept) in level.block_entities {
                    outer.block_entities.entry(pos).or_insert(kept);
                }
                outer
                    .removed_block_entities
                    .extend(level.removed_block_entities);
                for (pos, state) in level.blocks {
                    outer.blocks.entry(pos).or_insert(state);
                }
                for (chunk, randoms) in level.composter_randoms {
                    let kept = outer.composter_randoms.entry(chunk).or_default();
                    for (pos, random) in randoms {
                        kept.entry(pos).or_insert(random);
                    }
                }
                for (pos, biome) in level.biomes {
                    outer.biomes.entry(pos).or_insert(biome);
                }
                for (uuid, kept) in level.players {
                    outer.players.entry(uuid).or_insert(kept);
                }
                for (uuid, kept) in level.player_health {
                    outer.player_health.entry(uuid).or_insert(kept);
                }
                for (id, kept) in level.mob_health {
                    outer.mob_health.entry(id).or_insert(kept);
                }
                for (id, kept) in level.entities {
                    outer.entities.entry(id).or_insert(kept);
                }
                for (id, kept) in level.equipment {
                    outer.equipment.entry(id).or_insert(kept);
                }
                for (id, kept) in level.item_entities {
                    outer.item_entities.entry(id).or_insert(kept);
                }
                for (id, len) in level.mob_lengths {
                    outer.mob_lengths.entry(id).or_insert(len);
                }
                for (conn, kept) in level.menus {
                    outer.menus.entry(conn).or_insert(kept);
                }
                for (holder, kept) in level.attachments {
                    outer.attachments.entry(holder).or_insert(kept);
                }
                for (id, kept) in level.scratch_kept {
                    outer.scratch_kept.entry(id).or_insert(kept);
                }
                outer.save_gate = outer.save_gate.take().or(level.save_gate);
                for (chunk, mark) in level.chunk_marks {
                    outer.chunk_marks.entry(chunk).or_insert(mark);
                }
                outer.after.extend(level.after);
                // The nearest enclosing transaction of this level takes its entities back; a
                // parent in another level can't, and with none in this level they are told now.
                match (level.dimension)
                    .and_then(|dim| entities::open_in(&mut self.transactions, dim))
                {
                    Some(enclosing) => enclosing.lifetimes.merge(lifetimes),
                    None => self.commit_entity_journal(lifetimes),
                }
            }
            (true, None) => {
                self.root_commit(level);
                self.commit_entity_journal(lifetimes);
            }
            (false, _) => {
                // The entities it added go first, so nothing is put back into one.
                let arrivals = self.roll_back_entity_journal(lifetimes);
                let dimension = level.dimension;
                self.roll_back(level);
                if let Some(enclosing) =
                    dimension.and_then(|dim| entities::open_in(&mut self.transactions, dim))
                {
                    enclosing.lifetimes.keep_arrivals(arrivals);
                }
            }
        }
    }
    /// Closes every transaction left open, as aborted. Run before each tick, as a backstop:
    /// `host.transaction` is the only way in, and its drop guard closes each one it opens, so
    /// none is left open by a mod.
    pub(super) fn abort_open_transactions(&mut self) {
        while !self.transactions.is_empty() {
            self.close_storage_transaction(false);
        }
        self.world.block_entities.publish_retirement();
    }

    /// `ComposterWrapper.transactionalRandom`: snapshot only the position's private owner,
    /// including its initial state on a first access that later aborts.
    pub(super) fn take_composter_random(
        &mut self,
        pos: [i32; 3],
    ) -> horizon_core::random::LegacyRandom {
        let epoch = self.native_lifecycle.epoch();
        if self.transactions.composter_epoch != epoch {
            self.clear_composter_randoms();
            self.transactions.composter_epoch = epoch;
        }
        let chunk = (self.dim.key(), (pos[0] >> 4, pos[2] >> 4));
        let key = (pos[0], pos[1], pos[2]);
        let random = self
            .transactions
            .composter_randoms
            .entry(chunk)
            .or_default()
            .remove(&key)
            .unwrap_or_else(|| (*self.world.random).clone());
        if let Some(level) = self.transactions.last_mut() {
            level
                .composter_randoms
                .entry(chunk)
                .or_default()
                .entry(key)
                .or_insert_with(|| random.clone());
        }
        random
    }

    pub(super) fn put_composter_random(
        &mut self,
        pos: [i32; 3],
        random: horizon_core::random::LegacyRandom,
    ) {
        self.transactions
            .composter_randoms
            .entry((self.dim.key(), (pos[0] >> 4, pos[2] >> 4)))
            .or_default()
            .insert((pos[0], pos[1], pos[2]), random);
    }

    /// Retire only an unloading chunk's cached owners and their pending savepoints.
    pub(super) fn forget_composter_chunk(
        &mut self,
        dimension: mod_api::DimensionKey,
        chunk: [i32; 2],
    ) {
        let key = (dimension, (chunk[0], chunk[1]));
        self.transactions.composter_randoms.remove(&key);
        for level in &mut self.transactions.levels {
            level.composter_randoms.remove(&key);
        }
    }

    pub(super) fn clear_composter_randoms(&mut self) {
        self.transactions.composter_epoch = None;
        self.transactions.composter_randoms.clear();
        for level in &mut self.transactions.levels {
            level.composter_randoms.clear();
        }
    }
    /// Keeps the block entity at `pos` as it is, with its attachments and a hopper's
    /// cooldown, if a transaction is open and hasn't yet.
    pub(super) fn journal_block_entity(&mut self, pos: Pos) {
        let Some(level) = self.transactions.last_mut() else {
            return;
        };
        level
            .block_entities
            .entry(pos)
            .or_insert_with(|| Kept::capture(&self.world, &self.hoppers, pos));
    }
    /// Puts back the block entity at `pos` that a transaction removed, as `kept` holds it, with
    /// its generation, where none stands there now.
    fn restore_removed_block_entity(&mut self, pos: Pos, kept: &Kept) {
        let Some(entity) = &kept.entity else {
            return;
        };
        if self.world.block_entities.get(&pos).is_some() {
            return;
        }
        self.world.block_entities.insert(pos, entity.clone());
        if let Some(generation) = kept.generation {
            self.world
                .block_entities
                .restore_generation(pos, generation);
            self.world.block_entities.reattach_retired(pos, generation);
        }
    }
    /// Removes the block entity at `pos`, kept whole by the open transaction, which puts it back
    /// if it aborts: a block write in the transaction that drops it, so later reads in it see
    /// none. Outside a transaction, it just goes.
    pub(super) fn journal_remove_block_entity(&mut self, pos: Pos) {
        self.journal_block_entity(pos);
        if let Some(level) = self.transactions.last_mut() {
            level.removed_block_entities.insert(pos);
        }
        self.world.block_entities.remove(&pos);
    }
    /// How many block entities the open transactions keep whole, and how many single slots:
    /// what a test reads to tell a slot journal from a whole capture without timing it.
    #[cfg(test)]
    pub(in crate::game) fn journal_kept(&self) -> (usize, usize) {
        self.transactions
            .iter()
            .fold((0, 0), |(whole, slots), level| {
                (
                    whole + level.block_entities.len(),
                    slots + level.slots.len(),
                )
            })
    }
    /// Keeps item slot `slot` of the block entity at `pos` as it is, if a transaction is open
    /// and keeps neither it nor the whole entity yet. Only for a write that changes that slot
    /// and nothing else of the entity (no cooldown, cooking or saved data beside it), so the
    /// journal holds one stack however large the entity's saved data is. A slot past the end of
    /// the entity's slot list (a mod's list shorter than its container) is a write that grows
    /// the list: the entity is kept whole instead, length and all.
    pub(super) fn journal_block_entity_slot(&mut self, pos: Pos, slot: usize) {
        let exists = self
            .world
            .block_entities
            .get(&pos)
            .is_some_and(|entity| slot < entity.slots().len());
        if !exists {
            return self.journal_block_entity(pos);
        }
        let Some(level) = self.transactions.last_mut() else {
            return;
        };
        if level.block_entities.contains_key(&pos) {
            return;
        }
        level.slots.entry((pos, slot)).or_insert_with(|| KeptSlot {
            stack: self
                .world
                .block_entities
                .get(&pos)
                .and_then(|entity| entity.slots().get(slot).cloned().flatten()),
            generation: self.world.block_entities.generation(&pos),
        });
    }
    /// Keeps `holder`'s attachment map as it is, if a transaction is open and hasn't yet: a
    /// block entity's with the rest of it, any other's alone.
    pub(super) fn journal_attachments(&mut self, holder: Holder) {
        if let Some(level) = self.transactions.last_mut() {
            level
                .save_gate
                .get_or_insert_with(horizon_client::join_seq::visibility::PassLease::enter);
            // The chunk that a block entity's or chunk's map marks unsaved (`world_attachments`).
            let chunk = match holder {
                Holder::BlockEntity([x, _, z]) => Some((x >> 4, z >> 4)),
                Holder::Chunk(chunk) => Some((chunk[0], chunk[1])),
                _ => None,
            };
            if let Some(chunk) = chunk {
                let modified = &self.world.modified;
                level
                    .chunk_marks
                    .entry(chunk)
                    .or_insert_with(|| modified.mark(chunk));
            }
        }
        if let Holder::BlockEntity([x, y, z]) = holder {
            return self.journal_block_entity((x, y, z));
        }
        let Some(level) = self.transactions.last() else {
            return;
        };
        if level.attachments.contains_key(&holder) {
            return;
        }
        let entity_bound = if let Holder::Entity(id) = holder {
            std::num::NonZeroU32::new(id).and_then(|id| {
                match self.with_resident_slot(&self.dim.key(), id, |slot| slot.is_bound()) {
                    super::entity_residency::SlotAccess::Done(bound) => Some(bound),
                    _ => None,
                }
            })
        } else {
            None
        };
        let Ok(kept) = self
            .attachment_slot(holder, mod_api::AttachmentAccess::Existing)
            .map(|map| KeptAttachments {
                contents: map.clone(),
                indexed: entity_bound.unwrap_or_else(|| map.has_index_binding()),
            })
        else {
            return;
        };
        if let Some(level) = self.transactions.last_mut() {
            level.attachments.insert(holder, kept);
        }
    }
    /// Keeps the inventory of the player whose state is in `Game` now, `uuid`, as it is, if a
    /// transaction is open and hasn't yet. Called inside `native_as_player`.
    pub(super) fn journal_player(&mut self, uuid: u128) {
        let Some(level) = self.transactions.last_mut() else {
            return;
        };
        level.players.entry(uuid).or_insert_with(|| KeptPlayer {
            ender_items: self.ender_items.clone(),
            player_extra: self.level.player_extra.clone(),
            inventory: self.inventory.clone(),
            armor: self.armor.clone(),
            offhand: self.offhand.clone(),
            equipment: self.equipment.clone(),
            carried: self.carried.clone(),
        });
    }
    /// Called with the target player's body swapped in, before set_health changes it.
    pub(super) fn journal_player_health(&mut self) {
        let uuid = self.native_acting_id();
        let Some(level) = self.transactions.last_mut() else {
            return;
        };
        let Some(player) = self.player.as_ref() else {
            return;
        };
        let std::collections::hash_map::Entry::Vacant(entry) = level.player_health.entry(uuid)
        else {
            return;
        };
        let guest = self.lan_acting.and_then(|conn| {
            let guest = self
                .lan
                .as_ref()?
                .hub
                .guests
                .iter()
                .find(|guest| guest.conn == conn)?;
            Some((conn, guest.death_canceled, guest.death_time))
        });
        entry.insert(KeptPlayerHealth {
            data: player.health.data.clone(),
            events_len: player.events.len(),
            dead: player.health.dead,
            guest,
        });
    }
    pub(super) fn journal_mob_health(&mut self, id: u32) {
        let Some(level) = self.transactions.last_mut() else {
            return;
        };
        if let Some(mob) = self.mobs.iter().find(|m| m.id == id && !m.removed) {
            level.mob_health.entry(id).or_insert(KeptMobHealth {
                health: mob.health,
                absorption: mob.absorption,
                needs_sync: mob.needs_sync,
                death_time: mob.death_time,
                death_canceled: mob.death_canceled,
                processed: self.died.contains(&id),
            });
        }
    }
    /// Keeps `player`'s menu storage as it is, if a transaction is open and hasn't yet.
    pub(super) fn journal_menus(&mut self, player: mod_api::PlayerRef) {
        let Some(level) = self.transactions.last() else {
            return;
        };
        let Some(super::native_server::PlayerRead::Guest(g)) = self.player_handle_read(player)
        else {
            return;
        };
        let conn = g.conn;
        if level.menus.contains_key(&conn) {
            return;
        }
        let kept = KeptMenus {
            ender_items: g.ender_items.clone(),
            craft_grid: g.craft_grid.clone(),
            native_menu: g.native_menu.clone(),
        };
        if let Some(level) = self.transactions.last_mut() {
            level.menus.insert(conn, kept);
        }
    }
    /// Keeps entity `id`'s inventory as it is, if a transaction is open and hasn't yet.
    pub(super) fn journal_entity(&mut self, id: u32) {
        if self.transactions.is_empty() {
            return;
        }
        let Some(slots) = self.edit_entity_slots(id, |slots, _| slots.to_vec()) else {
            return;
        };
        if let Some(level) = self.transactions.last_mut() {
            level.entities.entry(id).or_insert(slots);
        }
    }
    /// Keeps mob `id`'s equipment as it is, if a transaction is open and hasn't yet (#932):
    /// an abort puts every slot back and makes the attribute map over again.
    pub(super) fn journal_equipment(&mut self, id: u32) {
        let Some(level) = self.transactions.last_mut() else {
            return;
        };
        if level.equipment.contains_key(&id) {
            return;
        }
        let kept = if let Some(mob) = self.mobs.iter().find(|m| m.id == id && !m.removed) {
            KeptEquipment::Mob(mob.equipment_stacks().iter().cloned().collect())
        } else if let Some(stand) = self.armor_stands.iter().find(|a| a.id == id) {
            KeptEquipment::Stand(stand.equipment.iter().cloned().collect())
        } else {
            return;
        };
        level.equipment.insert(id, kept);
    }
    /// `drop_plain` by the player whose state is in `Game` now, `uuid`: at once, or at the root
    /// commit inside a transaction.
    pub(super) fn drop_after_commit(&mut self, uuid: u128, stack: ItemStack) {
        match self.transactions.last_mut() {
            Some(level) => level.after.push(After::PlayerDrop(uuid, stack)),
            None => {
                self.drop_plain(stack);
            }
        }
    }
    /// Sets the block at `pos` to `state`. In a transaction, stored only, keeping the old
    /// state, and set with its updates at the root commit; otherwise at once
    /// (`Level.setBlock`).
    pub(super) fn journal_set_block(&mut self, pos: [i32; 3], state: BlockStateId) -> bool {
        let Some(level) = self.transactions.last_mut() else {
            return self.level_set(pos, state).is_ok();
        };
        let key = (pos[0], pos[1], pos[2]);
        let old = self.world.block(pos[0], pos[1], pos[2]);
        level.blocks.entry(key).or_insert(old);
        level.after.push(After::Block(key));
        self.world.set_stored(pos[0], pos[1], pos[2], state)
    }
    /// Keep the original cell without publishing the tentative write.
    pub(in crate::game) fn journal_biome(&mut self, pos: [i32; 3], old: u16) {
        if let Some(level) = self.transactions.last_mut() {
            level
                .biomes
                .entry((pos[0] >> 2, pos[1] >> 2, pos[2] >> 2))
                .or_insert(old);
        }
    }
    /// Queue a reusable interaction effect for the outermost successful commit.
    pub(super) fn defer_effect(&mut self, effect: DeferredEffect) {
        self.after_commit(After::Effect(effect));
    }
    /// Runs `after` at once, or at the root commit inside a transaction.
    pub(super) fn after_commit(&mut self, after: After) {
        match self.transactions.last_mut() {
            Some(level) => level.after.push(after),
            None => self.run_after(after),
        }
    }
    fn run_after(&mut self, after: After) {
        match after {
            After::Payload(payload) => payload.publish(),
            After::Packet(packet) => packet.publish(),
            After::Cauldron(effects) => self.commit_cauldron_effects(effects),
            After::Composter(effects) => self.commit_composter_effects(effects),
            After::FluidWrite(written) => self.commit_fluid_write(*written),
            After::FluidEffects(effects) => self.commit_fluid_effects(*effects),
            After::Changed(pos) => self.container_changed(pos),
            After::Updated(pos) => self.block_entity_updated(pos),
            After::NotifyClients(pos) => self.notify_block_clients(pos),
            After::Capabilities(pos) => self.world.capability_generations.invalidate(pos),

            After::BreakProgress(breaker, pos, progress) => {
                self.server_destroy_block_progress(breaker, pos, progress);
            }
            After::Biome(key) => {
                self.commit_biome_columns(&std::collections::HashSet::from([key]), &[key]);
            }
            After::Effect(effect) => match effect {
                DeferredEffect::CustomStat {
                    player,
                    key,
                    amount,
                } => {
                    self.native_as_player(player, |g| g.award_custom_stat(key, amount));
                }
                DeferredEffect::Sound {
                    sound,
                    at,
                    volume,
                    pitch,
                } => self.audio.play(sound, Some(at), volume, pitch),
            },
            // Only reached through a root commit, which sets the block itself.
            After::Block(_) => {}
            After::Drop(pos, stack) => {
                self.spawn_item(stack, pos.map(|p| f64::from(p) + 0.5), [0.0; 3], 10);
            }
            After::GivePickup(uuid) => {
                self.native_as_player(uuid, |g| g.give_pickup());
            }
            After::InventoryTrigger(uuid) => {
                self.native_as_player(uuid, |g| {
                    g.check_inventory_trigger();
                });
            }
            After::GiveRest(uuid, stack, owner) => {
                self.native_as_player(uuid, |g| g.give_remainder(stack, owner));
            }
            After::PlayerEquip(uuid, slot, stack) => {
                self.native_as_player(uuid, |g| {
                    g.player_equipped(&slot, stack.as_ref());
                    g.apply_equipment();
                });
            }
            After::PlayerDrop(uuid, stack) => {
                // A player who left meanwhile drops nothing; their inventory had changed for good.
                self.native_as_player(uuid, |g| {
                    g.drop_plain(stack);
                });
            }
            After::Spawn(at, stack) => self.spawn_item(stack, at, [0.0; 3], 10),
            After::Emptied(id) => {
                let emptied = self
                    .dropped
                    .iter()
                    .position(|e| e.id == id && e.stack.count == 0);
                if let Some(i) = emptied {
                    self.remove_dropped(i);
                }
            }
            After::Equip {
                at,
                sound,
                equips,
                who,
            } => {
                // `equip_item_effects` rebuilt from the entity it names: the
                // sculk context reads the mob as it is at the root commit.
                let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(who)), None);
                self.equip_item_effects(at, sound, equips, ctx);
            }
        }
    }
    fn root_commit(&mut self, level: Level) {
        let mut told = std::collections::HashSet::new();
        let mut biome_columns = std::collections::HashSet::new();
        for after in level.after {
            match after {
                // Each storage tells the world once, after every write.
                After::Changed(pos) if !told.insert(Ok(pos)) => {}
                After::Block(pos) => {
                    if !told.insert(Err(pos)) {
                        continue;
                    }
                    let now = self.world.block(pos.0, pos.1, pos.2);
                    let was = level.blocks[&pos];
                    // A block entity the transaction removed comes back where a later write
                    // there put back a block of the kind that had it.
                    let b = self.res.behaviours();
                    if level.removed_block_entities.contains(&pos)
                        && b.block(now) == b.block(was)
                        && let Some(kept) = level.block_entities.get(&pos)
                    {
                        self.restore_removed_block_entity(pos, kept);
                    }
                    if now != was {
                        // The stored state back, so the one real write sees the change.
                        self.world.set_stored(pos.0, pos.1, pos.2, was);
                        let _ = self.level_set([pos.0, pos.1, pos.2], now);
                    }
                }
                After::Biome(key) if !biome_columns.insert(key) => {}
                after => self.run_after(after),
            }
        }
        // A crafting grid the transaction changed has its result made again, once for each
        // player however many writes it took (`InventoryMenu.slotsChanged`).
        for (conn, kept) in &level.menus {
            let mut guests = self.lan.iter().flat_map(|host| host.hub.guests.iter());
            let changed =
                guests.any(|g| g.placed && g.conn == *conn && g.craft_grid != kept.craft_grid);
            if changed {
                self.guest_craft_result(*conn);
            }
        }
        for (pos, kept) in level.block_entities {
            let record = |entity: Option<&BlockEntity>| match entity {
                Some(BlockEntity::Jukebox(j)) => j.record().cloned(),
                _ => None,
            };
            let was = record(kept.entity.as_ref());
            let now = record(self.world.block_entities.get(&pos));
            if was == now {
                continue;
            }
            // The song of the record it had stops; the one it has now starts.
            let pos = [pos.0, pos.1, pos.2];
            if was.is_some() {
                self.jukebox_stopped(pos);
            }
            if now.is_some() {
                self.jukebox_inserted(pos);
            }
        }
    }
    fn roll_back(&mut self, mut level: Level) {
        self.scratch_roll_back(std::mem::take(&mut level.scratch_kept));
        // Discard deferred effects before undoing tentative action storage.
        level.after.clear();
        // Each mob's equipment goes back as it was, and the attribute map is
        // made over it again (#932).
        let equipment = std::mem::take(&mut level.equipment);
        for (id, kept) in equipment {
            match kept {
                KeptEquipment::Mob(stacks) => {
                    let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
                        continue;
                    };
                    for (slot, stack) in stacks.into_iter().enumerate() {
                        mob.set_equipment_stack(slot, stack);
                    }
                    self.collect_mob_equipment_attributes(id);
                }
                // An armor stand keeps no live attribute map of its gear; the
                // plain write puts its slots back as they were.
                KeptEquipment::Stand(stacks) => {
                    let Some(stand) = self.armor_stands.iter_mut().find(|a| a.id == id) else {
                        continue;
                    };
                    for (slot, stack) in stacks.into_iter().enumerate() {
                        stand.equipment.set(slot, stack);
                    }
                }
            }
        }

        for (pos, kept) in level.block_entities {
            match (&kept.entity, self.world.block_entities.get_mut(&pos)) {
                (Some(entity), Some(now)) => {
                    *now = entity.clone();
                    self.world
                        .block_entities
                        .restore_generation(pos, kept.generation.unwrap());
                }
                // One this transaction removed comes back.
                (Some(_), None) if level.removed_block_entities.contains(&pos) => {
                    self.restore_removed_block_entity(pos, &kept);
                }
                _ => {}
            }
            self.world
                .block_entities
                .put_back_attachments(pos, kept.generation, kept.attachments);
            match kept.name {
                Some(name) => self.world.block_names.insert(pos, name),
                None => self.world.block_names.remove(&pos),
            };
            self.world.set_block_entity_extras(pos, kept.extras);
            match kept.hopper {
                Some(hopper) => self.hoppers.insert(pos, hopper),
                None => self.hoppers.remove(&pos),
            };
        }
        // After the whole entities: a slot kept here was kept before any whole capture of its
        // entity at this depth, so its stack is the older one. Only into the block entity it
        // was taken from: one a write replaced (a `set_block` in the transaction) is left as is.
        for ((pos, slot), kept) in level.slots {
            if self.world.block_entities.generation(&pos) != kept.generation {
                continue;
            }
            if let Some(target) = self
                .world
                .block_entities
                .get_mut(&pos)
                .and_then(|entity| entity.slots_mut().get_mut(slot))
            {
                *target = kept.stack;
            }
        }
        let bottom_quart = self.world.bottom_y() >> 2;
        for ((qx, qy, qz), biome) in level.biomes {
            if let Some(chunk) = self.world.chunks.get_mut(&(qx >> 2, qz >> 2)) {
                std::sync::Arc::make_mut(chunk).set_biome_cell(
                    qx & 3,
                    qy - bottom_quart,
                    qz & 3,
                    biome,
                );
            }
        }
        for (pos, state) in level.blocks {
            self.world.set_stored(pos.0, pos.1, pos.2, state);
        }
        for (chunk, randoms) in level.composter_randoms {
            self.transactions
                .composter_randoms
                .entry(chunk)
                .or_default()
                .extend(randoms);
        }
        for (id, kept) in level.entities {
            self.edit_entity_slots(id, |slots, _| {
                for (slot, kept) in slots.iter_mut().zip(kept) {
                    *slot = kept;
                }
            });
        }
        for (id, len) in level.mob_lengths {
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                m.extra.items.truncate(len);
            }
        }
        for (id, kept) in level.item_entities {
            if let Some(e) = self.dropped.iter_mut().find(|e| e.id == id) {
                e.stack = kept;
            }
        }
        if let Some(host) = self.lan.as_mut() {
            host.hub.guests.update_all(|g| {
                if let Some(kept) = level.menus.get(&g.conn).filter(|_| g.placed) {
                    g.ender_items.clone_from(&kept.ender_items);
                    g.craft_grid.clone_from(&kept.craft_grid);
                    g.native_menu.clone_from(&kept.native_menu);
                }
            });
        }
        for (holder, kept) in level.attachments {
            if let Ok(map) = self.attachment_slot(holder, mod_api::AttachmentAccess::Existing) {
                map.replace_contents(kept.contents);
                if !kept.indexed {
                    map.unbind_index();
                }
            }
            if !kept.indexed
                && let Holder::Entity(id) = holder
                && let Some(id) = std::num::NonZeroU32::new(id)
            {
                self.with_resident_slot(&self.dim.key(), id, |slot| {
                    *slot = Default::default();
                });
            }
        }
        // After the maps: putting a chunk's map back marks it again.
        for (chunk, mark) in level.chunk_marks {
            self.world.modified.restore(chunk, mark);
        }
        for (id, kept) in level.mob_health {
            if let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) {
                mob.health = kept.health;
                mob.absorption = kept.absorption;
                mob.needs_sync = kept.needs_sync;
                mob.death_time = kept.death_time;
                mob.death_canceled = kept.death_canceled;
                if kept.processed {
                    self.died.insert(id);
                } else {
                    self.died.remove(&id);
                }
            }
        }
        for (uuid, kept) in level.player_health {
            // Restoration must also reach a body tentatively set to zero health:
            // native_as_player deliberately refuses dead bodies for public writes.
            let target = if uuid == self.native_acting_id() {
                Some(super::commands::Target::Player)
            } else {
                self.lan
                    .as_ref()
                    .and_then(|host| host.hub.holder_guest(uuid, self.dim.id()))
                    .map(|guest| super::commands::Target::Guest(guest.conn))
            };
            if let Some(target) = target {
                self.for_player(target, |g| {
                    if let Some(player) = g.player.as_mut() {
                        player.health.data = kept.data;
                        player.events.truncate(kept.events_len);
                        player.health.dead = kept.dead;
                    }
                    if let Some((conn, canceled, time)) = kept.guest
                        && let Some(host) = g.lan.as_mut()
                        && let Some(gi) =
                            host.hub.guests.iter().position(|guest| guest.conn == conn)
                    {
                        host.hub.guests.update(gi, |guest| {
                            guest.death_canceled = canceled;
                            guest.death_time = time;
                        });
                    }
                });
            }
        }
        level.actions.rollback_in(self);
        for (uuid, kept) in level.players {
            self.native_as_player(uuid, |g| {
                g.ender_items = kept.ender_items;
                g.level.player_extra = kept.player_extra;
                g.inventory = kept.inventory;
                g.armor = kept.armor;
                g.offhand = kept.offhand;
                g.equipment = kept.equipment;
                g.carried = kept.carried;
            });
        }
    }
}

include!("transactions/item_entity.rs");

#[cfg(test)]
#[path = "transactions/player_tests.rs"]
mod player_tests;

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::super::{Audio, BlockEntity, Game, Options, Progress, Settings, load_resources};
    use horizon_client::{chunk::Chunk, items::ItemStack};
    use mod_api::HostTransaction;
    use mod_api::ServerHost;
    use mod_api::{
        Authority, BlockSide, BlockUpdateFlags, FLUIDS, FluidStack, ITEMS, ItemResource,
        Transaction,
    };
    use std::{sync::Arc, time::Instant};

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn cauldron_stat_listener_observes_committed_state_and_can_cancel() {
        use super::super::native_entity_refs::tests::run_fresh_cases;
        use crate::game::api_values::TryIntoApi;
        use mod_api::{
            Flow, Hooks, ItemSpawnEvent, ItemTossEvent, ModEntry, Mods, PlayerStatAwardEvent,
        };
        use std::sync::Mutex;
        if run_fresh_cases(
            "game::transactions::tests::cauldron_stat_listener_observes_committed_state_and_can_cancel",
            "HORIZON_CAULDRON_STAT_CHILD",
            &["run"],
        ) {
            return;
        }
        struct Listener {
            seen: Arc<Mutex<Vec<(Option<mod_api::BlockState>, Option<String>)>>>,
            drops: Arc<Mutex<Vec<(&'static str, Option<mod_api::BlockState>)>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn player_stat_award(
                &self,
                host: &mut dyn ServerHost,
                event: &mut PlayerStatAwardEvent<'_>,
            ) -> Flow {
                if event.stat.as_str() != "minecraft:fill_cauldron" {
                    return Flow::Continue;
                }
                let level = host.level("minecraft:overworld").unwrap();
                let hand = mod_api::ItemPlace::PlayerHand {
                    player: event.player,
                    hand: mod_api::client::first_person::Hand::Main,
                };
                self.seen.lock().unwrap().push((
                    level.block_state(host, [2, 64, 2]),
                    hand.stack(host).map(|s| {
                        mod_api::__private::item_key_id(s.item())
                            .unwrap()
                            .to_string()
                    }),
                ));
                Flow::Cancel
            }
            fn item_toss(&self, host: &mut dyn ServerHost, event: &mut ItemTossEvent) -> Flow {
                let potion = event.item.stack(host).is_some_and(|stack| {
                    mod_api::__private::item_key_id(stack.item())
                        .is_some_and(|id| id == "minecraft:potion")
                });
                if potion {
                    let level = host.level("minecraft:overworld").unwrap();
                    self.drops
                        .lock()
                        .unwrap()
                        .push(("toss", level.block_state(host, [2, 64, 2])));
                }
                Flow::Continue
            }
            fn item_spawn(&self, host: &mut dyn ServerHost, event: &mut ItemSpawnEvent<'_>) {
                if event.item == "minecraft:potion" {
                    let level = host.level("minecraft:overworld").unwrap();
                    self.drops
                        .lock()
                        .unwrap()
                        .push(("spawn", level.block_state(host, [2, 64, 2])));
                }
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let drops = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "observer",
            Box::new(Listener {
                seen: seen.clone(),
                drops: drops.clone(),
            }),
        )]
        .into();
        mod_platform::__private::install(mods);
        super::super::native_guest_turn_tests::with_guests(|game| {
            let pos = [2, 64, 2];
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let empty = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cauldron")
                .unwrap();
            let full = game
                .pack
                .assets
                .registry
                .resolve(&horizon_world::chunk::BlockState {
                    name: "minecraft:water_cauldron".into(),
                    properties: [("level".into(), "3".into())].into_iter().collect(),
                })
                .unwrap();
            game.world.set(pos[0], pos[1], pos[2], empty).unwrap();
            game.as_guest(12, |game| {
                game.selected = 0;
                game.creative = false;
                game.inventory[0] = Some(ItemStack::new("minecraft:water_bucket", 1));
                game.open_storage_transaction();
                assert!(game.use_on_cauldron(pos).unwrap());
                assert!(seen.lock().unwrap().is_empty());
                game.close_storage_transaction(false);
                assert!(seen.lock().unwrap().is_empty());
                assert!(game.use_on_cauldron(pos).unwrap());
                assert_eq!(
                    seen.lock().unwrap().as_slice(),
                    &[(
                        Some(empty.try_into_api().unwrap()),
                        Some("minecraft:bucket".into())
                    )]
                );
                game.world.set(pos[0], pos[1], pos[2], empty).unwrap();
                game.inventory[0] = Some(ItemStack::new("minecraft:water_bucket", 1));
                game.open_storage_transaction();
                assert!(game.use_on_cauldron(pos).unwrap());
                assert_eq!(seen.lock().unwrap().len(), 1);
                game.close_storage_transaction(true);
            })
            .unwrap();
            assert_eq!(
                *seen.lock().unwrap(),
                vec![
                    (
                        Some(empty.try_into_api().unwrap()),
                        Some("minecraft:bucket".into())
                    ),
                    (
                        Some(full.try_into_api().unwrap()),
                        Some("minecraft:bucket".into())
                    )
                ]
            );
            let level2 = game
                .pack
                .assets
                .registry
                .resolve(&horizon_world::chunk::BlockState {
                    name: "minecraft:water_cauldron".into(),
                    properties: [("level".into(), "2".into())].into_iter().collect(),
                })
                .unwrap();
            game.as_guest(12, |game| {
                let setup = |game: &mut Game<'_>| {
                    game.world.set(pos[0], pos[1], pos[2], full).unwrap();
                    game.inventory
                        .iter_mut()
                        .for_each(|s| *s = Some(ItemStack::new("minecraft:stone", 64)));
                    game.offhand = Some(ItemStack::new("minecraft:stone", 64));
                    game.inventory[0] = Some(ItemStack::new("minecraft:glass_bottle", 2));
                };
                setup(game);
                assert!(game.use_on_cauldron(pos).unwrap());
                assert_eq!(
                    drops.lock().unwrap().as_slice(),
                    &[
                        ("toss", Some(full.try_into_api().unwrap())),
                        ("spawn", Some(full.try_into_api().unwrap()))
                    ]
                );
                drops.lock().unwrap().clear();
                setup(game);
                game.open_storage_transaction();
                assert!(game.use_on_cauldron(pos).unwrap());
                assert!(drops.lock().unwrap().is_empty());
                game.close_storage_transaction(false);
                assert!(drops.lock().unwrap().is_empty());
                game.open_storage_transaction();
                assert!(game.use_on_cauldron(pos).unwrap());
                game.close_storage_transaction(true);
                assert_eq!(
                    drops.lock().unwrap().as_slice(),
                    &[
                        ("toss", Some(level2.try_into_api().unwrap())),
                        ("spawn", Some(level2.try_into_api().unwrap()))
                    ]
                );
            })
            .unwrap();
            assert_eq!(
                game.lan.as_ref().unwrap().play[&12]
                    .stats
                    .get("custom", "minecraft:fill_cauldron"),
                0
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn cauldron_effects_wait_for_root_and_abort_with_inventory() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let pos = [2, 64, 2];
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let empty = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cauldron")
                .unwrap();
            game.world.set(pos[0], pos[1], pos[2], empty).unwrap();
            game.player = Some(
                super::super::PlayerMotion::new(
                    game.motion,
                    horizon_core::geometry::Vec3::new(0.5, 65.0, 3.5),
                )
                .unwrap(),
            );
            game.selected = 0;
            game.creative = false;
            game.inventory[0] = Some(ItemStack::new("minecraft:water_bucket", 1));
            let before = game.stats.get("custom", "minecraft:fill_cauldron");
            game.audio.capture = Some(Vec::new());
            game.open_storage_transaction();
            assert!(game.use_on_cauldron(pos).unwrap());
            assert_eq!(game.stats.get("custom", "minecraft:fill_cauldron"), before);
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            game.close_storage_transaction(false);
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), empty);
            assert_eq!(
                &*game.inventory[0].as_ref().unwrap().item,
                "minecraft:water_bucket"
            );
            assert_eq!(game.stats.get("custom", "minecraft:fill_cauldron"), before);
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            game.open_storage_transaction();
            game.open_storage_transaction();
            assert!(game.use_on_cauldron(pos).unwrap());
            game.close_storage_transaction(false);
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), empty);
            assert_eq!(
                &*game.inventory[0].as_ref().unwrap().item,
                "minecraft:water_bucket"
            );
            game.close_storage_transaction(true);
            assert_eq!(game.stats.get("custom", "minecraft:fill_cauldron"), before);
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            assert!(game.use_on_cauldron(pos).unwrap());
            assert_ne!(game.world.block(pos[0], pos[1], pos[2]), empty);
            assert_eq!(
                &*game.inventory[0].as_ref().unwrap().item,
                "minecraft:bucket"
            );
            assert_eq!(
                game.stats.get("custom", "minecraft:fill_cauldron"),
                before + 1
            );
            assert_eq!(
                game.audio
                    .capture
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|s| s.event.as_str())
                    .collect::<Vec<_>>(),
                ["minecraft:item.bucket.empty"]
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_menu_write_outside_a_transaction_keeps_no_journal() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let player = game.player(12).unwrap();
            let menu = game.player_container_menu(player).unwrap();
            let stone = ItemStack::new("minecraft:stone", 4);
            game.journal_menus(player);
            *game.native_menu_storage(menu, 1).unwrap() = Some(stone.clone());
            assert!(game.transactions.is_empty());
            assert_eq!(
                menu.slot(game, 1),
                Some(Some(stone.try_into_api().unwrap()))
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn menu_journals_keep_only_the_first_write_inside_a_transaction() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let player = game.player(12).unwrap();
            let menu = game.player_container_menu(player).unwrap();
            let stone = ItemStack::new("minecraft:stone", 4);
            *game.native_menu_storage(menu, 1).unwrap() = Some(stone.clone());
            let place = mod_api::ItemPlace::MenuSlot { menu, slot: 1 };
            let resource = ItemResource::of(&stone.try_into_api().unwrap());
            let aborted: Result<(), ()> = game.transaction(|game, tx| {
                assert_eq!(place.insert(game, tx, &resource, 2), 2);
                assert_eq!(place.insert(game, tx, &resource, 3), 3);
                let journal = &game.transactions.last().unwrap().menus;
                assert_eq!(journal.len(), 1);
                assert_eq!(journal[&12].craft_grid[0], Some(stone.clone()));
                assert_eq!(menu.slot(game, 1).unwrap().unwrap().count(), 9);
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert!(game.transactions.is_empty());
            assert_eq!(
                menu.slot(game, 1),
                Some(Some(stone.try_into_api().unwrap()))
            );
        });
    }

    fn insert(
        host: &mut dyn ServerHost,
        at: BlockSide,
        slot: usize,
        stack: ItemStack,
        tx: &Transaction,
    ) -> Option<ItemStack> {
        let handler = at
            .level
            .capability(host, &ITEMS, at.pos, at.side)
            .expect("item capability");
        let inserted = handler.insert(
            host,
            tx,
            slot,
            &ItemResource::of(&stack.try_into_api().unwrap()),
            stack.count,
        );
        (inserted < stack.count).then(|| ItemStack {
            count: stack.count - inserted,
            ..stack
        })
    }

    fn extract(
        host: &mut dyn ServerHost,
        at: BlockSide,
        slot: usize,
        amount: u32,
        tx: &Transaction,
    ) -> Option<mod_api::ItemStack> {
        let handler = at
            .level
            .capability(host, &ITEMS, at.pos, at.side)
            .expect("item capability");
        let resource = handler.resource(host, slot)?;
        let taken = handler.extract(host, tx, slot, &resource, amount);
        resource.to_stack(taken)
    }

    fn extract_fluid(
        host: &mut dyn ServerHost,
        at: BlockSide,
        tank: usize,
        amount: u32,
        tx: &Transaction,
    ) -> Option<FluidStack> {
        let handler = at
            .level
            .capability(host, &FLUIDS, at.pos, at.side)
            .expect("fluid capability");
        let resource = handler.resource(host, tank)?;
        let taken = handler.extract(host, tx, tank, &resource, amount);
        (taken > 0).then(|| resource.to_stack(taken))
    }

    fn chest_items(game: &Game<'_>) -> Vec<(usize, u32)> {
        let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&(0, 64, 0)) else {
            panic!("chest block entity missing")
        };
        let held = items.iter().enumerate();
        held.filter_map(|(i, s)| Some((i, s.as_ref()?.count)))
            .collect()
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn container_limits_preserve_capacity_and_transaction_rollback() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            for (pos, id) in [
                ([0, 64, 0], "minecraft:chest"),
                ([4, 64, 0], "minecraft:jukebox"),
            ] {
                let state = game.pack.assets.registry.default_state(id).unwrap();
                game.level_set_block(level, pos, state, BlockUpdateFlags::NEIGHBORS)
                    .unwrap();
            }
            let mut large = ItemStack::new("minecraft:stick", 1);
            large.data_mut().set_other(
                "minecraft:max_stack_size",
                Some(horizon_core::nbt::Tag::Int(99)),
            );
            let large = ItemResource::of(&large.try_into_api().unwrap());
            let handler = level.capability(game, &ITEMS, [0, 64, 0], None).unwrap();
            assert_eq!(handler.capacity(game, 0, &large), 99);
            assert_eq!(game.world.container_stack_limit((0, 64, 0)), None);
            let stick =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let host: &mut dyn ServerHost = game;
            let handler = level
                .capability(host, &mod_api::ITEMS, [0, 64, 0], None)
                .unwrap();
            let aborted = host.transaction(|host, tx| {
                assert!(handler.set_max_stack_size(host, tx, 10));
                assert_eq!(handler.insert(host, tx, 0, &stick, 64), 10);
                host.transaction(|host, tx| {
                    assert!(handler.set_max_stack_size(host, tx, 4));
                    Ok::<_, ()>(())
                })?;
                assert_eq!(handler.capacity(host, 0, &stick), 4);
                Err::<(), ()>(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!(handler.capacity(host, 0, &stick), 64);
            assert_eq!(handler.capacity(host, 0, &large), 99);
            assert_eq!(handler.amount(host, 0), 0);
            host.transaction(|host, tx| {
                assert!(handler.set_max_stack_size(host, tx, 10));
                let _ = host.transaction(|host, tx| {
                    assert!(handler.set_max_stack_size(host, tx, 4));
                    Err::<(), ()>(())
                });
                assert_eq!(handler.capacity(host, 0, &stick), 10);
                for invalid in [0, 100, u32::MAX] {
                    assert!(!handler.set_max_stack_size(host, tx, invalid));
                    assert_eq!(handler.capacity(host, 0, &stick), 10);
                }
                assert!(handler.set_max_stack_size(host, tx, 64));
                assert_eq!(handler.insert(host, tx, 0, &stick, 20), 20);
                assert!(handler.set_max_stack_size(host, tx, 10));
                assert_eq!(handler.amount(host, 0), 20);
                assert_eq!(handler.insert(host, tx, 0, &stick, 1), 0);
                assert_eq!(handler.extract(host, tx, 0, &stick, 15), 15);
                assert_eq!(handler.amount(host, 0), 5);
                assert_eq!(handler.insert(host, tx, 0, &stick, 64), 5);
                Err::<(), ()>(())
            })
            .unwrap_err();

            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _: Result<(), ()> = host.transaction(|host, tx| {
                    assert!(handler.set_max_stack_size(host, tx, 10));
                    assert_eq!(handler.insert(host, tx, 0, &stick, 64), 10);
                    panic!("abort container writes");
                });
            }));
            assert!(panic.is_err());
            assert_eq!(handler.capacity(host, 0, &stick), 64);
            assert_eq!(handler.capacity(host, 0, &large), 99);
            assert_eq!(handler.amount(host, 0), 0);
            host.transaction(|host, tx| {
                let pearl = ItemResource::of(
                    &ItemStack::new("minecraft:ender_pearl", 1)
                        .try_into_api()
                        .unwrap(),
                );
                assert!(handler.set_max_stack_size(host, tx, 32));
                assert_eq!(handler.insert(host, tx, 0, &pearl, 64), 16);
                let disc = level.capability(host, &ITEMS, [4, 64, 0], None).unwrap();
                assert!(disc.set_max_stack_size(host, tx, 10));
                let record = ItemResource::of(
                    &ItemStack::new("minecraft:music_disc_13", 1)
                        .try_into_api()
                        .unwrap(),
                );
                assert_eq!(disc.insert(host, tx, 0, &record, 64), 1);
                Err::<(), ()>(())
            })
            .unwrap_err();
            // Player clicks and shift-clicks use the menu's separate slot rules.
            let env = game.menu_env();
            let mut menu = horizon_client::menu::Menu::new(horizon_client::menu::Kind::Chest(1));
            menu.set_container_limits(vec![Some(10); 9]);
            menu.carried = Some(ItemStack::new("minecraft:stick", 20));
            menu.click(&env, 0, 0, horizon_client::menu::Input::Pickup)
                .unwrap();
            assert_eq!(menu.slot(0).unwrap().count, 10);
            assert_eq!(menu.carried.as_ref().unwrap().count, 10);
            menu.carried = None;
            menu.set_slot(&env, 9, Some(ItemStack::new("minecraft:stick", 20)));
            menu.click(&env, 9, 0, horizon_client::menu::Input::QuickMove)
                .unwrap();
            assert_eq!(menu.slot(1).unwrap().count, 10);
            assert_eq!(menu.slot(2).unwrap().count, 10);
            let hopper = horizon_world::chunk::BlockState {
                name: "minecraft:hopper".into(),
                properties: [("facing".into(), "east".into())].into_iter().collect(),
            };
            let hopper = game.pack.assets.registry.resolve(&hopper).unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            let flags = BlockUpdateFlags::NEIGHBORS;
            game.level_set_block(level, [0, 64, 4], hopper, flags)
                .unwrap();
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(0, 64, 4))
            {
                items[0] = Some(ItemStack::new("minecraft:stick", 2));
            }
            let sticks = |count| ItemStack::new("minecraft:stick", count);
            let slots = |game: &Game<'_>, pos| match game.world.block_entities.get(&pos) {
                Some(BlockEntity::Chest(items)) => {
                    items.iter().flatten().map(|s| s.count).sum::<u32>()
                }
                _ => panic!("missing chest"),
            };
            assert_eq!(
                game.level_set_block(level, [1, 64, 4], chest, flags),
                Ok(true)
            );
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(1, 64, 4))
            {
                items.fill(Some(sticks(10)));
                items[0] = Some(sticks(9));
            }
            let target = level.capability(game, &ITEMS, [1, 64, 4], None).unwrap();
            game.transaction(|host, tx| {
                assert!(target.set_max_stack_size(host, tx, 10));
                Ok::<_, ()>(())
            })
            .unwrap();
            game.hoppers.remove(&(0, 64, 4));
            game.hopper_try_move_items((0, 64, 4));
            assert_eq!(slots(game, (1, 64, 4)), 270);
            let _ = game.transaction(|host, tx| {
                assert!(target.set_max_stack_size(host, tx, 64));
                Err::<(), ()>(())
            });
            game.hoppers.remove(&(0, 64, 4));
            game.hopper_try_move_items((0, 64, 4));
            assert_eq!(slots(game, (1, 64, 4)), 270);
            assert_eq!(slots(game, (0, 64, 4)), 1);
            // Dropper uses add_item, dispenser ejects into the world instead.
            let dropper = horizon_world::chunk::BlockState {
                name: "minecraft:dropper".into(),
                properties: [("facing".into(), "east".into())].into_iter().collect(),
            };
            let dropper = game.pack.assets.registry.resolve(&dropper).unwrap();
            game.level_set_block(level, [0, 64, 4], dropper, flags)
                .unwrap();
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(0, 64, 4))
            {
                items[0] = Some(sticks(2));
            }
            game.dispense_from(horizon_core::position::BlockPos::new(0, 64, 4), dropper)
                .unwrap();
            assert_eq!(slots(game, (1, 64, 4)), 270);
            assert_eq!(slots(game, (0, 64, 4)), 2);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_aborted_transaction_puts_back_what_it_and_its_committed_children_changed() {
        let Some(root) = super::super::native_entity_refs::tests::declared_part_root(
            "game::transactions::tests::an_aborted_transaction_puts_back_what_it_and_its_committed_children_changed",
        ) else {
            return;
        };
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            super::super::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = super::super::execution_role::ExecutionRole::IntegratedServer;
        options.dedicated = Some(Vec::new());
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(Authority::IntegratedServer);
        super::super::native_entity_refs::tests::resolve_parts(&game).unwrap();
        // Pins the opaque backend: a real game always hands one out, so a None
        // from Host::backend is a bug, never a fallback.
        assert!(mod_api::Host::backend(&game).is_some());
        assert!(ServerHost::backend_mut(&mut game).is_some());
        let level = game.level("minecraft:overworld").unwrap();
        let registry = &game.pack.assets.registry;
        let chest = registry.default_state("minecraft:chest").unwrap();
        let full = horizon_world::chunk::BlockState {
            name: "minecraft:water_cauldron".to_owned(),
            properties: [("level".to_owned(), "3".to_owned())].into_iter().collect(),
        };
        let full = registry.resolve(&full).unwrap();
        let empty = registry.default_state("minecraft:cauldron").unwrap();
        let jukebox = registry.default_state("minecraft:jukebox").unwrap();
        let flags = BlockUpdateFlags::NEIGHBORS;
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], full, flags),
            Ok(true)
        );
        assert_eq!(
            game.level_set_block(level, [4, 64, 0], jukebox, flags),
            Ok(true)
        );
        let sticks = |count| ItemStack::new("minecraft:stick", count);
        let stick = ItemResource::of(&sticks(1).try_into_api().unwrap());
        let rod = ItemResource::of(
            &ItemStack::new("minecraft:blaze_rod", 1)
                .try_into_api()
                .unwrap(),
        );
        let on = |pos, side| BlockSide { level, pos, side };
        let host: &mut dyn ServerHost = &mut game;

        // Five in, then a committed child takes two and an aborted one adds three: the
        // outer abort puts all of it back, the cauldron's water too.
        let refused: Result<(), ()> = host.transaction(|host, tx| {
            assert!(insert(host, on([0, 64, 0], None), 0, sticks(5), tx).is_none());
            let taken = host
                .transaction(|host, tx| Ok::<_, ()>(extract(host, on([0, 64, 0], None), 0, 2, tx)));
            assert_eq!(taken, Ok(Some(sticks(2).try_into_api().unwrap())));
            let _ = host.transaction(|host, tx| {
                insert(host, on([0, 64, 0], None), 1, sticks(3), tx);
                Err::<(), ()>(())
            });
            let water = extract_fluid(host, on([2, 64, 0], None), 0, 1000, tx);
            assert_eq!(water.map(|w| w.amount), Some(1000));
            Err(())
        });
        assert_eq!(refused, Err(()));
        assert_eq!(chest_items(&game), []);
        assert_eq!(game.world.block(2, 64, 0), full);
        assert!(game.transactions.is_empty());

        // The same, committed: what the children left stands, the cauldron is set for good.
        let host: &mut dyn ServerHost = &mut game;
        let done: Result<(), ()> = host.transaction(|host, tx| {
            insert(host, on([0, 64, 0], None), 0, sticks(5), tx);
            let _ = host.transaction(|host, tx| {
                extract(host, on([0, 64, 0], None), 0, 2, tx);
                Ok::<_, ()>(())
            });
            let _ = host.transaction(|host, tx| {
                insert(host, on([0, 64, 0], None), 1, sticks(3), tx);
                Err::<(), ()>(())
            });
            extract_fluid(host, on([2, 64, 0], None), 0, 1000, tx);
            Ok(())
        });
        assert_eq!(done, Ok(()));
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert_eq!(game.world.block(2, 64, 0), empty);

        // A transaction a mod opened through the helpers and never closed is aborted before
        // the next tick.
        game.open_storage_transaction();
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            Ok::<_, ()>(insert(host, on([0, 64, 0], None), 0, sticks(1), tx))
        });
        game.abort_open_transactions();
        assert_eq!(chest_items(&game), [(0, 3)]);

        // A panic caught outside the root: what it changed is gone before the catch returns,
        // and the transaction is closed.
        let host: &mut dyn ServerHost = &mut game;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            host.transaction(|host, tx| -> Result<(), ()> {
                insert(host, on([0, 64, 0], None), 0, sticks(1), tx);
                panic!("a broken handler")
            })
        }));
        assert!(caught.is_err());
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert!(game.transactions.is_empty());

        // A transaction decoded from bytes, which the game never handed out, names no open
        // transaction (#2424): a write given it changes nothing, and a close given it ends
        // nothing, so the game's own transaction still puts back everything at its abort.
        let forged: mod_api::Transaction =
            mod_api::wire::Reader::decode(&0u64.to_le_bytes()).unwrap();
        let host: &mut dyn ServerHost = &mut game;
        assert!(insert(host, on([0, 64, 0], None), 0, sticks(1), &forged).is_some());
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert_eq!(chest_items(&game), [(0, 3)]);

        // A capability handler is lent the game's own transaction (a hopper's move,
        // `put_into_part`): it writes in it and tries every seal it can build from what it
        // knows, the write token and guesses, then the move fails after it. Its writes all go
        // back with the game's abort, which no close of the handler's got ahead of.
        let handler = |host: &mut dyn ServerHost, tx: &Transaction| {
            assert!(insert(host, on([0, 64, 0], None), 0, sticks(1), tx).is_none());
            let token = mod_api::__private::transaction_token(tx);
            for close in [0, 1, token, !token, u64::MAX] {
                let mut bytes = token.to_le_bytes().to_vec();
                bytes.extend(close.to_le_bytes());
                let seal: mod_api::TransactionSeal = mod_api::wire::Reader::decode(&bytes).unwrap();
                host.transaction_close(&seal, true);
            }
            assert!(insert(host, on([0, 64, 0], None), 0, sticks(1), tx).is_none());
        };
        let host: &mut dyn ServerHost = &mut game;
        let failed: Result<(), ()> = mod_api::run_transaction(host, |host, tx| {
            handler(host, tx);
            Err(())
        });
        assert_eq!(failed, Err(()));
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert!(game.transactions.is_empty());

        // A handler that opens a transaction of its own, writes in it and leaves it open does
        // not strand the game's: the failed move aborts both at once, not at the next tick.
        let host: &mut dyn ServerHost = &mut game;
        let failed: Result<(), ()> = mod_api::run_transaction(host, |host, _| {
            let left_open = host.transaction_open();
            let inner = left_open.transaction();
            assert!(insert(host, on([0, 64, 0], None), 0, sticks(1), inner).is_none());
            Err(())
        });
        assert_eq!(failed, Err(()));
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert!(
            game.transactions.is_empty(),
            "both closed before the next tick"
        );

        // An attachment map a write made inside an aborted transaction is gone with it.
        let chest_holder = mod_api::Holder::BlockEntity([0, 64, 0]);
        // So is the unsaved mark it put on a clean chunk, through a committed child too.
        assert!(game.world.block_entities.attachments((0, 64, 0)).is_none());
        game.world.modified.clear();
        game.open_storage_transaction();
        game.open_storage_transaction();
        assert!(
            game.attachment_map_quiet(chest_holder, mod_api::AttachmentAccess::Existing)
                .is_ok()
        );
        assert!(game.world.modified.contains(&(0, 0)));
        game.close_storage_transaction(true);
        assert!(game.world.block_entities.attachments((0, 64, 0)).is_some());
        game.close_storage_transaction(false);
        assert!(game.world.block_entities.attachments((0, 64, 0)).is_none());
        assert!(!game.world.modified.contains(&(0, 0)), "clean again");
        // A committed one keeps it.
        game.open_storage_transaction();
        assert!(
            game.attachment_map_quiet(chest_holder, mod_api::AttachmentAccess::Existing)
                .is_ok()
        );
        game.close_storage_transaction(true);
        assert!(game.world.modified.contains(&(0, 0)));
        game.world.block_entities.set_attachments((0, 64, 0), None);

        // An aborted cauldron change leaves no write behind in the world's queues.
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], full, flags),
            Ok(true)
        );
        game.world.block_log = Some(Vec::new());
        game.world.path_writes = Some(Vec::new());
        let queues = |game: &Game<'_>| {
            let world = &game.world;
            (
                world.block_log.as_ref().map(Vec::len),
                world.path_writes.as_ref().map(Vec::len),
                world.light_edits.len(),
                world.dirty_light.len(),
                world.watched_writes.len(),
            )
        };
        let before = queues(&game);
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            extract_fluid(host, on([2, 64, 0], None), 0, 1000, tx);
            Err::<(), ()>(())
        });
        assert_eq!(game.world.block(2, 64, 0), full);
        assert_eq!(queues(&game), before);

        // A disc swapped for another in one transaction changes the song: the old one's
        // stops and the new one's starts.
        let disc = |item: &str| ItemStack::new(item, 1);
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            let cat = disc("minecraft:music_disc_cat");
            Ok::<_, ()>(insert(host, on([4, 64, 0], None), 0, cat, tx))
        });
        let Some(BlockEntity::Jukebox(j)) = game.world.block_entities.get_mut(&(4, 64, 0)) else {
            panic!("jukebox block entity missing")
        };
        assert!(j.playing, "a disc put in through a transfer plays");
        j.ticks = 500;
        let host: &mut dyn ServerHost = &mut game;
        let swapped = host.transaction(|host, tx| {
            extract(host, on([4, 64, 0], None), 0, 1, tx);
            Ok::<_, ()>(insert(
                host,
                on([4, 64, 0], None),
                0,
                disc("minecraft:music_disc_13"),
                tx,
            ))
        });
        assert_eq!(swapped, Ok(None));
        let Some(BlockEntity::Jukebox(j)) = game.world.block_entities.get(&(4, 64, 0)) else {
            panic!("jukebox block entity missing")
        };
        assert_eq!(
            j.record().map(|r| &*r.item),
            Some("minecraft:music_disc_13")
        );
        assert!(j.playing);
        assert_eq!(j.ticks, 0, "the new disc's song starts from its beginning");

        // A player's place: one of three buckets in the hand is exchanged for a filled one; the
        // hand keeps two and the filled one goes to the inventory, and what doesn't fit would be
        // thrown. Aborted, the hand and the inventory are as they were, and nothing was thrown.
        game.player = Some(
            super::super::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::new(0.5, 65.0, 3.5),
            )
            .unwrap(),
        );
        let bucket = |item: &str, count| ItemStack::new(item, count);
        game.inventory
            .iter_mut()
            .for_each(|slot| *slot = Some(bucket("minecraft:stone", 64)));
        game.selected = 0;
        game.inventory[0] = Some(bucket("minecraft:bucket", 3));
        game.carried = None;
        let before = (
            game.inventory.clone(),
            game.offhand.clone(),
            game.carried.clone(),
        );
        let dropped = game.dropped.len();
        let me = game.native_acting_id();
        let hand = mod_api::ItemPlace::PlayerHand {
            player: me,
            hand: mod_api::client::first_person::Hand::Main,
        };
        let water = ItemResource::of(&bucket("minecraft:water_bucket", 1).try_into_api().unwrap());
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            assert_eq!(hand.exchange(host, tx, &water, 1), 1);
            Err::<(), ()>(())
        });
        assert_eq!(
            (
                game.inventory.clone(),
                game.offhand.clone(),
                game.carried.clone()
            ),
            before
        );
        assert_eq!(
            game.dropped.len(),
            dropped,
            "a thrown bucket waits for the root commit"
        );

        // Committed, the split stands and the full inventory throws the filled bucket.
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| Ok::<_, ()>(hand.exchange(host, tx, &water, 1)));
        assert_eq!(game.inventory[0], Some(bucket("minecraft:bucket", 2)));
        assert_eq!(game.dropped.len(), dropped + 1);
        let thrown = &game.dropped[game.dropped.len() - 1].stack;
        assert_eq!(&*thrown.item, "minecraft:water_bucket");

        // A chest minecart's, a chest boat's and a chested donkey's slots: an abort puts back
        // what an insert changed; a commit keeps an exchange, whose new item goes into the next
        // free slot.
        let mut chested = horizon_core::nbt::Compound::new();
        chested.insert("ChestedHorse".into(), horizon_core::nbt::Tag::Byte(1));
        let vehicles = [
            (
                "minecraft:chest_minecart",
                horizon_core::nbt::Compound::new(),
            ),
            (
                "minecraft:oak_chest_boat",
                horizon_core::nbt::Compound::new(),
            ),
            ("minecraft:donkey", chested),
        ];
        // An aborted write leaves a chested donkey's item list as long as it was.
        let id = game
            .summon(
                "minecraft:overworld",
                "minecraft:donkey",
                [8.5, 70.0, 8.5],
                {
                    let mut chested = horizon_core::nbt::Compound::new();
                    chested.insert("ChestedHorse".into(), horizon_core::nbt::Tag::Byte(1));
                    chested
                },
            )
            .unwrap();
        let len = |game: &Game<'_>| {
            game.mobs
                .iter()
                .find(|m| m.id == id)
                .unwrap()
                .extra
                .items
                .len()
        };
        // Its list is short (none kept yet): reading it as slots grows it, and the abort puts
        // that back too.
        let mob = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
        mob.extra.items.clear();
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            let place = mod_api::ItemPlace::EntitySlot {
                entity: id,
                slot: 0,
            };
            assert!(place.stack(host).is_none());
            assert_eq!(place.insert(host, tx, &stick, 1), 1);
            Err::<(), ()>(())
        });
        assert_eq!(len(&game), 0);
        // Grown three transactions deep, the middle one committing: the root's abort still
        // truncates it.
        let place = mod_api::ItemPlace::EntitySlot {
            entity: id,
            slot: 0,
        };
        let host: &mut dyn ServerHost = &mut game;
        let result = host.transaction(|host, _| {
            host.transaction(|host, _| {
                host.transaction(|host, tx| {
                    assert!(place.stack(host).is_none());
                    assert_eq!(place.insert(host, tx, &stick, 1), 1);
                    Ok::<_, ()>(())
                })?;
                Ok::<_, ()>(())
            })?;
            Err::<(), ()>(())
        });
        assert!(result.is_err());
        assert_eq!(len(&game), 0);

        for (kind, nbt) in vehicles {
            let id = game
                .summon("minecraft:overworld", kind, [8.5, 70.0, 8.5], nbt)
                .unwrap_or_else(|| panic!("{kind} not summoned"));
            let place = |slot| mod_api::ItemPlace::EntitySlot { entity: id, slot };
            let host: &mut dyn ServerHost = &mut game;
            let went =
                host.transaction(|host, tx| Ok::<_, ()>(place(0).insert(host, tx, &stick, 5)));
            assert_eq!(went, Ok(5), "{kind}");
            let _ = host.transaction(|host, tx| {
                assert_eq!(place(0).insert(host, tx, &stick, 4), 4, "{kind}");
                assert_eq!(place(1).insert(host, tx, &stick, 2), 2, "{kind}");
                Err::<(), ()>(())
            });
            let host: &mut dyn ServerHost = &mut game;
            assert_eq!(
                place(0).stack(host),
                Some(sticks(5).try_into_api().unwrap()),
                "{kind}"
            );
            assert_eq!(place(1).stack(host), None, "{kind}");
            let _ = host.transaction(|host, tx| Ok::<_, ()>(place(0).exchange(host, tx, &rod, 1)));
            let host: &mut dyn ServerHost = &mut game;
            assert_eq!(
                place(0).stack(host),
                Some(sticks(4).try_into_api().unwrap()),
                "{kind}"
            );
            let rod = place(1)
                .stack(host)
                .map(|s| mod_api::__private::item_key_id(s.item()).unwrap());
            assert_eq!(rod, Some("minecraft:blaze_rod"), "{kind}");
        }

        // An item entity: emptied in an aborted transaction, it's back whole; one of four
        // exchanged in a committed one is dropped beside it at the commit; emptied in a
        // committed one, it's gone.
        game.spawn_item(sticks(4), [0.5, 70.0, 6.5], [0.0; 3], 10);
        let id = game.dropped.last().unwrap().id;
        let entity = mod_api::ItemPlace::ItemEntity { id };
        let count = |game: &Game<'_>| game.dropped.len();
        let items = count(&game);
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            assert_eq!(entity.extract(host, tx, &stick, 4), 4);
            assert!(entity.stack(host).is_none());
            Err::<(), ()>(())
        });
        let host: &mut dyn ServerHost = &mut game;
        assert_eq!(entity.stack(host), Some(sticks(4).try_into_api().unwrap()));
        let _ = host.transaction(|host, tx| Ok::<_, ()>(entity.exchange(host, tx, &rod, 1)));
        let host: &mut dyn ServerHost = &mut game;
        assert_eq!(entity.stack(host), Some(sticks(3).try_into_api().unwrap()));
        assert_eq!(count(&game), items + 1);
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| Ok::<_, ()>(entity.extract(host, tx, &stick, 3)));
        assert!(game.dropped.iter().all(|e| e.id != id));
        assert_eq!(count(&game), items);

        // A hopper moving into a mod's part: a truthful part (a pipe into the chest east of
        // it) takes the item; a part answering more than it was offered has its move undone,
        // so the item stays in the hopper and the chest it wrote is as it was.
        let hopper = horizon_world::chunk::BlockState {
            name: "minecraft:hopper".to_owned(),
            properties: [("facing".to_owned(), "east".to_owned())]
                .into_iter()
                .collect(),
        };
        let hopper = game.pack.assets.registry.resolve(&hopper).unwrap();
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 4], hopper, flags),
            Ok(true)
        );
        assert_eq!(
            game.level_set_block(level, [2, 64, 4], chest, flags),
            Ok(true)
        );
        let slots = |game: &Game<'_>, pos| match game.world.block_entities.get(&pos) {
            Some(BlockEntity::Chest(items)) => items.iter().flatten().map(|s| s.count).sum::<u32>(),
            _ => panic!("no container at {pos:?}"),
        };
        for (id, moved) in [("test:pipe", 1), ("test:greedy", 0)] {
            let state = game.pack.assets.registry.default_state(id).unwrap();
            assert_eq!(
                game.level_set_block(level, [1, 64, 4], state, mod_api::BlockUpdateFlags::NONE),
                Ok(true)
            );
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(0, 64, 4))
            {
                items[0] = Some(sticks(2));
            }
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(2, 64, 4))
            {
                items.fill(None);
            }
            game.hoppers.remove(&(0, 64, 4));
            game.hopper_try_move_items((0, 64, 4));
            assert_eq!(slots(&game, (0, 64, 4)), 2 - moved, "{id}");
            assert_eq!(slots(&game, (2, 64, 4)), moved, "{id}");
            assert!(game.transactions.is_empty());
        }
    }
}

#[cfg(test)]
mod mod_capability_tests {
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::chunk::Chunk;
    use horizon_client::containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields};
    use horizon_core::nbt::{Compound, Tag};
    use mod_api::ServerHost;
    use mod_api::{BlockSide, HostTransaction, ResourceHandler, move_resources};
    use mod_platform::__private::part_tests::{CHEMICALS, ChemTank, Chemical};
    use std::sync::Arc;

    /// Two mods sharing a capability one of them defines, as Mekanism's chemicals are shared
    /// through its API: the first's tank and the second's vat each answer `CHEMICALS`, and a
    /// move between them in a committed transaction moves the chemical, kept in their saved
    /// data; in an aborted one, both are put back.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_chemical_moves_between_two_mods_parts_and_an_abort_puts_it_back() {
        use super::super::native_entity_refs::tests::{declared_part_root, with_game_at};
        let Some(root) = declared_part_root(
            "game::transactions::mod_capability_tests::a_chemical_moves_between_two_mods_parts_and_an_abort_puts_it_back",
        ) else {
            return;
        };
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let hydrogen = Chemical("test:hydrogen".into());
            let mut nbt = Compound::new();
            nbt.insert("chemical".into(), Tag::String("test:hydrogen".into()));
            nbt.insert("amount".into(), Tag::Int(600));
            for (x, id, nbt) in [
                (0, "test:chem_tank", nbt),
                (1, "test:chem_vat", Compound::new()),
            ] {
                let state = game.pack.assets.registry.default_state(id).unwrap();
                game.world.set(x, 64, 0, state).unwrap();
                let fields = ModBlockEntityFields {
                    nbt,
                    ..Default::default()
                };
                let entity = BlockEntity::Mod(Box::new(ModBlockEntity::new(id.into(), fields)));
                game.world.block_entities.insert((x, 64, 0), entity);
            }
            let host: &mut dyn ServerHost = game;
            let tank = level
                .capability(host, &CHEMICALS, [0, 64, 0], None)
                .unwrap();
            let vat = level
                .capability(host, &CHEMICALS, [1, 64, 0], None)
                .unwrap();
            // Each answered by its own mod.
            assert_ne!(tank.owner(), vat.owner());
            assert!(![tank.owner(), vat.owner()].contains(&"the game".to_owned()));
            let amounts = |host: &mut dyn ServerHost| {
                let side = |x| BlockSide {
                    level,
                    pos: [x, 64, 0],
                    side: None,
                };
                (
                    ChemTank(side(0)).amount(host, 0),
                    ChemTank(side(1)).amount(host, 0),
                )
            };

            let moved = host.transaction(|host, tx| {
                move_resources(host, &tank, &vat, |c| *c == hydrogen, 250, tx).map_err(drop)
            });
            assert_eq!(moved, Ok(250));
            assert_eq!(amounts(host), (350, 250));
            assert_eq!(vat.resource(host, 0), Some(hydrogen.clone()));

            let aborted = host.transaction(|host, tx| {
                let moved = move_resources(host, &tank, &vat, |_| true, 100, tx);
                assert_eq!(moved.ok(), Some(100));
                assert_eq!(amounts(host), (250, 350));
                Err::<(), ()>(())
            });
            assert!(aborted.is_err());
            assert_eq!(amounts(host), (350, 250));
            assert!(game.transactions.is_empty());
        });
    }

    /// A mod's attachments on an entity and on a player are kept by a transaction as a block
    /// entity's are: a commit keeps what it wrote, an abort puts each map back as it was before
    /// the transaction first reached it (none for an entity that had none), and a guest's is
    /// put back wherever their state lives at the abort, swapped in for their turn or not.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn entity_and_player_attachments_stay_on_commit_and_go_back_on_abort() {
        use horizon_client::entities::ArmorStand;
        use horizon_core::geometry::Vec3;
        use mod_api::{AttachmentType, Holder};
        const ENERGY: AttachmentType<i32> = AttachmentType {
            namespace: "robot",
            name: "energy",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(14, Vec3::ZERO, 0.0));
            let guest = 0x5eed_u128;
            let config = horizon_client::lan::hub::HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 10,
                simulation_distance: 10,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 20,
                integrated: false,
                levels: Default::default(),
            };
            game.lan_host(
                super::super::lan::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                horizon_client::lan::NetConfig {
                    announce: false,
                    ..horizon_client::lan::NetConfig::lan(0, "test".into(), 2)
                },
                config,
            )
            .unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let player_ref = game.fake_player(level, guest, "test").unwrap();
            let conn = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .find(|g| g.entity_id as u32 == player_ref.entity().id())
                .unwrap()
                .conn;
            let (stand, fresh, player) = (
                Holder::Entity(13),
                Holder::Entity(14),
                Holder::Player(guest),
            );
            let energy =
                |host: &mut dyn ServerHost, holder| ENERGY.get_existing(host, holder).unwrap();

            let host: &mut dyn ServerHost = game;
            let done = host.transaction(|host, _| {
                ENERGY.set(host, stand, 100)?;
                ENERGY.set(host, player, 40)
            });
            assert_eq!(done, Ok(()));
            assert_eq!(
                (energy(host, stand), energy(host, player)),
                (Some(100), Some(40))
            );

            // Aborted, with a committed child inside: both maps as they were, and the entity
            // that had no value has none.
            let aborted: Result<(), _> = host.transaction(|host, _| {
                ENERGY.set(host, stand, 250)?;
                ENERGY.set(host, player, 90)?;
                ENERGY.set(host, fresh, 7)?;
                host.transaction(|host, _| ENERGY.set(host, stand, 300))?;
                assert_eq!(energy(host, stand), Some(300));
                Err(mod_api::AttachmentError::MissingHolder)
            });
            assert!(aborted.is_err());
            assert_eq!(
                (energy(host, stand), energy(host, player)),
                (Some(100), Some(40))
            );
            assert_eq!(energy(host, fresh), None);

            // Written while the guest's state is swapped in for their turn, aborted after it
            // is swapped back out: the guest's own map is put back.
            game.open_storage_transaction();
            game.lan_acting_uuid = Some(guest);
            std::mem::swap(
                &mut game.player_state.attachments,
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .attachments_mut(conn)
                    .unwrap(),
            );
            let host: &mut dyn ServerHost = game;
            ENERGY.set(host, player, 70).unwrap();
            std::mem::swap(
                &mut game.player_state.attachments,
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .attachments_mut(conn)
                    .unwrap(),
            );
            game.lan_acting_uuid = None;
            game.close_storage_transaction(false);
            let host: &mut dyn ServerHost = game;
            assert_eq!(energy(host, player), Some(40));
            assert!(game.transactions.is_empty());
        });
    }
}
