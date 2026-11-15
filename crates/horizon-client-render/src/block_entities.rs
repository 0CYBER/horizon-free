//! The world's block entities and the order they tick in: vanilla's
//! `Level.blockEntityTickers`, kept beside the block entity map so a tick
//! visits only the block entities that tick, in vanilla's order, instead of
//! scanning the whole map once per kind.
//!
//! The list follows `LevelChunk.updateBlockEntityTicker` and
//! `Level.tickBlockEntities`:
//! - a block entity whose block state has a ticker gets a slot at the end of
//!   the list (through `pendingBlockEntityTickers`, so a slot made during a
//!   tick first ticks on the next one);
//! - replacing the block entity at a position (`setBlockEntity`), or changing
//!   its block state to another state with a ticker, keeps its slot
//!   (`RebindableTickingBlockEntityWrapper.rebind`);
//! - removing it (`removeBlockEntity`), unloading its chunk
//!   (`clearAllBlockEntities`) or changing its state to one without a ticker
//!   (a jukebox losing its record) frees the slot; a later ticker there is
//!   appended again;
//! - a loaded chunk's block entities are added in the iteration order of its
//!   `Object2ObjectOpenHashMap<BlockPos, BlockEntity>`
//!   (`registerAllBlockEntitiesAfterLevelLoad`), filled in the order the chunk
//!   listed them.
//!
//! Which states tick is a table by state id, built by the game from the block
//! registry; whether a slot is made is decided when the slot is promoted at
//! the start of the next tick, from the block state then.
use crate::chunk::block_entity::{BlockEntity, ModContainer, ModEnergyBuffer, ModFluidTanks};
use crate::fast_hash::{FxHashMap, FxHashSet};
use horizon_mod_runtime::attachments::{Attachments, observer::AttachmentObserver};
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

mod carry;
pub use carry::Carried;

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn next_token() -> u64 {
    let mut token = NEXT_TOKEN.load(Ordering::Relaxed);
    loop {
        let next = token
            .checked_add(1)
            .expect("block entity token space exhausted");
        match NEXT_TOKEN.compare_exchange_weak(token, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return token,
            Err(current) => token = current,
        }
    }
}

/// Exclusive authority to return one instance taken from a block entity store.
/// Invalidated by removal, replacement, unloading, or the next ticker sweep.
/// Dropping it does not restore the instance; it cannot be cloned or forged.
pub struct BlockEntityLease {
    pos: Pos,
    token: u64,
    generation: u64,
}

pub type Pos = (i32, i32, i32);

/// What a block state's block entity does each tick (`BlockState.getTicker`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ticking {
    /// No ticker, and the block has no block entity that could have one.
    #[default]
    No,
    /// A ticker.
    Yes,
    /// The block's block entity, but this state has no ticker (a jukebox
    /// without a record): a state change to it frees the slot.
    Idle,
}

#[derive(Default)]
pub struct BlockEntities {
    map: HashMap<Pos, BlockEntity>,
    generations: FxHashMap<Pos, u64>,
    on_load: std::collections::VecDeque<(Pos, u64)>,
    on_load_enabled: bool,
    attachments: FxHashMap<Pos, Attachments>,
    attachment_observer: Option<Arc<dyn AttachmentObserver<BlockEntityIdentity>>>,
    /// While a transaction is open (`defer_retirement`): the maps of block entities it
    /// removed or replaced, still bound to their index entries, by position and generation.
    /// The root commit retires them (`publish_retirement`); an abort puts back the ones it
    /// restores (`put_back_attachments`) first.
    retiring: Option<Vec<(Pos, u64, Attachments)>>,
    /// `Level.blockEntityTickers`: positions and the slot each was given.
    tickers: Arc<Vec<(Pos, u32)>>,
    /// `LevelChunk.tickersInLevel`: the live slot of each ticking position.
    live: FxHashMap<Pos, u32>,
    /// Positions that may need a slot, in the order vanilla would have made
    /// it (`pendingBlockEntityTickers`), each with a token; decided at the
    /// next promotion from the block state then.
    added: Vec<(Pos, u32)>,
    /// The token of each position's pending entry: an entry whose token is
    /// not here any more was cancelled.
    pending: FxHashMap<Pos, u32>,
    next_slot: u32,
    /// Whether a slot was freed since the list was last swept: otherwise
    /// every slot in it is still live, unless its block entity is `out`.
    freed: bool,
    /// Positions whose block entity `take` took and `put` has not brought
    /// back.
    out: FxHashMap<Pos, u64>,
    /// Positions a loaded chunk gave a new block entity over an old one: at
    /// the next tick their slot stays only if their block state ticks
    /// (`updateBlockEntityTicker`: rebind, or remove without a ticker).
    replaced: FxHashSet<Pos>,
    /// `Ticking` by block state id; empty until the game builds it.
    states: Vec<Ticking>,
    /// The containers of mod blocks' block entities, by block state id;
    /// built with `states`.
    mod_containers: FxHashMap<u32, Arc<ModContainer>>,
    /// The fluid tanks of mod blocks' block entities, by block state id;
    /// built with `states`, like the containers.
    mod_fluids: FxHashMap<u32, Arc<ModFluidTanks>>,
    /// State-aware native factory ownership. Shared across level snapshots;
    /// an empty slice means the declarations have not yet been compiled.
    stateful_entities: Arc<[bool]>,
    /// The energy buffers of mod blocks' block entities, by block state id;
    /// built with `states`, like the containers.
    mod_energy: FxHashMap<u32, Arc<ModEnergyBuffer>>,
    /// The positions of the mod block entities (`BlockEntity::Mod`) by chunk,
    /// for the renderer: the mesher lists block entities by their template,
    /// which leaves out a mod block whose template has none. One taken out
    /// (`take`) stays listed until `put` or `remove`.
    mods: FxHashMap<(i32, i32), FxHashSet<Pos>>,
    /// The positions of the block entities that draw a bounding box (structure and test
    /// instance blocks, `shows_box`), so a frame looks at them without scanning the map.
    /// Like `mods`, one taken out stays listed until `put` or `remove`.
    boxes: FxHashSet<Pos>,
    /// The signs a player's editor is on (`SignBlockEntity.playerWhoMayEdit`
    /// set), so the level's tick checks those alone instead of every sign.
    sign_editors: FxHashSet<Pos>,
    asleep: FxHashSet<Pos>,
    /// The block entities moving pistons carry, by the moving piston's
    /// position (`carry`): out of the map until they land.
    carried: FxHashMap<Pos, carry::Carried>,
    /// The block entity a player's break just removed, kept readable while that break's
    /// `block_drops` listeners run (`hold_broken`), and dropped after (`release_broken`).
    broken: Option<Broken>,
    /// The API snapshots a client's mod block entities are ticked with
    /// (`take_client_snapshots`), by position.
    client_snapshots: FxHashMap<Pos, ClientSnapshot>,
}

