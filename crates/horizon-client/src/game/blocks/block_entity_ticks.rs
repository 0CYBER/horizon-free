//! `Level.tickBlockEntities` for the block entities ticked together here
//! (hoppers, crafters, jukeboxes, beacons, creaking hearts, spawners, trial
//! spawners, vaults, beehives, furnaces and potent sulfur): only those whose state has a
//! ticker, in vanilla's list order (`horizon_client::block_entities`), in
//! chunks that tick blocks. Sculk, brewing stands and pistons tick in their
//! own passes.
use super::Game;
use crate::game::published_loops::{Attributed, LoopFlow, LoopMod};
use horizon_client::{
    Result,
    block_entities::{Pos, Ticking},
    block_entity_anim::Ticker,
    containers::{BlockEntity, FurnaceKind, HOPPER_SLOTS},
    furnace::FurnaceCooking,
};
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;

/// Shared decision for a borrowed position read and an owned tick-phase snapshot.
fn block_ticking_at(
    levels: &horizon_client::ticket_levels::Levels,
    pending: &horizon_client::fast_hash::FxHashSet<(i32, i32)>,
    x: i32,
    z: i32,
) -> bool {
    levels.block_ticking_at(x, z) && !pending.contains(&(x >> 4, z >> 4))
}

/// What `block_entities.tick` is visiting: the kinds its dispatch ticks, read once per entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TickedKind {
    /// `HopperBlockEntity` (its transfer state is `Game::hoppers[pos]`).
    Hopper,
    /// `CrafterBlockEntity`.
    Crafter,
    /// `JukeboxBlockEntity`, while it holds a record.
    Jukebox,
    /// `BeaconBlockEntity`.
    Beacon,
    /// `CreakingHeartBlockEntity`.
    CreakingHeart,
    /// `SpawnerBlockEntity`.
    Spawner,
    /// `TrialSpawnerBlockEntity`.
    TrialSpawner,
    /// `VaultBlockEntity`.
    Vault,
    /// `BeehiveBlockEntity` (beehives and bee nests).
    Beehive,
    /// `AbstractFurnaceBlockEntity` (furnaces, blast furnaces, smokers).
    Furnace,
    /// `BrewingStandBlockEntity`.
    BrewingStand,
    /// `DaylightDetectorBlockEntity`.
    DaylightDetector,
    /// `TheEndGatewayBlockEntity`.
    EndGateway,
    /// `CampfireBlockEntity`.
    Campfire,
    /// A mod's ticking block entity.
    Mod,
    /// `PotentSulfurBlockEntity`.
    PotentSulfur,
}

impl TickedKind {
    /// What `block_entities.tick` ticks `entity` as; `None` for a block entity it does not tick.
    #[inline(always)]
    fn of(entity: &BlockEntity) -> Option<Self> {
        Some(match entity {
            BlockEntity::Chest(items) if items.len() == HOPPER_SLOTS => Self::Hopper,
            BlockEntity::Crafter(_) => Self::Crafter,
            BlockEntity::Jukebox(_) => Self::Jukebox,
            BlockEntity::Beacon(_) => Self::Beacon,
            BlockEntity::CreakingHeart(_) => Self::CreakingHeart,
            BlockEntity::Spawner(_) => Self::Spawner,
            BlockEntity::TrialSpawner(_) => Self::TrialSpawner,
            BlockEntity::Vault(_) => Self::Vault,
            BlockEntity::Beehive(_) => Self::Beehive,
            BlockEntity::Furnace(_) => Self::Furnace,
            BlockEntity::Brewing(_) => Self::BrewingStand,
            BlockEntity::DaylightDetector => Self::DaylightDetector,
            BlockEntity::EndGateway(_) => Self::EndGateway,
            BlockEntity::Campfire(_) => Self::Campfire,
            BlockEntity::Mod(_) => Self::Mod,
            BlockEntity::PotentSulfur(_) => Self::PotentSulfur,
            _ => return None,
        })
    }
    /// [`Self::of`], with a mod's block entity's type: its numeric wire ID, kept on the entity
    /// since it was made (`ModBlockEntity::numeric_id`).
    #[inline(always)]
    fn with_type(entity: &BlockEntity) -> Option<(Self, Option<u32>)> {
        let kind = Self::of(entity)?;
        let mod_type = match entity {
            BlockEntity::Mod(entity) => entity.numeric_id(),
            _ => None,
        };
        Some((kind, mod_type))
    }
}

/// The block entity `block_entities.tick` is visiting, as a slot gets it: the loop's own state,
/// lent under the loop's `&mut self` for one call. Pinned to this build like a mixin's target.
#[allow(
    dead_code,
    reason = "read by `[[native.loop]]` helpers, compiled in a mod build"
)]
pub(crate) struct BlockEntityTick<'g, 'a> {
    /// The game the loop runs on: the level being ticked, `hoppers`, everything the loop has.
    pub(crate) game: &'g mut Game<'a>,
    /// The block entity's position.
    pub(crate) pos: Pos,
    /// What it is, from the block entity the dispatch is about to tick.
    pub(crate) kind: TickedKind,
    /// A mod's block entity's type, as a numeric wire ID; read through [`OwnBlockEntities`].
    mod_type: Option<u32>,
}

#[allow(
    dead_code,
    reason = "read by `[[native.loop]]` helpers, compiled in a mod build"
)]
impl BlockEntityTick<'_, '_> {
    /// The visited block entity.
    pub(crate) fn block_entity(&mut self) -> Option<&mut BlockEntity> {
        self.game.world.block_entities.get_mut(&self.pos)
    }
}

/// A mod's own block entity types, for a slot of `block_entities.tick` to pick out its mod's block
/// entities (book: Published loops). Made from the types' ids; the first [`Self::contains`]
/// resolves them once into a flag for each numeric wire ID. A mod block entity carries its own
/// type's numeric wire ID from when it was made, whatever block it stands on, so each visit after
/// that reads one flag: no string work, no hashing, no allocation. A mod with no slot never makes
/// one.
///
/// Both the flags and the entities' IDs assume the block entity type table is frozen for the
/// process (`block_entity_types::install` refuses a change; a new table needs a restart), and so are
/// the block states the types were made valid on; nothing here reads a block state.
#[allow(
    dead_code,
    reason = "read by `[[native.loop]]` helpers, compiled in a mod build"
)]
pub(crate) struct OwnBlockEntities {
    /// The types' ids, as the mod registered them (`mymod:machine`).
    ids: &'static [&'static str],
    /// By numeric wire ID, whether the type is one of `ids`; `None` until a visit finds the game's
    /// type table.
    by_numeric_id: Option<Box<[bool]>>,
}

#[allow(
    dead_code,
    reason = "read by `[[native.loop]]` helpers, compiled in a mod build"
)]
impl OwnBlockEntities {
    /// The block entity types `ids`, such as `&["mymod:machine"]`; resolved on the first visit,
    /// which logs a warning for each id the game has no block entity type of. Such an id matches
    /// nothing.
    pub(crate) const fn new(ids: &'static [&'static str]) -> Self {
        Self {
            ids,
            by_numeric_id: None,
        }
    }
    /// Whether the visited block entity is a mod's of one of these types. Before the type table is
    /// installed nothing is resolved or kept, and a later visit resolves.
    #[inline]
    pub(crate) fn contains(&mut self, tick: &BlockEntityTick<'_, '_>) -> bool {
        let Some(mod_type) = tick.mod_type else {
            return false;
        };
        if self.by_numeric_id.is_none() {
            self.by_numeric_id = Self::resolve(self.ids);
        }
        self.by_numeric_id
            .as_ref()
            .and_then(|by_numeric_id| by_numeric_id.get(mod_type as usize))
            .copied()
            .unwrap_or(false)
    }
    /// For each numeric wire ID, whether it names one of `ids`; `None` while the game has no type
    /// table, or an empty one.
    #[cold]
    fn resolve(ids: &[&str]) -> Option<Box<[bool]>> {
        let types = horizon_client::block_entity_types::current()?;
        if types.numeric_id_bound() == 0 {
            return None;
        }
        let mut by_numeric_id = vec![false; types.numeric_id_bound()];
        for id in ids {
            match types.numeric_id(id) {
                Some(numeric_id) => by_numeric_id[numeric_id as usize] = true,
                None => log::warn!(
                    "Published loop block_entities.tick: OwnBlockEntities names `{id}`, which is no \
                     registered block entity type; it matches nothing"
                ),
            }
        }
        Some(by_numeric_id.into())
    }
}

