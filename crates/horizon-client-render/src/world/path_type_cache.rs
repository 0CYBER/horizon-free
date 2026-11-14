//! One direct-mapped static path cache per logical server level, shared by its workers.
use super::World;
use horizon_core::position::BlockPos;
use mod_api::PathType;
use std::sync::Mutex;

const SLOTS: usize = 4096;
#[derive(Default)]
struct Entry {
    packed: u64,
    kind: Option<PathType>,
}
pub(super) struct PathTypeCache {
    slots: Box<[Mutex<Entry>]>,
}
fn slot(packed: u64) -> usize {
    let mut mixed = packed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    mixed ^= mixed >> 32;
    mixed ^= mixed >> 16;
    mixed as usize & (SLOTS - 1)
}
impl PathTypeCache {
    fn new() -> Self {
        Self {
            slots: (0..SLOTS).map(|_| Mutex::new(Entry::default())).collect(),
        }
    }
    fn get_or_compute(&self, pos: BlockPos, compute: impl FnOnce() -> PathType) -> PathType {
        let packed = pos.pack() as u64;
        let cell = &self.slots[slot(packed)];
        {
            let entry = cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if entry.packed == packed
                && let Some(kind) = entry.kind
            {
                return kind;
            }
        }
        // In particular, never hold a slot lock while a mod runs, even on a miss.
        let kind = compute();
        *cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Entry {
            packed,
            kind: Some(kind),
        };
        kind
    }
    fn invalidate(&self, pos: BlockPos) {
        let packed = pos.pack() as u64;
        let mut entry = self.slots[slot(packed)]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entry.packed == packed {
            entry.kind = None;
        }
    }
}
impl World {
    /// Initialize once at the logical server's mob-view boundary, before workers
    /// borrow the World. Clients and raw storage worlds leave this uninitialized.
    pub fn enable_path_type_cache(&self) {
        self.path_type_cache.get_or_init(PathTypeCache::new);
    }
    /// Cache the final static classification, including Open for void air.
    /// A hit takes one slot lock, a miss two; callbacks run outside both guards.
    /// World writes cannot overlap its safe shared worker borrows.
    pub fn cached_path_type(&self, pos: BlockPos, compute: impl FnOnce() -> PathType) -> PathType {
        match self.path_type_cache.get() {
            Some(cache) => cache.get_or_compute(pos, compute),
            None => compute(),
        }
    }
    /// The cache side of sendBlockUpdated, independent of optional client logs.
    /// Ordinary writes call this only after their actual-change, UPDATE_CLIENTS
    /// and full-status checks. Direct block-entity notifications clear even when
    /// state is unchanged. Raw writes, time, reload and chunk lifetime do not clear.
    pub fn path_type_block_updated(&self, pos: BlockPos) {
        if let Some(cache) = self.path_type_cache.get() {
            cache.invalidate(pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::registry::BlockStateId;
    use std::cell::Cell;

    #[test]
    fn slot_matches_long_mix_vectors() {
        for ((x, y, z), expected) in [
            ((0, 0, 0), 0),
            ((1, 64, -1), 1270),
            ((100, 70, 200), 1753),
            ((-30_000_000, -64, 29_999_999), 69),
        ] {
            assert_eq!(slot(BlockPos::new(x, y, z).pack() as u64), expected);
        }
    }
    #[test]
    fn collisions_replace_but_unrelated_notifications_do_not_evict() {
        // Pigeonhole principle supplies two distinct keys in one slot; hash
        // correctness is pinned independently by the literal vectors above.
        let mut previous = [None; SLOTS];
        let (a, b) = (0..=SLOTS)
            .find_map(|x| {
                let pos = BlockPos::new(x as i32, 64, 0);
                let index = slot(pos.pack() as u64);
                let old = previous[index].replace(pos);
                old.map(|old| (old, pos))
            })
            .unwrap();
        let cache = PathTypeCache::new();
        assert_eq!(cache.get_or_compute(a, || PathType::Open), PathType::Open);
        cache.invalidate(b);
        assert_eq!(
            cache.get_or_compute(a, || panic!("unrelated key cleared a hit")),
            PathType::Open
        );
        assert_eq!(
            cache.get_or_compute(b, || PathType::Blocked),
            PathType::Blocked
        );
        assert_eq!(
            cache.get_or_compute(a, || PathType::Damaging),
            PathType::Damaging
        );
        cache.invalidate(a);
        assert_eq!(cache.get_or_compute(a, || PathType::Water), PathType::Water);
    }
    #[test]
    fn raw_worlds_do_not_cache_and_raw_writes_do_not_invalidate_server_cache() {
        let mut world = World::new(BlockStateId(0), 0);
        let pos = BlockPos::new(0, 64, 0);
        let calls = Cell::new(0);
        let compute = || {
            calls.set(calls.get() + 1);
            PathType::Open
        };
        world.cached_path_type(pos, compute);
        world.cached_path_type(pos, compute);
        assert_eq!(calls.get(), 2);
        world.enable_path_type_cache();
        world.cached_path_type(pos, compute);
        world.cached_path_type(pos, compute);
        assert_eq!(calls.get(), 3);
        // A storage edit without sendBlockUpdated must leave even a cached
        // void-air Open stale, matching the absence of UPDATE_CLIENTS.
        world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
        );
        world.set(0, 64, 0, BlockStateId(1)).unwrap();
        assert_eq!(
            world.cached_path_type(pos, || PathType::Blocked),
            PathType::Open
        );
        world.path_type_block_updated(pos);
        assert_eq!(
            world.cached_path_type(pos, || PathType::Blocked),
            PathType::Blocked
        );
    }
}
