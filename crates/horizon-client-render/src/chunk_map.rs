//! Resident chunk membership, with a token for consumers that cache its keys.
use crate::{
    chunk::Chunk,
    fast_hash::{FxHashMap, FxHashSet},
};
use std::{
    collections::{
        VecDeque,
        hash_map::{Iter, ValuesMut},
    },
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use horizon_mod_runtime::attachments::{
    ATTACHMENTS_KEY, AttachmentError, Attachments, Holder, attachment_compound,
    observer::AttachmentObserver,
};

type Key = (i32, i32);
type Columns = FxHashMap<Key, Arc<Chunk>>;
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);
fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

/// The membership changes one map keeps for the consumers that read them
/// every tick (`TickChunkMembership.synchronize`); older ones are dropped.
const MAX_RECENT_DELTAS: usize = 256;

/// One membership change of one `ChunkMap`, stamped with the revision it
/// brought the map to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkMembershipDelta {
    Added(Key),
    Removed(Key),
    Reset,
}

/// Direct-mapped toroidal lookup entry for resident chunk columns.
#[derive(Clone)]
struct CachedSlot {
    key_packed: u64,
    chunk: Option<Arc<Chunk>>,
}
impl CachedSlot {
    const EMPTY: Self = Self {
        key_packed: 0,
        chunk: None,
    };
}

#[inline(always)]
fn pack_key(cx: i32, cz: i32) -> u64 {
    u64::from(cx as u32) | (u64::from(cz as u32) << 32)
}

/// Read access behaves like a map. Structural writes are explicit so edits to
/// blocks or light don't invalidate consumers of the loaded chunk key set.
/// Tokens are process-wide: replacing the entire map also invalidates caches.
pub struct ChunkMap {
    /// Toroidal power-of-two direct-mapped cache (`(cz & mask) << shift | (cx & mask)`)
    /// for O(1) hash-free `get`, `get_chunk` and `contains_key` lookups.
    /// Entries are cleared before mutable chunk access so `Arc::make_mut` sees
    /// unique ownership and avoids copy-on-write cloning.
    grid: Box<[CachedSlot]>,
    grid_mask: i32,
    grid_shift: u8,
    /// True when every resident chunk in `columns` sits in its `grid` slot
    /// (no toroidal collisions and no unrefreshed `get_mut`/`values_mut` evictions),
    /// letting unloaded-chunk queries return without hashing into `columns`.
    grid_complete: bool,
    columns: Columns,
    revision: u64,
    /// The oldest revision `recent_deltas` still answers for: the revision of
    /// the change it last dropped, or the revision that cleared them. A
    /// consumer holding an older one rebuilds, it does not replay.
    min_valid_revision: u64,
    /// This map's recent membership changes, in revision order.
    /// Revisions are process-wide: the numbers missing in between belong to
    /// the other maps (a server thread's, another dimension's), which say
    /// nothing about this one's keys.
    recent_deltas: VecDeque<(u64, ChunkMembershipDelta)>,
    /// The resident chunks generation has not placed its animals in yet.
    fresh: FxHashSet<Key>,
    attachment_observer: Option<Arc<dyn AttachmentObserver>>,
    /// For each column, how many of the 3×3 columns around it (itself
    /// included) are resident; absent when none is. `surrounded` asks one
    /// map instead of nine every tick, for every ticking chunk.
    near: FxHashMap<Key, u8>,
    /// Which map this is, for the consumers of `touched_since` (a replaced or swapped map).
    identity: u64,
    /// The columns a writer was handed, inserted or removed, in order: a consumer that keeps a
    /// per-column answer re-reads only these (`touched_since`).
    touched: VecDeque<Key>,
    /// The sequence number of `touched[0]`.
    touched_base: u64,
    /// The sequence number from which every column may have changed (`values_mut`, `clear`).
    touched_all: u64,
}
/// The columns `ChunkMap::touched_since` remembers; an older cursor rebuilds.
const MAX_TOUCHED: usize = 8192;
impl Default for ChunkMap {
    fn default() -> Self {
        let revision = next_revision();
        Self {
            grid: Box::new([]),
            grid_mask: 0,
            grid_shift: 0,
            grid_complete: true,
            columns: Columns::default(),
            revision,
            min_valid_revision: revision,
            recent_deltas: VecDeque::new(),
            fresh: FxHashSet::default(),
            attachment_observer: None,
            near: FxHashMap::default(),
            identity: next_revision(),
            touched: VecDeque::new(),
            touched_base: 0,
            touched_all: 0,
        }
    }
}
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ChunkMap>();
};
// Only residency installation binds saved maps. Lighting COW leaves these shared
// entries indexed; attachment access below rebinds an extras COW before its write.
fn bind_saved_attachments(
    observer: &dyn AttachmentObserver,
    key: Key,
    chunk: &mut Arc<Chunk>,
) -> bool {
    let Some(saved) = &chunk.saved else {
        return false;
    };
    if !saved.attachments.has_index_entries() && !saved.attachments.has_index_binding() {
        return false;
    }
    let saved = Arc::make_mut(Arc::make_mut(chunk).saved.as_mut().unwrap());
    observer
        .bind(Holder::Chunk([key.0, key.1]), &mut saved.attachments)
        .is_some()
}