/// A `[[native.loop]]` slot of `block_entities.tick` (book: Published loops). The loop visits the
/// block entities in vanilla's order, only those in block-ticking chunks, inside the border, live and
/// not asleep, and asks each slot about each. Pinned to this build like a mixin's target: its
/// signatures and the cursor may change in any build.
///
/// The slots get the whole game, so they can change the block entities. The pass visits the list
/// it took when it started: one a slot adds joins the list for the next pass, as one placed by a
/// tick does, and one it removes (or replaces) later in the list is passed over when the loop gets
/// there, as one removed by an earlier tick is. For the visited one, see [`Self::before`].
pub(crate) trait BlockEntityTickSlot {
    /// Whether [`Self::before`] may change the visited block entity, so the loop checks it again
    /// after a [`LoopFlow::Continue`]. False only for `()`, the game's own loop, whose `before`
    /// does nothing, so it has no check; a mod's slot always has it.
    const RECHECK: bool = true;
    /// Just before the visited block entity ticks. [`LoopFlow::Skip`] leaves it out this tick;
    /// [`LoopFlow::Stop`] ends the pass (what follows the loop, such as shulker box lids, still runs).
    /// After [`LoopFlow::Continue`] from every slot the loop looks again: when the visited block
    /// entity was removed, or replaced by one of another [`TickedKind`] or a mod's of another type,
    /// it does not tick and no slot's [`Self::after`] is called for it.
    #[inline(always)]
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        let _ = tick;
        LoopFlow::Continue
    }
    /// Just after the visited block entity ticked; not called for one a slot skipped.
    #[inline(always)]
    fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
        let _ = tick;
    }
}

/// No mod: the game's own loop.
impl BlockEntityTickSlot for () {
    const RECHECK: bool = false;
}

/// Two slots in order: `A` first, and `B` only while `A` answers [`LoopFlow::Continue`].
impl<A: BlockEntityTickSlot, B: BlockEntityTickSlot> BlockEntityTickSlot for (A, B) {
    const RECHECK: bool = A::RECHECK || B::RECHECK;
    #[inline(always)]
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        match self.0.before(tick) {
            LoopFlow::Continue => self.1.before(tick),
            answer => answer,
        }
    }
    #[inline(always)]
    fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
        self.0.after(tick);
        self.1.after(tick);
    }
}

impl<N: LoopMod, T: BlockEntityTickSlot> BlockEntityTickSlot for Attributed<N, T> {
    #[inline(always)]
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        self.call(|slot| slot.before(tick))
    }
    #[inline(always)]
    fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
        self.call(|slot| slot.after(tick));
    }
}

/// A pass's working state: the item and mob caches and the spawners ticked so far.
type PassScratch = (
    Option<crate::game::machines::ItemColumns>,
    Option<crate::game::potent_sulfur::PassMobs>,
    Vec<Pos>,
    Vec<Pos>,
);
/// Puts the enclosing pass's scratch back when the pass inside it ends, however it ends:
/// a slot may catch a nested pass's panic and let the enclosing pass go on.
struct Enclosing<'g, 'a> {
    game: &'g mut Game<'a>,
    scratch: Option<PassScratch>,
}
impl Drop for Enclosing<'_, '_> {
    fn drop(&mut self) {
        // A pass that ended early left its own; a pass in progress is known by them.
        self.game.item_columns = None;
        self.game.pass_mobs = None;
        if let Some(scratch) = self.scratch.take() {
            (
                self.game.item_columns,
                self.game.pass_mobs,
                self.game.spawner_tick_positions,
                self.game.trial_spawner_tick_positions,
            ) = scratch;
        }
    }
}

/// The block entity type a sample is attributed to while its ticker runs.
fn ticker_name(entity: &BlockEntity) -> &'static str {
    match entity {
        BlockEntity::Chest(_) => "hopper",
        BlockEntity::Crafter(_) => "crafter",
        BlockEntity::Jukebox(_) => "jukebox",
        BlockEntity::Beacon(_) => "beacon",
        BlockEntity::CreakingHeart(_) => "creaking_heart",
        BlockEntity::Spawner(_) => "mob_spawner",
        BlockEntity::TrialSpawner(_) => "trial_spawner",
        BlockEntity::Vault(_) => "vault",
        BlockEntity::Beehive(_) => "beehive",
        BlockEntity::Furnace(_) => "furnace",
        BlockEntity::Brewing(_) => "brewing_stand",
        BlockEntity::DaylightDetector => "daylight_detector",
        BlockEntity::Campfire(_) => "campfire",
        BlockEntity::Mod(_) => "mod_block_entity",
        BlockEntity::PotentSulfur(_) => "potent_sulfur",
        _ => "block_entity",
    }
}

