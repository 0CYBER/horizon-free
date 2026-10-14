//! Resident-key synchronization used by Game (registration callback work excluded).
//! Run: CARGO_INCREMENTAL=0 cargo run -p horizon-client --profile fast --example tick_chunk_membership_bench
use horizon_client::{
    chunk::Chunk, chunk_map::ChunkMap, tick_chunk_membership::TickChunkMembership,
};
use std::{collections::HashSet, hint::black_box, sync::Arc, time::Instant};

type Key = (i32, i32);
#[derive(Default)]
struct Old {
    keys: HashSet<Key>,
}
impl Old {
    fn sync(&mut self, map: &ChunkMap) -> (usize, usize) {
        let loaded: HashSet<_> = map.keys().copied().collect();
        let mut counts = (0, 0);
        for key in loaded.difference(&self.keys.clone()) {
            black_box(key);
            counts.0 += 1;
        }
        for key in self.keys.difference(&loaded) {
            black_box(key);
            counts.1 += 1;
        }
        self.keys = loaded;
        counts
    }
}
fn new_sync(cache: &mut TickChunkMembership, map: &ChunkMap) -> (usize, usize) {
    if !cache.synchronize(map) {
        return (0, 0);
    }
    let added = cache.take_added();
    for key in &added {
        black_box(key);
    }
    for key in cache.removed() {
        black_box(key);
    }
    let counts = (added.len(), cache.removed().len());
    cache.recycle_added(added);
    counts
}
fn map(offset: i32) -> ChunkMap {
    let mut map = ChunkMap::default();
    // Share a payload: the synchronization path reads only keys, not block data.
    let chunk = Arc::new(Chunk::empty(0, 0));
    for x in -16..=16 {
        for z in -16..=16 {
            map.insert((x + offset, z), chunk.clone());
        }
    }
    map
}
fn run(new: bool, scenario: usize, iterations: usize) -> (f64, (usize, usize)) {
    let mut a = map(0);
    let b = map(33);
    let chunk = Arc::new(Chunk::empty(0, 0));
    let mut old = Old::default();
    let mut cache = TickChunkMembership::default();
    for _ in 0..64 {
        if new {
            black_box(new_sync(&mut cache, &a));
        } else {
            black_box(old.sync(&a));
        }
    }
    let mut total = (0, 0);
    let start = Instant::now();
    for i in 0..iterations {
        let resident = match scenario {
            0 => &a,
            1 => {
                let (previous, next) = if i % 2 == 0 {
                    ((-16, -16), (-17, -16))
                } else {
                    ((-17, -16), (-16, -16))
                };
                a.remove(&previous);
                a.insert(next, chunk.clone());
                &a
            }
            2 => {
                if i % 2 == 0 {
                    &b
                } else {
                    &a
                }
            }
            _ => unreachable!(),
        };
        let counts = if new {
            new_sync(black_box(&mut cache), black_box(resident))
        } else {
            old.sync(black_box(resident))
        };
        total.0 += counts.0;
        total.1 += counts.1;
    }
    (
        start.elapsed().as_secs_f64() * 1e9 / iterations as f64,
        black_box(total),
    )
}
fn main() {
    println!("1089 keys, 64 warmup calls, 9 samples; ns/sync, median");
    for (scenario, name, iterations) in [
        (0, "stable", 10_000),
        (1, "one-key same-count replacement", 2_000),
        (2, "whole-map replacement", 1_000),
    ] {
        let mut before = Vec::new();
        let mut after = Vec::new();
        for sample in 0..9 {
            let (old, new) = if sample % 2 == 0 {
                (
                    run(false, scenario, iterations),
                    run(true, scenario, iterations),
                )
            } else {
                let new = run(true, scenario, iterations);
                (run(false, scenario, iterations), new)
            };
            assert_eq!(old.1, new.1);
            let expected = match scenario {
                0 => (0, 0),
                1 => (iterations, iterations),
                _ => (1089 * iterations, 1089 * iterations),
            };
            assert_eq!(new.1, expected);
            before.push(old.0);
            after.push(new.0);
        }
        before.sort_by(f64::total_cmp);
        after.sort_by(f64::total_cmp);
        println!(
            "{name}: {:.1} -> {:.1} ns, {:.2}x; {iterations} iterations/sample",
            before[4],
            after[4],
            before[4] / after[4]
        );
    }
}