impl Drop for ChunkMap {
    fn drop(&mut self) {
        self.unload_attachments();
    }
}

impl ChunkMap {
    /// Install the authoritative dimension's observer, including already loaded
    /// saved entries before their first read. Client/worker maps have no observer.
    pub fn observe_attachments(&mut self, observer: Arc<dyn AttachmentObserver>) {
        self.unload_attachments();
        if self.grid_mask != 0 {
            self.invalidate_grid();
        }
        for (key, chunk) in &mut self.columns {
            bind_saved_attachments(&*observer, *key, chunk);
        }
        if self.grid_mask != 0 {
            self.rebuild_grid(self.grid_shift);
        }
        self.attachment_observer = Some(observer);
    }

    fn unload_attachments(&self) {
        if let Some(observer) = &self.attachment_observer {
            for key in self.columns.keys() {
                observer.unload(Holder::Chunk([key.0, key.1]));
            }
        }
    }

    #[inline(always)]
    fn grid_idx(&self, cx: i32, cz: i32) -> usize {
        (((cz & self.grid_mask) as usize) << self.grid_shift) | ((cx & self.grid_mask) as usize)
    }

    fn rebuild_grid(&mut self, shift: u8) {
        let side = 1usize << shift;
        let len = side * side;
        if self.grid.len() != len {
            self.grid = vec![CachedSlot::EMPTY; len].into_boxed_slice();
        } else {
            self.grid.fill(CachedSlot::EMPTY);
        }
        self.grid_shift = shift;
        self.grid_mask = (side - 1) as i32;
        self.grid_complete = true;
        for (&(cx, cz), chunk) in &self.columns {
            let idx = (((cz & self.grid_mask) as usize) << self.grid_shift)
                | ((cx & self.grid_mask) as usize);
            if self.grid[idx].chunk.is_some() {
                self.grid_complete = false;
            }
            self.grid[idx] = CachedSlot {
                key_packed: pack_key(cx, cz),
                chunk: Some(Arc::clone(chunk)),
            };
        }
    }

    #[inline]
    fn invalidate_slot(&mut self, key: Key) {
        if self.grid_mask != 0 {
            let idx = self.grid_idx(key.0, key.1);
            let slot = &mut self.grid[idx];
            if slot.chunk.is_some() && slot.key_packed == pack_key(key.0, key.1) {
                slot.chunk = None;
                self.grid_complete = false;
            }
        }
    }

    fn invalidate_grid(&mut self) {
        if self.grid_mask != 0 {
            self.grid.fill(CachedSlot::EMPTY);
            self.grid_complete = self.columns.is_empty();
        }
    }

    /// Reference to the resident `Arc<Chunk>` at `key`, served from the O(1)
    /// toroidal grid without hashing on the hot path.
    #[inline(always)]
    pub fn get(&self, key: &Key) -> Option<&Arc<Chunk>> {
        let mask = self.grid_mask;
        if mask == 0 {
            debug_assert!(self.columns.is_empty());
            return None;
        }
        let (cx, cz) = *key;
        let idx = (((cz & mask) as usize) << self.grid_shift) | ((cx & mask) as usize);
        if let Some(slot) = self.grid.get(idx) {
            if let Some(chunk) = &slot.chunk
                && slot.key_packed == pack_key(cx, cz)
            {
                return Some(chunk);
            }
            if self.grid_complete {
                return None;
            }
        }
        self.columns.get(key)
    }

    /// Direct reference to the resident `Chunk` at `(cx, cz)`, served from the
    /// O(1) toroidal grid without hashing on the hot path.
    #[inline(always)]
    pub fn get_chunk(&self, cx: i32, cz: i32) -> Option<&Chunk> {
        self.get(&(cx, cz)).map(|c| &**c)
    }

    /// Whether chunk `key` is resident, served from the O(1) toroidal grid on the
    /// hot path before falling back to `columns`.
    #[inline(always)]
    pub fn contains_key(&self, key: &Key) -> bool {
        self.get(key).is_some()
    }

    /// Mutates a resident chunk in place while keeping its toroidal grid slot
    /// refreshed (drops the grid's `Arc` before `f` so `Arc::make_mut` avoids
    /// copy-on-write cloning, then restores it afterward).
    #[inline]
    pub(crate) fn mutate_chunk<R>(
        &mut self,
        key: Key,
        f: impl FnOnce(&mut Arc<Chunk>) -> R,
    ) -> Option<R> {
        self.mutate_chunk_with(key, true, f)
    }

    #[inline]
    fn mutate_chunk_with<R>(
        &mut self,
        key: Key,
        touch: bool,
        f: impl FnOnce(&mut Arc<Chunk>) -> R,
    ) -> Option<R> {
        if touch {
            self.touch(key);
        }
        let packed = pack_key(key.0, key.1);
        let had_slot = if self.grid_mask != 0 {
            let idx = self.grid_idx(key.0, key.1);
            let slot = &mut self.grid[idx];
            if slot.chunk.is_some() && slot.key_packed == packed {
                slot.chunk = None;
                true
            } else {
                false
            }
        } else {
            false
        };
        let was_complete = self.grid_complete;
        if had_slot {
            self.grid_complete = false;
        }
        let chunk = self.columns.get_mut(&key)?;
        let out = f(chunk);
        let cached = Arc::clone(chunk);
        if self.grid_mask != 0 {
            let idx = self.grid_idx(key.0, key.1);
            let slot = &mut self.grid[idx];
            if slot.chunk.is_none() || slot.key_packed == packed {
                *slot = CachedSlot {
                    key_packed: packed,
                    chunk: Some(cached),
                };
                if had_slot {
                    self.grid_complete = was_complete;
                }
            }
        }
        Some(out)
    }