impl Game<'_> {
    /// The published loop `block_entities.tick` (`published_loops.rs`): with no
    /// `[[native.loop]]` mod its slot is `()`, and a mod build installs its own copy of this
    /// function, with the mods' slots, through its replacement cell.
    #[horizon_mod_api_macros::horizon_loop(name = "block_entities.tick", side = "server")]
    pub(super) fn tick_block_entities(&mut self) -> Result<()> {
        self.tick_block_entities_with(&mut ())
    }
    /// `tick_block_entities` with `slot` asked before and after each block entity's tick. Generic
    /// over the slot only, so it has no replacement cell of its own: a mod build compiles it.
    ///
    /// A run from inside a pass (a slot that ticks the block entities again) is a whole pass of
    /// its own, the work after the loop included. It sets the enclosing pass's scratch (the item
    /// and mob caches, the spawners ticked so far) aside and puts it back when it ends, so the
    /// enclosing pass goes on with its own.
    pub(crate) fn tick_block_entities_with<S: BlockEntityTickSlot>(
        &mut self,
        slot: &mut S,
    ) -> Result<()> {
        // Only a pass in progress holds item columns: every pass drops them as it ends.
        let scratch = self.item_columns.is_some().then(|| {
            (
                self.item_columns.take(),
                self.pass_mobs.take(),
                std::mem::take(&mut self.spawner_tick_positions),
                std::mem::take(&mut self.trial_spawner_tick_positions),
            )
        });
        // The pass below is written on `this`, the game the guard lends back.
        let enclosing = Enclosing {
            game: self,
            scratch,
        };
        let this = &mut *enclosing.game;
        this.fire_pending_block_entity_loads();
        this.tick_mount_menu();
        this.tick_craft_table();
        this.tick_container_reach();
        this.load_entity_types();
        this.ensure_stateful_entity_table();
        if !this.world.block_entities.has_states() {
            let states = this.ticking_states();
            this.world.block_entities.set_states(states);
            let containers = this.native_containers();
            this.world.block_entities.set_mod_containers(containers);
            let fluids = this.native_fluid_tables();
            this.world.block_entities.set_mod_fluids(fluids);
            let energy = this.native_energy_tables();
            this.world.block_entities.set_mod_energy(energy);
        }
        // Only lingering cooldowns are dropped; a hopper that ticks checks
        // its own block. Once a second is soon enough for the rest.
        if this.world.game_time % 20 == 0 {
            this.forget_gone_hoppers();
        }
        let ticking = this.block_ticking_area();
        this.spawner_tick_positions.clear();
        this.trial_spawner_tick_positions.clear();
        this.item_columns = Some(Default::default());
        this.pass_mobs = Some(Default::default());
        let tickers = this.world.block_entity_tickers();
        let mut last_chunk = (i32::MIN, i32::MIN);
        let mut last_chunk_ticking = false;
        for ticker in tickers.iter().copied() {
            let pos = ticker.0;
            let chunk = (pos.0 >> 4, pos.2 >> 4);
            let chunk_ticking = if chunk == last_chunk {
                last_chunk_ticking
            } else {
                last_chunk = chunk;
                last_chunk_ticking = ticking(pos.0, pos.2);
                last_chunk_ticking
            };
            if !chunk_ticking {
                continue;
            }
            // Ticks before this one can remove it (`TickingBlockEntity.isRemoved`).
            if this.world.block_entities.is_asleep(&pos)
                || !this.world.block_entities.is_live(ticker)
                || !this.border.contains(f64::from(pos.0), f64::from(pos.2))
            {
                continue;
            }
            let Some((kind, mod_type)) = this
                .world
                .block_entities
                .get(&pos)
                .and_then(TickedKind::with_type)
            else {
                continue;
            };
            match slot.before(&mut BlockEntityTick {
                game: this,
                pos,
                kind,
                mod_type,
            }) {
                LoopFlow::Continue => {}
                LoopFlow::Skip => continue,
                LoopFlow::Stop => break,
            }
            // `before` had the game: the entity it was asked about may be gone, another kind or a
            // mod's of another type.
            if S::RECHECK
                && (!this.world.block_entities.is_live(ticker)
                    || this
                        .world
                        .block_entities
                        .get(&pos)
                        .and_then(TickedKind::with_type)
                        != Some((kind, mod_type)))
            {
                continue;
            }
            horizon_threads::work::set_chunk(chunk.0, chunk.1);
            horizon_threads::work::set_entity(horizon_threads::work::entity_id(
                this.world.block_entities.get(&pos).map_or("", ticker_name),
            ));
            match kind {
                TickedKind::Hopper => this.tick_hopper(pos),
                TickedKind::Crafter => this.tick_crafter(pos),
                TickedKind::Jukebox => this.tick_jukebox(pos),
                TickedKind::Beacon => this.tick_beacon_at(pos),
                TickedKind::CreakingHeart => this.tick_creaking_heart(pos)?,
                TickedKind::Spawner => {
                    this.tick_spawner(pos)?;
                    this.spawner_tick_positions.push(pos);
                }
                TickedKind::TrialSpawner => {
                    this.tick_trial_spawner(pos)?;
                    this.trial_spawner_tick_positions.push(pos);
                }
                TickedKind::Vault => this.tick_vault(pos)?,
                TickedKind::Beehive => this.tick_beehive(pos),
                TickedKind::Furnace => this.tick_furnace(pos)?,
                TickedKind::BrewingStand => this.tick_brewing_stand(pos)?,
                TickedKind::DaylightDetector => this.tick_daylight_detector(pos),
                TickedKind::EndGateway => this.tick_end_gateway(pos),
                TickedKind::Campfire => this.tick_campfire(pos)?,
                TickedKind::Mod => this.tick_mod_block_entity(pos),
                TickedKind::PotentSulfur => {
                    this.tick_potent_sulfur(pos)?;
                    // A window that simulates the world itself is its own
                    // client; a server's clients tick it on their own.
                    if this.options.execution_role.is_client() {
                        this.client_tick_potent_sulfur(pos);
                    }
                }
            }
            slot.after(&mut BlockEntityTick {
                game: this,
                pos,
                kind,
                mod_type,
            });
        }
        horizon_threads::work::clear_work();
        // Items move between passes: the columns hold for this one only.
        this.item_columns = None;
        this.pass_mobs = None;
        this.tick_sign_editors();
        this.update_song_volumes();
        this.scan_for_hearts();
        // The client ticks (particles, sounds, the turning mob) are for a
        // window; a dedicated or integrated server has none.
        if this.options.dedicated.is_none() {
            let spawners = std::mem::take(&mut this.spawner_tick_positions);
            this.client_tick_spawners(&spawners);
            this.spawner_tick_positions = spawners;
            let trial_spawners = std::mem::take(&mut this.trial_spawner_tick_positions);
            for key in trial_spawners.iter().copied() {
                this.client_tick_trial_spawner(key);
            }
            this.trial_spawner_tick_positions = trial_spawners;
        }
        // `ShulkerBoxBlockEntity.tick`: the lids of the boxes this level
        // animates. Their grown collision shape reaches the collision queries
        // through `World::shulkers`, and an opening lid pushes what it runs
        // into (`moveCollidedEntities`), as a moving piston does.
        let pushes = this.tick_shulkers();
        this.sync_shulkers();
        this.push_shulker_entities(pushes);
        Ok(())
    }
    /// `ShulkerBoxBlockEntity.tick` for the boxes in block-ticking chunks, in
    /// position order: the lid's new progress, and the swept box of an opening
    /// lid for `moveCollidedEntities`. A box whose block is gone is dropped.
    /// The boxes are those the level animates: a box opens (`start_open` and
    /// block event 1) before it has one, exactly as vanilla's block entity is
    /// there from the start.
    fn tick_shulkers(&mut self) -> Vec<super::piston_push::ShulkerPush> {
        let ticking = self.block_ticking_area();
        let mut pushes = Vec::new();
        let mut gone = Vec::new();
        let positions: Vec<[i32; 3]> = self.be_anims.shulkers.keys().copied().collect();
        for pos in positions {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let Some(Ticker::Shulker { facing }) = self.be_anims.ticker(state.0) else {
                gone.push(pos);
                continue;
            };
            if !ticking(pos[0], pos[2])
                || !self.border.contains(f64::from(pos[0]), f64::from(pos[2]))
            {
                continue;
            }
            let tick = self
                .be_anims
                .shulkers
                .get_mut(&pos)
                .map(|shulker| shulker.tick(pos, facing));
            let Some(tick) = tick else { continue };
            // `ShulkerBoxBlockEntity.doNeighborUpdates` at the start and the
            // end of an opening or closing lid: the box's collision shape
            // moved, so the neighbours' shapes and their redstone updates run.
            for _ in 0..tick.neighbor_updates {
                let state = self.world.block(pos[0], pos[1], pos[2]);
                let at = BlockPos::new(pos[0], pos[1], pos[2]);
                self.with_level(|b, level, ticks| {
                    b.shulker_neighbor_updates(level, ticks, at, state)
                });
            }
            if let Some(push) = tick.push {
                pushes.push((pos, facing, push));
            }
        }
        for pos in gone {
            self.be_anims.shulkers.remove(&pos);
        }
        pushes
    }
    /// Copies the shulker box lids into the world, where the collision queries
    /// read them (`SceneCollisions::shulker_collision_state`), as
    /// `sync_moving_pistons` does for pistons.
    pub(super) fn sync_shulkers(&mut self) {
        self.world.shulkers.clear();
        self.world
            .shulkers
            .extend(self.be_anims.shulkers.iter().map(|(p, s)| (*p, s.clone())));
    }
    /// `BlockState.getTicker` of every state, for the block entities ticked
    /// here. A creaking heart keeps its slot while uprooted (see
    /// `tick_creaking_heart`).
    pub(in crate::game) fn ticking_states(&self) -> Vec<Ticking> {
        let registry = self.pack.assets.registry;
        (0..registry.len())
            .map(|i| {
                let state = BlockStateId(i as u32);
                let name: &str = &self.res.facts().get(state).name;
                let factory = horizon_client::block_entity_types::factory_block(name);
                match factory {
                    "minecraft:hopper"
                    | "minecraft:crafter"
                    | "minecraft:beacon"
                    | "minecraft:creaking_heart"
                    | "minecraft:spawner"
                    | "minecraft:trial_spawner"
                    | "minecraft:vault"
                    | "minecraft:beehive"
                    | "minecraft:bee_nest"
                    | "minecraft:brewing_stand"
                    | "minecraft:daylight_detector"
                    | "minecraft:end_gateway"
                    // Lit, cookTick; unlit, cooldownTick.
                    | "minecraft:campfire"
                    | "minecraft:soul_campfire" => Ticking::Yes,
                    // Every state but dry has a ticker.
                    "minecraft:potent_sulfur" => {
                        let dry = registry
                            .state(state)
                            .and_then(|s| s.properties.get("potent_sulfur_state"))
                            .is_some_and(|v| v == "dry");
                        if dry { Ticking::Idle } else { Ticking::Yes }
                    }
                    "minecraft:jukebox" => {
                        let record = registry
                            .state(state)
                            .and_then(|s| s.properties.get("has_record"))
                            .is_some_and(|v| v == "true");
                        if record { Ticking::Yes } else { Ticking::Idle }
                    }
                    n if FurnaceKind::from_block(n).is_some() => Ticking::Yes,
                    _ if !name.starts_with("minecraft:") && self.native_ticks(name) => Ticking::Yes,
                    _ => Ticking::No,
                }
            })
            .collect()
    }
    /// `LevelChunk.isTicking` (block coordinates): a block-ticking chunk from
    /// every player's tickets (`update_chunk_tickets`), its entities loaded
    /// (`ServerLevel.areEntitiesLoaded`).
    pub(super) fn block_ticking_area(&self) -> impl Fn(i32, i32) -> bool + use<> {
        let levels = self.tickets.levels();
        let pending = self.world.entities_pending.clone();
        move |x, z| block_ticking_at(&levels, &pending, x, z)
    }
    /// Single-position read without copying the pending-entity set.
    pub(super) fn block_ticking_at(&self, x: i32, z: i32) -> bool {
        block_ticking_at(&self.tickets.levels(), &self.world.entities_pending, x, z)
    }
    /// `DaylightDetectorBlock.tickEntity` of the detector at `pos`.
    fn tick_daylight_detector(&mut self, pos: Pos) {
        // It updates every 20 game ticks (the behaviour checks this too).
        if self.world.game_time % 20 != 0 {
            return;
        }
        let at = BlockPos::new(pos.0, pos.1, pos.2);
        self.with_level(|b, level, ticks| b.tick_daylight_detector(level, ticks, at));
        let _ = self.process_level_events();
    }
    /// `AbstractFurnaceBlockEntity.serverTick` of the furnace at `pos`, and
    /// its block's `lit` following it.
    pub(super) fn tick_furnace(&mut self, pos: Pos) -> Result<()> {
        // What the hooks need from before the tick, without cloning stacks: the
        // fuel's id only while the furnace is out (it can light this tick), the
        // input's id (a last item is gone after it smelts) and the result count.
        let before = match self.world.block_entities.get(&pos) {
            Some(BlockEntity::Furnace(f)) => (
                f.cooking_timer,
                (f.lit_remaining == 0)
                    .then(|| f.items[1].as_ref().map(|s| s.item.clone()))
                    .flatten(),
                f.items[0].as_ref().map(|s| s.item.clone()),
                f.items[2].as_ref().map_or(0, |s| s.count),
                f.lit_remaining,
            ),
            _ => return Ok(()),
        };
        let mut drops = Vec::new();
        let (lit, burning, smelted, smelting, fueled) = {
            let needs_part = mod_api::__private::has_part_hook(mod_api::HookId::CraftingRemainder)
                && matches!(self.world.block_entities.get(&pos), Some(BlockEntity::Furnace(f))
                    if f.lit_remaining <= 1 && f.items[1].as_ref().is_some_and(|s|
                        mod_api::__private::item_listens(mod_api::HookId::CraftingRemainder, &s.item)));
            let lit = if needs_part {
                let Some(BlockEntity::Furnace(original)) = self.world.block_entities.get(&pos)
                else {
                    return Ok(());
                };
                let mut furnace = original.clone();
                let lit = furnace.tick(
                    &self.loop_context,
                    world_recipes!(self),
                    world_items!(self),
                    self.res.providers(),
                    &mut drops,
                    &|s| self.crafting_remainder(s),
                );
                self.world
                    .block_entities
                    .insert(pos, BlockEntity::Furnace(furnace));
                lit
            } else {
                let Some(BlockEntity::Furnace(furnace)) = self.world.block_entities.get_mut(&pos)
                else {
                    return Ok(());
                };
                let items = world_items!(self);
                furnace.tick(
                    &self.loop_context,
                    world_recipes!(self),
                    items,
                    self.res.providers(),
                    &mut drops,
                    &|s| crate::crafting::CraftingRemainder::crafting_remainder(items, s),
                )
            };
            let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get(&pos) else {
                return Ok(());
            };
            let burning = before.1.filter(|_| f.lit_remaining > 0);
            let smelted = f.items[2]
                .as_ref()
                .filter(|after| after.count > before.3)
                .map(|after| (before.2.clone(), after.item.clone()));
            let started = before.0 == 0 && f.cooking_timer > 0;
            let smelting = started
                .then(|| f.items[0].as_ref().map(|item| item.item.clone()))
                .flatten();
            // The tick takes one off `lit_remaining`; only consuming fuel
            // sets it higher (a lava bucket leaves a bucket of the same count).
            let fueled = f.lit_remaining > before.4.saturating_sub(1);
            (lit, burning, smelted, smelting, fueled)
        };
        // `AbstractFurnaceBlockEntity.serverTick`: `burn` and consuming fuel
        // set the changed flag, so a completed smelt or a refuel marks the
        // chunk and tells comparators even though the block state stays LIT.
        if smelted.is_some() || fueled {
            self.container_changed(pos);
        }
        if let Some(item) = smelting {
            self.native_furnace_start_smelt([pos.0, pos.1, pos.2], &item);
        }
        if let Some(fuel) = burning {
            self.native_furnace_burn([pos.0, pos.1, pos.2], &fuel);
        }
        if let Some((Some(item), result)) = smelted {
            self.native_furnace_smelt([pos.0, pos.1, pos.2], &item, &result);
        }
        for stack in drops {
            self.campfire_drop(pos, stack);
        }
        let Some(lit) = lit else {
            if matches!(self.world.block_entities.get(&pos), Some(BlockEntity::Furnace(f)) if f.idle())
            {
                self.world.block_entities.sleep(pos);
            }
            return Ok(());
        };
        let state = self.world.block(pos.0, pos.1, pos.2);
        if let Some(mut s) = self.pack.assets.registry.state(state).cloned()
            && s.properties.contains_key("lit")
        {
            s.properties.insert("lit".into(), lit.to_string());
            if let Ok(id) = self.pack.assets.registry.resolve(&s) {
                self.level_set([pos.0, pos.1, pos.2], id)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod jukebox_stop_tests {
    use super::Game;
    use horizon_client::items::ItemStack;

    /// A windowless game from the prepared resources with a resident chunk and
    /// a jukebox at (0, 64, 0), recording its game events; `None` without the
    /// resources. `f` gets the game and the disc.
    fn with_jukebox(f: impl FnOnce(&mut Game<'_>, &ItemStack)) -> Option<Vec<String>> {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES")?);
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .ok()?;
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
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
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        let jukebox = game
            .pack
            .assets
            .registry
            .default_state("minecraft:jukebox")?;
        game.world.set(0, 64, 0, jukebox).unwrap();
        let disc = ItemStack::new("minecraft:music_disc_cat", 1);
        crate::game::dispense_fixture::start_recording();
        f(&mut game, &disc);
        Some(crate::game::dispense_fixture::stop_recording())
    }

    fn stops(lines: &[String]) -> Vec<&String> {
        lines
            .iter()
            .filter(|l| l.ends_with("minecraft:jukebox_stop_play"))
            .collect()
    }

    /// `JukeboxSongPlayer.stop` posts `JUKEBOX_STOP_PLAY` at the jukebox, and
    /// `popOutTheItem` (the disc taken out by hand) and the end of a song both
    /// reach it. Horizon stopped the sound and told the parrots but posted the
    /// vibration only when the block itself was broken, so pulling the disc out
    /// was silent to a sculk sensor beside it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_jukebox_that_stops_tells_the_listeners() {
        let Some(lines) = with_jukebox(|game, disc| {
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    Some(disc),
                    horizon_world::level::UseBranch::ItemOn,
                ) == Some(mod_api::InteractionResult::Success),
                "the disc goes in"
            );
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    None,
                    horizon_world::level::UseBranch::WithoutItem,
                ) == Some(mod_api::InteractionResult::Success),
                "the empty hand pops it out"
            );
        }) else {
            return;
        };
        assert_eq!(
            stops(&lines),
            vec![&"G 0,64,0 minecraft:jukebox_stop_play".to_owned()],
            "{lines:?}"
        );

        // The end of a song is the same stop: tick until the song is over.
        let Some(lines) = with_jukebox(|game, disc| {
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    Some(disc),
                    horizon_world::level::UseBranch::ItemOn,
                ) == Some(mod_api::InteractionResult::Success),
                "the disc goes in"
            );
            for _ in 0..40_000 {
                let playing = matches!(
                    game.world.block_entities.get(&(0, 64, 0)),
                    Some(horizon_client::containers::BlockEntity::Jukebox(j)) if j.playing
                );
                if !playing {
                    break;
                }
                game.tick_jukebox((0, 64, 0));
            }
            // JukeboxSongPlayer.stop posts only while a song plays: taking
            // the finished disc out is silent.
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    None,
                    horizon_world::level::UseBranch::WithoutItem,
                ) == Some(mod_api::InteractionResult::Success),
                "the empty hand pops it out"
            );
        }) else {
            return;
        };
        assert_eq!(
            stops(&lines).len(),
            1,
            "the song's own end posts it once, its removal after it nothing: {lines:?}"
        );

        // A broken jukebox posts it from the world's spill instead, and a
        // jukebox that never held a disc posts nothing at all.
        let Some(lines) = with_jukebox(|game, disc| {
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    Some(disc),
                    horizon_world::level::UseBranch::ItemOn,
                ) == Some(mod_api::InteractionResult::Success),
                "the disc goes in"
            );
            game.jukebox_removed([0, 64, 0]);
        }) else {
            return;
        };
        assert!(stops(&lines).is_empty(), "not twice: {lines:?}");
        let Some(lines) = with_jukebox(|game, _| {
            for _ in 0..41 {
                game.tick_jukebox((0, 64, 0));
            }
        }) else {
            return;
        };
        assert!(lines.is_empty(), "{lines:?}");
    }
}

