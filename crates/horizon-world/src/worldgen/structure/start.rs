//! Structures, structure sets and their placements, structure starts,
//! references, terrain adaptation and per-chunk piece placement.
use super::{
    BoundingBox, StructureResources,
    jigsaw::{JigsawConfig, JigsawContext, PlaceFeature, PoolPiece, Pools, Projection, assemble},
    legacy::{LegacyPiece, PieceContext},
};
use crate::{
    heightmap::HeightmapType,
    worldgen::{
        DecodeError,
        anchor::WorldGenerationContext,
        biome::BiomeId,
        block_feature::FeatureBlockBindings,
        feature::FeatureWorld,
        height_provider::HeightProvider,
        placement::{PlacementContext, PlacementQueryError},
        weighted::WeightedList,
    },
};
use horizon_core::{
    density::{Beardifier, JigsawJunction, RigidPiece, StructureBounds, TerrainAdjustment},
    position::{BlockPos, ChunkPos},
    random::{LegacyRandom, PositionalRandom, RandomSource, WorldgenRandom},
};
use serde_json::{Value, value::RawValue};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const STEPS: [&str; 11] = [
    "raw_generation",
    "lakes",
    "local_modifications",
    "underground_structures",
    "surface_structures",
    "strongholds",
    "underground_ores",
    "underground_decoration",
    "fluid_springs",
    "vegetal_decoration",
    "top_layer_modification",
];

