//! Stable entity material grouping with renderer-owned, reusable CPU scratch.
use super::Vertex;

#[derive(Default)]
pub(super) struct EntityBatch {
    // Quad indices avoid copying vertices twice while grouping materials.
    // Keep initialized high-water storage; local counts delimit active entries.
    // Each bucket can hold all quads, removing per-quad Vec growth checks.
    buckets: [Vec<usize>; 3],
    pub(super) combined: Vec<Vertex>,
}

impl EntityBatch {
    pub(super) fn update(
        &mut self,
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
        see_through: &[Vertex],
    ) -> [u32; 3] {
        let quads = entities.as_chunks::<4>().0;
        for bucket in &mut self.buckets {
            if bucket.len() < quads.len() {
                bucket.resize(quads.len(), 0);
            }
        }
        let mut counts = [0usize; 3];
        for (index, quad) in quads.iter().enumerate() {
            let material = match quad[0].layer {
                11 => 1,
                12 => 2,
                _ => 0,
            };
            self.buckets[material][counts[material]] = index;
            counts[material] += 1;
        }
        let entity_vertices = quads.len() * 4;
        self.combined.reserve(
            (entity_vertices + additive.len() + hand.len() + see_through.len())
                .saturating_sub(self.combined.len()),
        );
        self.combined
            .resize(entity_vertices, bytemuck::Zeroable::zeroed());
        if counts.contains(&quads.len()) {
            // Uniform materials are already ordered; copy the whole batch.
            self.combined.copy_from_slice(&entities[..entity_vertices]);
        } else {
            let mut offset = 0;
            for (bucket, &count) in self.buckets.iter().zip(&counts) {
                let bucket = &bucket[..count];
                let end = offset + count * 4;
                for (output, &index) in self.combined[offset..end]
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(bucket)
                {
                    *output = quads[index];
                }
                offset = end;
            }
        }
        self.combined.extend_from_slice(additive);
        self.combined.extend_from_slice(hand);
        self.combined.extend_from_slice(see_through);
        counts.map(|count| count as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{hint::black_box, time::Instant};

    // Retain the original implementation as the equivalence/benchmark oracle.
    fn baseline(
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
    ) -> (Vec<Vertex>, [u32; 3]) {
        let mut combined = Vec::with_capacity(entities.len() + additive.len() + hand.len());
        let mut counts = [0; 3];
        for (i, layer) in [0, 11, 12].into_iter().enumerate() {
            for quad in entities.as_chunks::<4>().0 {
                let material = match quad[0].layer {
                    11 => 11,
                    12 => 12,
                    _ => 0,
                };
                if material == layer {
                    combined.extend_from_slice(quad);
                    counts[i] += 1;
                }
            }
        }
        combined.extend_from_slice(additive);
        combined.extend_from_slice(hand);
        (combined, counts)
    }

    fn geometry(quads: usize, mixed: bool) -> Vec<Vertex> {
        (0..quads * 4)
            .map(|index| Vertex {
                pos: [index as f32, 2.0, 3.0],
                uv: [index as f32, 5.0],
                layer: if mixed {
                    [7, 11, 12, 4, 0][index / 4 % 5]
                } else {
                    0
                },
                color: index as u32,
                light: (index * 3) as u32,
                normal: [1, 2, 3, 4],
            })
            .collect()
    }

    fn equivalent(
        batch: &mut EntityBatch,
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
    ) {
        let (expected, counts) = baseline(entities, additive, hand);
        assert_eq!(batch.update(entities, additive, hand, &[]), counts);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&batch.combined),
            bytemuck::cast_slice::<_, u8>(&expected)
        );
    }

    #[test]
    fn stable_material_order_tails_and_reuse() {
        let mut batch = EntityBatch::default();
        let mut entities = geometry(4096, true);
        // Only the first vertex determines a quad's material.
        entities[1].layer = 12;
        let additive = geometry(3, true);
        let hand = geometry(2, false);
        // Warm every bucket and the combined buffer to their maximum sizes.
        equivalent(&mut batch, &entities, &additive, &hand);
        let capacities = batch.buckets.each_ref().map(|b| b.capacity());
        let capacity = batch.combined.capacity();
        let pointer = batch.combined.as_ptr();
        for length in [0, 1, 3, 4, 511, entities.len(), 17, 0, entities.len()] {
            equivalent(&mut batch, &entities[..length], &additive[..5], &hand[..3]);
            assert_eq!(batch.buckets.each_ref().map(|b| b.capacity()), capacities);
            assert_eq!(batch.combined.capacity(), capacity);
            assert_eq!(batch.combined.as_ptr(), pointer);
        }
        equivalent(&mut batch, &[], &[], &[]);
        assert!(batch.combined.is_empty());
        // Grow again, then confirm empty/small frames retain the new storage.
        let larger = geometry(8192, true);
        equivalent(&mut batch, &larger, &additive, &hand);
        let capacities = batch.buckets.each_ref().map(|b| b.capacity());
        let capacity = batch.combined.capacity();
        for entities in [&larger[..7], &larger[..], &[]] {
            equivalent(&mut batch, entities, &[], &[]);
            assert_eq!(batch.buckets.each_ref().map(|b| b.capacity()), capacities);
            assert_eq!(batch.combined.capacity(), capacity);
        }
    }

    #[test]
    #[ignore = "manual production-helper microbenchmark"]
    fn benchmark_entity_batch() {
        let additive = geometry(16, false);
        let hand = geometry(8, false);
        for (name, mixed, quads) in [
            ("mixed", true, 65_536),
            ("single", false, 65_536),
            ("runs", true, 65_536),
            ("mixed", true, 262_144),
            ("single", false, 262_144),
        ] {
            let mut entities = geometry(quads, mixed);
            if name == "runs" {
                // Model/material runs: normal 16, glint 4, overlay 4, shadow 2.
                for (index, quad) in entities.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let layer = match index % 26 {
                        0..16 => 0,
                        16..20 => 11,
                        20..24 => 12,
                        _ => 4,
                    };
                    for vertex in quad {
                        vertex.layer = layer;
                    }
                }
            }
            let mut batch = EntityBatch::default();
            equivalent(&mut batch, &entities, &additive, &hand);
            for _ in 0..50 {
                black_box(baseline(black_box(&entities), &additive, &hand));
                black_box(batch.update(black_box(&entities), &additive, &hand, &[]));
                black_box(&batch.combined);
            }
            let mut before = Vec::new();
            let mut after = Vec::new();
            for _ in 0..15 {
                let start = Instant::now();
                for _ in 0..100 {
                    black_box(baseline(black_box(&entities), &additive, &hand));
                }
                before.push(start.elapsed().as_nanos() / 100);
                let start = Instant::now();
                for _ in 0..100 {
                    black_box(batch.update(black_box(&entities), &additive, &hand, &[]));
                    black_box(&batch.combined);
                }
                after.push(start.elapsed().as_nanos() / 100);
            }
            before.sort_unstable();
            after.sort_unstable();
            println!(
                "{name}: {quads} quads, baseline={} ns/frame optimized={} ns/frame speedup={:.2}x scratch={} bytes",
                before[7],
                after[7],
                before[7] as f64 / after[7] as f64,
                batch.combined.capacity() * std::mem::size_of::<Vertex>()
                    + batch
                        .buckets
                        .iter()
                        .map(|b| b.capacity() * std::mem::size_of::<usize>())
                        .sum::<usize>()
            );
        }
    }
}