#[cfg(test)]
mod add_blocks_tests {
    use super::*;
    use mod_api::*;
    use std::sync::{Arc, OnceLock};

    #[test]
    #[ignore = "needs prepared resources; isolates process-wide registries"]
    fn vanilla_template_membership_creates_ticks_and_saves() {
        const FLAG: &str = "HORIZON_ADD_BLOCKS_CASE";
        const BLOCK: &str = "add_blocks_test:vault";
        const SIGN: &str = "add_blocks_test:sign";
        const DECLARED: &str = "add_blocks_test:declared";
        if std::env::var_os(FLAG).is_none() {
            for case in ["valid", "quiet", "bad_class", "bad_block", "bad_type"] {
                let scratch = std::env::temp_dir()
                    .join(format!("horizon-add-blocks-{}-{case}", std::process::id()));
                std::fs::create_dir_all(&scratch).unwrap();
                let mut child = std::process::Command::new(std::env::current_exe().unwrap());
                child.args(["--ignored", "--exact", "game::block_entity_ticks::add_blocks_tests::vanilla_template_membership_creates_ticks_and_saves", "--nocapture"])
                    .current_dir(&scratch).env(FLAG, case).env("HORIZON_NO_AUDIO", "1").env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1");
                for key in [
                    "HORIZON_HOME",
                    "HORIZON_LAUNCHER_HOME",
                    "XDG_DATA_HOME",
                    "XDG_CONFIG_HOME",
                ] {
                    child.env(key, scratch.join(key));
                }
                let status = child.status().unwrap();
                std::fs::remove_dir_all(&scratch).unwrap();
                assert!(status.success(), "case {case}");
            }
            return;
        }
        let case = std::env::var(FLAG).unwrap();
        #[derive(Default)]
        struct Keys(
            OnceLock<(BlockEntityTypeKey, BlockEntityTypeKey, BlockKey, BlockKey)>,
            std::sync::atomic::AtomicUsize,
        );
        struct Listener(Arc<Keys>, String);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let kind = r.block_entity_type_key(if self.1 == "bad_type" {
                    "minecraft:missing"
                } else {
                    "minecraft:vault"
                });
                let extra = r.block_entity_type("extra").blocks(&[]).build();
                r.block_entity_type("own").blocks(&[DECLARED]).build();
                let declared = r.block_key(DECLARED);
                let block = r.block_key(if self.1 == "bad_block" {
                    "add_blocks_test:missing"
                } else {
                    BLOCK
                });
                assert_eq!(
                    *self.0.0.get_or_init(|| (kind, extra, block, declared)),
                    (kind, extra, block, declared)
                );
            }
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                for (id, template) in [
                    (
                        BLOCK,
                        if self.1 == "bad_class" {
                            "minecraft:stone"
                        } else {
                            "minecraft:vault"
                        },
                    ),
                    (SIGN, "minecraft:oak_sign"),
                    (DECLARED, "minecraft:vault"),
                ] {
                    event.blocks.push(mod_api::Block::new(id, template));
                }
            }
            fn block_entity_type_add_blocks(
                &self,
                _: &RegistrationContext,
                event: &mut BlockEntityTypeAddBlocksEvent<'_>,
            ) {
                self.0.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let &(kind, extra, block, declared) = self.0.0.get().unwrap();
                event.modify(kind, &[block, declared]);
                event.modify_matching(|key| key == kind || key == extra, &[block]);
            }
        }
        struct Quiet(Listener);
        #[mod_api::__subscriptions]
        impl Hooks for Quiet {
            fn register(&self, r: &mut dyn Registrar, context: &RegistrationContext) {
                self.0.register(r, context);
            }
            fn blocks(&self, context: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                self.0.blocks(context, event);
            }
        }
        let observed = Arc::new(Keys::default());
        let listener = Listener(observed.clone(), case.clone());
        let hooks: Box<dyn Hooks> = if case == "quiet" {
            Box::new(Quiet(listener))
        } else {
            Box::new(listener)
        };
        let mods: Mods = vec![ModEntry::new("add_blocks_test", hooks)].into();
        mod_platform::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = crate::game::mods::content_root(&base, &Default::default());
        if case.starts_with("bad_") {
            let error = root
                .expect_err("invalid addition must fail startup")
                .to_string();
            assert!(
                error.contains(match case.as_str() {
                    "bad_class" => "must inherit VaultBlock",
                    "bad_block" => "unknown block",
                    _ => "unknown block entity type",
                }),
                "{error}"
            );
            return;
        }
        let root = root.unwrap();
        // Reload reuses frozen validity without redispatching registration.
        crate::game::mods::content_root(&base, &Default::default()).unwrap();
        assert_eq!(
            observed.1.load(std::sync::atomic::Ordering::SeqCst),
            usize::from(case != "quiet")
        );
        crate::game::native_entity_refs::tests::with_game_at(
            root.prepared().to_path_buf(),
            |game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                let registry = game.pack.assets.registry;
                let state = registry.default_state(BLOCK).unwrap();
                let level = game.level(game.dim.id()).unwrap();
                assert!(
                    game.level_set_block(level, [1, 64, 1], state, BlockUpdateFlags::NONE)
                        .unwrap()
                );
                assert!(matches!(
                    BlockEntity::for_block(SIGN),
                    Some(BlockEntity::Sign(_))
                ));
                assert!(matches!(
                    BlockEntity::for_block("minecraft:vault"),
                    Some(BlockEntity::Vault(_))
                ));
                if case == "quiet" {
                    assert!(game.world.block_entities.get(&(1, 64, 1)).is_none());
                    return;
                }
                let states = game.ticking_states();
                assert_eq!(states[state.0 as usize], Ticking::Yes);
                game.world.block_entities.set_states(states);
                assert!(
                    game.world
                        .block_entity_tickers()
                        .iter()
                        .any(|slot| slot.0 == (1, 64, 1))
                );
                game.world.game_time = 100;
                game.tick_vault((1, 64, 1)).unwrap();
                let entity = game.world.block_entities.get(&(1, 64, 1)).unwrap();
                let BlockEntity::Vault(vault) = entity else {
                    panic!("must create vanilla vault")
                };
                assert_eq!(vault.state_updating_resumes_at, 120);
                let tag =
                    horizon_client::save::block_entity_encode::encode((1, 64, 1), entity, BLOCK);
                let (_, restored) =
                    horizon_client::save::decode_block_entity_for_block(&tag, BLOCK).unwrap();
                assert_eq!(
                    horizon_client::save::block_entity_encode::encode((1, 64, 1), &restored, BLOCK),
                    tag
                );
                let BlockEntity::Vault(restored) = restored else {
                    panic!("saved type must stay vanilla")
                };
                assert_eq!(restored.state_updating_resumes_at, 120);
                assert!(matches!(
                    horizon_client::save::decode_block_entity_for_block(&tag, DECLARED),
                    Some((_, BlockEntity::Vault(_)))
                ));
                let own_state = registry.default_state(DECLARED).unwrap();
                assert!(
                    game.level_set_block(level, [3, 64, 1], own_state, BlockUpdateFlags::NONE)
                        .unwrap()
                );
                assert!(
                    matches!(game.world.block_entities.get(&(3, 64, 1)), Some(BlockEntity::Mod(entity)) if entity.id() == "add_blocks_test:own")
                );
                assert_eq!(
                    horizon_client::block_entity_types::valid(BLOCK, "minecraft:vault"),
                    Some(true)
                );
                assert_eq!(
                    horizon_client::block_entity_types::valid(BLOCK, "add_blocks_test:extra"),
                    Some(true)
                );
                assert_eq!(horizon_client::block_entity_types::type_for(BLOCK), None);
                assert_eq!(
                    horizon_client::block_entity_types::factory_block(BLOCK),
                    "minecraft:vault"
                );
            },
        );
    }
}