/// A removed block entity held for the `block_drops` event of its break: its position, the
/// identity it had and its saved data, read before the removal took its `CustomName` away.
pub struct Broken {
    pub pos: Pos,
    pub generation: u64,
    pub data: horizon_core::nbt::Compound,
}

/// A client copy of a mod block entity as its client tick shows it to mods: made once for each
/// revision of the copy (an update merged into it is a new one), not once a tick.
pub struct ClientSnapshot {
    /// The copy's revision the snapshot was made from.
    pub revision: crate::chunk::block_entity::ModRevision,
    /// The snapshot, or `None` when that revision could not be made into one.
    pub entity: Option<mod_api::ModBlockEntity>,
}

/// Storage-owned identity passed to attachment residency observers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct BlockEntityIdentity {
    pub position: [i32; 3],
    pub generation: u64,
}

fn identity(pos: Pos, generations: &FxHashMap<Pos, u64>) -> BlockEntityIdentity {
    BlockEntityIdentity {
        position: [pos.0, pos.1, pos.2],
        generation: generations[&pos],
    }
}

fn retire_attachments(
    maps: &mut FxHashMap<Pos, Attachments>,
    observer: Option<&Arc<dyn AttachmentObserver<BlockEntityIdentity>>>,
    generations: &FxHashMap<Pos, u64>,
    pos: &Pos,
) {
    if let Some(previous) = maps.remove(pos) {
        if let Some(observer) = observer {
            observer.unload(identity(*pos, generations));
        }
        drop(previous);
    }
}

impl Drop for BlockEntities {
    fn drop(&mut self) {
        self.publish_retirement();
        self.unload_attachments();
    }
}

/// A structure or test instance block: the block entities whose box the frame may draw.
fn shows_box(entity: &BlockEntity) -> bool {
    matches!(
        entity,
        BlockEntity::StructureBlock(_) | BlockEntity::TestInstanceBlock(_)
    )
}

fn chunk_of((x, _, z): Pos) -> (i32, i32) {
    (x >> 4, z >> 4)
}

/// Takes `pos` out of its chunk's mod block entity set.
fn forget_mod(mods: &mut FxHashMap<(i32, i32), FxHashSet<Pos>>, pos: Pos) {
    let chunk = chunk_of(pos);
    if let Some(set) = mods.get_mut(&chunk) {
        set.remove(&pos);
        if set.is_empty() {
            mods.remove(&chunk);
        }
    }
}

impl std::ops::Deref for BlockEntities {
    type Target = HashMap<Pos, BlockEntity>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl<'a> IntoIterator for &'a BlockEntities {
    type Item = (&'a Pos, &'a BlockEntity);
    type IntoIter = std::collections::hash_map::Iter<'a, Pos, BlockEntity>;
    fn into_iter(self) -> Self::IntoIter {
        self.map.iter()
    }
}

/// `Vec3i.hashCode`.
fn java_hash((x, y, z): Pos) -> i32 {
    y.wrapping_add(z.wrapping_mul(31))
        .wrapping_mul(31)
        .wrapping_add(x)
}
/// `HashCommon.mix`.
fn mix(x: i32) -> usize {
    let h = (x as u32).wrapping_mul(0x9E37_79B9);
    (h ^ (h >> 16)) as usize
}

/// The order `values().forEach` visits a `new Object2ObjectOpenHashMap<>()`
/// keyed by these positions, put in this order (repeats keep their first
/// slot): open addressing with linear probing, a rehash to twice the size
/// past three quarters full, and iteration from the last slot down.
pub fn chunk_map_order(positions: impl IntoIterator<Item = Pos>) -> Vec<Pos> {
    // HashCommon.arraySize(16, .75f), and maxFill of each size.
    let mut slots: Vec<Option<Pos>> = vec![None; 32];
    let max_fill = |n: usize| (n * 3).div_ceil(4).min(n - 1);
    let mut size = 0;
    for p in positions {
        let mask = slots.len() - 1;
        let mut at = mix(java_hash(p)) & mask;
        let mut known = false;
        while let Some(k) = slots[at] {
            if k == p {
                known = true;
                break;
            }
            at = (at + 1) & mask;
        }
        if known {
            continue;
        }
        slots[at] = Some(p);
        // `if (size++ >= maxFill) rehash(arraySize(size + 1, f))`, which
        // puts the keys back from the last slot down.
        size += 1;
        if size > max_fill(slots.len()) {
            let n = ((size + 1) * 4).div_ceil(3).next_power_of_two();
            let old = std::mem::replace(&mut slots, vec![None; n]);
            for k in old.into_iter().rev().flatten() {
                let mut at = mix(java_hash(k)) & (n - 1);
                while slots[at].is_some() {
                    at = (at + 1) & (n - 1);
                }
                slots[at] = Some(k);
            }
        }
    }
    slots.into_iter().rev().flatten().collect()
}

impl BlockEntities {
    /// Gameplay supplies its subscription decision; storage knows no hook identifiers.
    pub fn set_on_load_enabled(&mut self, enabled: bool) {
        self.on_load_enabled = enabled;
        if !enabled {
            self.on_load.clear();
        }
    }
    /// Whether any attachment notices remain, including possibly retired identities.
    pub fn has_pending_on_load(&self) -> bool {
        !self.on_load.is_empty()
    }
    /// Number of queued notices, including retired entries, at a drain boundary.
    pub fn pending_on_load_len(&self) -> usize {
        self.on_load.len()
    }
    /// Take a live notice from a bounded prefix, counting stale entries too.
    pub fn next_on_load_in_batch(&mut self, remaining: &mut usize) -> Option<(Pos, u64)> {
        while *remaining > 0 {
            *remaining -= 1;
            let (pos, generation) = self.on_load.pop_front()?;
            if self.generation(&pos) == Some(generation)
                && matches!(self.map.get(&pos), Some(BlockEntity::Mod(_)))
            {
                return Some((pos, generation));
            }
        }
        None
    }
    /// Take the next newly attached mod identity. Removed or replaced entries
    /// are stale and skipped; payload rewrites retain the queued generation.
    pub fn next_on_load(&mut self) -> Option<(Pos, u64)> {
        let mut remaining = self.on_load.len();
        self.next_on_load_in_batch(&mut remaining)
    }

    /// Runtime identity, retained while an entity is temporarily taken for its tick.
    pub fn generation(&self, pos: &Pos) -> Option<u64> {
        self.generations.get(pos).copied()
    }
    /// Restore the identity captured with an existing entity by a transaction.
    pub fn restore_generation(&mut self, pos: Pos, generation: u64) {
        assert!(self.map.contains_key(&pos));
        if self.generation(&pos) != Some(generation) {
            retire_attachments(
                &mut self.attachments,
                self.attachment_observer.as_ref(),
                &self.generations,
                &pos,
            );
            self.generations.insert(pos, generation);
        }
    }
    fn new_generation(&mut self, pos: Pos) {
        self.generations.insert(pos, next_token());
    }
    /// Bind this dimension's authoritative maps, including saved entries not yet read.
    pub fn observe_attachments(
        &mut self,
        observer: Arc<dyn AttachmentObserver<BlockEntityIdentity>>,
    ) {
        self.unload_attachments();
        for (pos, map) in &mut self.attachments {
            if map.has_index_entries() || map.has_index_binding() {
                let _ = observer.bind(identity(*pos, &self.generations), map);
            }
        }
        self.attachment_observer = Some(observer);
    }

