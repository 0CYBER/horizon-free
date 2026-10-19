//! Seed-stable visual vegetation from the world's own feature data.
//! shortcut: these positions approximate native placement; saved/visited and
//! completed native chunks replace them rather than changing gameplay worldgen.
use crate::{
    fast_hash::FxHashMap,
    lod::{Column, Span, Surface},
};
use horizon_world::{
    registry::{BlockRegistry, BlockStateId},
    worldgen::generator::GeneratorData,
};
use serde_json::Value;

#[derive(Clone, Copy)]
struct Tree {
    leaves: BlockStateId,
    log: BlockStateId,
    height: i32,
    variation: i32,
    radius: i32,
    count: f64,
}
pub(super) struct Vegetation {
    trees: FxHashMap<u16, Vec<Tree>>,
    seed: u64,
    registry: &'static BlockRegistry,
}
fn mean(v: &Value) -> f64 {
    if let Some(n) = v.as_f64() {
        return n;
    }
    let distribution = v["distribution"].as_array();
    if let Some(entries) = distribution {
        let total: f64 = entries
            .iter()
            .map(|e| e["weight"].as_f64().unwrap_or(0.))
            .sum();
        if total > 0. {
            return entries
                .iter()
                .map(|e| mean(&e["data"]) * e["weight"].as_f64().unwrap_or(0.))
                .sum::<f64>()
                / total;
        }
    }
    (v["min_inclusive"].as_f64().unwrap_or(0.) + v["max_inclusive"].as_f64().unwrap_or(0.)) * 0.5
}
fn block(v: &Value, registry: &BlockRegistry) -> Option<BlockStateId> {
    let name = v
        .as_str()
        .or_else(|| v["id"].as_str())
        .or_else(|| v["Name"].as_str())?;
    registry.default_state(name)
}
fn gather(
    data: &GeneratorData,
    value: &Value,
    placed: bool,
    count: f64,
    depth: usize,
    out: &mut Vec<Tree>,
) {
    if depth > 12 || count <= 0. {
        return;
    }
    if let Some(name) = value.as_str() {
        let json = if placed {
            data.placed_feature_json(name)
        } else {
            data.configured_feature_json(name)
        };
        if let Some(json) = json.and_then(|s| serde_json::from_str::<Value>(s).ok()) {
            gather(data, &json, placed, count, depth + 1, out);
        }
        return;
    }
    if placed {
        let mut count = count;
        if let Some(modifiers) = value["placement"].as_array() {
            for modifier in modifiers {
                match modifier["type"]
                    .as_str()
                    .unwrap_or("")
                    .strip_prefix("minecraft:")
                    .unwrap_or("")
                {
                    "count" | "count_on_every_layer" => count *= mean(&modifier["count"]),
                    "rarity_filter" => count /= modifier["chance"].as_f64().unwrap_or(1.).max(1.),
                    "count_extra" => {
                        count *= modifier["count"].as_f64().unwrap_or(0.)
                            + modifier["extra_count"].as_f64().unwrap_or(0.)
                                * modifier["extra_chance"].as_f64().unwrap_or(0.)
                    }
                    _ => {}
                }
            }
        }
        gather(data, &value["feature"], false, count, depth + 1, out);
        return;
    }
    let config = if value["config"].is_object() {
        &value["config"]
    } else {
        value
    };
    match value["type"].as_str().unwrap_or("") {
        "minecraft:tree" => {
            let (Some(leaves), Some(log)) = (
                block(&config["foliage_provider"], data.registry()),
                block(&config["trunk_provider"], data.registry()),
            ) else {
                return;
            };
            let trunk = &config["trunk_placer"];
            out.push(Tree {
                leaves,
                log,
                height: trunk["base_height"].as_i64().unwrap_or(5) as i32,
                variation: (trunk["height_rand_a"].as_i64().unwrap_or(0)
                    + trunk["height_rand_b"].as_i64().unwrap_or(0))
                    as i32,
                radius: mean(&config["foliage_placer"]["radius"])
                    .round()
                    .clamp(2., 6.) as i32,
                count,
            });
        }
        "minecraft:random_selector" => {
            let mut remaining = count;
            if let Some(features) = config["features"].as_array() {
                for entry in features {
                    let chance = entry["chance"].as_f64().unwrap_or(0.).clamp(0., 1.);
                    gather(
                        data,
                        &entry["feature"],
                        true,
                        remaining * chance,
                        depth + 1,
                        out,
                    );
                    remaining *= 1. - chance;
                }
            }
            gather(data, &config["default"], true, remaining, depth + 1, out);
        }
        "minecraft:random_boolean_selector" => {
            for key in ["feature_true", "feature_false"] {
                gather(data, &config[key], true, count * 0.5, depth + 1, out);
            }
        }
        "minecraft:simple_random_selector" => {
            if let Some(features) = config["features"].as_array() {
                for feature in features {
                    gather(
                        data,
                        feature,
                        true,
                        count / features.len().max(1) as f64,
                        depth + 1,
                        out,
                    );
                }
            }
        }
        _ => {}
    }
}
fn hash(seed: u64, x: i32, z: i32) -> u64 {
    let mut n = seed
        ^ (x as u64).wrapping_mul(0x9e3779b97f4a7c15)
        ^ (z as u64).wrapping_mul(0xd1b54a32d192ed03);
    n = (n ^ n >> 30).wrapping_mul(0xbf58476d1ce4e5b9);
    n = (n ^ n >> 27).wrapping_mul(0x94d049bb133111eb);
    n ^ n >> 31
}
impl Vegetation {
    pub(super) fn new(data: &GeneratorData, map: &[u16], seed: i64) -> Self {
        let mut trees = FxHashMap::default();
        for (index, &biome) in map.iter().enumerate() {
            let Ok(json) = serde_json::from_str::<Value>(
                data.biome_json(horizon_world::worldgen::biome::BiomeId(index as u32)),
            ) else {
                continue;
            };
            let mut list = Vec::new();
            if let Some(steps) = json["features"].as_array() {
                for step in steps {
                    if let Some(features) = step.as_array() {
                        for feature in features {
                            gather(data, feature, true, 1., 0, &mut list);
                        }
                    }
                }
            }
            if !list.is_empty() {
                trees.insert(biome, list);
            }
        }
        Self {
            trees,
            seed: seed as u64,
            registry: data.registry(),
        }
    }
    pub(super) fn apply(&self, column: &mut Column, x: i32, z: i32, width: i32, light: u16) {
        let Some(surface) = column.surface else {
            return;
        };
        if surface.water.is_some_and(|(y, _)| y >= surface.height) {
            return;
        }
        let ground = self
            .registry
            .state(surface.state)
            .map_or("", |s| s.name.as_str());
        if !matches!(
            ground,
            "minecraft:grass_block"
                | "minecraft:dirt"
                | "minecraft:coarse_dirt"
                | "minecraft:podzol"
                | "minecraft:mycelium"
                | "minecraft:rooted_dirt"
                | "minecraft:moss_block"
        ) {
            return;
        }
        let Some(trees) = self.trees.get(&surface.biome) else {
            return;
        };
        let total: f64 = trees.iter().map(|tree| tree.count).sum();
        let choose = |n: u64| {
            let mut value = (n as u32 as f64 / u32::MAX as f64) * total;
            for tree in trees {
                value -= tree.count;
                if value <= 0. {
                    return tree;
                }
            }
            &trees[trees.len() - 1]
        };
        let mut best = None;
        if width >= 16 {
            let n = hash(self.seed, x.div_euclid(4), z.div_euclid(4));
            let tree = choose(n);
            let coverage = 1.
                - (-total * f64::from(tree.radius * tree.radius) * std::f64::consts::PI / 256.)
                    .exp();
            if coverage >= 0.25 {
                let height = tree.height + ((n >> 32) & 1) as i32;
                best = Some((
                    surface.height + height.max(4),
                    surface.height + height.max(4) - tree.radius * 2,
                    *tree,
                    false,
                ));
            }
        } else {
            let radius = trees.iter().map(|t| t.radius).max().unwrap_or(2);
            for cz in (z - radius).div_euclid(4)..=(z + radius).div_euclid(4) {
                for cx in (x - radius).div_euclid(4)..=(x + radius).div_euclid(4) {
                    let n = hash(self.seed, cx, cz);
                    if (n as u32 as f64 / u32::MAX as f64) >= (total / 16.).min(1.) {
                        continue;
                    }
                    let tree = *choose(n.rotate_left(21));
                    let tx = cx * 4 + ((n >> 32) & 3) as i32;
                    let tz = cz * 4 + ((n >> 34) & 3) as i32;
                    let d2 = (x - tx).pow(2) + (z - tz).pow(2);
                    if d2 > tree.radius.pow(2) {
                        continue;
                    }
                    let height =
                        tree.height + ((n >> 40) % (tree.variation.max(0) + 1) as u64) as i32;
                    let cap = ((tree.radius.pow(2) - d2) as f32).sqrt().ceil() as i32;
                    let top = surface.height + height + cap.min(2);
                    let bottom = surface.height + height - tree.radius;
                    if best.as_ref().is_none_or(|&(y, _, _, _)| top > y) {
                        best = Some((top, bottom, tree, x == tx && z == tz));
                    }
                }
            }
        }
        if let Some((top, bottom, tree, trunk)) = best {
            let bottom = bottom.max(surface.height + 1);
            if trunk {
                column.push(Span {
                    min: surface.height as i16,
                    max: bottom as i16,
                    state: tree.log,
                    biome: surface.biome,
                    light,
                });
            }
            column.push(Span {
                min: bottom as i16,
                max: top.max(bottom + 1) as i16,
                state: tree.leaves,
                biome: surface.biome,
                light,
            });
            column.surface = Some(Surface {
                height: top.max(bottom + 1),
                state: tree.leaves,
                ..surface
            });
        }
    }
}