#[cfg(test)]
mod furnace_changed_tests {
    use crate::game::game_tests::tests::server_game;
    use horizon_client::{
        containers::{BlockEntity, Furnace, FurnaceKind},
        items::ItemStack,
    };
    use std::collections::HashMap;

    /// `AbstractFurnaceBlockEntity.serverTick`'s changed flag: `burn` sets it,
    /// and the old furnace calls `setChanged` at the end of the tick. A smelt
    /// that completes while the furnace stays LIT changes the output and the
    /// input without changing the block state, so nothing else marks it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_finished_smelt_marks_the_chunk() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:furnace")
            .unwrap();
        let pos = (8, 64, 8);
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        game.world.block_entities.insert(
            pos,
            BlockEntity::Furnace(Furnace {
                kind: FurnaceKind::Furnace,
                items: [
                    Some(ItemStack::new("minecraft:raw_iron", 1)),
                    Some(ItemStack::new("minecraft:coal", 1)),
                    None,
                ],
                lit_remaining: 5,
                lit_total: 200,
                cooking_timer: 199,
                cooking_total: 200,
                speed: 1.0,
                recipes_used: HashMap::new(),
            }),
        );
        game.world.modified.clear();

        game.tick_furnace(pos).expect("the furnace ticks");

        let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&pos) else {
            panic!("no furnace");
        };
        assert_eq!(
            f.items[2].as_ref().map(|s| &*s.item),
            Some("minecraft:iron_ingot"),
            "the smelt completed"
        );
        assert!(
            game.world.modified.contains(&(0, 0)),
            "a finished smelt left the chunk unmarked"
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_refuel_while_lit_marks_the_chunk() {
        // litTimeRemaining goes 1 -> 0 and the furnace takes new fuel in the
        // same tick: it stays LIT, and consuming the fuel sets changed.
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:furnace")
            .unwrap();
        let pos = (8, 64, 8);
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        game.world.block_entities.insert(
            pos,
            BlockEntity::Furnace(Furnace {
                kind: FurnaceKind::Furnace,
                items: [
                    Some(ItemStack::new("minecraft:raw_iron", 1)),
                    Some(ItemStack::new("minecraft:coal", 2)),
                    None,
                ],
                lit_remaining: 1,
                lit_total: 1600,
                cooking_timer: 10,
                cooking_total: 200,
                speed: 1.0,
                recipes_used: HashMap::new(),
            }),
        );
        game.world.modified.clear();

        game.tick_furnace(pos).expect("the furnace ticks");

        let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&pos) else {
            panic!("no furnace");
        };
        assert_eq!(f.items[1].as_ref().map(|s| s.count), Some(1), "fuel used");
        assert!(f.items[2].is_none(), "nothing smelted");
        assert!(
            game.world.modified.contains(&(0, 0)),
            "a refuel left the chunk unmarked"
        );
    }
}

