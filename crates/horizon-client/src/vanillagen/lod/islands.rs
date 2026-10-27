//! Seed-stable small End islands from the world's own feature data.
//!
//! The End's small islands are decoration (`minecraft:end_island_decorated` in
//! `minecraft:small_end_islands`), not terrain density, so the direct far
//! sampler has to estimate them. Refined, visited and saved chunks replace the
//! estimate, exactly as they do for trees and chorus plants.
use super::vegetation::hash;
use crate::{
    fast_hash::FxHashMap,
    lod::{Column, Span, Surface},
};
use horizon_world::{
    registry::BlockStateId,
    worldgen::{biome::BiomeId, generator::GeneratorData},
};
use serde_json::Value;
use std::{collections::VecDeque, sync::Arc};

/// A cell is drawn as a whole column; past this width a 4–6 block island would
/// be a slab. At 8 blocks a whole island is about one cell, which is enough at
/// the distance where direct sampling takes over.
const END_ISLAND_CELL: i32 = 8;
/// `EndIslandFeature` starts at radius 4..=6.
const MAX_RADIUS: i32 = 6;
const CACHE: usize = 4096;

#[derive(Clone, Debug)]
struct Placement {
    /// `rarity_filter`: one attempt per this many chunks.
    rarity: f64,
    /// `count`, as (weight, islands). The vanilla list is 1 (3/4) or 2 (1/4).
    counts: Vec<(f64, i32)>,
    y_min: i32,
    y_max: i32,
    in_square: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Island {
    x: i32,
    z: i32,
    y: i32,
    /// Top radius, 4..=6 blocks.
    radius: i32,
    /// Per-layer shrink in half blocks: 1 = 0.5, 3 = 1.5.
    shrink: i32,
}

pub(super) struct EndIslands {
    /// Only biomes whose feature list contains `minecraft:end_island_decorated`.
    placements: FxHashMap<u16, Placement>,
    seed: u64,
    end_stone: Option<BlockStateId>,
    cache: FxHashMap<(i32, i32, u16), Arc<[Island]>>,
    order: VecDeque<(i32, i32, u16)>,
}
impl EndIslands {
    pub(super) fn new(data: &GeneratorData, map: &[u16], seed: i64) -> Self {
        let mut placements = FxHashMap::default();
        for (index, &biome) in map.iter().enumerate() {
            let Ok(json) = serde_json::from_str::<Value>(data.biome_json(BiomeId(index as u32)))
            else {
                continue;
            };
            let Some(steps) = json["features"].as_array() else {
                continue;
            };
            for step in steps {
                let Some(features) = step.as_array() else {
                    continue;
                };
                for feature in features {
                    let Some(name) = feature.as_str() else {
                        continue;
                    };
                    if let Some(placement) = parse_placement(data, name) {
                        placements.insert(biome, placement);
                    }
                }
            }
        }
        Self {
            placements,
            seed: seed as u64,
            end_stone: data.registry().default_state("minecraft:end_stone"),
            cache: FxHashMap::default(),
            order: VecDeque::new(),
        }
    }
    /// Independent sampling state: the placements are shared, the cache is not.
    pub(super) fn fork(&self) -> Self {
        Self {
            placements: self.placements.clone(),
            seed: self.seed,
            end_stone: self.end_stone,
            cache: FxHashMap::default(),
            order: VecDeque::new(),
        }
    }
    /// Paint the small islands of `biome` into a direct-sampled void column.
    pub(super) fn apply(
        &mut self,
        column: &mut Column,
        biome: u16,
        x: i32,
        z: i32,
        width: i32,
        light: u16,
    ) {
        let Some(state) = self.end_stone else {
            return;
        };
        if width > END_ISLAND_CELL || width <= 0 || column.surface.is_some() {
            return;
        }
        let Some(placement) = self.placements.get(&biome).cloned() else {
            return;
        };
        let (last_x, last_z) = (x + width - 1, z + width - 1);
        let mut spans = Vec::new();
        for cz in (z - MAX_RADIUS).div_euclid(16)..=(last_z + MAX_RADIUS).div_euclid(16) {
            for cx in (x - MAX_RADIUS).div_euclid(16)..=(last_x + MAX_RADIUS).div_euclid(16) {
                let islands = self.layout(cx, cz, biome, &placement);
                for island in islands.iter() {
                    let nx = island.x.clamp(x, last_x);
                    let nz = island.z.clamp(z, last_z);
                    let d2 = (nx - island.x) * (nx - island.x) + (nz - island.z) * (nz - island.z);
                    let Some(bottom) = island_bottom(island, d2) else {
                        continue;
                    };
                    spans.push((bottom, island.y + 1));
                }
            }
        }
        if spans.is_empty() {
            return;
        }
        spans.sort_unstable_by_key(|span| span.0);
        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
        for (min, max) in spans {
            if let Some(last) = merged.last_mut()
                && min <= last.1
            {
                last.1 = last.1.max(max);
            } else {
                merged.push((min, max));
            }
        }
        for &(min, max) in &merged {
            column.push(Span {
                min: min as i16,
                max: max as i16,
                state,
                biome,
                light,
            });
        }
        let top = merged.last().unwrap().1;
        column.surface = Some(Surface {
            height: top,
            state,
            water: None,
            biome,
        });
    }
    fn layout(&mut self, cx: i32, cz: i32, biome: u16, placement: &Placement) -> Arc<[Island]> {
        let key = (cx, cz, biome);
        if let Some(islands) = self.cache.get(&key) {
            return islands.clone();
        }
        let islands: Arc<[Island]> = self.build(cx, cz, placement).into();
        self.cache.insert(key, islands.clone());
        self.order.push_back(key);
        while self.cache.len() > CACHE {
            if let Some(old) = self.order.pop_front() {
                self.cache.remove(&old);
            }
        }
        islands
    }
    #[cfg(test)]
    pub(super) fn has_placement(&self, biome: u16) -> bool {
        self.placements.contains_key(&biome)
    }
    fn build(&self, cx: i32, cz: i32, placement: &Placement) -> Vec<Island> {
        let mut islands = Vec::new();
        let gate = hash(self.seed, cx, cz);
        if (gate as u32 as f64 / u32::MAX as f64) >= 1. / placement.rarity.max(1.) {
            return islands;
        }
        let count = choose_count(&placement.counts, gate.rotate_left(17));
        for i in 0..count {
            let n = hash(
                self.seed ^ (i as u64 + 1).wrapping_mul(0x9e3779b97f4a7c15),
                cx,
                cz,
            );
            let (x, z) = if placement.in_square {
                (
                    cx * 16 + ((n >> 24) & 15) as i32,
                    cz * 16 + ((n >> 32) & 15) as i32,
                )
            } else {
                (cx * 16 + 8, cz * 16 + 8)
            };
            let y = placement.y_min
                + ((n >> 40) % (placement.y_max - placement.y_min + 1) as u64) as i32;
            let radius = 4 + ((n >> 48) % 3) as i32;
            let shrink = 1 + 2 * ((n >> 52) & 1) as i32;
            islands.push(Island {
                x,
                z,
                y,
                radius,
                shrink,
            });
        }
        islands
    }
}

fn parse_placement(data: &GeneratorData, name: &str) -> Option<Placement> {
    let json: Value = serde_json::from_str(data.placed_feature_json(name)?).ok()?;
    if json["feature"].as_str()? != "minecraft:end_island" {
        return None;
    }
    let mut placement = Placement {
        rarity: 1.,
        counts: vec![(1., 1)],
        y_min: 55,
        y_max: 70,
        in_square: false,
    };
    for modifier in json["placement"].as_array()? {
        let kind = modifier["type"].as_str().unwrap_or("");
        match kind.strip_prefix("minecraft:").unwrap_or(kind) {
            "rarity_filter" => {
                placement.rarity = modifier["chance"].as_f64().unwrap_or(1.).max(1.);
            }
            "count" => placement.counts = counts(&modifier["count"]),
            "height_range" => {
                if let Some(min) = absolute(&modifier["height"]["min_inclusive"]) {
                    placement.y_min = min;
                }
                if let Some(max) = absolute(&modifier["height"]["max_inclusive"]) {
                    placement.y_max = max;
                }
            }
            "in_square" => placement.in_square = true,
            _ => {}
        }
    }
    Some(placement)
}
fn counts(value: &Value) -> Vec<(f64, i32)> {
    if let Some(count) = value.as_i64() {
        return vec![(1., count as i32)];
    }
    let entries = value["distribution"].as_array();
    if let Some(entries) = entries {
        let list: Vec<_> = entries
            .iter()
            .filter_map(|entry| Some((entry["weight"].as_f64()?, entry["data"].as_i64()? as i32)))
            .collect();
        if !list.is_empty() {
            return list;
        }
    }
    vec![(1., 1)]
}
fn absolute(value: &Value) -> Option<i32> {
    value
        .as_i64()
        .or_else(|| value["absolute"].as_i64())
        .map(|value| value as i32)
}
fn choose_count(counts: &[(f64, i32)], n: u64) -> usize {
    let total: f64 = counts.iter().map(|(weight, _)| weight).sum();
    let mut value = (n as u32 as f64 / u32::MAX as f64) * total;
    for &(weight, count) in counts {
        value -= weight;
        if value <= 0. {
            return count.max(0) as usize;
        }
    }
    counts
        .last()
        .map_or(1, |(_, count)| (*count).max(0) as usize)
}
/// The lowest layer of the island's cone that reaches a column at squared
/// distance `d2`. The top layer is always `island.y` when this returns `Some`.
fn island_bottom(island: &Island, d2: i32) -> Option<i32> {
    let mut half = island.radius * 2;
    let mut y = island.y;
    let mut bottom = None;
    while half > 1 {
        if d2 * 4 <= half * half {
            bottom = Some(y);
        }
        half -= island.shrink;
        y -= 1;
    }
    bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement() -> Placement {
        Placement {
            rarity: 14.,
            counts: vec![(3., 1), (1., 2)],
            y_min: 55,
            y_max: 70,
            in_square: true,
        }
    }
    fn estimator() -> EndIslands {
        EndIslands {
            placements: FxHashMap::default(),
            seed: 7,
            end_stone: None,
            cache: FxHashMap::default(),
            order: VecDeque::new(),
        }
    }

    #[test]
    fn small_islands_match_the_placed_feature_density_and_shape() {
        let placement = placement();
        let estimator = estimator();
        let mut chunks = 0;
        let mut with_islands = 0;
        let mut islands = 0;
        for cx in 0..256 {
            for cz in 0..256 {
                let built = estimator.build(cx, cz, &placement);
                chunks += 1;
                if !built.is_empty() {
                    with_islands += 1;
                }
                islands += built.len();
                for island in &built {
                    assert!((55..=70).contains(&island.y), "y {}", island.y);
                    assert!((4..=6).contains(&island.radius), "radius {}", island.radius);
                    assert!(island.shrink == 1 || island.shrink == 3);
                    assert!(
                        (0..16).contains(&(island.x - cx * 16))
                            && (0..16).contains(&(island.z - cz * 16))
                    );
                }
            }
        }
        let per_chunk = islands as f64 / chunks as f64;
        assert!(
            (0.07..=0.12).contains(&per_chunk),
            "islands per chunk: {per_chunk}"
        );
        let share = with_islands as f64 / chunks as f64;
        assert!((0.045..=0.10).contains(&share), "non-empty chunks: {share}");
        assert!(islands > with_islands, "some chunks hold two islands");
    }

    #[test]
    fn island_columns_are_cones_with_a_flat_top() {
        let island = Island {
            x: 0,
            z: 0,
            y: 60,
            radius: 4,
            shrink: 1,
        };
        assert_eq!(island_bottom(&island, 0), Some(54));
        assert_eq!(island_bottom(&island, 3), Some(56));
        assert_eq!(island_bottom(&island, 16), Some(60));
        assert_eq!(island_bottom(&island, 17), None);
        let steep = Island {
            shrink: 3,
            ..island
        };
        assert_eq!(island_bottom(&steep, 0), Some(58));
    }
}