    fn unload_attachments(&self) {
        if let Some(observer) = &self.attachment_observer {
            for pos in self.attachments.keys() {
                observer.unload(identity(*pos, &self.generations));
            }
        }
    }

    /// The client tick snapshots, lent for one pass over the tickers: give them back with
    /// `keep_client_snapshots`.
    pub fn take_client_snapshots(&mut self) -> FxHashMap<Pos, ClientSnapshot> {
        std::mem::take(&mut self.client_snapshots)
    }
    /// Takes back what `take_client_snapshots` lent, without the positions that no longer tick.
    pub fn keep_client_snapshots(&mut self, mut snapshots: FxHashMap<Pos, ClientSnapshot>) {
        snapshots.retain(|pos, _| self.live.contains_key(pos));
        self.client_snapshots = snapshots;
    }
    /// Whether the ticking table has been built.
    pub fn has_states(&self) -> bool {
        !self.states.is_empty()
    }
    pub fn set_states(&mut self, states: Vec<Ticking>) {
        self.states = states;
    }
    pub fn ticking(&self, state: u32) -> Ticking {
        self.states.get(state as usize).copied().unwrap_or_default()
    }
    pub fn set_mod_containers(&mut self, containers: FxHashMap<u32, Arc<ModContainer>>) {
        self.mod_containers = containers;
    }
    /// The container of a mod block's block entity in block state `state`.
    pub fn mod_container(&self, state: u32) -> Option<&Arc<ModContainer>> {
        self.mod_containers.get(&state)
    }
    pub fn set_mod_fluids(&mut self, fluids: FxHashMap<u32, Arc<ModFluidTanks>>) {
        self.mod_fluids = fluids;
    }
    /// The fluid tanks of a mod block's block entity in block state `state`.
    pub fn mod_fluids(&self, state: u32) -> Option<&Arc<ModFluidTanks>> {
        self.mod_fluids.get(&state)
    }
    pub fn set_stateful_entities(&mut self, states: Vec<bool>) {
        self.stateful_entities = states.into();
    }
    pub fn has_stateful_entity_table(&self) -> bool {
        !self.stateful_entities.is_empty()
    }
    pub fn stateful_entities(&self) -> Arc<[bool]> {
        self.stateful_entities.clone()
    }
    pub fn stateful_entity(&self, state: u32) -> bool {
        self.stateful_entities
            .get(state as usize)
            .copied()
            .unwrap_or(false)
    }
    pub fn set_mod_energy(&mut self, energy: FxHashMap<u32, Arc<ModEnergyBuffer>>) {
        self.mod_energy = energy;
    }
    /// The energy buffer of a mod block's block entity in block state `state`.
    pub fn mod_energy(&self, state: u32) -> Option<&Arc<ModEnergyBuffer>> {
        self.mod_energy.get(&state)
    }

    fn next(&mut self) -> u32 {
        let n = self.next_slot;
        self.next_slot = n.wrapping_add(1);
        n
    }
    /// A ticker may be made for `pos` now (`updateBlockEntityTicker`).
    fn add(&mut self, pos: Pos) {
        let token = self.next();
        self.pending.insert(pos, token);
        self.added.push((pos, token));
    }
    /// `removeBlockEntityTicker`: no slot, made or pending.
    fn cancel(&mut self, pos: &Pos) {
        self.asleep.remove(pos);
        self.freed |= self.live.remove(pos).is_some();
        self.pending.remove(pos);
    }