#[derive(Debug)]
pub struct JigsawDef {
    start_pool: String,
    start_jigsaw: Option<String>,
    max_depth: i32,
    start_height: HeightProvider,
    use_expansion_hack: bool,
    project_to: Option<HeightmapType>,
    max_distance: [i32; 2],
    aliases: Vec<Value>,
    padding: [i32; 2],
    waterlogging: bool,
    /// A carried mod's start rules (`"horizon:start_checks"`).
    checks: super::start_checks::StartChecks,
}
#[derive(Debug)]
pub enum StructureKind {
    Jigsaw(JigsawDef),
    BuriedTreasure,
    Igloo,
    SwampHut,
    DesertPyramid,
    JungleTemple,
    RuinedPortal(Vec<super::ruined_portal::Setup>),
    Mansion,
    /// The biomes allowed around a monument (#required_ocean_monument_surrounding).
    Monument(BTreeSet<BiomeId>),
    Shipwreck {
        beached: bool,
    },
    OceanRuin {
        warm: bool,
        large: f32,
        cluster: f32,
    },
    Mineshaft {
        mesa: bool,
        /// #minecraft:mineshaft_blocking
        blocking: Arc<BTreeSet<BiomeId>>,
    },
    Stronghold,
    Fortress,
    NetherFossil(HeightProvider),
    EndCity,
    /// A structure type whose generator is not implemented yet.
    Unsupported(String),
}
#[derive(Debug)]
pub struct StructureDef {
    pub name: String,
    pub kind: StructureKind,
    pub biomes: BTreeSet<BiomeId>,
    pub step: usize,
    pub terrain: TerrainAdjustment,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reduction {
    Default,
    Legacy1,
    Legacy2,
    Legacy3,
}
#[derive(Debug)]
enum Placement {
    RandomSpread {
        spacing: i32,
        separation: i32,
        triangular: bool,
    },
    ConcentricRings {
        distance: i32,
        spread: i32,
        count: i32,
        preferred: BTreeSet<BiomeId>,
    },
    Unsupported,
}
/// Concentric ring chunk positions by structure set name.
pub type Rings = BTreeMap<String, Vec<ChunkPos>>;
#[derive(Debug)]
pub struct StructureSet {
    pub name: String,
    pub structures: Vec<(usize, i32)>,
    placement: Placement,
    salt: i32,
    frequency: f32,
    reduction: Reduction,
    exclusion: Option<(String, i32)>,
    /// A carried mod's placement rules: no start nearer the world origin
    /// than this many blocks (`"horizon:min_distance_from_world_origin"`),
    /// and none within `chunk_count` chunks of any of several sets
    /// (`"horizon:exclusion_zone"`: `other_sets`, `chunk_count`).
    min_distance: Option<i64>,
    avoid: Option<(Vec<String>, i32)>,
}

fn heightmap(name: &str) -> Option<HeightmapType> {
    HeightmapType::ALL.into_iter().find(|k| k.name() == name)
}
fn terrain(name: Option<&str>) -> TerrainAdjustment {
    match name {
        Some("bury") => TerrainAdjustment::Bury,
        Some("beard_thin") => TerrainAdjustment::BeardThin,
        Some("beard_box") => TerrainAdjustment::BeardBox,
        Some("encapsulate") => TerrainAdjustment::Encapsulate,
        _ => TerrainAdjustment::None,
    }
}
fn pair(value: Option<&Value>, a: &str, b: &str, default: i32) -> [i32; 2] {
    match value {
        Some(Value::Number(n)) => [n.as_i64().unwrap_or(0) as i32; 2],
        Some(Value::Object(o)) => {
            let get = |k: &str| o.get(k).and_then(Value::as_i64).unwrap_or(0) as i32;
            let first = get(a);
            [
                first,
                o.get(b).and_then(Value::as_i64).map_or(first, |v| v as i32),
            ]
        }
        _ => [default; 2],
    }
}

/// All structures and structure sets of a resource bundle, in registry
/// (identifier) order.
#[derive(Debug)]
pub struct StructureRegistry {
    pub structures: Vec<StructureDef>,
    pub sets: Vec<StructureSet>,
    pub pools: Pools,
}
impl StructureRegistry {
    pub fn new(
        structures: &BTreeMap<String, String>,
        sets: &BTreeMap<String, String>,
        pools: BTreeMap<String, String>,
        biome_set: &dyn Fn(&Value) -> Result<BTreeSet<BiomeId>, DecodeError>,
    ) -> Result<Self, DecodeError> {
        let mut defs = Vec::new();
        for (name, json) in structures {
            let v: Value = serde_json::from_str(json)?;
            let kind = match v["type"].as_str().unwrap_or_default() {
                "minecraft:jigsaw" => StructureKind::Jigsaw(JigsawDef {
                    start_pool: v["start_pool"].as_str().unwrap_or_default().to_owned(),
                    start_jigsaw: v["start_jigsaw_name"].as_str().map(str::to_owned),
                    max_depth: v["size"].as_i64().unwrap_or(0) as i32,
                    start_height: HeightProvider::from_json(&v["start_height"].to_string())?,
                    use_expansion_hack: v["use_expansion_hack"].as_bool().unwrap_or(false),
                    project_to: v["project_start_to_heightmap"].as_str().and_then(heightmap),
                    max_distance: pair(
                        v.get("max_distance_from_center"),
                        "horizontal",
                        "vertical",
                        80,
                    ),
                    aliases: v["pool_aliases"].as_array().cloned().unwrap_or_default(),
                    padding: pair(v.get("dimension_padding"), "bottom", "top", 0),
                    waterlogging: v["liquid_settings"].as_str() != Some("ignore_waterlogging"),
                    checks: super::start_checks::StartChecks::decode(v.get("horizon:start_checks")),
                }),
                "minecraft:buried_treasure" => StructureKind::BuriedTreasure,
                "minecraft:igloo" => StructureKind::Igloo,
                "minecraft:swamp_hut" => StructureKind::SwampHut,
                "minecraft:desert_pyramid" => StructureKind::DesertPyramid,
                "minecraft:jungle_temple" => StructureKind::JungleTemple,
                "minecraft:woodland_mansion" => StructureKind::Mansion,
                "minecraft:ocean_monument" => StructureKind::Monument(biome_set(
                    &serde_json::json!("#minecraft:required_ocean_monument_surrounding"),
                )?),
                "minecraft:ruined_portal" => StructureKind::RuinedPortal(
                    v["setups"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(super::ruined_portal::Setup::decode)
                        .collect(),
                ),
                "minecraft:shipwreck" => StructureKind::Shipwreck {
                    beached: v["is_beached"].as_bool().unwrap_or(false),
                },
                "minecraft:ocean_ruin" => StructureKind::OceanRuin {
                    warm: v["biome_temp"].as_str() == Some("warm"),
                    large: v["large_probability"].as_f64().unwrap_or(0.) as f32,
                    cluster: v["cluster_probability"].as_f64().unwrap_or(0.) as f32,
                },
                "minecraft:mineshaft" => StructureKind::Mineshaft {
                    mesa: v["mineshaft_type"].as_str() == Some("mesa"),
                    blocking: Arc::new(biome_set(&Value::String(
                        "#minecraft:mineshaft_blocking".into(),
                    ))?),
                },
                "minecraft:stronghold" => StructureKind::Stronghold,
                "minecraft:fortress" => StructureKind::Fortress,
                "minecraft:end_city" => StructureKind::EndCity,
                "minecraft:nether_fossil" => StructureKind::NetherFossil(
                    HeightProvider::from_json(&v["height"].to_string())?,
                ),
                other => StructureKind::Unsupported(other.to_owned()),
            };
            defs.push(StructureDef {
                name: name.clone(),
                kind,
                biomes: biome_set(&v["biomes"])?,
                step: STEPS
                    .iter()
                    .position(|s| Some(*s) == v["step"].as_str())
                    .unwrap_or(4),
                terrain: terrain(v["terrain_adaptation"].as_str()),
            });
        }
        let index = |name: &str| defs.iter().position(|d| d.name == name);
        let mut out = Vec::new();
        for (name, json) in sets {
            let v: Value = serde_json::from_str(json)?;
            let p = &v["placement"];
            let placement = match p["type"].as_str().unwrap_or_default() {
                "minecraft:random_spread" => Placement::RandomSpread {
                    spacing: p["spacing"].as_i64().unwrap_or(1) as i32,
                    separation: p["separation"].as_i64().unwrap_or(0) as i32,
                    triangular: p["spread_type"].as_str() == Some("triangular"),
                },
                "minecraft:concentric_rings" => Placement::ConcentricRings {
                    distance: p["distance"].as_i64().unwrap_or(0) as i32,
                    spread: p["spread"].as_i64().unwrap_or(0) as i32,
                    count: p["count"].as_i64().unwrap_or(0) as i32,
                    preferred: biome_set(&p["preferred_biomes"])?,
                },
                _ => Placement::Unsupported,
            };
            out.push(StructureSet {
                name: name.clone(),
                structures: v["structures"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| {
                        Some((
                            index(e["structure"].as_str()?)?,
                            e["weight"].as_i64().unwrap_or(1) as i32,
                        ))
                    })
                    .collect(),
                placement,
                salt: p["salt"].as_i64().unwrap_or(0) as i32,
                frequency: p["frequency"].as_f64().unwrap_or(1.) as f32,
                reduction: match p["frequency_reduction_method"].as_str() {
                    Some("legacy_type_1") => Reduction::Legacy1,
                    Some("legacy_type_2") => Reduction::Legacy2,
                    Some("legacy_type_3") => Reduction::Legacy3,
                    _ => Reduction::Default,
                },
                exclusion: p.get("exclusion_zone").map(|z| {
                    (
                        z["other_set"].as_str().unwrap_or_default().to_owned(),
                        z["chunk_count"].as_i64().unwrap_or(0) as i32,
                    )
                }),
                min_distance: p
                    .get("horizon:min_distance_from_world_origin")
                    .and_then(Value::as_i64),
                avoid: p.get("horizon:exclusion_zone").map(|z| {
                    (
                        z["other_sets"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|s| Some(s.as_str()?.to_owned()))
                            .collect(),
                        z["chunk_count"].as_i64().unwrap_or(0) as i32,
                    )
                }),
            });
        }
        Ok(Self {
            structures: defs,
            sets: out,
            pools: Pools::new(pools),
        })
    }
    /// ChunkGeneratorStructureState.generatePositions: the ring positions of
    /// each concentric-ring set among `possible_sets` before their biome
    /// searches (set index, searches in order).
    pub fn ring_searches(
        &self,
        possible_sets: &[usize],
        seed: i64,
    ) -> Vec<(usize, Vec<super::stronghold::RingSearch>)> {
        possible_sets
            .iter()
            .filter_map(|&i| match &self.sets[i].placement {
                Placement::ConcentricRings {
                    distance,
                    spread,
                    count,
                    ..
                } => Some((
                    i,
                    super::stronghold::ring_searches(seed, *distance, *spread, *count),
                )),
                _ => None,
            })
            .collect()
    }
    /// The preferred biomes of a concentric-ring set (empty for others).
    pub fn ring_biomes(&self, set: usize) -> &BTreeSet<BiomeId> {
        static NONE: BTreeSet<BiomeId> = BTreeSet::new();
        match &self.sets[set].placement {
            Placement::ConcentricRings { preferred, .. } => preferred,
            _ => &NONE,
        }
    }
    /// RandomSpreadStructurePlacement.getPotentialStructureChunk et al.
    /// RandomSpreadStructurePlacement.getPotentialStructureChunk for the
    /// sector holding chunk (`x`, `z`); `None` for other placements.
    pub fn potential_chunk(set: &StructureSet, seed: i64, x: i32, z: i32) -> Option<ChunkPos> {
        let Placement::RandomSpread {
            spacing,
            separation,
            triangular,
        } = set.placement
        else {
            return None;
        };
        let (gx, gz) = (x.div_euclid(spacing), z.div_euclid(spacing));
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        random.set_large_feature_with_salt(seed, gx, gz, set.salt);
        let limit = spacing - separation;
        let spread = |r: &mut WorldgenRandom<LegacyRandom>| {
            if triangular {
                (r.next_int_bound(limit).unwrap_or(0) + r.next_int_bound(limit).unwrap_or(0)) / 2
            } else {
                r.next_int_bound(limit).unwrap_or(0)
            }
        };
        let sx = spread(&mut random);
        let sz = spread(&mut random);
        Some(ChunkPos::new(gx * spacing + sx, gz * spacing + sz))
    }
    pub fn is_structure_chunk(
        &self,
        set: &StructureSet,
        seed: i64,
        x: i32,
        z: i32,
        rings: &Rings,
    ) -> bool {
        let placed = match set.placement {
            Placement::RandomSpread {
                spacing,
                separation,
                triangular,
            } => {
                let (gx, gz) = (x.div_euclid(spacing), z.div_euclid(spacing));
                let mut random = WorldgenRandom::new(LegacyRandom::new(0));
                random.set_large_feature_with_salt(seed, gx, gz, set.salt);
                let limit = spacing - separation;
                let spread = |r: &mut WorldgenRandom<LegacyRandom>| {
                    if triangular {
                        (r.next_int_bound(limit).unwrap_or(0)
                            + r.next_int_bound(limit).unwrap_or(0))
                            / 2
                    } else {
                        r.next_int_bound(limit).unwrap_or(0)
                    }
                };
                let sx = spread(&mut random);
                let sz = spread(&mut random);
                gx * spacing + sx == x && gz * spacing + sz == z
            }
            Placement::ConcentricRings { .. } => rings
                .get(&set.name)
                .is_some_and(|r| r.contains(&ChunkPos::new(x, z))),
            Placement::Unsupported => false,
        };
        if !placed {
            return false;
        }
        // AdvancedRandomSpread.isPlacementChunk, in blocks from the chunk's corner.
        if let Some(d) = set.min_distance {
            let (bx, bz) = (i64::from(x) * 16, i64::from(z) * 16);
            if bx * bx + bz * bz < d * d {
                return false;
            }
        }
        if let Some((others, range)) = &set.avoid {
            if others
                .iter()
                .any(|other| self.has_start_in_range(other, seed, x, z, *range, rings))
            {
                return false;
            }
        }
        if set.frequency < 1. && !reduce(set.reduction, seed, set.salt, x, z, set.frequency) {
            return false;
        }
        match &set.exclusion {
            Some((other, range)) => !self.has_start_in_range(other, seed, x, z, *range, rings),
            None => true,
        }
    }
    /// ChunkGeneratorStructureState.hasStructureChunkInRange: whether set
    /// `other` places a structure within `range` chunks of (`x`, `z`).
    fn has_start_in_range(
        &self,
        other: &str,
        seed: i64,
        x: i32,
        z: i32,
        range: i32,
        rings: &Rings,
    ) -> bool {
        let Some(other) = self.sets.iter().find(|s| s.name == other) else {
            return false;
        };
        (x - range..=x + range).any(|tx| {
            (z - range..=z + range).any(|tz| self.is_structure_chunk(other, seed, tx, tz, rings))
        })
    }
}
impl StructureSet {
    /// The random spread's spacing in chunks (`None` for rings).
    pub fn spacing(&self) -> Option<i32> {
        match self.placement {
            Placement::RandomSpread { spacing, .. } => Some(spacing),
            _ => None,
        }
    }
}
fn reduce(method: Reduction, seed: i64, salt: i32, x: i32, z: i32, probability: f32) -> bool {
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    match method {
        Reduction::Default => {
            random.set_large_feature_with_salt(seed, salt, x, z);
            random.next_float() < probability
        }
        Reduction::Legacy1 => {
            let (cx, cz) = (x >> 4, z >> 4);
            random.set_seed(i64::from(cx ^ (cz << 4)) ^ seed);
            random.next_int();
            random
                .next_int_bound((1. / probability) as i32)
                .unwrap_or(1)
                == 0
        }
        Reduction::Legacy2 => {
            random.set_large_feature_with_salt(seed, x, z, 10_387_320);
            random.next_float() < probability
        }
        Reduction::Legacy3 => {
            random.set_large_feature_seed(seed, x, z);
            random.next_double() < f64::from(probability)
        }
    }
}

#[derive(Debug)]
pub enum Piece {
    Pool(PoolPiece),
    Legacy(Box<dyn LegacyPiece>),
}
impl Piece {
    pub fn bounding_box(&self) -> BoundingBox {
        match self {
            Self::Pool(p) => p.bounding_box,
            Self::Legacy(p) => p.bounding_box(),
        }
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Pool(_) => "minecraft:jigsaw",
            Self::Legacy(p) => p.type_name(),
        }
    }
}
fn legacy(pieces: Vec<impl LegacyPiece + 'static>) -> Vec<Piece> {
    pieces
        .into_iter()
        .map(|p| Piece::Legacy(Box::new(p)))
        .collect()
}
/// Structure.onTopOfChunkCenter: the start position, or None when no valid
/// biome can exist in the chunk-center column.
fn top_of_chunk_center(
    context: &mut impl StartContext,
    chunk: ChunkPos,
    kind: HeightmapType,
) -> Option<[i32; 3]> {
    let (x, z) = (chunk.x * 16 + 8, chunk.z * 16 + 8);
    let (min, max) = (context.min_y(), context.max_y());
    if !context.column_has_valid_biome(x, z, min - 1, max) {
        return None;
    }
    Some([x, context.base_height(x, z, kind) - 1, z])
}
fn first_occupied(context: &mut impl StartContext, x: i32, z: i32) -> i32 {
    context.base_height(x, z, HeightmapType::WorldSurfaceWg) - 1
}
fn corner_heights(context: &mut impl StartContext, x: i32, sx: i32, z: i32, sz: i32) -> [i32; 4] {
    [
        first_occupied(context, x, z),
        first_occupied(context, x, z + sz),
        first_occupied(context, x + sx, z),
        first_occupied(context, x + sx, z + sz),
    ]
}
#[derive(Debug)]
pub struct StructureStart {
    pub structure: usize,
    pub name: String,
    pub chunk: ChunkPos,
    pub pieces: Vec<Piece>,
    /// Piece union, inflated by 12 for terrain-adapting structures.
    pub bounding_box: BoundingBox,
}

/// Biome and height queries for one structure generation attempt.
pub trait StartContext: JigsawContext {
    fn seed(&self) -> i64;
    fn sea_level(&self) -> i32;
    /// ChunkGenerator.getBaseColumn as heightmap opacity, indexed from min_y.
    fn base_column(&mut self, x: i32, z: i32, kind: HeightmapType) -> Vec<bool>;
    /// base_column for several heightmap kinds from one noise-column pass.
    fn base_columns(&mut self, x: i32, z: i32, kinds: &[HeightmapType]) -> Vec<Vec<bool>>;
    /// Biome.coldEnoughToSnow at a position.
    fn cold_enough_to_snow(&mut self, biome: BiomeId, pos: [i32; 3]) -> bool;
    fn generation(&self) -> WorldGenerationContext;
    /// Noise biome at a quart position (caching resolver, no zoom).
    fn noise_biome(&mut self, quart: [i32; 3]) -> BiomeId;
    fn set_valid_biomes(&mut self, biomes: &BTreeSet<BiomeId>);
}

/// ChunkGenerator.createStructures for one chunk: the starts it owns.
pub fn create_starts(
    registry: &StructureRegistry,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    context: &mut impl StartContext,
    possible_sets: &[usize],
    rings: &Rings,
    chunk: ChunkPos,
) -> Vec<StructureStart> {
    let seed = context.seed();
    let mut starts = Vec::new();
    for &set_index in possible_sets {
        let set = &registry.sets[set_index];
        if !registry.is_structure_chunk(set, seed, chunk.x, chunk.z, rings) {
            continue;
        }
        if set.structures.len() == 1 {
            if let Some(start) = try_generate(
                registry,
                resources,
                bindings,
                context,
                set.structures[0].0,
                chunk,
            ) {
                starts.push(start);
            }
            continue;
        }
        let mut options = set.structures.clone();
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        random.set_large_feature_seed(seed, chunk.x, chunk.z);
        let mut total: i32 = options.iter().map(|o| o.1).sum();
        while !options.is_empty() {
            let mut choice = random.next_int_bound(total).unwrap_or(0);
            let mut index = 0;
            for option in &options {
                choice -= option.1;
                if choice < 0 {
                    break;
                }
                index += 1;
            }
            let selected = options[index];
            if let Some(start) =
                try_generate(registry, resources, bindings, context, selected.0, chunk)
            {
                starts.push(start);
                break;
            }
            options.remove(index);
            total -= selected.1;
        }
    }
    starts
}
fn try_generate(
    registry: &StructureRegistry,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    context: &mut impl StartContext,
    structure: usize,
    chunk: ChunkPos,
) -> Option<StructureStart> {
    let def = &registry.structures[structure];
    context.set_valid_biomes(&def.biomes);
    let seed = context.seed();
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    random.set_large_feature_seed(seed, chunk.x, chunk.z);
    let (position, pieces) = match &def.kind {
        StructureKind::Jigsaw(jigsaw) => {
            let height = jigsaw
                .start_height
                .sample(&mut random, context.generation())
                .ok()?;
            if !jigsaw
                .checks
                .pass(context, &def.biomes, chunk, height, jigsaw.project_to)
            {
                return None;
            }
            let start = [chunk.x * 16, height, chunk.z * 16];
            let aliases = resolve_aliases(&jigsaw.aliases, start, seed);
            let config = JigsawConfig {
                start_pool: &jigsaw.start_pool,
                start_jigsaw: jigsaw.start_jigsaw.as_deref(),
                max_depth: jigsaw.max_depth,
                use_expansion_hack: jigsaw.use_expansion_hack,
                project_to: jigsaw.project_to,
                max_distance: jigsaw.max_distance,
                aliases: &aliases,
                padding: jigsaw.padding,
                waterlogging: jigsaw.waterlogging,
            };
            let (position, mut pieces) = assemble(
                &config,
                context,
                &registry.pools,
                resources,
                bindings,
                &mut random,
                start,
            )?;
            if jigsaw.max_depth <= 0 {
                pieces.clear();
            }
            (
                position,
                pieces.into_iter().map(Piece::Pool).collect::<Vec<_>>(),
            )
        }
        StructureKind::BuriedTreasure => {
            let stub = top_of_chunk_center(context, chunk, HeightmapType::OceanFloorWg)?;
            let piece = super::simple::BuriedTreasurePiece {
                position: [chunk.x * 16 + 9, 90, chunk.z * 16 + 9],
            };
            (stub, legacy(vec![piece]))
        }
        StructureKind::Igloo => {
            let stub = top_of_chunk_center(context, chunk, HeightmapType::WorldSurfaceWg)?;
            let rotation = super::Rotation::random(&mut random);
            let pieces = super::simple::igloo_pieces(
                resources,
                [chunk.x * 16, 90, chunk.z * 16],
                rotation,
                &mut random,
            )?;
            (stub, legacy(pieces))
        }
        StructureKind::SwampHut => {
            let stub = top_of_chunk_center(context, chunk, HeightmapType::WorldSurfaceWg)?;
            let piece = super::simple::SwampHutPiece::new(&mut random, chunk.x * 16, chunk.z * 16);
            (stub, legacy(vec![piece]))
        }
        StructureKind::DesertPyramid | StructureKind::JungleTemple => {
            let desert = matches!(def.kind, StructureKind::DesertPyramid);
            let (width, depth) = if desert { (21, 21) } else { (12, 15) };
            let (x, z) = (chunk.x * 16, chunk.z * 16);
            if !context.column_has_valid_biome(x + 8, z + 8, context.min_y() - 1, context.max_y()) {
                return None;
            }
            let lowest = corner_heights(context, x, width, z, depth)
                .into_iter()
                .min()
                .unwrap_or(0);
            if lowest < context.sea_level() {
                return None;
            }
            let stub = [x + 8, first_occupied(context, x + 8, z + 8), z + 8];
            let piece: Box<dyn LegacyPiece> = if desert {
                Box::new(super::temple::DesertPyramidPiece::new(&mut random, x, z))
            } else {
                Box::new(super::temple::JungleTemplePiece::new(&mut random, x, z))
            };
            (stub, vec![Piece::Legacy(piece)])
        }
        StructureKind::Monument(surrounding) => {
            let (x, z) = (chunk.x * 16 + 9, chunk.z * 16 + 9);
            let (y, r) = (context.sea_level(), 29);
            for qz in (z - r) >> 2..=(z + r) >> 2 {
                for qx in (x - r) >> 2..=(x + r) >> 2 {
                    for qy in (y - r) >> 2..=(y + r) >> 2 {
                        if !surrounding.contains(&context.noise_biome([qx, qy, qz])) {
                            return None;
                        }
                    }
                }
            }
            let stub = top_of_chunk_center(context, chunk, HeightmapType::OceanFloorWg)?;
            let direction = super::legacy::random_horizontal(&mut random);
            let piece = super::monument::MonumentBuilding::new(
                &mut random,
                chunk.x * 16 - 29,
                chunk.z * 16 - 29,
                direction,
            );
            (stub, legacy(vec![piece]))
        }
        StructureKind::Mansion => {
            let (x, z) = (chunk.x * 16 + 7, chunk.z * 16 + 7);
            if !context.column_has_valid_biome(x, z, context.min_y() - 1, context.max_y()) {
                return None;
            }
            let rotation = super::Rotation::random(&mut random);
            let (sx, sz) = match rotation {
                super::Rotation::None => (5, 5),
                super::Rotation::Clockwise90 => (-5, 5),
                super::Rotation::Clockwise180 => (-5, -5),
                super::Rotation::Counterclockwise90 => (5, -5),
            };
            let y = corner_heights(context, x, sx, z, sz)
                .into_iter()
                .min()
                .unwrap_or(0);
            if y < 60 {
                return None;
            }
            let start = [x, y, z];
            let pieces = super::mansion::pieces(resources, start, rotation, &mut random)?;
            (start, legacy(pieces))
        }
        StructureKind::RuinedPortal(setups) => {
            let (position, piece) = super::ruined_portal::start(
                context,
                resources,
                bindings,
                &mut random,
                chunk,
                setups,
            )?;
            (position, legacy(vec![piece]))
        }
        StructureKind::Shipwreck { beached } => {
            let kind = if *beached {
                HeightmapType::WorldSurfaceWg
            } else {
                HeightmapType::OceanFloorWg
            };
            let stub = top_of_chunk_center(context, chunk, kind)?;
            let rotation = super::Rotation::random(&mut random);
            let piece = super::simple::shipwreck_piece(
                resources,
                [chunk.x * 16, 90, chunk.z * 16],
                rotation,
                &mut random,
                *beached,
            )?;
            if super::simple::shipwreck_too_big(&piece.template) {
                let b = piece.bounding_box;
                let [sx, _, sz] = b.span();
                let heights = corner_heights(context, b.min[0], sx, b.min[2], sz);
                let height = if *beached {
                    let min = heights.into_iter().min().unwrap_or(0);
                    min - piece.template.size[1] / 2 - random.next_int_bound(3).unwrap_or(0)
                } else {
                    heights.iter().sum::<i32>() / 4
                };
                *piece.state.lock().unwrap() = Some(height);
            }
            (stub, legacy(vec![piece]))
        }
        StructureKind::OceanRuin {
            warm,
            large,
            cluster,
        } => {
            let stub = top_of_chunk_center(context, chunk, HeightmapType::OceanFloorWg)?;
            let rotation = super::Rotation::random(&mut random);
            let config = super::ocean_ruin::OceanRuinConfig {
                warm: *warm,
                large_probability: *large,
                cluster_probability: *cluster,
            };
            let pieces = super::ocean_ruin::pieces(
                resources,
                bindings,
                [chunk.x * 16, 90, chunk.z * 16],
                rotation,
                &mut random,
                &config,
            )?;
            (stub, legacy(pieces))
        }
        StructureKind::Mineshaft { mesa, blocking } => {
            let (sea, min) = (context.sea_level(), context.min_y());
            let (position, pieces) = super::mineshaft::generate(
                &mut random,
                [chunk.x, chunk.z],
                *mesa,
                blocking.clone(),
                sea,
                min,
                &mut |x, z| context.base_height(x, z, HeightmapType::WorldSurfaceWg),
            );
            (position, legacy(pieces))
        }
        StructureKind::Stronghold => {
            let (sea, min) = (context.sea_level(), context.min_y());
            let pieces =
                super::stronghold::generate(&mut random, seed, [chunk.x, chunk.z], sea, min);
            ([chunk.x * 16, 0, chunk.z * 16], legacy(pieces))
        }
        StructureKind::EndCity => {
            let (x, z) = (chunk.x * 16 + 7, chunk.z * 16 + 7);
            if !context.column_has_valid_biome(x, z, context.min_y() - 1, context.max_y()) {
                return None;
            }
            let rotation = super::Rotation::random(&mut random);
            let (sx, sz) = match rotation {
                super::Rotation::None => (5, 5),
                super::Rotation::Clockwise90 => (-5, 5),
                super::Rotation::Clockwise180 => (-5, -5),
                super::Rotation::Counterclockwise90 => (5, -5),
            };
            let y = corner_heights(context, x, sx, z, sz)
                .into_iter()
                .min()
                .unwrap_or(0);
            if y < 60 {
                return None;
            }
            let start = [x, y, z];
            if !def
                .biomes
                .contains(&context.noise_biome(start.map(|v| v >> 2)))
            {
                return None;
            }
            let pieces = super::end_city::pieces(resources, start, rotation, &mut random)?;
            (start, legacy(pieces))
        }
        StructureKind::Fortress => {
            let pieces = super::fortress::generate(&mut random, [chunk.x, chunk.z]);
            ([chunk.x * 16, 64, chunk.z * 16], legacy(pieces))
        }
        StructureKind::NetherFossil(height) => {
            let x = chunk.x * 16 + random.next_int_bound(16).unwrap_or(0);
            let z = chunk.z * 16 + random.next_int_bound(16).unwrap_or(0);
            let sea = context.sea_level();
            let y = height.sample(&mut random, context.generation()).ok()?;
            let columns = context.base_columns(
                x,
                z,
                &[HeightmapType::WorldSurfaceWg, HeightmapType::OceanFloorWg],
            );
            let y = super::nether_fossil::find_floor(
                &columns[0],
                &columns[1],
                context.min_y(),
                y,
                sea,
            )?;
            let position = [x, y, z];
            let quart = position.map(|v| v >> 2);
            if !def.biomes.contains(&context.noise_biome(quart)) {
                return None;
            }
            let piece = super::nether_fossil::piece(resources, &mut random, position)?;
            (position, legacy(vec![piece]))
        }
        StructureKind::Unsupported(_) => return None,
    };
    let quart = position.map(|v| v >> 2);
    if !def.biomes.contains(&context.noise_biome(quart)) || pieces.is_empty() {
        return None;
    }
    let mut bounding_box = pieces[0].bounding_box();
    for piece in &pieces[1..] {
        bounding_box.encapsulate(&piece.bounding_box());
    }
    if def.terrain != TerrainAdjustment::None {
        bounding_box = bounding_box.inflated(12, 12);
    }
    Some(StructureStart {
        structure,
        name: def.name.clone(),
        chunk,
        pieces,
        bounding_box,
    })
}
/// PoolAliasLookup.create.
fn resolve_aliases(bindings: &[Value], position: [i32; 3], seed: i64) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if bindings.is_empty() {
        return map;
    }
    let mut random = LegacyRandom::new(seed).fork_positional().at(BlockPos::new(
        position[0],
        position[1],
        position[2],
    ));
    fn visit(binding: &Value, random: &mut LegacyRandom, map: &mut BTreeMap<String, String>) {
        let weighted = |key: &str| -> Option<WeightedList<Value>> {
            let raw: Box<RawValue> = serde_json::from_str(&binding[key].to_string()).ok()?;
            WeightedList::decode(&raw, |v| Ok(serde_json::from_str::<Value>(v.get())?)).ok()
        };
        match binding["type"].as_str().unwrap_or_default() {
            "minecraft:direct" => {
                map.insert(
                    binding["alias"].as_str().unwrap_or_default().to_owned(),
                    binding["target"].as_str().unwrap_or_default().to_owned(),
                );
            }
            "minecraft:random" => {
                if let Some(target) =
                    weighted("targets").and_then(|w| w.sample(random).ok().cloned())
                {
                    map.insert(
                        binding["alias"].as_str().unwrap_or_default().to_owned(),
                        target.as_str().unwrap_or_default().to_owned(),
                    );
                }
            }
            "minecraft:random_group" => {
                if let Some(group) = weighted("groups").and_then(|w| w.sample(random).ok().cloned())
                {
                    for b in group.as_array().into_iter().flatten() {
                        visit(b, random, map);
                    }
                }
            }
            _ => {}
        }
    }
    for binding in bindings {
        visit(binding, &mut random, &mut map);
    }
    map
}