#[cfg(test)]
mod slot_guard_tests {
    use super::{BlockEntityTick, BlockEntityTickSlot, TickedKind};
    use crate::game::game_tests::tests::server_game;
    use crate::game::published_loops::{Attributed, LoopFlow, LoopMod, LoopSlots};
    use mod_api::content::crash::{ModCall, current_native_call};

    struct Row;
    impl LoopMod for Row {
        const CALL: ModCall = ModCall {
            id: "slot_guard",
            hook: "loop block_entities.tick",
            part: Some("slot_guard::Probe"),
        };
    }

    /// What ran, and the mod call each ran inside.
    #[derive(Default)]
    struct Probe {
        before: Vec<Option<ModCall>>,
        after: Vec<Option<ModCall>>,
    }
    #[derive(Default)]
    struct Both(Probe);
    impl BlockEntityTickSlot for Both {
        fn before(&mut self, _: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
            self.0.before.push(current_native_call());
            LoopFlow::Continue
        }
        fn after(&mut self, _: &mut BlockEntityTick<'_, '_>) {
            self.0.after.push(current_native_call());
        }
    }

    /// Each call of a slot, before and after an entry, runs inside its mod's guard.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_slot_is_entered_as_its_mod_before_and_after_an_entry() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let mut tick = BlockEntityTick {
            game: &mut game,
            pos: (8, 64, 8),
            kind: TickedKind::Hopper,
            mod_type: None,
        };
        let mut slots = <(Attributed<Row, Both>, ())>::make();
        assert_eq!(slots.before(&mut tick), LoopFlow::Continue);
        slots.after(&mut tick);
        let seen = slots.0.call(|slot| std::mem::take(&mut slot.0));
        assert_eq!(seen.before, [Some(Row::CALL)]);
        assert_eq!(seen.after, [Some(Row::CALL)]);
        assert_eq!(current_native_call(), None);
    }
}

/// What a slot that changes the block entities does to the pass (`BlockEntityTickSlot`'s docs).
#[cfg(test)]
mod slot_mutation_tests {
    use super::{BlockEntityTick, BlockEntityTickSlot};
    use crate::game::Game;
    use crate::game::game_tests::tests::server_game;
    use crate::game::published_loops::LoopFlow;
    use horizon_client::block_entities::Pos;
    use horizon_client::containers::BlockEntity;
    use horizon_client::items::ItemStack;
    use horizon_core::geometry::Vec3;

    const A: Pos = (1, 64, 1);
    const B: Pos = (3, 64, 1);
    const C: Pos = (5, 64, 1);

    /// Notes the entities it is asked about and those it sees after their tick, and once, on its
    /// first visit, does `act`.
    struct Mutate {
        act: Option<fn(&mut BlockEntityTick<'_, '_>)>,
        before: Vec<Pos>,
        after: Vec<Pos>,
    }
    impl Mutate {
        fn new(act: fn(&mut BlockEntityTick<'_, '_>)) -> Self {
            Self {
                act: Some(act),
                before: Vec::new(),
                after: Vec::new(),
            }
        }
    }
    impl BlockEntityTickSlot for Mutate {
        fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
            self.before.push(tick.pos);
            if let Some(act) = self.act.take() {
                act(tick);
            }
            LoopFlow::Continue
        }
        fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
            self.after.push(tick.pos);
        }
    }

    fn place(game: &mut Game<'_>, pos: Pos, block: &str) {
        let state = game.pack.assets.registry.default_state(block).unwrap();
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        game.world
            .block_entities
            .insert(pos, BlockEntity::for_block(block).unwrap());
    }

    /// A server game with a player beside `A`, `B` and `C`, whose chunk ticks blocks, and
    /// `blocks` placed in order.
    fn with_blocks(blocks: &[(Pos, &str)], test: impl FnOnce(&mut Game<'_>)) {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 65.0, 8.5))
                .unwrap(),
        );
        game.update_chunk_tickets();
        assert!(game.block_ticking_area()(A.0, A.2));
        for (pos, block) in blocks {
            place(&mut game, *pos, block);
        }
        test(&mut game);
    }

    fn stone_above(game: &mut Game<'_>, pos: Pos) {
        let at = [
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 1.0,
            f64::from(pos.2) + 0.5,
        ];
        game.spawn_item(ItemStack::new("minecraft:stone", 1), at, [0.0; 3], 0);
    }
    fn hopper_count(game: &Game<'_>, pos: Pos) -> u32 {
        match game.world.block_entities.get(&pos) {
            Some(BlockEntity::Chest(items)) => items.iter().flatten().map(|s| s.count).sum(),
            other => panic!("no hopper at {pos:?}: {other:?}"),
        }
    }

    /// A slot that removes the entity it is asked about and answers Continue: it does not tick,
    /// and no `after` is called for it; the next one ticks.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_removed_current_entity_does_not_tick() {
        with_blocks(
            &[(A, "minecraft:hopper"), (B, "minecraft:hopper")],
            |game| {
                let mut slot = Mutate::new(|tick| {
                    tick.game.world.block_entities.remove(&tick.pos);
                });
                game.tick_block_entities_with(&mut slot).unwrap();
                assert_eq!(slot.before, [A, B]);
                assert_eq!(slot.after, [B]);
            },
        );
    }

    /// A slot that puts another kind of block entity in the visited one's place: the old kind's
    /// tick is not run on it, and no `after` is called for it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_replaced_current_entity_of_another_kind_does_not_tick() {
        with_blocks(
            &[(A, "minecraft:hopper"), (B, "minecraft:hopper")],
            |game| {
                stone_above(game, A);
                let mut slot = Mutate::new(|tick| {
                    let furnace = BlockEntity::for_block("minecraft:furnace").unwrap();
                    tick.game.world.block_entities.insert(tick.pos, furnace);
                });
                game.tick_block_entities_with(&mut slot).unwrap();
                assert_eq!(slot.before, [A, B]);
                assert_eq!(slot.after, [B]);
                assert!(matches!(
                    game.world.block_entities.get(&A),
                    Some(BlockEntity::Furnace(_))
                ));
                assert_eq!(game.dropped.len(), 1, "no hopper tick took the item");
            },
        );
    }

    /// One a slot adds is not visited in this pass (the pass's list was taken when it started)
    /// and is from the next.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_added_entity_ticks_from_the_next_pass() {
        with_blocks(
            &[(A, "minecraft:hopper"), (B, "minecraft:hopper")],
            |game| {
                let mut slot = Mutate::new(|tick| place(tick.game, C, "minecraft:hopper"));
                game.tick_block_entities_with(&mut slot).unwrap();
                assert_eq!(slot.before, [A, B]);
                assert_eq!(slot.after, [A, B]);
                let mut next = Mutate::new(|_| {});
                game.tick_block_entities_with(&mut next).unwrap();
                assert_eq!(next.before, [A, B, C]);
                assert_eq!(next.after, [A, B, C]);
            },
        );
    }

    /// A slot that runs the whole pass again from inside it: the nested pass ticks everything,
    /// then the enclosing pass goes on with its own scratch. Its later hopper takes an item, and
    /// its spawner list holds the spawner once, as its own pass ticked it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_nested_pass_leaves_the_enclosing_pass_its_scratch() {
        with_blocks(
            &[
                (A, "minecraft:hopper"),
                (B, "minecraft:spawner"),
                (C, "minecraft:hopper"),
            ],
            |game| {
                let mut slot = Mutate::new(|tick| {
                    tick.game.tick_block_entities().unwrap();
                    // The nested pass ticked the spawner; the outer one has not yet.
                    assert_eq!(tick.game.spawner_tick_positions, []);
                    assert!(tick.game.item_columns.is_some(), "the outer pass's columns");
                    stone_above(tick.game, C);
                });
                game.tick_block_entities_with(&mut slot).unwrap();
                assert_eq!(slot.before, [A, B, C]);
                assert_eq!(slot.after, [A, B, C]);
                assert_eq!(
                    hopper_count(game, C),
                    1,
                    "the outer pass's hopper took the item"
                );
                assert!(game.dropped.is_empty());
                assert_eq!(game.spawner_tick_positions, [B]);
                assert!(game.item_columns.is_none());
            },
        );
    }

    /// A slot that catches a nested pass's panic and goes on: the enclosing pass still has its own
    /// scratch, so the spawner it ticked is listed once and its later hopper takes an item.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_caught_nested_panic_leaves_the_enclosing_pass_its_scratch() {
        struct Boom;
        impl BlockEntityTickSlot for Boom {
            fn before(&mut self, _: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
                panic!("the nested pass's slot");
            }
        }
        struct Catch;
        impl BlockEntityTickSlot for Catch {
            fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
                if tick.pos == C {
                    assert_eq!(tick.game.spawner_tick_positions, [A]);
                    let nested = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        tick.game.tick_block_entities_with(&mut Boom)
                    }));
                    assert!(nested.is_err());
                    assert_eq!(tick.game.spawner_tick_positions, [A]);
                    assert!(tick.game.item_columns.is_some(), "the outer pass's columns");
                    assert!(tick.game.pass_mobs.is_some(), "the outer pass's mobs");
                    stone_above(tick.game, C);
                }
                LoopFlow::Continue
            }
        }
        with_blocks(
            &[
                (A, "minecraft:spawner"),
                (B, "minecraft:hopper"),
                (C, "minecraft:hopper"),
            ],
            |game| {
                game.tick_block_entities_with(&mut Catch).unwrap();
                assert_eq!(
                    hopper_count(game, C),
                    1,
                    "the outer pass's hopper took the item"
                );
                assert_eq!(game.spawner_tick_positions, [A]);
                assert!(game.item_columns.is_none());
            },
        );
    }
}