    /// Attachment map for a live block entity.
    pub fn attachments_mut(
        &mut self,
        pos: Pos,
    ) -> Option<&mut horizon_mod_runtime::attachments::Attachments> {
        // Identity survives a temporary take and retires on removal or an
        // abandoned-lease sweep, exactly like attachment ownership.
        if !self.generations.contains_key(&pos) {
            return None;
        }
        let map = self.attachments.entry(pos).or_default();
        if let Some(observer) = &self.attachment_observer
            && !map.has_index_binding()
        {
            let _ = observer.bind(identity(pos, &self.generations), map);
        }
        Some(map)
    }
    /// The attachment map of the block entity at `pos`, if it has one.
    pub fn attachments(&self, pos: Pos) -> Option<&horizon_mod_runtime::attachments::Attachments> {
        self.attachments.get(&pos)
    }
    /// Puts the attachment map of the live block entity at `pos` back as `data`, or removes it
    /// where it had none: a transaction's abort.
    pub fn set_attachments(
        &mut self,
        pos: Pos,
        data: Option<horizon_mod_runtime::attachments::Attachments>,
    ) {
        match data {
            Some(data) => self.restore_attachments(pos, data),
            None => {
                retire_attachments(
                    &mut self.attachments,
                    self.attachment_observer.as_ref(),
                    &self.generations,
                    &pos,
                );
            }
        }
    }
    /// Retires the map of the block entity leaving `pos`: at once, or, while a transaction
    /// defers retirement, kept bound until it closes.
    fn retire(&mut self, pos: Pos) {
        if let Some(retiring) = &mut self.retiring {
            if let (Some(map), Some(&generation)) =
                (self.attachments.remove(&pos), self.generations.get(&pos))
            {
                retiring.push((pos, generation, map));
            }
            return;
        }
        retire_attachments(
            &mut self.attachments,
            self.attachment_observer.as_ref(),
            &self.generations,
            &pos,
        );
    }
    /// From now until `publish_retirement`, a block entity that goes keeps its map bound to
    /// its index entries: the root transaction opened, and only its commit publishes what it
    /// removed.
    pub fn defer_retirement(&mut self) {
        self.retiring.get_or_insert_with(Vec::new);
    }
    /// Whether a transaction is open: between `defer_retirement` and `publish_retirement`.
    pub fn defers_retirement(&self) -> bool {
        self.retiring.is_some()
    }
    /// Retires every map kept since `defer_retirement`, and stops deferring: the root
    /// transaction closed. What an abort did not put back goes: those of block entities a
    /// committed transaction removed, and those of block entities an aborted one made.
    pub fn publish_retirement(&mut self) {
        for (pos, generation, map) in self.retiring.take().into_iter().flatten() {
            if let Some(observer) = &self.attachment_observer {
                observer.unload(BlockEntityIdentity {
                    position: [pos.0, pos.1, pos.2],
                    generation,
                });
            }
            drop(map);
        }
    }
    /// Gives the block entity of `generation` that stands at `pos` again the map it went with
    /// in this transaction, as it was then, still bound: a removed block entity a later write
    /// in the same transaction put back.
    pub fn reattach_retired(&mut self, pos: Pos, generation: u64) {
        if self.generation(&pos) != Some(generation) || self.attachments.contains_key(&pos) {
            return;
        }
        let Some(retiring) = &mut self.retiring else {
            return;
        };
        if let Some(at) = retiring
            .iter()
            .position(|(p, g, _)| *p == pos && *g == generation)
        {
            let (_, _, map) = retiring.swap_remove(at);
            self.attachments.insert(pos, map);
        }
    }
    /// Puts the attachment map of the block entity at `pos` back as a transaction kept it,
    /// `data`, for the block entity of `generation` that stands there again: the very map it
    /// had, still bound, where it went in this transaction, so its index entries never left.
    pub fn put_back_attachments(
        &mut self,
        pos: Pos,
        generation: Option<u64>,
        data: Option<horizon_mod_runtime::attachments::Attachments>,
    ) {
        let kept = self.retiring.as_mut().and_then(|retiring| {
            let at = retiring
                .iter()
                .position(|(p, g, _)| *p == pos && Some(*g) == generation)?;
            Some(retiring.swap_remove(at).2)
        });
        match (kept, data) {
            (Some(mut map), Some(data)) if self.generation(&pos) == generation => {
                map.replace_contents(data);
                if let Some(current) = self.attachments.insert(pos, map) {
                    // A map the transaction made for the same block entity after it went:
                    // never bound apart from this one, which holds the index entries.
                    drop(current);
                }
            }
            (Some(map), _) => {
                if let (Some(observer), Some(generation)) = (&self.attachment_observer, generation)
                {
                    observer.unload(BlockEntityIdentity {
                        position: [pos.0, pos.1, pos.2],
                        generation,
                    });
                }
                drop(map);
                self.set_attachments(pos, None);
            }
            (None, data) => self.set_attachments(pos, data),
        }
    }
    /// Attachment maps in a loaded chunk, for a save snapshot.
    pub fn chunk_attachments(
        &self,
        x: i32,
        z: i32,
    ) -> Vec<(Pos, horizon_mod_runtime::attachments::Attachments)> {
        self.attachments
            .iter()
            .filter(|(p, _)| p.0 >> 4 == x && p.2 >> 4 == z)
            .map(|(p, data)| (*p, data.clone()))
            .collect()
    }
    /// Restore a loaded block entity's attachment map.
    pub fn restore_attachments(
        &mut self,
        pos: Pos,
        data: horizon_mod_runtime::attachments::Attachments,
    ) {
        if self.map.contains_key(&pos) || self.out.contains_key(&pos) {
            let map = self.attachments.entry(pos).or_default();
            map.replace_contents(data);
            if let Some(observer) = &self.attachment_observer
                && map.has_index_entries()
                && !map.has_index_binding()
            {
                let _ = observer.bind(identity(pos, &self.generations), map);
            }
        }
    }
    /// Whether any mod block entity is in the map: until one is, a block change
    /// never looks one up (`LevelAccess::has_mod_block_entity`).
    pub fn has_mods(&self) -> bool {
        !self.mods.is_empty()
    }
    /// The structure and test instance blocks (a frame draws their boxes), in no order.
    pub fn boxes(&self) -> impl Iterator<Item = (&Pos, &BlockEntity)> {
        self.boxes
            .iter()
            .filter_map(|pos| Some((pos, self.map.get(pos)?)))
    }
    /// Keeps `pos` listed in `boxes` exactly while `entity` shows a box.
    fn note_box(&mut self, pos: Pos, entity: Option<&BlockEntity>) {
        if entity.is_some_and(shows_box) {
            self.boxes.insert(pos);
        } else {
            self.boxes.remove(&pos);
        }
    }
    /// The mod block entities within `radius` blocks of `x`, `z` (by chunk, so
    /// far ones are skipped whole; the caller checks each distance).
    pub fn mods_near(&self, x: f64, z: f64, radius: f64) -> impl Iterator<Item = &Pos> {
        let far = move |c: i32, at: f64| {
            let (lo, hi) = (f64::from(c * 16), f64::from(c * 16 + 16));
            at < lo - radius || at > hi + radius
        };
        self.mods
            .iter()
            .filter(move |((cx, cz), _)| !far(*cx, x) && !far(*cz, z))
            .flat_map(|(_, set)| set)
    }
    /// The mod block entities of chunk `chunk`, in no order; one taken out is
    /// listed too.
    pub fn mods_in(&self, chunk: (i32, i32)) -> impl Iterator<Item = &Pos> {
        self.mods.get(&chunk).into_iter().flatten()
    }
    /// `SignBlockEntity.playerWhoMayEdit` of the sign at `pos`.
    pub fn sign_editor(&self, pos: &Pos) -> Option<u128> {
        match self.map.get(pos) {
            Some(BlockEntity::Sign(sign)) => sign.allowed_editor,
            _ => None,
        }
    }
    /// `SignBlockEntity.setAllowedPlayerEditor`: the one player whose editor
    /// is on the sign at `pos`, or `None` for nobody's.
    pub fn set_sign_editor(&mut self, pos: Pos, who: Option<u128>) {
        let Some(BlockEntity::Sign(sign)) = self.map.get_mut(&pos) else {
            return;
        };
        sign.allowed_editor = who;
        if who.is_some() {
            self.sign_editors.insert(pos);
        } else {
            self.sign_editors.remove(&pos);
        }
    }
    /// The positions of the signs a player's editor is on, in no order.
    pub fn sign_editors(&self) -> impl Iterator<Item = &Pos> {
        self.sign_editors.iter()
    }
    /// Drops `pos` from the signs an editor is on, for a sign that is gone or
    /// no longer holds one. `set_sign_editor` keeps the list itself.
    pub fn forget_sign_editor(&mut self, pos: Pos) {
        self.sign_editors.remove(&pos);
    }
    /// Keeps `pos` listed in `sign_editors` exactly while `entity` holds an
    /// editor, as `note_box` does for the boxes.
    fn note_sign_editor(&mut self, pos: Pos, entity: &BlockEntity) {
        if matches!(entity, BlockEntity::Sign(sign) if sign.allowed_editor.is_some()) {
            self.sign_editors.insert(pos);
        } else {
            self.sign_editors.remove(&pos);
        }
    }
    /// `LevelChunk.getBlockEntity`: the block entity at `pos`, if any.
    pub fn get(&self, pos: &Pos) -> Option<&BlockEntity> {
        self.map.get(pos)
    }
    pub fn get_mut(&mut self, pos: &Pos) -> Option<&mut BlockEntity> {
        self.asleep.remove(pos);
        self.map.get_mut(pos)
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Pos, &mut BlockEntity)> {
        self.asleep.clear();
        self.map.iter_mut()
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut BlockEntity> {
        self.asleep.clear();
        self.map.values_mut()
    }
    pub fn sleep(&mut self, pos: Pos) {
        if self.map.contains_key(&pos) {
            self.asleep.insert(pos);
        }
    }
    pub fn is_asleep(&self, pos: &Pos) -> bool {
        self.asleep.contains(pos)
    }
    fn replace_taken(&mut self, pos: Pos) {
        if self.out.remove(&pos).is_some() {
            self.replaced.insert(pos);
        }
    }
    /// `LevelChunk.setBlockEntity` (+ `addAndRegisterBlockEntity`): a new
    /// block entity may get a slot; one replacing another keeps its slot.
    /// One of the same type rewrites it in place (a jukebox taking a disc,
    /// `/data merge block`), where vanilla keeps the object, so its
    /// attachments stay; any other replacement starts them over.
    pub fn insert(&mut self, pos: Pos, entity: BlockEntity) -> Option<BlockEntity> {
        self.insert_impl(pos, entity, false)
    }
    /// Attach a different instance, even when its type matches the old instance.
    /// Payload-only writes use `insert` and retain their identity instead.
    pub fn replace(&mut self, pos: Pos, entity: BlockEntity) -> Option<BlockEntity> {
        self.insert_impl(pos, entity, true)
    }
    fn insert_impl(
        &mut self,
        pos: Pos,
        entity: BlockEntity,
        replace_identity: bool,
    ) -> Option<BlockEntity> {
        let new_is_mod = matches!(entity, BlockEntity::Mod(_));
        let same_type = |old: &BlockEntity| match (old, &entity) {
            (BlockEntity::Mod(old), BlockEntity::Mod(new)) => old.id() == new.id(),
            (old, new) => std::mem::discriminant(old) == std::mem::discriminant(new),
        };
        let new_identity = replace_identity || !self.map.get(&pos).is_some_and(same_type);
        if new_identity {
            self.retire(pos);
            self.new_generation(pos);
        }
        self.replace_taken(pos);
        let new_box = shows_box(&entity);
        self.note_sign_editor(pos, &entity);
        self.asleep.remove(&pos);
        let old = self.map.insert(pos, entity);
        if self.on_load_enabled && new_identity && new_is_mod {
            self.on_load.push_back((pos, self.generations[&pos]));
        }
        if old.is_none() {
            self.add(pos);
        }
        if new_box {
            self.boxes.insert(pos);
        } else {
            self.boxes.remove(&pos);
        }
        if new_is_mod {
            self.mods.entry(chunk_of(pos)).or_default().insert(pos);
        } else {
            forget_mod(&mut self.mods, pos);
        }
        old
    }
    /// The block entity at `pos`, made by `make` when there is none.
    pub fn get_or_insert_with(
        &mut self,
        pos: Pos,
        make: impl FnOnce() -> BlockEntity,
    ) -> &mut BlockEntity {
        let absent = !self.map.contains_key(&pos);
        if absent {
            self.replace_taken(pos);
            self.retire(pos);
            self.new_generation(pos);
            forget_mod(&mut self.mods, pos);
            self.boxes.remove(&pos);
            self.add(pos);
        }
        self.asleep.remove(&pos);
        let entity = self.map.entry(pos).or_insert_with(make);
        if absent && matches!(entity, BlockEntity::Mod(_)) {
            if self.on_load_enabled {
                self.on_load.push_back((pos, self.generations[&pos]));
            }
            self.mods.entry(chunk_of(pos)).or_default().insert(pos);
        }
        if absent && shows_box(entity) {
            self.boxes.insert(pos);
        }
        entity
    }
    /// Keep `data`, the saved data (with its `CustomName`) of the block entity just removed from
    /// `pos`, where it had `generation`, readable through [`Self::broken`] until
    /// [`Self::release_broken`]. The map and the generations do not know it: a handle to it
    /// answers reads, and nothing else. One slot: a second hold replaces the first. Listeners of
    /// the event it serves read through a shared host and cannot start another break, so the two
    /// do not nest today; a hook that could would hold per break instead.
    pub fn hold_broken(&mut self, pos: Pos, generation: u64, data: horizon_core::nbt::Compound) {
        self.broken = Some(Broken {
            pos,
            generation,
            data,
        });
    }
    /// The data [`Self::hold_broken`] holds, if it is that of the block entity at `pos` with
    /// `generation`.
    pub fn broken(&self, pos: &Pos, generation: u64) -> Option<&Broken> {
        self.broken
            .as_ref()
            .filter(|held| held.pos == *pos && held.generation == generation)
    }
    /// The generation of the block entity held at `pos`, if any.
    pub fn broken_generation(&self, pos: &Pos) -> Option<u64> {
        self.broken
            .as_ref()
            .filter(|held| held.pos == *pos)
            .map(|held| held.generation)
    }
    /// Drop the held data: every handle to it is stale from now on.
    pub fn release_broken(&mut self) {
        self.broken = None;
    }
    /// `LevelChunk.removeBlockEntity`: the block entity and its slot.
    pub fn remove(&mut self, pos: &Pos) -> Option<BlockEntity> {
        self.out.remove(pos);
        self.retire(*pos);
        self.generations.remove(pos);
        self.asleep.remove(pos);
        self.cancel(pos);
        let removed = self.map.remove(pos);
        forget_mod(&mut self.mods, *pos);
        self.boxes.remove(pos);
        self.sign_editors.remove(pos);
        removed
    }
    /// The block entity at `pos`, out of the map for a while: its slot stays
    /// for `put` to bring it back (a tick working on it).
    pub fn take(&mut self, pos: &Pos) -> Option<(BlockEntityLease, BlockEntity)> {
        self.asleep.remove(pos);
        let entity = self.map.remove(pos)?;
        let token = next_token();
        self.out.insert(*pos, token);
        let generation = self.generations[pos];
        Some((
            BlockEntityLease {
                pos: *pos,
                token,
                generation,
            },
            entity,
        ))
    }
    /// Re-key a live lease when an intentional state change keeps the instance.
    /// Ordinary removal still revokes it; a retired or foreign lease cannot re-key.
    pub fn rekey(&mut self, pos: &Pos, lease: &mut BlockEntityLease) -> bool {
        if lease.pos != *pos
            || self.out.get(pos) != Some(&lease.token)
            || self.generation(pos) != Some(lease.generation)
        {
            return false;
        }
        lease.token = next_token();
        self.out.insert(*pos, lease.token);
        true
    }
    /// Return the taken instance in its slot; false if its lease was retired.
    pub fn put(&mut self, lease: BlockEntityLease, entity: BlockEntity) -> bool {
        let pos = lease.pos;
        if self.out.get(&pos) != Some(&lease.token)
            || self.generation(&pos) != Some(lease.generation)
        {
            return false;
        }
        self.out.remove(&pos);
        self.asleep.remove(&pos);
        if matches!(entity, BlockEntity::Mod(_)) {
            self.mods.entry(chunk_of(pos)).or_default().insert(pos);
        }
        self.note_box(pos, Some(&entity));
        self.note_sign_editor(pos, &entity);
        if self.map.insert(pos, entity).is_none()
            && !self.live.contains_key(&pos)
            && !self.pending.contains_key(&pos)
        {
            self.add(pos);
        }
        true
    }
    fn retire_taken(&mut self, mut gone: impl FnMut(&Pos) -> bool) {
        let mut out = std::mem::take(&mut self.out);
        out.retain(|pos, _| {
            if gone(pos) {
                self.remove(pos);
                false
            } else {
                true
            }
        });
        self.out = out;
    }
    /// [`Self::retain`] for unloading chunks: removes the block entities at
    /// the positions `gone` names and changes none it keeps, so those that
    /// sleep stay asleep (a moving player unloads chunks most ticks).
    pub fn unload(&mut self, mut gone: impl FnMut(&Pos) -> bool) {
        self.carried.retain(|p, _| !gone(p));
        let asleep = std::mem::take(&mut self.asleep);
        self.retain(|p, _| !gone(p));
        self.retire_taken(gone);
        self.asleep = asleep;
        let map = &self.map;
        self.asleep.retain(|p| map.contains_key(p));
    }
    /// Removes the block entities `keep` refuses, and their slots
    /// (`clearAllBlockEntities` of unloaded chunks).
    pub fn retain(&mut self, mut keep: impl FnMut(&Pos, &mut BlockEntity) -> bool) {
        let (live, pending, freed, attachments) = (
            &mut self.live,
            &mut self.pending,
            &mut self.freed,
            &mut self.attachments,
        );
        let (mods, boxes) = (&mut self.mods, &mut self.boxes);
        let generations = &mut self.generations;
        let observer = self.attachment_observer.as_ref();
        let sign_editors = &mut self.sign_editors;
        self.asleep.clear();
        self.map.retain(|p, e| {
            let kept = keep(p, e);
            if !kept {
                retire_attachments(attachments, observer, generations, p);
                generations.remove(p);
                *freed |= live.remove(p).is_some();
                pending.remove(p);
                forget_mod(mods, *p);
                boxes.remove(p);
                sign_editors.remove(p);
            }
            kept
        });
    }
    pub fn clear(&mut self) {
        self.unload_attachments();
        self.generations.clear();
        self.on_load.clear();
        self.attachments.clear();
        self.map.clear();
        self.mods.clear();
        self.boxes.clear();
        self.sign_editors.clear();
        self.asleep.clear();
        self.tickers = Arc::default();
        self.live.clear();
        self.added.clear();
        self.pending.clear();
        self.out.clear();
        self.replaced.clear();
        self.carried.clear();
        self.client_snapshots.clear();
        self.freed = false;
    }
    /// A loaded or generated chunk's block entities, in the order the chunk
    /// lists them; they tick in its block entity map's order
    /// (`registerAllBlockEntitiesAfterLevelLoad`). One replacing an entity
    /// already at its position (`LevelChunk.setBlockEntity`) drops that one;
    /// its slot is decided by the block state at the next tick.
    pub fn load_chunk(&mut self, entities: Vec<(Pos, BlockEntity)>) {
        let order = chunk_map_order(entities.iter().map(|(p, _)| *p));
        for (p, e) in entities {
            self.replace_taken(p);
            retire_attachments(
                &mut self.attachments,
                self.attachment_observer.as_ref(),
                &self.generations,
                &p,
            );
            self.new_generation(p);
            self.asleep.remove(&p);
            let new_is_mod = matches!(e, BlockEntity::Mod(_));
            self.note_box(p, Some(&e));
            self.note_sign_editor(p, &e);
            let old = self.map.insert(p, e);
            if old.is_some() {
                self.replaced.insert(p);
            }
            if new_is_mod {
                self.mods.entry(chunk_of(p)).or_default().insert(p);
            } else {
                forget_mod(&mut self.mods, p);
            }
        }
        for p in order {
            if self.on_load_enabled && matches!(self.map.get(&p), Some(BlockEntity::Mod(_))) {
                self.on_load.push_back((p, self.generations[&p]));
            }
            self.add(p);
        }
    }
    /// `LevelChunk.setBlockState` on a block entity's position, from state
    /// `old` to `new`: a ticker made, kept or dropped.
    pub fn state_changed(&mut self, pos: Pos, old: u32, new: u32) {
        self.asleep.remove(&pos);
        if self.states.is_empty() || !self.map.contains_key(&pos) {
            return;
        }
        match self.ticking(new) {
            // A new ticker, unless one was made or is pending (rebound).
            Ticking::Yes => {
                if self.ticking(old) != Ticking::Yes && !self.live.contains_key(&pos) {
                    self.add(pos);
                }
            }
            Ticking::Idle => self.cancel(&pos),
            // Another block: the game removes or replaces the block entity.
            Ticking::No => {}
        }
    }
    /// The start of `Level.tickBlockEntities`: pending slots join the list
    /// and freed ones leave it. `state` reads the block state at a position.
    /// Returns the slots to tick, in order; check each with `is_live` before
    /// ticking it, as the ticks before it can free it.
    pub fn tickers(&mut self, state: impl Fn(Pos) -> u32) -> Arc<Vec<(Pos, u32)>> {
        for (pos, token) in std::mem::take(&mut self.added) {
            if self.pending.get(&pos) != Some(&token) {
                continue;
            }
            self.pending.remove(&pos);
            if self.live.contains_key(&pos)
                || !self.map.contains_key(&pos)
                || self.ticking(state(pos)) != Ticking::Yes
            {
                continue;
            }
            let slot = self.next();
            self.live.insert(pos, slot);
            Arc::make_mut(&mut self.tickers).push((pos, slot));
        }
        for pos in std::mem::take(&mut self.replaced) {
            if self.live.contains_key(&pos) && self.ticking(state(pos)) != Ticking::Yes {
                self.cancel(&pos);
            }
        }
        if !std::mem::take(&mut self.freed) && self.out.is_empty() {
            return Arc::clone(&self.tickers);
        }
        self.retire_taken(|_| true);
        self.freed = false;
        let (live, map) = (&mut self.live, &self.map);
        Arc::make_mut(&mut self.tickers).retain(|(p, slot)| {
            let mine = live.get(p) == Some(slot);
            if mine && !map.contains_key(p) {
                live.remove(p);
            }
            mine && map.contains_key(p)
        });
        Arc::clone(&self.tickers)
    }
    /// Whether a slot from `tickers` still ticks.
    pub fn is_live(&self, (pos, slot): (Pos, u32)) -> bool {
        self.live.get(&pos) == Some(&slot) && self.map.contains_key(&pos)
    }
    /// Frees the slot at `pos` (the block entity stays).
    pub fn forget(&mut self, pos: &Pos) {
        self.cancel(pos);
    }
}

#[cfg(test)]
mod attachment_tests;

#[cfg(test)]
mod lease_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn hopper() -> BlockEntity {
        BlockEntity::Chest(vec![None; 5])
    }
    /// States: 0 air, 1 ticks, 2 idle.
    fn level() -> BlockEntities {
        let mut b = BlockEntities::default();
        b.set_states(vec![Ticking::No, Ticking::Yes, Ticking::Idle]);
        b
    }
    fn order(b: &mut BlockEntities, states: &HashMap<Pos, u32>) -> Vec<Pos> {
        b.tickers(|p| states.get(&p).copied().unwrap_or(0))
            .iter()
            .map(|(p, _)| *p)
            .collect()
    }