/// Beardifier.forStructuresInChunk over the starts referencing a chunk.
pub fn beardifier(
    registry: &StructureRegistry,
    starts: &[Arc<StructureStart>],
    chunk: ChunkPos,
) -> Option<Beardifier> {
    let (bx, bz) = (chunk.x * 16, chunk.z * 16);
    let mut rigids = Vec::new();
    let mut junctions = Vec::new();
    let mut any: Option<BoundingBox> = None;
    let include = |b: BoundingBox, any: &mut Option<BoundingBox>| match any {
        Some(a) => a.encapsulate(&b),
        None => *any = Some(b),
    };
    for start in starts {
        let adjustment = registry.structures[start.structure].terrain;
        if adjustment == TerrainAdjustment::None {
            continue;
        }
        for piece in &start.pieces {
            let b = piece.bounding_box();
            let near = b.max[0] >= bx - 12
                && b.min[0] <= bx + 15 + 12
                && b.max[2] >= bz - 12
                && b.min[2] <= bz + 15 + 12;
            if !near {
                continue;
            }
            let Piece::Pool(pool) = piece else {
                rigids.push(RigidPiece {
                    bounds: StructureBounds::new(b.min, b.max)?,
                    adjustment,
                    ground_level_delta: 0,
                });
                include(b, &mut any);
                continue;
            };
            if pool.element.projection() == Projection::Rigid {
                rigids.push(RigidPiece {
                    bounds: StructureBounds::new(b.min, b.max)?,
                    adjustment,
                    ground_level_delta: pool.ground_level_delta,
                });
                include(b, &mut any);
            }
            for j in &pool.junctions {
                if j.source_x > bx - 12
                    && j.source_z > bz - 12
                    && j.source_x < bx + 15 + 12
                    && j.source_z < bz + 15 + 12
                {
                    junctions.push(JigsawJunction {
                        source_x: j.source_x,
                        source_ground_y: j.source_ground_y,
                        source_z: j.source_z,
                    });
                    let p = [j.source_x, j.source_ground_y, j.source_z];
                    include(BoundingBox::new(p, p), &mut any);
                }
            }
        }
    }
    let any = any?;
    let affected = any.inflated(24, 24);
    Some(Beardifier::new(
        rigids,
        junctions,
        StructureBounds::new(affected.min, affected.max),
    ))
}