    /// The live map, rebound after extras COW but never merely for lighting COW.
    pub fn attachments_mut(&mut self, key: &Key) -> Result<&mut Attachments, AttachmentError> {
        self.invalidate_slot(*key);
        let chunk = self
            .columns
            .get_mut(key)
            .ok_or(AttachmentError::MissingHolder)?;
        let saved = Arc::make_mut(Arc::make_mut(chunk).saved.get_or_insert_default());
        attachment_compound(saved.root.get(&ATTACHMENTS_KEY.into()))?;
        if let Some(observer) = &self.attachment_observer
            && !saved.attachments.has_index_binding()
        {
            let _ = observer.bind(Holder::Chunk([key.0, key.1]), &mut saved.attachments);
        }
        Ok(&mut saved.attachments)
    }

    #[inline]
    pub fn membership_revision(&self) -> u64 {
        self.revision
    }
    /// The map's identity and the sequence number the next touch takes: a consumer that
    /// has read every column now keeps them for `touched_since`.
    pub fn touch_cursor(&self) -> (u64, u64) {
        (self.identity, self.touched_base + self.touched.len() as u64)
    }
    /// The columns touched (handed out mutably, inserted, removed) since `cursor`, a
    /// `touch_cursor` of this map; `None` when everything may have changed since (another
    /// map, `values_mut`, `clear`, or a cursor older than what is kept): read it all again.
    /// A column not listed is the one the consumer read last.
    pub fn touched_since(&self, cursor: (u64, u64)) -> Option<impl Iterator<Item = &Key> + '_> {
        let (identity, seq) = cursor;
        if identity != self.identity || seq < self.touched_all || seq < self.touched_base {
            return None;
        }
        Some(self.touched.iter().skip((seq - self.touched_base) as usize))
    }
    fn touch(&mut self, key: Key) {
        if self.touched.len() >= MAX_TOUCHED {
            self.touched.pop_front();
            self.touched_base += 1;
        }
        self.touched.push_back(key);
    }
    fn touch_all(&mut self) {
        // One more than the entries dropped: the cursor moves even for a consumer that had
        // read every entry (its sequence number is then the old end of the log).
        self.touched_base += self.touched.len() as u64 + 1;
        self.touched.clear();
        self.touched_all = self.touched_base;
    }
    fn record_delta(&mut self, delta: ChunkMembershipDelta) {
        if self.recent_deltas.len() >= MAX_RECENT_DELTAS
            && let Some((evicted, _)) = self.recent_deltas.pop_front()
        {
            self.min_valid_revision = evicted;
        }
        self.recent_deltas.push_back((self.revision, delta));
    }
    /// This map's membership changes since `last_revision`, one of its own
    /// earlier `membership_revision`s, or `None` when they can no longer be
    /// answered: the consumer's revision predates what the ring kept, the map
    /// was cleared since (a reset rebuilds everything), or the revision is not
    /// this map's, which its own revisions give away: every one of them in the
    /// answered range is the opening revision or a change the ring still
    /// holds, so a revision another map took in between is refused, not
    /// replayed.
    pub fn deltas_since(
        &self,
        last_revision: u64,
    ) -> Option<impl Iterator<Item = (u64, ChunkMembershipDelta)> + '_> {
        if last_revision == 0
            || last_revision < self.min_valid_revision
            || last_revision > self.revision
            || self.recent_deltas.is_empty()
        {
            return None;
        }
        let pos = self
            .recent_deltas
            .partition_point(|(rev, _)| *rev <= last_revision);
        // A revision this map never took: another map's, drawing on the same
        // process-wide counter. Only the opening revision and the ones the
        // ring still holds are this map's.
        let is_own_revision = last_revision == self.min_valid_revision
            || (pos > 0 && self.recent_deltas[pos - 1].0 == last_revision);
        if !is_own_revision {
            return None;
        }
        let slice = self.recent_deltas.range(pos..);
        if slice
            .clone()
            .any(|(_, d)| *d == ChunkMembershipDelta::Reset)
        {
            return None;
        }
        Some(slice.copied())
    }
    pub fn insert(&mut self, key: Key, mut chunk: Arc<Chunk>) -> Option<Arc<Chunk>> {
        self.touch(key);
        let limits = chunk
            .saved
            .as_ref()
            .map(|saved| {
                saved
                    .block_entity_sources
                    .iter()
                    .filter_map(|(pos, tag)| match tag.get("horizon:stack_limit") {
                        Some(horizon_core::nbt::Tag::Int(n)) if (1..=99).contains(n) => {
                            Some((*pos, *n as u32))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if chunk.container_stack_limits != limits {
            Arc::make_mut(&mut chunk).container_stack_limits = limits;
        }
        if let Some(observer) = &self.attachment_observer {
            let bound = bind_saved_attachments(&**observer, key, &mut chunk);
            if !bound && self.columns.contains_key(&key) {
                observer.unload(Holder::Chunk([key.0, key.1]));
            }
        }
        if chunk.fresh {
            self.fresh.insert(key);
        } else {
            self.fresh.remove(&key);
        }
        let cached = Arc::clone(&chunk);
        let previous = self.columns.insert(key, chunk);
        if previous.is_none() {
            for z in key.1 - 1..=key.1 + 1 {
                for x in key.0 - 1..=key.0 + 1 {
                    *self.near.entry((x, z)).or_insert(0) += 1;
                }
            }
            self.revision = next_revision();
            self.record_delta(ChunkMembershipDelta::Added(key));
        }
        if self.grid_mask == 0 {
            self.rebuild_grid(4);
        } else if self.columns.len() > 2048 && self.grid_shift < 7 {
            self.rebuild_grid(7);
        } else if self.columns.len() > 128 && self.grid_shift < 6 {
            self.rebuild_grid(6);
        } else {
            let idx = self.grid_idx(key.0, key.1);
            let packed = pack_key(key.0, key.1);
            let slot = &mut self.grid[idx];
            if slot.chunk.is_some() && slot.key_packed != packed {
                self.grid_complete = false;
            }
            *slot = CachedSlot {
                key_packed: packed,
                chunk: Some(cached),
            };
        }
        previous
    }
    pub fn remove(&mut self, key: &Key) -> Option<Arc<Chunk>> {
        self.touch(*key);
        self.fresh.remove(key);
        if self.grid_mask != 0 {
            let idx = self.grid_idx(key.0, key.1);
            let slot = &mut self.grid[idx];
            if slot.chunk.is_some() && slot.key_packed == pack_key(key.0, key.1) {
                slot.chunk = None;
            }
        }
        let previous = self.columns.remove(key);
        if previous.is_some() {
            if self.columns.is_empty() {
                self.grid_complete = true;
            }
            if let Some(observer) = &self.attachment_observer {
                observer.unload(Holder::Chunk([key.0, key.1]));
            }
            for z in key.1 - 1..=key.1 + 1 {
                for x in key.0 - 1..=key.0 + 1 {
                    if let Some(n) = self.near.get_mut(&(x, z)) {
                        *n -= 1;
                        if *n == 0 {
                            self.near.remove(&(x, z));
                        }
                    }
                }
            }
            self.revision = next_revision();
            self.record_delta(ChunkMembershipDelta::Removed(*key));
        }
        previous
    }
    pub fn clear(&mut self) {
        self.touch_all();
        if !self.columns.is_empty() {
            self.unload_attachments();
            self.columns.clear();
            self.fresh.clear();
            self.near.clear();
            if self.grid.len() > 4096 {
                self.grid = Box::new([]);
                self.grid_mask = 0;
                self.grid_shift = 0;
            } else if self.grid_mask != 0 {
                self.grid.fill(CachedSlot::EMPTY);
            }
            self.grid_complete = true;
            self.revision = next_revision();
            self.recent_deltas.clear();
            self.recent_deltas
                .push_back((self.revision, ChunkMembershipDelta::Reset));
            self.min_valid_revision = self.revision;
        }
    }
    /// Whether the column at `key` and its eight neighbours are all resident.
    #[inline]
    pub fn surrounded(&self, key: Key) -> bool {
        self.near.get(&key) == Some(&9)
    }
    #[inline]
    pub fn has_fresh(&self) -> bool {
        !self.fresh.is_empty()
    }
    /// The fresh keys in the chunk map's own iteration order, so the spawns
    /// they drive run in the order a scan of the columns gave.
    pub fn fresh_keys_ordered(&self) -> Vec<Key> {
        if self.fresh.is_empty() {
            Vec::new()
        } else if self.fresh.len() == 1 {
            self.fresh.iter().copied().collect()
        } else {
            self.columns
                .keys()
                .filter(|k| self.fresh.contains(k))
                .copied()
                .collect()
        }
    }
    pub fn remove_fresh(&mut self, key: &Key) {
        self.fresh.remove(key);
    }
    pub fn get_mut(&mut self, key: &Key) -> Option<&mut Arc<Chunk>> {
        self.touch(*key);
        self.invalidate_slot(*key);
        self.columns.get_mut(key)
    }
    pub fn values_mut(&mut self) -> ValuesMut<'_, Key, Arc<Chunk>> {
        self.touch_all();
        self.invalidate_grid();
        self.columns.values_mut()
    }
}
impl Deref for ChunkMap {
    type Target = Columns;
    #[inline]
    fn deref(&self) -> &Columns {
        &self.columns
    }
}
impl std::ops::Index<&Key> for ChunkMap {
    type Output = Arc<Chunk>;
    #[inline(always)]
    fn index(&self, key: &Key) -> &Arc<Chunk> {
        self.get(key).expect("no entry found for key")
    }
}
impl<'a> IntoIterator for &'a ChunkMap {
    type Item = (&'a Key, &'a Arc<Chunk>);
    type IntoIter = Iter<'a, Key, Arc<Chunk>>;
    fn into_iter(self) -> Self::IntoIter {
        self.columns.iter()
    }
}
impl crate::light::LightWorld for ChunkMap {
    fn section(&self, cx: i32, s: usize, cz: i32) -> Option<&Arc<crate::chunk::Section>> {
        self.get_chunk(cx, cz)
            .filter(|c| c.lit)
            .map(|c| &c.sections[s])
    }
    fn set_section_light(&mut self, cx: i32, s: usize, cz: i32, light: crate::chunk::Light) {
        self.mutate_chunk_with((cx, cz), false, |chunk| {
            Arc::make_mut(&mut Arc::make_mut(chunk).sections[s]).light = light;
        });
    }
    fn layout(&self) -> crate::chunk::ColumnLayout {
        self.columns.layout()
    }
    fn dynamic_emission(&self, x: i32, y: i32, z: i32) -> Option<u8> {
        let chunk = self.get_chunk(x >> 4, z >> 4)?;
        chunk
            .dynamic_emission
            .get(&chunk.dynamic_cell(x, y, z)?)
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::nbt::Tag;
    use horizon_mod_runtime::attachments::{
        AttachmentCodec, AttachmentType, index::AttachmentIndex,
        observer::IndexedAttachmentObserver,
    };
    use mod_api::{AttachmentHolder, ChunkRef, DimensionKey, LevelRef, WorldEpoch};
    use std::sync::{Mutex, Weak, atomic::AtomicUsize};

    type Index = Arc<Mutex<AttachmentIndex<AttachmentHolder>>>;
    static VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "chunk_test",
        name: "value",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
            decode: |form| match form.get(&"n".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: false,
        sync: false,
    };
    fn observe(map: &mut ChunkMap, index: &Index, dimension: DimensionKey) -> Arc<AtomicUsize> {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let level = LevelRef::new(WorldEpoch::new(42), dimension);
        map.observe_attachments(Arc::new(IndexedAttachmentObserver::new(
            index,
            move |holder| {
                counter.fetch_add(1, Ordering::Relaxed);
                match holder {
                    Holder::Chunk(pos) => Some(AttachmentHolder::Chunk(ChunkRef::new(level, pos))),
                    _ => None,
                }
            },
        )));
        calls
    }
    fn key() -> horizon_mod_runtime::attachments::index::AttachmentKey {
        ("chunk_test".into(), "value".into())
    }
    fn saved_chunk() -> Arc<Chunk> {
        let mut source = Attachments::default();
        VALUE.set(&mut source, Holder::Chunk([3, 5]), 7).unwrap();
        let form = source.to_nbt();
        let mut chunk = Chunk::empty(3, 5);
        chunk.saved = Some(Arc::new(crate::chunk::ChunkExtras {
            attachments: Attachments::from_nbt(form),
            ..Default::default()
        }));
        Arc::new(chunk)
    }

    #[test]
    fn chunk_attachment_residency_load_cow_unload_and_dimension_swap() {
        let index = Index::default();
        let other_index = Index::default();
        let mut map = ChunkMap::default();
        // Observer installation also handles chunks loaded before level-open.
        map.insert((3, 5), saved_chunk());
        let calls = observe(&mut map, &index, DimensionKey::OVERWORLD);
        let old = index.lock().unwrap().snapshot(&key());
        assert_eq!(old.len(), 1);
        assert_eq!(
            old[0].holder,
            AttachmentHolder::Chunk(ChunkRef::new(
                LevelRef::new(WorldEpoch::new(42), DimensionKey::OVERWORLD),
                [3, 5]
            ))
        );
        let save = map[&(3, 5)].clone();
        Arc::make_mut(map.get_mut(&(3, 5)).unwrap()).lit = true;
        crate::light::LightWorld::set_section_light(
            &mut map,
            3,
            0,
            5,
            crate::chunk::Light::Uniform(0),
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(Arc::ptr_eq(&old, &index.lock().unwrap().snapshot(&key())));
        VALUE
            .set(
                map.attachments_mut(&(3, 5)).unwrap(),
                Holder::Chunk([3, 5]),
                9,
            )
            .unwrap();
        let current = index.lock().unwrap().snapshot(&key());
        assert_eq!(current[0].entry.read(&VALUE), Ok(&9));
        assert_eq!(old[0].entry.read(&VALUE), Ok(&7));
        assert_eq!(
            save.saved
                .as_ref()
                .unwrap()
                .attachments
                .entry("chunk_test", "value")
                .unwrap()
                .unwrap()
                .read(&VALUE),
            Ok(&7)
        );
        assert!(map.attachments_mut(&(3, 5)).is_ok());
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        let current_save = map[&(3, 5)].clone();
        // Unrelated extras COW detaches the map; residency must not rely on it.
        let chunk = Arc::make_mut(map.get_mut(&(3, 5)).unwrap());
        Arc::make_mut(chunk.saved.as_mut().unwrap()).last_update = 20;
        let mut other = ChunkMap::default();
        observe(&mut other, &other_index, DimensionKey::NETHER);
        other.insert((3, 5), saved_chunk());
        std::mem::swap(&mut map, &mut other);
        map.remove(&(3, 5));
        assert!(other_index.lock().unwrap().snapshot(&key()).is_empty());
        assert_eq!(index.lock().unwrap().snapshot(&key()).len(), 1);
        other.remove(&(3, 5));
        assert!(index.lock().unwrap().snapshot(&key()).is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), 3);
        assert_eq!(current[0].entry.read(&VALUE), Ok(&9));
        assert_eq!(
            current_save
                .saved
                .as_ref()
                .unwrap()
                .attachments
                .entry("chunk_test", "value")
                .unwrap()
                .unwrap()
                .read(&VALUE),
            Ok(&9)
        );
    }

    #[test]
    fn untouched_empty_chunks_do_not_register_before_attachment_access() {
        let index = Index::default();
        let mut map = ChunkMap::default();
        let calls = observe(&mut map, &index, DimensionKey::OVERWORLD);
        map.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let mut saved_empty = Chunk::empty(1, 0);
        saved_empty.saved = Some(Arc::default());
        map.insert((1, 0), Arc::new(saved_empty));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        VALUE
            .set(
                map.attachments_mut(&(0, 0)).unwrap(),
                Holder::Chunk([0, 0]),
                5,
            )
            .unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let batch = index.lock().unwrap().snapshot(&key());
        assert_eq!(batch[0].entry.read(&VALUE), Ok(&5));
    }

    #[test]
    fn chunk_replacement_clear_and_map_drop_retire_only_resident_rows() {
        let index = Index::default();
        let mut map = ChunkMap::default();
        observe(&mut map, &index, DimensionKey::OVERWORLD);
        map.insert((3, 5), saved_chunk());
        let retained = index.lock().unwrap().snapshot(&key());
        let mut replacement = Chunk::empty(3, 5);
        let mut extras = crate::chunk::ChunkExtras::default();
        VALUE
            .set(&mut extras.attachments, Holder::Chunk([3, 5]), 11)
            .unwrap();
        replacement.saved = Some(Arc::new(extras));
        map.insert((3, 5), Arc::new(replacement));
        let replaced = index.lock().unwrap().snapshot(&key());
        assert_eq!(replaced[0].entry.read(&VALUE), Ok(&11));
        map.insert((3, 5), Arc::new(Chunk::empty(3, 5)));
        assert!(index.lock().unwrap().snapshot(&key()).is_empty());
        map.insert((3, 5), saved_chunk());
        map.clear();
        assert!(index.lock().unwrap().snapshot(&key()).is_empty());
        map.insert((3, 5), saved_chunk());
        drop(map);
        assert!(index.lock().unwrap().snapshot(&key()).is_empty());
        assert_eq!(retained[0].entry.read(&VALUE), Ok(&7));
    }

    #[test]
    fn chunk_removal_clear_and_drop_release_mod_payloads_unlocked() {
        #[derive(Clone)]
        struct Probe {
            index: Weak<Mutex<AttachmentIndex<AttachmentHolder>>>,
            drops: Arc<Mutex<Vec<bool>>>,
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                let unlocked = self.index.upgrade().unwrap().try_lock().is_ok();
                self.drops.lock().unwrap().push(unlocked);
            }
        }
        static PROBE: AttachmentType<Probe> = AttachmentType {
            namespace: "chunk_test",
            name: "value",
            default: || unreachable!(),
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        let index = Index::default();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let payload = || {
            let mut chunk = Chunk::empty(3, 5);
            let mut extras = crate::chunk::ChunkExtras::default();
            PROBE
                .set(
                    &mut extras.attachments,
                    Holder::Chunk([3, 5]),
                    Probe {
                        index: Arc::downgrade(&index),
                        drops: drops.clone(),
                    },
                )
                .unwrap();
            chunk.saved = Some(Arc::new(extras));
            Arc::new(chunk)
        };
        let mut map = ChunkMap::default();
        observe(&mut map, &index, DimensionKey::OVERWORLD);
        map.insert((3, 5), payload());
        drop(map.remove(&(3, 5)));
        map.insert((3, 5), payload());
        map.clear();
        map.insert((3, 5), payload());
        drop(map);
        assert_eq!(*drops.lock().unwrap(), [true; 3]);
    }

    #[test]
    fn membership_tokens_track_keys_and_map_replacement_only() {
        let mut map = ChunkMap::default();
        let empty = map.membership_revision();
        map.clear();
        map.remove(&(0, 0));
        assert_eq!(empty, map.membership_revision());
        let chunk = Arc::new(Chunk::empty(0, 0));
        map.insert((0, 0), chunk.clone());
        let inserted = map.membership_revision();
        assert_ne!(empty, inserted);
        map.insert((0, 0), chunk.clone());
        Arc::make_mut(map.get_mut(&(0, 0)).unwrap()).lit = true;
        for column in map.values_mut() {
            Arc::make_mut(column).light_loaded = true;
        }
        crate::light::LightWorld::set_section_light(
            &mut map,
            0,
            0,
            0,
            crate::chunk::Light::Uniform(0),
        );
        assert_eq!(inserted, map.membership_revision());
        map.remove(&(0, 0));
        map.insert((1, 0), chunk);
        assert_eq!(map.len(), 1);
        assert_ne!(inserted, map.membership_revision());
        let replaced = map.membership_revision();
        map.clear();
        assert_ne!(replaced, map.membership_revision());
        let cleared = map.membership_revision();
        map = ChunkMap::default();
        assert_ne!(cleared, map.membership_revision());
    }

    /// The deltas' kinds, checking the ring answers in revision order.
    fn kinds(deltas: Vec<(u64, ChunkMembershipDelta)>) -> Vec<ChunkMembershipDelta> {
        let mut last = 0;
        for (rev, _) in &deltas {
            assert!(*rev > last, "the ring answers in revision order");
            last = *rev;
        }
        deltas.into_iter().map(|(_, d)| d).collect()
    }

    /// Revision numbers are process-wide: the ones another map takes in
    /// between, and a payload-only rewrite that takes none at all, say nothing
    /// about this map's keys.
    #[test]
    fn deltas_since_ignores_revisions_other_maps_take() {
        let mut map = ChunkMap::default();
        let mut other = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        map.insert((0, 0), chunk.clone());
        let seen = map.membership_revision();
        for key in [(1, 0), (2, 0), (3, 0)] {
            other.insert(key, chunk.clone());
        }
        map.insert((0, 0), Arc::new(Chunk::empty(0, 0))); // Payload only.
        map.insert((4, 0), chunk.clone());
        map.remove(&(0, 0));
        assert_eq!(
            kinds(map.deltas_since(seen).unwrap().collect()),
            [
                ChunkMembershipDelta::Added((4, 0)),
                ChunkMembershipDelta::Removed((0, 0)),
            ]
        );
        // The consumer's own revision answers, with nothing new since.
        assert!(
            map.deltas_since(map.membership_revision())
                .unwrap()
                .next()
                .is_none()
        );
        // A revision another map took in between: refused, not replayed.
        assert!(map.deltas_since(other.membership_revision()).is_none());
    }

    /// A map built ahead of the live one, its revisions interleaved with it,
    /// answering for a revision it never took: refused, so the consumer
    /// rebuilds rather than replay another map's changes.
    #[test]
    fn deltas_since_none_for_interleaved_revision_from_another_map() {
        let mut map = ChunkMap::default();
        let mut other = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        map.insert((0, 0), chunk.clone());
        let seen = map.membership_revision();
        other.insert((1, 0), chunk.clone());
        let foreign = other.membership_revision();
        map.insert((2, 0), chunk.clone());
        assert!(map.deltas_since(foreign).is_none());
        assert_eq!(
            kinds(map.deltas_since(seen).unwrap().collect()),
            [ChunkMembershipDelta::Added((2, 0))]
        );
    }

    /// The ring answers for the changes it kept, and only those.
    #[test]
    fn deltas_since_none_for_a_revision_the_ring_dropped() {
        let mut map = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        let created = map.membership_revision();
        let mut revisions = Vec::new();
        let changes = MAX_RECENT_DELTAS as i32 + 1;
        for i in 0..changes {
            map.insert((i, 0), chunk.clone());
            revisions.push(map.membership_revision());
        }
        let (first, kept) = revisions.split_first().unwrap();
        // The oldest change fell out with the consumer's revision: rebuild.
        assert!(map.deltas_since(created).is_none());
        assert_eq!(map.deltas_since(*first).unwrap().count(), kept.len());
    }

    /// A reset rebuilds whatever the consumer last saw, and the ring answers
    /// for what follows it.
    #[test]
    fn deltas_since_none_across_a_clear() {
        let mut map = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        map.insert((0, 0), chunk.clone());
        let before = map.membership_revision();
        map.clear();
        assert!(map.deltas_since(before).is_none());
        let cleared = map.membership_revision();
        map.insert((1, 0), chunk);
        assert_eq!(
            kinds(map.deltas_since(cleared).unwrap().collect()),
            [ChunkMembershipDelta::Added((1, 0))]
        );
    }

    #[test]
    fn fresh_chunks_tracking() {
        let mut map = ChunkMap::default();
        assert!(!map.has_fresh());
        let mut chunk = Chunk::empty(1, 2);
        chunk.fresh = true;
        map.insert((1, 2), Arc::new(chunk));
        assert!(map.has_fresh());
        assert_eq!(map.fresh_keys_ordered(), vec![(1, 2)]);

        map.remove_fresh(&(1, 2));
        assert!(!map.has_fresh());

        let mut chunk2 = Chunk::empty(3, 4);
        chunk2.fresh = true;
        map.insert((3, 4), Arc::new(chunk2));
        assert!(map.has_fresh());

        map.remove(&(3, 4));
        assert!(!map.has_fresh());
    }

    #[test]
    fn fresh_keys_ordered_matches_chunk_map_iteration_order() {
        let mut map = ChunkMap::default();
        let keys = [(-5, 10), (12, -3), (0, 0), (7, 8), (-1, -1), (4, 4)];
        for (i, &key) in keys.iter().enumerate() {
            let mut chunk = Chunk::empty(key.0, key.1);
            chunk.fresh = i % 2 == 0;
            map.insert(key, Arc::new(chunk));
        }

        let ordered = map.fresh_keys_ordered();
        let expected: Vec<Key> = map
            .keys()
            .copied()
            .filter(|k| map.fresh.contains(k))
            .collect();
        assert_eq!(ordered, expected);
        assert_eq!(ordered.len(), 3);
    }

    #[test]
    fn surrounded_follows_loads_unloads_and_clears() {
        // Brute force: the column and its eight neighbours are all resident.
        fn expected(map: &ChunkMap, (x, z): Key) -> bool {
            (-1..=1).all(|dz| (-1..=1).all(|dx| map.contains_key(&(x + dx, z + dz))))
        }
        let mut map = ChunkMap::default();
        let mut seed = 0x2545_f491_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for step in 0..4000 {
            let key = ((next() % 9) as i32 - 4, (next() % 9) as i32 - 4);
            if next() % 3 == 0 {
                map.remove(&key);
            } else {
                // Replacing a resident column changes nothing.
                map.insert(key, Arc::new(Chunk::empty(key.0, key.1)));
            }
            if step == 2500 {
                map.clear();
            }
            for x in -6..=6 {
                for z in -6..=6 {
                    assert_eq!(
                        map.surrounded((x, z)),
                        expected(&map, (x, z)),
                        "{x},{z} at {step}"
                    );
                }
            }
        }
    }

    #[test]
    fn toroidal_grid_handles_growth_collisions_cow_and_extreme_coords() {
        let mut map = ChunkMap::default();
        assert!(map.get_chunk(0, 0).is_none());
        assert!(!map.contains_key(&(0, 0)));

        // Grow through 16x16 -> 64x64 -> 128x128 with RD 32 footprint (-32..=32).
        for cz in -32..=32 {
            for cx in -32..=32 {
                let mut c = Chunk::empty(cx, cz);
                c.set(0, 64, 0, ((cx + 100) * 1000 + (cz + 100)) as u32);
                map.insert((cx, cz), Arc::new(c));
            }
        }
        assert!(
            map.grid_complete,
            "RD 32 fits in 128x128 with no collisions"
        );
        assert_eq!(map.grid_shift, 7);
        for cz in -33..=33 {
            for cx in -33..=33 {
                let expected = (-32..=32).contains(&cx) && (-32..=32).contains(&cz);
                assert_eq!(map.contains_key(&(cx, cz)), expected);
                let got = map.get_chunk(cx, cz);
                assert_eq!(got.is_some(), expected);
                if let Some(c) = got {
                    assert_eq!(c.get(0, 64, 0), ((cx + 100) * 1000 + (cz + 100)) as u32);
                }
            }
        }

        // Unshared chunks mutate in place without COW cloning in both mutate_chunk and get_mut.
        let before_ptr = Arc::as_ptr(map.get(&(0, 0)).unwrap());
        map.mutate_chunk((0, 0), |c| {
            Arc::make_mut(c).set(1, 64, 1, 66);
        });
        assert!(map.grid_complete);
        assert_eq!(Arc::as_ptr(map.get(&(0, 0)).unwrap()), before_ptr);
        assert_eq!(map.get_chunk(0, 0).unwrap().get(1, 64, 1), 66);

        let before_ptr2 = Arc::as_ptr(map.get(&(2, 2)).unwrap());
        Arc::make_mut(map.get_mut(&(2, 2)).unwrap()).set(1, 64, 1, 55);
        assert_eq!(Arc::as_ptr(map.get(&(2, 2)).unwrap()), before_ptr2);
        assert_eq!(map.get_chunk(2, 2).unwrap().get(1, 64, 1), 55);

        // Rebuild via re-insert restores grid_complete; shared chunks still COW cleanly.
        let shared = map.get(&(0, 0)).unwrap().clone();
        map.mutate_chunk((0, 0), |c| {
            Arc::make_mut(c).set(1, 64, 1, 77);
        });
        assert_eq!(map.get_chunk(0, 0).unwrap().get(1, 64, 1), 77);
        assert_eq!(shared.get(1, 64, 1), 66);

        // External get_mut + COW never returns stale data.
        let shared2 = map.get(&(1, 1)).unwrap().clone();
        Arc::make_mut(map.get_mut(&(1, 1)).unwrap()).set(2, 64, 2, 88);
        assert_eq!(map.get_chunk(1, 1).unwrap().get(2, 64, 2), 88);
        assert_eq!(shared2.get(2, 64, 2), 0);

        // Toroidal collision (128 chunks apart) and large negative coordinates.
        let mut far = Chunk::empty(128, 0);
        far.set(0, 64, 0, 999);
        let far_arc = Arc::new(far);
        let far_weak = Arc::downgrade(&far_arc);
        map.insert((128, 0), far_arc);
        let neg_key = (-100_000, -100_000);
        let mut neg_chunk = Chunk::empty(neg_key.0, neg_key.1);
        neg_chunk.set(0, 64, 0, 555);
        map.insert(neg_key, Arc::new(neg_chunk));
        assert!(!map.grid_complete);
        assert_eq!(map.get_chunk(0, 0).unwrap().get(1, 64, 1), 77);
        assert_eq!(map.get_chunk(128, 0).unwrap().get(0, 64, 0), 999);
        assert_eq!(
            map.get_chunk(neg_key.0, neg_key.1).unwrap().get(0, 64, 0),
            555
        );
        assert!(map.contains_key(&(0, 0)));
        assert!(map.contains_key(&(128, 0)));
        assert!(map.contains_key(&neg_key));

        // Remove & clear drop cached Arcs immediately and restore invariants.
        drop(map.remove(&(128, 0)));
        assert_eq!(far_weak.strong_count(), 0);
        assert!(!map.contains_key(&(128, 0)));
        assert!(map.get_chunk(0, 0).is_some());
        map.clear();
        assert!(map.grid_complete);
        assert!(!map.contains_key(&(0, 0)));
        assert!(map.get_chunk(0, 0).is_none());
    }
}