    #[test]
    fn generations_follow_identity_not_position_or_tick_storage() {
        let mut b = level();
        let p = (1, 64, 1);
        b.get_or_insert_with(p, hopper);
        let first = b.generation(&p).unwrap();
        b.insert(p, hopper()); // Same-type payload load, as /data does.
        assert_eq!(b.generation(&p), Some(first));
        let (lease, taken) = b.take(&p).unwrap();
        assert_eq!(b.generation(&p), Some(first));
        assert!(b.put(lease, taken));
        assert_eq!(b.generation(&p), Some(first));
        b.insert(
            p,
            BlockEntity::Mod(Box::new(crate::chunk::block_entity::ModBlockEntity::new(
                "test:other".into(),
                Default::default(),
            ))),
        );
        let replaced = b.generation(&p).unwrap();
        assert_ne!(replaced, first);
        b.remove(&p);
        assert_eq!(b.generation(&p), None);
        b.insert(p, hopper());
        let recreated = b.generation(&p).unwrap();
        assert_ne!(recreated, replaced);
        b.unload(|pos| *pos == p);
        assert_eq!(b.generation(&p), None);
        b.load_chunk(vec![(p, hopper())]);
        let loaded = b.generation(&p).unwrap();
        assert_ne!(loaded, recreated);
        b.load_chunk(vec![(p, hopper())]);
        assert_ne!(b.generation(&p), Some(loaded));
        let before_clear = b.generation(&p).unwrap();
        b.clear();
        assert_eq!(b.generation(&p), None);
        b.insert(p, hopper());
        assert_ne!(b.generation(&p), Some(before_clear));
        let mut another = level();
        another.insert(p, hopper());
        assert_ne!(b.generation(&p), another.generation(&p));
    }