/// Slots that pick out their own mod's block entities (`OwnBlockEntities`).
#[cfg(test)]
mod own_block_entity_tests {
    use super::{BlockEntityTick, BlockEntityTickSlot, OwnBlockEntities, TickedKind};
    use crate::game::Game;
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};
    use crate::game::published_loops::LoopFlow;
    use horizon_client::block_entities::Pos;
    use horizon_client::containers::{BlockEntity, ModBlockEntity};
    use horizon_core::geometry::Vec3;
    use mod_api::*;
    use std::sync::{Arc, OnceLock};

    const A: &str = "own_a:machine";
    const B: &str = "own_b:machine";
    const DETECTOR: &str = "minecraft:daylight_detector";

    /// A mod whose block `block` makes its ticking block entity type of the same id, which is
    /// also valid on the block `also_on` (`BlockEntityType.validBlocks`, added by this mod).
    struct Machines {
        block: &'static str,
        also_on: Option<&'static str>,
        keys: OnceLock<(BlockEntityTypeKey, BlockKey)>,
    }
    impl Machines {
        fn new(block: &'static str, also_on: Option<&'static str>) -> Self {
            Self {
                block,
                also_on,
                keys: OnceLock::new(),
            }
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for Machines {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let kind = r.block_entity_type("machine").blocks(&[self.block]).build();
            r.block_entity_part(&kind, Ticks(self.block)).unwrap();
            if let Some(other) = self.also_on {
                let keys = (kind, r.block_key(other));
                assert_eq!(*self.keys.get_or_init(|| keys), keys);
            }
        }
        fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
            event
                .blocks
                .push(mod_api::Block::new(self.block, "minecraft:stone"));
        }
        fn block_entity_type_add_blocks(
            &self,
            _: &RegistrationContext,
            event: &mut BlockEntityTypeAddBlocksEvent<'_>,
        ) {
            if let Some(&(kind, block)) = self.keys.get() {
                event.modify(kind, &[block]);
            }
        }
    }
    /// Makes block `self.0`'s block entity tick, and notes each tick in the entity's data:
    /// [`mark`] of the type whose part ran.
    struct Ticks(&'static str);
    #[mod_api::horizon_part]
    impl BlockPart for Ticks {}
    #[mod_api::horizon_part]
    impl BlockEntityPart for Ticks {
        fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
            if event.block == self.0 {
                event.ticks = true;
            }
        }
        fn block_entity_tick(&self, _: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
            event.entity.data.push(mark(self.0));
        }
    }
    /// What a tick of mod A's type (1) or mod B's (2) leaves in a block entity's data.
    fn mark(kind: &str) -> i32 {
        if kind == A { 1 } else { 2 }
    }

    /// Counts what it is asked about, and notes those of its own types.
    struct Count {
        own: OwnBlockEntities,
        visited: u32,
        mine: Vec<Pos>,
    }
    impl Count {
        fn new(ids: &'static [&'static str]) -> Self {
            Self {
                own: OwnBlockEntities::new(ids),
                visited: 0,
                mine: Vec::new(),
            }
        }
    }
    impl BlockEntityTickSlot for Count {
        fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
            self.visited += 1;
            if self.own.contains(tick) {
                self.mine.push(tick.pos);
            }
            LoopFlow::Continue
        }
    }

    /// `mods` installed, a game on the prepared resources with a player over chunk (0, 0), whose
    /// blocks tick, and `placed` set there as a player's placement sets them.
    fn with_machines(mods: Mods, placed: &[(Pos, &str)], test: impl FnOnce(&mut Game<'_>)) {
        mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_parts(&mods).unwrap();
            mod_api::__private::mark_game_thread();
            let base =
                std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
            let root = crate::game::mods::content_root(&base, &Default::default()).unwrap();
            with_game_at(root.prepared().to_path_buf(), |game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                game.player = Some(
                    horizon_client::motion::PlayerMotion::new(
                        game.motion,
                        Vec3::new(8.5, 65.0, 8.5),
                    )
                    .unwrap(),
                );
                game.update_chunk_tickets();
                let level = game.level(game.dim.id()).unwrap();
                for &((x, y, z), block) in placed {
                    let state = game.pack.assets.registry.default_state(block).unwrap();
                    assert!(
                        game.level_set_block(level, [x, y, z], state, BlockUpdateFlags::NONE)
                            .unwrap()
                    );
                }
                test(game);
            });
        });
    }

    /// The block `block` at `pos` with a new block entity of the mod type `kind` on it, as a save
    /// that holds it loads it.
    fn place_mod_entity(game: &mut Game<'_>, pos: Pos, block: &str, kind: &str) {
        let state = game.pack.assets.registry.default_state(block).unwrap();
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        let entity = ModBlockEntity::new(kind.into(), Default::default());
        game.world
            .block_entities
            .insert(pos, BlockEntity::Mod(Box::new(entity)));
    }

    /// The data of the mod block entity at `pos`: a [`mark`] for each of its ticks.
    fn ticks_of(game: &Game<'_>, pos: Pos) -> Vec<i32> {
        match game.world.block_entities.get(&pos) {
            Some(BlockEntity::Mod(entity)) => entity.data.clone(),
            _ => panic!("no mod block entity at {pos:?}"),
        }
    }

    /// Two mods' block entities and a hopper in one world, one of mod B's standing on mod A's block
    /// (B made its type valid there): each mod's slot is asked about all of them and counts only
    /// its own entities, whatever block they stand on. An id with no type counts nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
    fn each_slot_counts_only_its_own_mods_block_entities() {
        // The content root freezes the process's block entity types.
        if run_fresh_cases(
            "game::block_entity_ticks::own_block_entity_tests::each_slot_counts_only_its_own_mods_block_entities",
            "HORIZON_OWN_BLOCK_ENTITIES_CHILD",
            &["run"],
        ) {
            return;
        }
        let mods: Mods = vec![
            ModEntry::new("own_a", Box::new(Machines::new(A, None))),
            ModEntry::new("own_b", Box::new(Machines::new(B, Some(A)))),
        ]
        .into();
        let (a, b, c) = ((1, 64, 1), (3, 64, 1), (5, 64, 1));
        let placed = [(a, A), (b, B), (c, A), ((7, 64, 1), "minecraft:hopper")];
        with_machines(mods, &placed, |game| {
            // Mod B's entity on mod A's block.
            let d = (9, 64, 1);
            assert!(horizon_client::block_entity_types::additional_valid(A, B));
            place_mod_entity(game, d, A, B);
            let mut slots = (
                Count::new(&[A]),
                (Count::new(&[B]), Count::new(&["own_a:missing"])),
            );
            for _ in 0..2 {
                game.tick_block_entities_with(&mut slots).unwrap();
            }
            let (own_a, (own_b, missing)) = &mut slots;
            assert_eq!(
                (own_a.visited, own_b.visited, missing.visited),
                (10, 10, 10)
            );
            own_a.mine.sort_unstable();
            own_b.mine.sort_unstable();
            assert_eq!(own_a.mine, [a, a, c, c]);
            assert_eq!(own_b.mine, [b, b, d, d]);
            assert_eq!(missing.mine, []);
        });
    }

    /// Mod B's block entity standing on a vanilla block, a daylight detector, that B made its type
    /// valid on, beside each mod's own block and a detector with its vanilla block entity: mod B's
    /// slot counts it, mod A's doesn't, and it ticks as B's. Neither counts the vanilla detector.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
    fn a_mods_block_entity_on_a_vanilla_block_is_only_its_own_mods() {
        // The content root freezes the process's block entity types.
        if run_fresh_cases(
            "game::block_entity_ticks::own_block_entity_tests::a_mods_block_entity_on_a_vanilla_block_is_only_its_own_mods",
            "HORIZON_OWN_BLOCK_ENTITIES_CHILD",
            &["run"],
        ) {
            return;
        }
        let mods: Mods = vec![
            ModEntry::new("own_a", Box::new(Machines::new(A, None))),
            ModEntry::new("own_b", Box::new(Machines::new(B, Some(DETECTOR)))),
        ]
        .into();
        let (a, b, vanilla) = ((1, 64, 1), (3, 64, 1), (7, 64, 1));
        with_machines(mods, &[(a, A), (b, B), (vanilla, DETECTOR)], |game| {
            assert!(matches!(
                game.world.block_entities.get(&vanilla),
                Some(BlockEntity::DaylightDetector)
            ));
            let d = (5, 64, 1);
            assert!(horizon_client::block_entity_types::additional_valid(
                DETECTOR, B
            ));
            assert!(!horizon_client::block_entity_types::additional_valid(
                DETECTOR, A
            ));
            place_mod_entity(game, d, DETECTOR, B);
            let mut slots = (Count::new(&[A]), Count::new(&[B]));
            for _ in 0..2 {
                game.tick_block_entities_with(&mut slots).unwrap();
            }
            let (own_a, own_b) = &mut slots;
            assert_eq!((own_a.visited, own_b.visited), (8, 8));
            own_a.mine.sort_unstable();
            own_b.mine.sort_unstable();
            assert_eq!(own_a.mine, [a, a]);
            assert_eq!(own_b.mine, [b, b, d, d]);
            assert_eq!(ticks_of(game, d), [2, 2]);
        });
    }

    /// Once, when asked about the block entity at `at`, replaces it in place (the loop's ticker
    /// stays live) with a new one of mod B's type and answers Continue. Notes the entities it
    /// sees after their tick.
    struct Swap {
        at: Option<Pos>,
        after: Vec<Pos>,
    }
    impl BlockEntityTickSlot for Swap {
        fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
            if self.at == Some(tick.pos) {
                self.at = None;
                let Some(BlockEntity::Mod(entity)) = tick.block_entity() else {
                    panic!("no mod block entity at {:?}", tick.pos);
                };
                **entity = ModBlockEntity::new(B.into(), Default::default());
            }
            LoopFlow::Continue
        }
        fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
            self.after.push(tick.pos);
        }
    }

    /// For each entry: its position, whether the cursor is of mod A's type and of mod B's, and
    /// the type of the block entity standing there.
    type Entry = (Pos, bool, bool, &'static str);

    /// Notes each entry, as [`Entry`], before and after its tick.
    struct Seen {
        a: OwnBlockEntities,
        b: OwnBlockEntities,
        before: Vec<Entry>,
        after: Vec<Entry>,
    }
    impl Seen {
        fn new() -> Self {
            Self {
                a: OwnBlockEntities::new(&[A]),
                b: OwnBlockEntities::new(&[B]),
                before: Vec::new(),
                after: Vec::new(),
            }
        }
        fn entry(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> Entry {
            assert_eq!(tick.kind, TickedKind::Mod);
            let standing = match tick.block_entity() {
                Some(BlockEntity::Mod(entity)) if entity.id() == A => A,
                Some(BlockEntity::Mod(entity)) if entity.id() == B => B,
                _ => "other",
            };
            (
                tick.pos,
                self.a.contains(tick),
                self.b.contains(tick),
                standing,
            )
        }
    }
    impl BlockEntityTickSlot for Seen {
        fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
            let entry = self.entry(tick);
            self.before.push(entry);
            LoopFlow::Continue
        }
        fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
            let entry = self.entry(tick);
            self.after.push(entry);
        }
    }

    /// A slot's `before` replaces mod A's block entity with one of mod B's type and continues: a
    /// later slot's `before` in the same entry still gets the cursor from before the edit (mod
    /// A's type, though B's entity stands there), the replaced entity does not tick, as A's or at
    /// all, and no slot's `after` sees it. The next pass visits it as B's.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
    fn an_entity_replaced_by_another_mods_type_does_not_tick_as_the_old_one() {
        // The content root freezes the process's block entity types.
        if run_fresh_cases(
            "game::block_entity_ticks::own_block_entity_tests::an_entity_replaced_by_another_mods_type_does_not_tick_as_the_old_one",
            "HORIZON_OWN_BLOCK_ENTITIES_CHILD",
            &["run"],
        ) {
            return;
        }
        let mods: Mods = vec![
            ModEntry::new("own_a", Box::new(Machines::new(A, None))),
            ModEntry::new("own_b", Box::new(Machines::new(B, Some(A)))),
        ]
        .into();
        let (a, c) = ((1, 64, 1), (3, 64, 1));
        with_machines(mods, &[(a, A), (c, A)], |game| {
            let mut slots = (
                Swap {
                    at: Some(a),
                    after: Vec::new(),
                },
                Seen::new(),
            );
            game.tick_block_entities_with(&mut slots).unwrap();
            let (swap, seen) = &mut slots;
            seen.before.sort_unstable();
            assert_eq!(seen.before, [(a, true, false, B), (c, true, false, A)]);
            assert_eq!(seen.after, [(c, true, false, A)]);
            assert_eq!(swap.after, [c]);
            assert!(ticks_of(game, a).is_empty());
            assert_eq!(ticks_of(game, c), [1]);

            let (swap, seen) = &mut slots;
            (swap.after, seen.before, seen.after) = Default::default();
            game.tick_block_entities_with(&mut slots).unwrap();
            let (swap, seen) = &mut slots;
            seen.before.sort_unstable();
            seen.after.sort_unstable();
            swap.after.sort_unstable();
            assert_eq!(seen.before, [(a, false, true, B), (c, true, false, A)]);
            assert_eq!(seen.after, [(a, false, true, B), (c, true, false, A)]);
            assert_eq!(swap.after, [a, c]);
            assert_eq!(ticks_of(game, a), [2]);
            assert_eq!(ticks_of(game, c), [1, 1]);
        });
    }
}
