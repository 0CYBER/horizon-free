//! One direct-mapped static path cache per logical server level, shared by its workers.
use super::World;
use horizon_core::position::BlockPos;
use mod_api::PathType;
use std::hint::spin_loop;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering, fence};

const SLOTS: usize = 4096;
/// Sentinel stored in [`Entry::kind`] when a slot is empty or invalidated.
const EMPTY: u8 = u8::MAX;
const _: () = assert!(PathType::COUNT < EMPTY as usize);
/// Bounded optimistic retry count for seqlock readers and cache-fill writers.
const MAX_OPTIMISTIC_RETRIES: usize = 4;

struct Entry {
    /// Seqlock generation: even = stable snapshot, odd = writer in progress.
    seq: AtomicU64,
    /// `BlockPos::pack() as u64`.
    packed: AtomicU64,
    /// `PathType as u8`, or [`EMPTY`].
    kind: AtomicU8,
}
const _: () = assert!(std::mem::size_of::<Entry>() <= 24);
impl Entry {
    fn new() -> Self {
        Self {
            seq: AtomicU64::new(0),
            packed: AtomicU64::new(0),
            kind: AtomicU8::new(EMPTY),
        }
    }
    #[inline]
    fn try_get(&self, packed: u64) -> Option<PathType> {
        for _ in 0..MAX_OPTIMISTIC_RETRIES {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 & 1 != 0 {
                spin_loop();
                continue;
            }
            let p = self.packed.load(Ordering::Relaxed);
            let k = self.kind.load(Ordering::Relaxed);
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 {
                return if p == packed && usize::from(k) < PathType::COUNT {
                    Some(PathType::ALL[usize::from(k)])
                } else {
                    None
                };
            }
            spin_loop();
        }
        None
    }
    #[inline]
    fn try_store(&self, packed: u64, kind: PathType) {
        let mut s = self.seq.load(Ordering::Relaxed);
        for _ in 0..MAX_OPTIMISTIC_RETRIES {
            if s & 1 != 0 {
                spin_loop();
                s = self.seq.load(Ordering::Relaxed);
                continue;
            }
            match self.seq.compare_exchange(
                s,
                s.wrapping_add(1),
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // Release fence orders the odd-seq publication before payload writes
                    // so concurrent readers' acquire fence + seq re-read detects any tear.
                    fence(Ordering::Release);
                    self.packed.store(packed, Ordering::Relaxed);
                    self.kind.store(kind as u8, Ordering::Relaxed);
                    self.seq.store(s.wrapping_add(2), Ordering::Release);
                    return;
                }
                Err(observed) => {
                    s = observed;
                    spin_loop();
                }
            }
        }
    }
    #[inline]
    fn invalidate(&self, packed: u64) {
        // Fast lock-free check: an unrelated or already-empty stable slot needs
        // no RMW and dirties no cache line.
        let s1 = self.seq.load(Ordering::Acquire);
        if s1 & 1 == 0 {
            let p = self.packed.load(Ordering::Relaxed);
            let k = self.kind.load(Ordering::Relaxed);
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 && (p != packed || k == EMPTY) {
                return;
            }
        }
        let mut s = self.seq.load(Ordering::Relaxed);
        loop {
            if s & 1 != 0 {
                spin_loop();
                s = self.seq.load(Ordering::Relaxed);
                continue;
            }
            match self.seq.compare_exchange_weak(
                s,
                s.wrapping_add(1),
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    if self.packed.load(Ordering::Relaxed) == packed
                        && self.kind.load(Ordering::Relaxed) != EMPTY
                    {
                        fence(Ordering::Release);
                        self.kind.store(EMPTY, Ordering::Relaxed);
                    }
                    self.seq.store(s.wrapping_add(2), Ordering::Release);
                    return;
                }
                Err(observed) => {
                    s = observed;
                    spin_loop();
                }
            }
        }
    }
}
pub(super) struct PathTypeCache {
    slots: Box<[Entry]>,
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
            slots: (0..SLOTS).map(|_| Entry::new()).collect(),
        }
    }
    fn get_or_compute(&self, pos: BlockPos, compute: impl FnOnce() -> PathType) -> PathType {
        let packed = pos.pack() as u64;
        let cell = &self.slots[slot(packed)];
        if let Some(kind) = cell.try_get(packed) {
            return kind;
        }
        // In particular, never hold a slot write sequence while a mod runs, even on a miss.
        let kind = compute();
        cell.try_store(packed, kind);
        kind
    }
    fn invalidate(&self, pos: BlockPos) {
        let packed = pos.pack() as u64;
        self.slots[slot(packed)].invalidate(packed);
    }
}
impl World {
    /// Initialize once at the logical server's mob-view boundary, before workers
    /// borrow the World. Clients and raw storage worlds leave this uninitialized.
    pub fn enable_path_type_cache(&self) {
        self.path_type_cache.get_or_init(PathTypeCache::new);
    }
    /// Cache the final static classification, including Open for void air.
    /// Hits read lock-free through a per-slot seqlock with no RMW; misses run
    /// `compute` outside the slot write sequence before publishing.
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
    #[test]
    fn seqlock_slot_size_and_all_path_types_round_trip() {
        let cache = PathTypeCache::new();
        assert!(
            cache.slots.len() * std::mem::size_of::<Entry>() <= 96 * 1024,
            "4096-slot table should stay within 96 KiB"
        );
        // Origin packs to 0; verify EMPTY sentinel distinguishes an empty slot from (0, 0, 0).
        let origin = BlockPos::new(0, 0, 0);
        assert_eq!(origin.pack(), 0);
        assert_eq!(cache.slots[slot(0)].try_get(0), None);
        for (i, &kind) in PathType::ALL.iter().enumerate() {
            let pos = if i == 0 {
                origin
            } else {
                BlockPos::new(i as i32 * 17, (i as i32) - 10, -(i as i32) * 13)
            };
            assert_eq!(cache.get_or_compute(pos, || kind), kind);
            assert_eq!(
                cache.get_or_compute(pos, || panic!("expected seqlock hit for {kind:?}")),
                kind
            );
            cache.invalidate(pos);
            let replacement = PathType::ALL[(i + 1) % PathType::COUNT];
            assert_eq!(cache.get_or_compute(pos, || replacement), replacement);
            cache.invalidate(pos);
        }
    }
    #[test]
    fn multithreaded_seqlock_stress_never_tears_or_deadlocks() {
        let cache = PathTypeCache::new();
        // Collect colliding keys across two hot slots so threads hammer the exact same cells.
        let target_slots = [
            slot(BlockPos::new(0, 64, 0).pack() as u64),
            slot(BlockPos::new(1, 64, 0).pack() as u64),
        ];
        let mut keys = Vec::new();
        for target in target_slots {
            let mut found = 0;
            for x in 0..(SLOTS * 8) as i32 {
                let pos = BlockPos::new(x, 64, target as i32);
                if slot(pos.pack() as u64) == target {
                    keys.push(pos);
                    found += 1;
                    if found == 4 {
                        break;
                    }
                }
            }
            assert_eq!(found, 4);
        }
        let expected: Vec<PathType> = (0..keys.len())
            .map(|i| PathType::ALL[(i * 3 + 1) % PathType::COUNT])
            .collect();
        std::thread::scope(|scope| {
            for tid in 0..6usize {
                let cache = &cache;
                let keys = &keys;
                let expected = &expected;
                scope.spawn(move || {
                    for iter in 0..4000usize {
                        let idx = (tid * 131 + iter * 7) % keys.len();
                        let pos = keys[idx];
                        let want = expected[idx];
                        let got = cache.get_or_compute(pos, || want);
                        assert_eq!(got, want, "torn seqlock read on slot collision");
                    }
                });
            }
            for tid in 0..2usize {
                let cache = &cache;
                let keys = &keys;
                scope.spawn(move || {
                    for iter in 0..4000usize {
                        let idx = (tid * 97 + iter * 11) % keys.len();
                        cache.invalidate(keys[idx]);
                    }
                });
            }
        });
        // Every slot must remain unlocked (even seq) and cleanly invalidatable after stress.
        for &pos in &keys {
            cache.invalidate(pos);
        }
        for target in target_slots {
            assert_eq!(cache.slots[target].seq.load(Ordering::Relaxed) & 1, 0);
        }
        for &pos in &keys {
            assert_eq!(
                cache.get_or_compute(pos, || PathType::WaterBorder),
                PathType::WaterBorder
            );
            assert_eq!(
                cache.get_or_compute(pos, || panic!("expected cached WaterBorder after stress")),
                PathType::WaterBorder
            );
            cache.invalidate(pos);
        }
    }
}
