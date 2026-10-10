//! `NaturalSpawner.getRandomSpawnMobAt` and `mobsAt`: which spawn list a
//! position uses (the nether fortress bounds, then structure spawn overrides
//! by piece or bounding box, then the biome's natural mob spawns), the
//! reduced water ambient odds, the weighted pick and the group size sample.
use crate::biome::SpawnEntry;
use horizon_core::random::RandomSource;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

/// A `natural_mob_spawns` / `spawn_overrides` entry list.
pub fn parse_entries(list: &Value) -> Vec<SpawnEntry> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let count = &e["count"];
            let (min, max, constant) = match count {
                Value::Number(n) => {
                    let n = n.as_u64().unwrap_or(1) as u32;
                    (n, n, true)
                }
                c if c["type"].as_str() == Some("minecraft:constant") => {
                    let n = c["value"].as_u64().unwrap_or(1) as u32;
                    (n, n, true)
                }
                c => (
                    c["min_inclusive"].as_u64().unwrap_or(1) as u32,
                    c["max_inclusive"].as_u64().unwrap_or(1) as u32,
                    false,
                ),
            };
            Some(SpawnEntry {
                entity: e["type"].as_str()?.to_owned(),
                weight: e["weight"].as_u64().unwrap_or(1) as u32,
                min,
                max,
                constant,
            })
        })
        .collect()
}

/// A structure's spawn override for one category.
#[derive(Clone, Debug)]
pub struct Override {
    /// `bounding_box: piece` (else the whole structure's box).
    pub piece: bool,
    pub spawns: Vec<SpawnEntry>,
}
/// Every structure's `spawn_overrides`, by structure id then category.
#[derive(Clone, Debug, Default)]
pub struct StructureSpawns {
    pub overrides: HashMap<String, HashMap<String, Override>>,
}
impl StructureSpawns {
    /// The overrides of the resource root first asked for, loaded once.
    pub fn shared(root: &Path) -> &'static Self {
        static SHARED: std::sync::OnceLock<StructureSpawns> = std::sync::OnceLock::new();
        SHARED.get_or_init(|| Self::load(root))
    }
    pub fn load(root: &Path) -> Self {
        let mut overrides = HashMap::new();
        let dir = root.join("data/minecraft/worldgen/structure");
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(v) = fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            else {
                continue;
            };
            let map: HashMap<String, Override> = v["spawn_overrides"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(category, o)| {
                    (
                        category.clone(),
                        Override {
                            piece: o["bounding_box"].as_str() == Some("piece"),
                            spawns: parse_entries(&o["spawns"]),
                        },
                    )
                })
                .collect();
            if !map.is_empty() {
                overrides.insert(format!("minecraft:{stem}"), map);
            }
        }
        Self { overrides }
    }
    pub fn get(&self, structure: &str, category: &str) -> Option<&Override> {
        self.overrides.get(structure)?.get(category)
    }
}

/// A structure whose references reach a position.
#[derive(Clone, Debug)]
pub struct StructureAt {
    pub id: String,
    /// A piece of one of its starts holds the position.
    pub in_piece: bool,
    /// One of its starts' bounding boxes holds the position.
    pub in_bounds: bool,
}

/// `NetherFortressStructure.FORTRESS_ENEMIES`.
pub fn fortress_enemies() -> Vec<SpawnEntry> {
    let e = |entity: &str, weight, min, max, constant| SpawnEntry {
        entity: entity.to_owned(),
        weight,
        min,
        max,
        constant,
    };
    vec![
        e("minecraft:blaze", 10, 2, 3, false),
        e("minecraft:zombified_piglin", 5, 4, 4, true),
        e("minecraft:wither_skeleton", 8, 5, 5, true),
        e("minecraft:skeleton", 2, 5, 5, true),
        e("minecraft:magma_cube", 3, 4, 4, true),
    ]
}

/// `NaturalSpawner.mobsAt`.
pub fn mobs_at(
    spawns: &StructureSpawns,
    biome: &HashMap<String, Vec<SpawnEntry>>,
    category: &str,
    structures: &[StructureAt],
    below_nether_bricks: bool,
) -> Vec<SpawnEntry> {
    // isInNetherFortressBounds.
    if category == "monster"
        && below_nether_bricks
        && structures
            .iter()
            .any(|s| s.id == "minecraft:fortress" && s.in_bounds)
    {
        return fortress_enemies();
    }
    // ChunkGenerator.getMobsAt: the first structure whose override holds.
    for s in structures {
        if let Some(o) = spawns.get(&s.id, category)
            && (if o.piece { s.in_piece } else { s.in_bounds })
        {
            return o.spawns.clone();
        }
    }
    biome.get(category).cloned().unwrap_or_default()
}

/// `WeightedList.getRandom`.
pub fn weighted_pick(list: &[SpawnEntry], random: &mut impl RandomSource) -> Option<SpawnEntry> {
    let total: i64 = list.iter().map(|e| i64::from(e.weight)).sum();
    if total <= 0 {
        return None;
    }
    let mut selection = i64::from(random.next_int_bound(total as i32).ok()?);
    for e in list {
        selection -= i64::from(e.weight);
        if selection < 0 {
            return Some(e.clone());
        }
    }
    None
}

/// `NaturalSpawner.getRandomSpawnMobAt`.
pub fn random_spawn_mob_at(
    list: &[SpawnEntry],
    category: &str,
    reduced_water_ambient: bool,
    random: &mut impl RandomSource,
) -> Option<SpawnEntry> {
    if category == "water_ambient" && reduced_water_ambient && random.next_float() < 0.98 {
        return None;
    }
    weighted_pick(list, random)
}

/// `SpawnerData.count().sample(random)`: a constant draws nothing.
pub fn sample_count(entry: &SpawnEntry, random: &mut impl RandomSource) -> u32 {
    if entry.constant {
        entry.min
    } else {
        let span = entry.max.saturating_sub(entry.min) as i32 + 1;
        entry.min + random.next_int_bound(span).unwrap_or(0) as u32
    }
}