    /// Unloading a chunk removes its block entities and leaves the others'
    /// sleep alone; `retain`, which may change what it keeps, wakes them all.
    #[test]
    fn unloading_a_chunk_keeps_the_others_asleep() {
        let mut b = level();
        let (kept, gone) = ((1, 64, 1), (17, 64, 1));
        b.insert(kept, hopper());
        b.insert(gone, hopper());
        b.sleep(kept);
        b.sleep(gone);
        b.unload(|p| p.0 >> 4 == 1);
        assert!(b.get(&gone).is_none());
        assert!(!b.is_asleep(&gone));
        assert!(b.is_asleep(&kept));
        b.retain(|_, _| true);
        assert!(!b.is_asleep(&kept));
    }

    fn near(b: &BlockEntities, x: f64, z: f64, radius: f64) -> Vec<Pos> {
        let mut found: Vec<Pos> = b.mods_near(x, z, radius).copied().collect();
        found.sort();
        found
    }
    #[test]
    fn a_sleeping_block_entity_wakes_on_any_change() {
        let p = (1, 64, 1);
        let mut b = level();
        b.sleep(p);
        assert!(!b.is_asleep(&p));
        b.insert(p, hopper());
        let asleep = |b: &mut BlockEntities, wake: &dyn Fn(&mut BlockEntities)| {
            b.sleep(p);
            assert!(b.is_asleep(&p));
            wake(b);
            b.is_asleep(&p)
        };
        assert!(asleep(&mut b, &|_| {}));
        assert!(!asleep(&mut b, &|b| {
            b.get_mut(&p);
        }));
        assert!(!asleep(&mut b, &|b| {
            b.insert(p, hopper());
        }));
        assert!(!asleep(&mut b, &|b| {
            b.get_or_insert_with(p, hopper);
        }));
        assert!(!asleep(&mut b, &|b| {
            let (lease, held) = b.take(&p).unwrap();
            b.put(lease, held);
        }));
        assert!(!asleep(&mut b, &|b| b.state_changed(p, 1, 1)));
        assert!(!asleep(&mut b, &|b| b.forget(&p)));
        assert!(!asleep(&mut b, &|b| {
            b.iter_mut().count();
        }));
        assert!(!asleep(&mut b, &|b| {
            b.values_mut().count();
        }));
        assert!(!asleep(&mut b, &|b| b.retain(|_, _| true)));
        assert!(!asleep(&mut b, &|b| b.load_chunk(vec![(p, hopper())])));
        assert!(!asleep(&mut b, &|b| {
            b.remove(&p);
            b.insert(p, hopper());
        }));
        b.sleep(p);
        b.clear();
        assert!(!b.is_asleep(&p));
    }
    #[test]
    fn mod_block_entities_are_listed_by_chunk_until_they_go() {
        let mut b = level();
        let modded = || BlockEntity::Mod(Box::default());
        b.insert((1, 64, 1), modded());
        b.insert((200, 64, 1), modded());
        b.insert((2, 64, 2), hopper());
        b.get_or_insert_with((3, 64, 3), modded);
        b.load_chunk(vec![((4, 64, 4), modded()), ((5, 64, 5), hopper())]);
        // Only mods, and only chunks within reach.
        assert_eq!(
            near(&b, 0.0, 0.0, 64.0),
            [(1, 64, 1), (3, 64, 3), (4, 64, 4)]
        );
        assert_eq!(near(&b, 200.0, 0.0, 8.0), [(200, 64, 1)]);
        // Taken out for a tick, still listed; removed or replaced, gone.
        let (lease, taken) = b.take(&(1, 64, 1)).unwrap();
        assert_eq!(near(&b, 0.0, 0.0, 64.0).len(), 3);
        b.put(lease, taken);
        b.remove(&(3, 64, 3));
        b.insert((4, 64, 4), hopper());
        assert_eq!(near(&b, 0.0, 0.0, 64.0), [(1, 64, 1)]);
        b.retain(|p, _| p.0 != 1);
        assert!(near(&b, 0.0, 0.0, 64.0).is_empty());
        b.clear();
        assert!(near(&b, 200.0, 0.0, 8.0).is_empty());
    }
    #[test]
    fn attachments_follow_block_entity_lifetime() {
        use horizon_mod_runtime::attachments::{AttachmentType, Holder};
        let ty = AttachmentType {
            namespace: "test",
            name: "value",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        let p = (1, 64, 2);
        let mut entities = level();
        assert!(entities.attachments_mut(p).is_none());
        entities.insert(p, hopper());
        ty.set(entities.attachments_mut(p).unwrap(), Holder::Level, 4)
            .unwrap();
        let (lease, held) = entities.take(&p).unwrap();
        entities.put(lease, held);
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(4)
        );
        entities.insert(p, hopper());
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(4)
        );
        let machine = |id: &str| {
            BlockEntity::Mod(Box::new(crate::chunk::block_entity::ModBlockEntity::new(
                id.into(),
                Default::default(),
            )))
        };
        entities.insert(p, machine("test:a"));
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(0)
        );
        ty.set(entities.attachments_mut(p).unwrap(), Holder::Level, 4)
            .unwrap();
        entities.insert(p, machine("test:a"));
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(4)
        );
        entities.insert(p, machine("test:b"));
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(0)
        );
        ty.set(entities.attachments_mut(p).unwrap(), Holder::Level, 4)
            .unwrap();
        entities.remove(&p);
        entities.insert(p, hopper());
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(0)
        );
        ty.set(entities.attachments_mut(p).unwrap(), Holder::Level, 5)
            .unwrap();
        entities.retain(|_, _| false);
        entities.insert(p, hopper());
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(0)
        );
    }

    #[test]
    fn slots_follow_vanilla_lifecycle() {
        let mut b = level();
        let mut s = HashMap::new();
        for p in [(0, 0, 0), (1, 0, 0), (2, 0, 0)] {
            s.insert(p, 1);
            b.insert(p, hopper());
        }
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (1, 0, 0), (2, 0, 0)]);
        // Replacing keeps the slot; removing and placing again appends.
        b.insert((0, 0, 0), hopper());
        b.remove(&(1, 0, 0));
        b.insert((1, 0, 0), hopper());
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (2, 0, 0), (1, 0, 0)]);
        // Take and put keep it; an idle state frees it, ticking again appends.
        let (lease, e) = b.take(&(0, 0, 0)).unwrap();
        b.put(lease, e);
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (2, 0, 0), (1, 0, 0)]);
        s.insert((2, 0, 0), 2);
        b.state_changed((2, 0, 0), 1, 2);
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (1, 0, 0)]);
        s.insert((2, 0, 0), 1);
        b.state_changed((2, 0, 0), 2, 1);
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (1, 0, 0), (2, 0, 0)]);
        // A slot freed during a tick is not live any more.
        let slots = b.tickers(|p| s.get(&p).copied().unwrap_or(0));
        b.retain(|p, _| *p != (1, 0, 0));
        assert!(b.is_live(slots[0]) && !b.is_live(slots[1]));
        // A block entity taken and never brought back loses its slot.
        b.take(&(0, 0, 0));
        assert_eq!(order(&mut b, &s), [(2, 0, 0)]);
        assert_eq!(order(&mut b, &s), [(2, 0, 0)]);
    }

    #[test]
    fn chunk_loads_in_open_hash_map_order() {
        // A few keys: their mixed hashes over 32 slots, read from the top.
        let positions = [
            (0, 64, 0),
            (1, 64, 0),
            (0, 64, 1),
            (15, -64, 15),
            (3, 70, 9),
        ];
        let slot = |p: &Pos| mix(java_hash(*p)) & 31;
        let mut expected = positions.to_vec();
        expected.sort_by_key(|p| std::cmp::Reverse(slot(p)));
        let distinct: std::collections::HashSet<usize> = positions.iter().map(slot).collect();
        assert_eq!(distinct.len(), positions.len());
        assert_eq!(chunk_map_order(positions), expected);
        // Past 24 keys the table doubles; every key stays, once.
        let many: Vec<Pos> = (0..100).map(|i| (i % 16, i / 16, 3)).collect();
        let order = chunk_map_order(many.iter().copied().chain(many.iter().copied()));
        assert_eq!(order.len(), 100);
    }

    /// A chunk loaded over a block entity already there: the new one replaces
    /// it, and keeps the old slot only while its block state ticks.
    #[test]
    fn loaded_chunk_replaces_block_entity_by_state() {
        let mut b = level();
        let mut s = HashMap::new();
        let (p, q) = ((0, 0, 0), (1, 0, 0));
        for pos in [p, q] {
            s.insert(pos, 1);
        }
        // Two chunks, so the order is the order they load in.
        b.load_chunk(vec![(p, hopper())]);
        b.load_chunk(vec![(q, hopper())]);
        assert_eq!(order(&mut b, &s), [p, q]);
        // Replaced where the state ticks: the slot stays in its place.
        b.load_chunk(vec![(p, hopper())]);
        assert_eq!(order(&mut b, &s), [p, q]);
        // Replaced where the state has no ticker: the slot is freed, and the
        // new block entity is the one in the map.
        s.insert(p, 2);
        b.load_chunk(vec![(p, BlockEntity::Chest(vec![None; 27]))]);
        assert_eq!(order(&mut b, &s), [q]);
        assert!(matches!(b.get(&p), Some(BlockEntity::Chest(items)) if items.len() == 27));
        // Ticking again later appends it, as any new ticker does.
        s.insert(p, 1);
        b.state_changed(p, 2, 1);
        assert_eq!(order(&mut b, &s), [q, p]);
    }

    #[test]
    fn boxes_list_exactly_the_structure_and_test_blocks() {
        let test_block = || BlockEntity::TestInstanceBlock(Box::default());
        let listed = |b: &BlockEntities| {
            let mut v: Vec<Pos> = b.boxes().map(|(p, _)| *p).collect();
            v.sort();
            v
        };
        let mut b = level();
        let (p, q, r) = ((0, 0, 0), (1, 0, 0), (2, 0, 0));
        b.insert(p, test_block());
        b.insert(q, hopper());
        assert_eq!(listed(&b), [p]);
        // Replaced by another kind, and back.
        b.insert(p, hopper());
        assert_eq!(listed(&b), []);
        b.insert(q, test_block());
        b.get_or_insert_with(r, test_block);
        assert_eq!(listed(&b), [q, r]);
        // Taken out: not shown until put back; replacement during take wins.
        let (lease, taken) = b.take(&q).unwrap();
        assert_eq!(listed(&b), [r]);
        b.put(lease, taken);
        assert_eq!(listed(&b), [q, r]);
        let (lease, taken) = b.take(&r).unwrap();
        b.insert(r, hopper());
        assert!(!b.put(lease, taken));
        assert_eq!(listed(&b), [q]);
        b.remove(&q);
        assert_eq!(listed(&b), []);
        b.load_chunk(vec![(p, test_block()), (q, hopper())]);
        assert_eq!(listed(&b), [p]);
        b.retain(|pos, _| *pos != p);
        assert_eq!(listed(&b), []);
        b.insert(r, test_block());
        b.clear();
        assert!(b.boxes.is_empty());
    }
}

#[cfg(test)]
#[path = "block_entities/on_load_tests.rs"]
mod on_load_tests;