/// StructureStart.placeInChunk for pieces intersecting the chunk's writable box.
#[allow(clippy::too_many_arguments)]
pub fn place_in_chunk<W: FeatureWorld + PlacementContext, R: RandomSource + 'static>(
    start: &StructureStart,
    world: &mut W,
    resources: &StructureResources,
    random: &mut R,
    chunk_box: BoundingBox,
    place_feature: &mut PlaceFeature<'_, W, R>,
) -> Result<(), PlacementQueryError> {
    let Some(first) = start.pieces.first() else {
        return Ok(());
    };
    let center_box = first.bounding_box();
    let center = center_box.center();
    let reference = [center[0], center_box.min[1], center[2]];
    for piece in &start.pieces {
        if piece.bounding_box().intersects(&chunk_box) {
            match piece {
                Piece::Pool(p) => {
                    p.place(
                        world,
                        resources,
                        random,
                        chunk_box,
                        reference,
                        place_feature,
                    )?;
                }
                Piece::Legacy(p) => {
                    let mut context = PieceContext {
                        world,
                        resources,
                        random,
                        chunk_box,
                        chunk: ChunkPos::new(chunk_box.min[0] >> 4, chunk_box.min[2] >> 4),
                        reference,
                    };
                    p.post_process(&mut context)?;
                }
            }
        }
    }
    if let Piece::Legacy(p) = first {
        let boxes: Vec<BoundingBox> = start.pieces.iter().map(Piece::bounding_box).collect();
        let mut context = PieceContext {
            world,
            resources,
            random,
            chunk_box,
            chunk: ChunkPos::new(chunk_box.min[0] >> 4, chunk_box.min[2] >> 4),
            reference,
        };
        p.after_place(&mut context, &boxes)?;
    }
    Ok(())
}
