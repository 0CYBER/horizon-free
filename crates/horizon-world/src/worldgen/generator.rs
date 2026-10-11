//! Production noise-chunk generation for one dimension of a world preset.
//!
//! `GeneratorData` holds seed-independent resources parsed once from a bundle
//! directory (see `GeneratorData::load`). `OverworldGenerator` binds a world seed
//! and is shared across threads; each thread owns a `GeneratorWorker`.
//! `generate_proto` runs vanilla's BIOMES and TERRAIN statuses (noise fill with
//! aquifers, material surface, carvers) and primes the post-terrain heightmaps.
//! `decorate` runs the FEATURES status for the center of a 3×3 neighborhood with
//! whatever feature algorithms are implemented; structures are not generated.
use super::{
    DecodeError,
    anchor::WorldGenerationContext,
    aquifer::{
        AquiferConfig, AquiferField, AquiferSamplers, DisabledAquifer, GlobalFluidPicker,
        NoiseBasedAquifer, RegistryAquiferBlockStates,
    },
    biome::BiomeId,
    biome_chunk::{BiomeChunk, BiomeRegion},
    biome_source::{BiomeSourceCatalog, ResolvedBiomeSource},
    biome_temperature::{BiomeTemperature, BiomeTemperatureSettings},
    carver::{CarverCatalog, WorldCarver, generate_carving_mask},
    carving::{CarvingBlocks, apply_carving_mask},
    climate::{ClimateSampler, quantize},
    decode_noise,
    density::DensityRegistry,
    material_condition::{
        MaterialConditionRegistry, MaterialRule, MaterialRuleError, MaterialRuleLibrary,
        MaterialSurfaceQueries,
    },
    material_extensions::{ExtensionBlocks, ExtensionNoises, MaterialExtensions},
    material_stage::{SurfaceInputs, TopMaterialInputs, build_surface},
    material_surface::{ClayBandBlocks, MaterialSurface},
    router::{NoiseRouter, RouterField, RouterSamplers},
    tags::TagDefinitions,
    terrain::{PreLightChunkStage, TerrainAquifer, TerrainBlockStates, TerrainChunk, fill_noise},
};
use crate::{
    heightmap::{HeightmapState, HeightmapType},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{
    biome::{BiomeZoom, obfuscate_seed},
    density::{DensitySampler, SamplerContext},
    noise::{DensityVolume, Noise},
    position::{BlockPos, ChunkPos},
    random::{LegacyRandom, PositionalRandom, RandomSource, XoroshiroRandom},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs, io,
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Debug)]
pub enum GeneratorError {
    Io(io::Error),
    Decode(DecodeError),
    /// A resource is missing or inconsistent with the block registry.
    Resource(String),
    /// A generation stage failed; the chunk is incomplete.
    Stage(&'static str, String),
}
impl std::fmt::Display for GeneratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Decode(e) => e.fmt(f),
            Self::Resource(e) => write!(f, "invalid generator resources: {e}"),
            Self::Stage(stage, e) => write!(f, "{stage} failed: {e}"),
        }
    }
}
impl std::error::Error for GeneratorError {}
impl From<io::Error> for GeneratorError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<DecodeError> for GeneratorError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}
impl From<serde_json::Error> for GeneratorError {
    fn from(e: serde_json::Error) -> Self {
        Self::Decode(e.into())
    }
}
fn stage<E: std::fmt::Display>(name: &'static str) -> impl FnOnce(E) -> GeneratorError {
    move |e| GeneratorError::Stage(name, e.to_string())
}
fn resource(message: impl Into<String>) -> GeneratorError {
    GeneratorError::Resource(message.into())
}

/// Heightmap-relevant block behavior, one entry per registry state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GenBlockState {
    pub air: bool,
    pub air_block: bool,
    pub fluid: bool,
    pub motion: bool,
    pub no_leaves: bool,
}
impl HeightmapState for GenBlockState {
    fn is_air(self) -> bool {
        self.air
    }
    fn is_air_block(self) -> bool {
        self.air_block
    }
    fn has_fluid(self) -> bool {
        self.fluid
    }
    fn blocks_motion_in_heightmap(self) -> bool {
        self.motion
    }
    fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
        self.no_leaves
    }
}
/// Shared, cheaply cloned state table bound into every generated chunk.
#[derive(Clone, Debug)]
pub struct GenBlockStates(Arc<[GenBlockState]>);
impl GenBlockStates {
    pub fn get(&self, id: BlockStateId) -> GenBlockState {
        self.0.get(id.0 as usize).copied().unwrap_or_default()
    }
}
impl TerrainBlockStates for GenBlockStates {
    type State = GenBlockState;
    fn heightmap_state(&self, id: BlockStateId) -> GenBlockState {
        self.get(id)
    }
}

/// Feature resources for the FEATURES status, keyed by resource identifier.
pub(super) struct FeatureResources {
    pub(super) features: BTreeMap<String, String>,
    pub(super) placed: BTreeMap<String, String>,
    pub(super) feature_tags: BTreeMap<String, String>,
    pub(super) fluid_tags: BTreeMap<String, Vec<String>>,
    pub(super) state_providers: BTreeMap<String, String>,
    /// Block properties, support, collision and survival for placement checks.
    pub(super) blocks: Option<super::generator_blocks::BlockBehavior>,
}

/// Raw structure, structure-set and template-pool resources.
pub(super) struct StructureBundles {
    pub(super) structures: BTreeMap<String, String>,
    pub(super) sets: BTreeMap<String, String>,
    pub(super) pools: BTreeMap<String, String>,
}

/// Seed-independent generation resources, loaded once and shared by every
/// generator. Biome identities are indices into the sorted biome registry.
pub struct GeneratorData {
    registry: &'static BlockRegistry,
    states: GenBlockStates,
    integers: BTreeSet<(String, String)>,
    noises: BTreeMap<String, String>,
    densities: BTreeMap<String, String>,
    rules: BTreeMap<String, String>,
    settings: BTreeMap<String, String>,
    presets: BTreeMap<String, String>,
    world_presets: BTreeMap<String, String>,
    conditions: MaterialConditionRegistry,
    source_catalog: BiomeSourceCatalog,
    biome_names: Vec<Arc<str>>,
    biome_ids: BTreeMap<String, BiomeId>,
    biome_json: Vec<String>,
    temperatures: Vec<BiomeTemperatureSettings>,
    biome_carvers: Vec<Vec<Arc<WorldCarver>>>,
    carving_blocks: CarvingBlocks,
    pub(super) block_tags: BTreeMap<String, Vec<String>>,
    pub(super) biome_tags: BTreeMap<String, Vec<String>>,
    pub(super) feature_resources: Option<FeatureResources>,
    /// Templates, processor lists and per-state template transforms.
    pub(super) structures: Option<Arc<super::structure::StructureResources>>,
    pub(super) structure_resources: Option<StructureBundles>,
    /// Block shape-update behaviour (`block-behaviours.json` next to the
    /// worldgen directory) for template placement's neighbor updates.
    pub(super) behaviours: Option<Arc<crate::level::Behaviours>>,
}
type Bundle = BTreeMap<String, String>;
fn read_bundle(dir: &Path, name: &str) -> Result<Bundle, GeneratorError> {
    let path = dir.join(format!("{name}.json"));
    let text =
        fs::read_to_string(&path).map_err(|e| resource(format!("{}: {e}", path.display())))?;
    Ok(serde_json::from_str(&text)?)
}
fn read_optional_bundle(dir: &Path, name: &str) -> Result<Option<Bundle>, GeneratorError> {
    if dir.join(format!("{name}.json")).exists() {
        read_bundle(dir, name).map(Some)
    } else {
        Ok(None)
    }
}
fn build_tags(
    raw: &Bundle,
    known: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, Vec<String>>, GeneratorError> {
    let mut definitions = TagDefinitions::default();
    for (name, json) in raw {
        definitions.apply_json(name, "vanilla", json)?;
    }
    let built = definitions
        .build(|id, _| known(id).then(|| id.to_owned()))
        .map_err(|e| resource(format!("tags: {e:?}")))?;
    if !built.missing.is_empty() {
        return Err(resource(format!(
            "unresolved tag entries in {:?}",
            built.missing.keys().collect::<Vec<_>>()
        )));
    }
    Ok(built.tags)
}
/// Convert a data-generator `biome_parameters` report into the quantized
/// preset table format, using vanilla's float `quantizeCoord`.
fn quantize_parameter_report(json: &str) -> Result<String, GeneratorError> {
    let report: Value = serde_json::from_str(json)?;
    let float = |v: &Value| -> Result<i64, GeneratorError> {
        let text = v.to_string();
        text.parse::<f32>()
            .map(quantize)
            .map_err(|_| resource("biome parameter value"))
    };
    let mut entries = Vec::new();
    for entry in report["biomes"]
        .as_array()
        .ok_or_else(|| resource("biome parameter report"))?
    {
        let parameters = &entry["parameters"];
        let mut coordinates = Vec::new();
        for key in [
            "temperature",
            "humidity",
            "continentalness",
            "erosion",
            "depth",
            "weirdness",
        ] {
            let value = &parameters[key];
            coordinates.push(match value.as_array() {
                Some(range) if range.len() == 2 => {
                    serde_json::json!([float(&range[0])?, float(&range[1])?])
                }
                None if value.is_number() => {
                    let v = float(value)?;
                    serde_json::json!([v, v])
                }
                _ => return Err(resource("biome parameter interval")),
            });
        }
        entries.push(serde_json::json!({
            "coordinates": coordinates,
            "offset": float(&parameters["offset"])?,
            "biome": entry["biome"],
        }));
    }
    Ok(Value::Array(entries).to_string())
}
impl GeneratorData {
    /// Load a worldgen bundle directory. Required files are JSON objects mapping
    /// resource identifiers to the resource's JSON text: `noises`, `densities`,
    /// `rules` (material rules), `conditions` (material conditions), `settings`
    /// (noise settings), `carvers`, `biomes`, `biome_tags`, `block_tags`,
    /// `world_presets`; `block_flags.json` is an array of `[is_air, has_fluid]`
    /// per registry state. Multi-noise tables come from `presets.json`
    /// (quantized) or `biome_parameters/<name>.json` data-generator reports.
    /// Optional decoration inputs: `features`, `placed_features`,
    /// `feature_tags`, `fluid_tags`, `state_providers` and the Java-exported
    /// block behavior files in `blocks/` (see `generator_blocks`). Integer block properties are derived from
    /// the registry: properties whose every value is a decimal integer.
    pub fn load(dir: &Path, registry: &'static BlockRegistry) -> Result<Arc<Self>, GeneratorError> {
        let flags: Vec<(bool, bool)> =
            serde_json::from_str(&fs::read_to_string(dir.join("block_flags.json"))?)?;
        if flags.len() != registry.len() {
            return Err(resource("block_flags.json does not match the registry"));
        }
        let block_tags = build_tags(&read_bundle(dir, "block_tags")?, |id| {
            registry.default_state(id).is_some()
        })?;
        let tag_set = |name: &str| -> Result<BTreeSet<&str>, GeneratorError> {
            Ok(block_tags
                .get(name)
                .ok_or_else(|| resource(format!("missing block tag {name}")))?
                .iter()
                .map(String::as_str)
                .collect())
        };
        let motion = tag_set("minecraft:blocks_motion_in_heightmap")?;
        let no_leaves = tag_set("minecraft:blocks_motion_in_heightmap_no_leaves")?;
        let mut values = BTreeMap::<(String, String), bool>::new();
        let states = (0..registry.len())
            .map(|i| {
                let state = registry.state(BlockStateId(i as u32)).unwrap();
                for (key, value) in &state.properties {
                    let integer = !value.is_empty()
                        && value
                            .trim_start_matches('-')
                            .bytes()
                            .all(|b| b.is_ascii_digit());
                    *values
                        .entry((state.name.clone(), key.clone()))
                        .or_insert(true) &= integer;
                }
                GenBlockState {
                    air: flags[i].0,
                    air_block: state.name == "minecraft:air",
                    fluid: flags[i].1,
                    motion: motion.contains(state.name.as_str()),
                    no_leaves: no_leaves.contains(state.name.as_str()),
                }
            })
            .collect::<Vec<_>>();
        let integers = values
            .into_iter()
            .filter_map(|(key, integer)| integer.then_some(key))
            .collect();
        let biomes = read_bundle(dir, "biomes")?;
        let biome_tags = build_tags(&read_bundle(dir, "biome_tags")?, |id| {
            biomes.contains_key(id)
        })?;
        let biome_set = biomes.keys().cloned().collect::<BTreeSet<_>>();
        let conditions = MaterialConditionRegistry::from_json_with_biome_tags(
            &read_bundle(dir, "conditions")?,
            &biome_set,
            &biome_tags,
        )?;
        let mut presets = read_optional_bundle(dir, "presets")?.unwrap_or_default();
        let reports = dir.join("biome_parameters");
        if reports.is_dir() {
            for entry in fs::read_dir(&reports)? {
                let path = entry?.path();
                let Some(stem) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .filter(|_| path.extension().is_some_and(|e| e == "json"))
                else {
                    continue;
                };
                presets
                    .entry(format!("minecraft:{stem}"))
                    .or_insert(quantize_parameter_report(&fs::read_to_string(&path)?)?);
            }
        }
        let source_catalog =
            BiomeSourceCatalog::new(&biome_set, &biome_tags, &presets.keys().cloned().collect())?;
        let carvers = CarverCatalog::from_json(&read_bundle(dir, "carvers")?)?;
        let mut biome_names = Vec::new();
        let mut biome_ids = BTreeMap::new();
        let mut biome_json = Vec::new();
        let mut temperatures = Vec::new();
        let mut biome_carvers = Vec::new();
        for (name, json) in &biomes {
            biome_ids.insert(name.clone(), BiomeId(biome_names.len() as u32));
            biome_names.push(Arc::<str>::from(name.as_str()));
            temperatures.push(BiomeTemperatureSettings::from_json(json)?);
            let value: Value = serde_json::from_str(json)?;
            biome_carvers.push(carvers.decode_set(&value["carvers"].to_string())?);
            biome_json.push(json.clone());
        }
        let carving_blocks = CarvingBlocks::from_registry(
            registry,
            &tag_set("minecraft:uncarvable")?
                .into_iter()
                .map(str::to_owned)
                .collect(),
        )
        .map_err(|e| resource(format!("carving blocks: {e}")))?;
        let feature_resources = match (
            read_optional_bundle(dir, "features")?,
            read_optional_bundle(dir, "placed_features")?,
        ) {
            (Some(features), Some(placed)) => {
                let fluid_tags = match read_optional_bundle(dir, "fluid_tags")? {
                    Some(raw) => build_tags(&raw, |id| {
                        matches!(
                            id,
                            "minecraft:water"
                                | "minecraft:flowing_water"
                                | "minecraft:lava"
                                | "minecraft:flowing_lava"
                                | "minecraft:empty"
                        )
                    })?,
                    None => BTreeMap::new(),
                };
                Some(FeatureResources {
                    features,
                    placed,
                    feature_tags: read_optional_bundle(dir, "feature_tags")?.unwrap_or_default(),
                    blocks: super::generator_blocks::BlockBehavior::load(
                        &dir.join("blocks"),
                        registry,
                        &block_tags,
                        &fluid_tags,
                    )?,
                    fluid_tags,
                    state_providers: read_optional_bundle(dir, "state_providers")?
                        .unwrap_or_default(),
                })
            }
            _ => None,
        };
        let structures = match fs::read_to_string(dir.join("blocks/structure-blocks.json")) {
            Ok(json) => Some(Arc::new(super::structure::StructureResources::new(
                registry,
                super::structure::StateTransforms::from_json(&json, registry.len())?,
                &dir.join("structure"),
                read_optional_bundle(dir, "processor_lists")?.unwrap_or_default(),
                &block_tags,
            ))),
            Err(_) => None,
        };
        let structure_resources = match (
            read_optional_bundle(dir, "structures")?,
            read_optional_bundle(dir, "structure_sets")?,
            read_optional_bundle(dir, "template_pools")?,
        ) {
            (Some(structures), Some(sets), Some(pools)) => Some(StructureBundles {
                structures,
                sets,
                pools,
            }),
            _ => None,
        };
        let behaviours = dir
            .parent()
            .filter(|root| root.join("block-behaviours.json").exists())
            .and_then(|root| crate::level::Behaviours::load(registry, root).ok())
            .map(Arc::new);
        Ok(Arc::new(Self {
            behaviours,
            structure_resources,
            structures,
            registry,
            states: GenBlockStates(states.into()),
            integers,
            noises: read_bundle(dir, "noises")?,
            densities: read_bundle(dir, "densities")?,
            rules: read_bundle(dir, "rules")?,
            settings: read_bundle(dir, "settings")?,
            world_presets: read_bundle(dir, "world_presets")?,
            presets,
            conditions,
            source_catalog,
            biome_names,
            biome_ids,
            biome_json,
            temperatures,
            biome_carvers,
            carving_blocks,
            block_tags,
            biome_tags,
            feature_resources,
        }))
    }
    pub fn registry(&self) -> &'static BlockRegistry {
        self.registry
    }
    pub fn block_states(&self) -> &GenBlockStates {
        &self.states
    }
    /// Biome identifiers in registry (identifier-sorted) order.
    pub fn biome_names(&self) -> &[Arc<str>] {
        &self.biome_names
    }
    pub fn biome_name(&self, id: BiomeId) -> &str {
        &self.biome_names[id.0 as usize]
    }
    pub fn biome_id(&self, name: &str) -> Option<BiomeId> {
        self.biome_ids.get(name).copied()
    }
    /// The biome's resource JSON (climate, effects such as water/sky/foliage
    /// colors, attributes, carvers and features).
    pub fn biome_json(&self, id: BiomeId) -> &str {
        &self.biome_json[id.0 as usize]
    }
    pub(super) fn integers(&self) -> &BTreeSet<(String, String)> {
        &self.integers
    }
}

/// A generated chunk with its quart biomes. Blocks and heightmaps live in
/// `terrain`; after `generate_proto` it is at the TERRAIN status with the
/// WORLD_SURFACE(_WG), OCEAN_FLOOR(_WG) and MOTION_BLOCKING(_NO_LEAVES)
/// heightmaps primed. Fluid positions scheduled for postprocessing are kept in
/// `terrain.postprocessing()`.
pub struct ProtoChunk {
    pub terrain: TerrainChunk<'static, GenBlockStates>,
    biomes: BiomeChunk<BiomeId>,
    data: Arc<GeneratorData>,
    decorated: bool,
    /// Block-entity data placed by structures and template features, in
    /// placement order (positions are absolute block coordinates).
    pub block_entities: Vec<([i32; 3], crate::nbt::Compound)>,
    /// Structure starts referencing this chunk (STRUCTURE_REFERENCES).
    pub(super) structure_starts: Vec<Arc<super::structure::start::StructureStart>>,
    /// Entities placed by structures (`ProtoChunk.addEntity`), as NBT with
    /// absolute `Pos`.
    pub entities: Vec<crate::nbt::Compound>,
}
impl ProtoChunk {
    pub fn position(&self) -> ChunkPos {
        self.terrain.position()
    }
    pub fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.terrain.block(x, y, z)
    }
    pub fn block_name(&self, x: i32, y: i32, z: i32) -> &str {
        &self
            .data
            .registry
            .state(self.block(x, y, z))
            .expect("registered state")
            .name
    }
    /// Stored quart biome (4×4×4 cell) containing this block, without the
    /// client's biome zoom. X/Z wrap to this chunk; Y clamps to its range.
    pub fn biome_id_at(&self, x: i32, y: i32, z: i32) -> BiomeId {
        *self.biomes.noise_biome([x >> 2, y >> 2, z >> 2])
    }
    pub fn biome_at(&self, x: i32, y: i32, z: i32) -> &str {
        self.data.biome_name(self.biome_id_at(x, y, z))
    }
    pub fn biomes(&self) -> &BiomeChunk<BiomeId> {
        &self.biomes
    }
    pub fn data(&self) -> &Arc<GeneratorData> {
        &self.data
    }
    /// Structure starts referencing this chunk: (structure id, start chunk,
    /// inclusive bounding box min/max).
    pub fn structures(&self) -> Vec<(&str, ChunkPos, [i32; 3], [i32; 3])> {
        self.structure_starts
            .iter()
            .map(|s| {
                (
                    s.name.as_str(),
                    s.chunk,
                    s.bounding_box.min,
                    s.bounding_box.max,
                )
            })
            .collect()
    }
    pub fn is_decorated(&self) -> bool {
        self.decorated
    }
    pub(super) fn set_decorated(&mut self) {
        self.decorated = true;
    }
    /// Highest non-air block + 1 in the column (WORLD_SURFACE).
    pub fn surface_height(&self, x: i32, z: i32) -> i32 {
        self.terrain
            .heightmap(HeightmapType::WorldSurface)
            .and_then(|map| map.first_available(x & 15, z & 15).ok())
            .unwrap_or(self.terrain.min_y())
    }
}

/// Build range (minimum Y, height) of the vanilla dimension types.
fn dimension_type_height(name: &str) -> Option<(i32, i32)> {
    match name {
        "minecraft:overworld" | "minecraft:overworld_caves" => Some((-64, 384)),
        "minecraft:the_nether" | "minecraft:the_end" => Some((0, 256)),
        _ => None,
    }
}

/// Per-noise-settings values resolved from the dimension's generator JSON.
struct Settings {
    noise_min_y: i32,
    noise_height: i32,
    /// The dimension type's build range (LevelHeightAccessor): proto chunks,
    /// their heightmaps and the decoration region span it, while the noise
    /// only fills `noise_min_y..noise_min_y + noise_height`.
    level_min_y: i32,
    level_height: i32,
    sea_level: i32,
    default_block: BlockStateId,
    default_fluid: BlockStateId,
    legacy: bool,
}

/// The seed-bound pipeline for one random-source kind.
struct Pipeline<R: RandomSource> {
    data: Arc<GeneratorData>,
    seed: i64,
    settings: Settings,
    root: R::Positional,
    router: RouterSamplers,
    climate: ClimateSampler,
    aquifer: Option<[Arc<dyn DensitySampler>; 6]>,
    rule: MaterialRule,
    preliminary: Arc<dyn DensitySampler>,
    source: ResolvedBiomeSource,
    surface: MaterialSurface<R::Positional>,
    noises: BTreeMap<String, Arc<dyn Noise>>,
    extensions: MaterialExtensions<R::Positional>,
    picker: GlobalFluidPicker,
    zoom: BiomeZoom,
    biome_cache: BiomeCache,
    carver_cache: Mutex<HashMap<(i32, i32), BiomeId>>,
    pub(super) decoration: Option<super::generator_decoration::Decoration>,
    pub(super) structures: Option<super::generator_structures::StructureState>,
}
const CACHE_LIMIT: usize = 16384;
type BiomeCache = Mutex<HashMap<(i32, i32), Arc<BiomeChunk<BiomeId>>>>;

fn state(registry: &BlockRegistry, raw: &Value) -> Result<BlockStateId, GeneratorError> {
    let name = raw
        .get("Name")
        .and_then(Value::as_str)
        .or_else(|| raw.as_str())
        .ok_or_else(|| resource("block state"))?;
    let properties = raw.get("Properties").and_then(Value::as_object);
    match properties {
        Some(properties) if !properties.is_empty() => {
            let descriptor = crate::chunk::BlockState {
                name: super::tags::identifier(name)?,
                properties: properties
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                    .collect(),
            };
            registry
                .resolve(&descriptor)
                .map_err(|e| resource(format!("block state {name}: {e}")))
        }
        _ => registry
            .default_state(&super::tags::identifier(name)?)
            .ok_or_else(|| resource(format!("unknown block {name}"))),
    }
}

impl<R: RandomSource> Pipeline<R>
where
    R::Positional: Clone + Send + Sync,
{
    fn new(
        data: Arc<GeneratorData>,
        seed: i64,
        mut random: R,
        generator: &Value,
        settings_json: &Value,
        with_structures: bool,
        level: Option<(i32, i32)>,
    ) -> Result<Self, GeneratorError> {
        let registry = data.registry;
        let noise = &settings_json["noise"];
        let min_y = noise["min_y"]
            .as_i64()
            .ok_or_else(|| resource("noise min_y"))? as i32;
        let height = noise["height"]
            .as_i64()
            .ok_or_else(|| resource("noise height"))? as i32;
        let (level_min_y, level_height) = level.unwrap_or((min_y, height));
        if min_y < level_min_y || min_y + height > level_min_y + level_height {
            return Err(resource("noise range outside the dimension's build height"));
        }
        let settings = Settings {
            noise_min_y: min_y,
            noise_height: height,
            level_min_y,
            level_height,
            sea_level: settings_json["sea_level"]
                .as_i64()
                .ok_or_else(|| resource("sea_level"))? as i32,
            default_block: state(registry, &settings_json["default_block"])?,
            default_fluid: state(registry, &settings_json["default_fluid"])?,
            legacy: settings_json["legacy_random_source"]
                .as_bool()
                .unwrap_or(false),
        };
        let root = random.fork_positional();
        let codec = crate::registry::BlockStateJsonCodec::new(registry, data.integers.clone())
            .map_err(|e| resource(e.to_string()))?;
        let mut densities = DensityRegistry::from_json(&data.noises, &data.densities)?;
        let router =
            NoiseRouter::from_json(&settings_json["noise_router"].to_string(), &mut densities)?;
        let aquifers = settings_json
            .get("aquifers")
            .filter(|v| !v.is_null())
            .map(|v| AquiferConfig::from_json(&v.to_string(), &mut densities))
            .transpose()?;
        let mut library = MaterialRuleLibrary::from_json(
            &data.rules,
            &data.conditions,
            &codec,
            densities,
            seed,
            settings.legacy,
        )?;
        let router = library.compile_router(router)?;
        let aquifer = aquifers
            .map(|config| library.compile_aquifer(config))
            .transpose()?
            .map(|samplers| AquiferField::ALL.map(|field| samplers.get(field).clone()));
        let rule_name = settings_json["material_rule"]
            .as_str()
            .ok_or_else(|| resource("material_rule"))?;
        let rule = library.rule(rule_name)?;
        let preliminary = router.get(RouterField::ChunkSurfaceLevel).clone();
        let source = data
            .source_catalog
            .decode(&generator["biome_source"].to_string())?
            .resolve(&data.presets)
            .map_err(|e| resource(format!("biome source: {e}")))?;
        let climate = router.climate_sampler();
        let mut noises = BTreeMap::new();
        for (name, json) in &data.noises {
            let noise: Arc<dyn Noise> = Arc::new(
                decode_noise(json)?
                    .create(&mut root.from_hash_of(name))
                    .map_err(DecodeError::from)?,
            );
            noises.insert(name.clone(), noise);
        }
        let noise = |name: &str| -> Result<Arc<dyn Noise>, GeneratorError> {
            noises
                .get(name)
                .cloned()
                .ok_or_else(|| resource(format!("missing noise {name}")))
        };
        let surface = MaterialSurface::new(
            root.clone(),
            ClayBandBlocks::from_registry(registry).ok_or_else(|| resource("clay bands"))?,
            noise("minecraft:clay_bands_offset")?,
            noise("minecraft:surface")?,
            noise("minecraft:surface_secondary")?,
        );
        let default = |name: &str| {
            registry
                .default_state(name)
                .ok_or_else(|| resource(format!("missing block {name}")))
        };
        let extensions = MaterialExtensions::new(
            root.clone(),
            ExtensionBlocks {
                default: settings.default_block,
                water: default("minecraft:water")?,
                packed_ice: default("minecraft:packed_ice")?,
                snow: default("minecraft:snow_block")?,
            },
            ExtensionNoises {
                badlands_surface: noise("minecraft:badlands_surface")?,
                badlands_pillar: noise("minecraft:badlands_pillar")?,
                badlands_roof: noise("minecraft:badlands_pillar_roof")?,
                iceberg_surface: noise("minecraft:iceberg_surface")?,
                iceberg_pillar: noise("minecraft:iceberg_pillar")?,
                iceberg_roof: noise("minecraft:iceberg_pillar_roof")?,
            },
            settings.sea_level,
        );
        let picker = GlobalFluidPicker::new(
            settings.sea_level,
            settings.default_fluid,
            default("minecraft:lava")?,
            default("minecraft:air")?,
        );
        let mut pipeline = Self {
            zoom: BiomeZoom::new(obfuscate_seed(seed)),
            data,
            seed,
            settings,
            root,
            router,
            climate,
            aquifer,
            rule,
            preliminary,
            source,
            surface,
            noises,
            extensions,
            picker,
            biome_cache: Mutex::new(HashMap::new()),
            carver_cache: Mutex::new(HashMap::new()),
            decoration: None,
            structures: None,
        };
        pipeline.decoration =
            super::generator_decoration::Decoration::new(&pipeline.data, &pipeline.source)?;
        if with_structures {
            let possible = pipeline
                .source
                .possible_biomes()
                .into_iter()
                .filter_map(|name| pipeline.data.biome_id(name))
                .collect();
            pipeline.structures =
                super::generator_structures::StructureState::new(&pipeline.data, &possible)?;
        }
        Ok(pipeline)
    }
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(
            self.settings.noise_min_y,
            self.settings.noise_height,
            self.settings.sea_level,
        )
    }
    fn biome_chunk(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Result<Arc<BiomeChunk<BiomeId>>, GeneratorError> {
        if let Some(chunk) = self.biome_cache.lock().unwrap().get(&(pos.x, pos.z)) {
            return Ok(chunk.clone());
        }
        let (min, height) = (self.settings.level_min_y, self.settings.level_height);
        let mut chunk = BiomeChunk::new(pos, min, height, BiomeId(0)).map_err(stage("biomes"))?;
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        let context = &mut worker.biome_context;
        let volume = resolver
            .prepare_volume(
                context,
                [
                    pos.x.wrapping_shl(4) >> 2,
                    min >> 2,
                    pos.z.wrapping_shl(4) >> 2,
                ],
                [4, height >> 2, 4],
            )
            .map_err(stage("biomes"))?;
        let ids = &self.data.biome_ids;
        chunk
            .fill(min, min + height - 1, |quart| {
                resolver
                    .sample_prepared(context, volume.as_ref(), quart)
                    .map_err(|e| e.to_string())
                    .and_then(|name| ids.get(name).copied().ok_or_else(|| name.to_owned()))
            })
            .map_err(stage("biomes"))?;
        let chunk = Arc::new(chunk);
        let mut cache = self.biome_cache.lock().unwrap();
        if cache.len() >= CACHE_LIMIT {
            cache.clear();
        }
        cache.insert((pos.x, pos.z), chunk.clone());
        Ok(chunk)
    }
    fn carver_biome(&self, worker: &mut WorkerState, origin: ChunkPos) -> Result<BiomeId, String> {
        if let Some(id) = self.carver_cache.lock().unwrap().get(&(origin.x, origin.z)) {
            return Ok(*id);
        }
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        let quart = [
            origin.x.wrapping_shl(4) >> 2,
            0,
            origin.z.wrapping_shl(4) >> 2,
        ];
        let name = resolver
            .sample(&mut worker.carver_context, quart)
            .map_err(|e| e.to_string())?;
        let id = self.data.biome_id(name).ok_or_else(|| name.to_owned())?;
        let mut cache = self.carver_cache.lock().unwrap();
        if cache.len() >= CACHE_LIMIT * 4 {
            cache.clear();
        }
        cache.insert((origin.x, origin.z), id);
        Ok(id)
    }
    fn generate(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Result<ProtoChunk, GeneratorError> {
        let data = &self.data;
        let (min, height, sea) = (
            self.settings.noise_min_y,
            self.settings.noise_height,
            self.settings.sea_level,
        );
        let mut clock = std::time::Instant::now();
        let mut neighbors = Vec::with_capacity(9);
        let mut possible = BTreeSet::new();
        for dz in -1..=1 {
            for dx in -1..=1 {
                let chunk = self.biome_chunk(
                    worker,
                    ChunkPos::new(pos.x.wrapping_add(dx), pos.z.wrapping_add(dz)),
                )?;
                let mut section = 0;
                while let Some(biomes) = chunk.section(section) {
                    possible.extend(biomes.iter().copied());
                    section += 1;
                }
                neighbors.push((*chunk).clone());
            }
        }
        let center = neighbors[4].clone();
        let biomes =
            BiomeRegion::new(neighbors, obfuscate_seed(self.seed)).map_err(stage("biomes"))?;
        let possible = possible
            .into_iter()
            .map(|id| data.biome_name(id).to_owned())
            .collect();
        worker.timings.biomes += lap(&mut clock);
        let mut chunk = TerrainChunk::new(
            pos,
            self.settings.level_min_y,
            self.settings.level_height,
            data.registry,
            data.states.clone(),
        )
        .map_err(stage("noise"))?;
        chunk.set_generation_stage(PreLightChunkStage::Biomes);
        let references = match &self.structures {
            Some(structures) => structures.references(
                &mut StructureQueries {
                    pipeline: self,
                    worker,
                    valid: BTreeSet::new(),
                },
                pos,
            ),
            None => Vec::new(),
        };
        let mut context = SamplerContext::cached();
        if let Some(beard) = self
            .structures
            .as_ref()
            .and_then(|s| s.beardifier(&references, pos))
        {
            context = context.with_world_fields(
                horizon_core::density::WorldFields::default().with_field(
                    horizon_core::density::WorldField::Beardifier,
                    Arc::new(beard),
                ),
            );
        }
        worker.timings.structures += lap(&mut clock);
        let volume = DensityVolume::new(
            [16, height, 16],
            [pos.x.wrapping_shl(4), min, pos.z.wrapping_shl(4)],
            [1; 3],
        )
        .map_err(stage("noise"))?;
        worker.timings.noise += lap(&mut clock);
        let mut aquifer: Box<dyn TerrainAquifer> = match &self.aquifer {
            Some(samplers) => Box::new(
                NoiseBasedAquifer::new(
                    AquiferSamplers::new(samplers.clone()),
                    self.picker,
                    self.root
                        .from_hash_of("minecraft:aquifer")
                        .fork_positional(),
                    RegistryAquiferBlockStates::new(data.registry).map_err(stage("aquifer"))?,
                    volume,
                    &mut context,
                )
                .map_err(stage("aquifer"))?,
            ),
            None => Box::new(DisabledAquifer::new(
                self.picker,
                data.registry.default_state("minecraft:air").unwrap(),
            )),
        };
        worker.timings.noise += lap(&mut clock);
        fill_noise(
            &mut chunk,
            volume,
            self.router.get(RouterField::FinalDensity).as_ref(),
            &mut context,
            aquifer.as_mut(),
            self.settings.default_block,
        )
        .map_err(stage("noise"))?;
        let generation = self.generation();
        worker.timings.noise += lap(&mut clock);
        build_surface(
            &mut chunk,
            self.rule.clone(),
            SurfaceInputs::<R, _> {
                generation,
                noise_volume: volume,
                root: self.root.clone(),
                surface: SurfaceQueries {
                    pipeline: self,
                    temperatures: &mut worker.temperatures,
                    biome: Biomes::Region(&biomes),
                    possible: Some(possible),
                },
                preliminary_surface: self.preliminary.clone(),
                density_context: &mut context,
                write_min_y: min,
                write_max_y: min + height - 1,
            },
            &self.extensions,
        )
        .map_err(stage("surface"))?;
        worker.timings.surface += lap(&mut clock);
        let carvers = &data.biome_carvers;
        let mask = generate_carving_mask(self.seed, pos, generation, false, |origin| {
            self.carver_biome(worker, origin)
                .map(|id| carvers[id.0 as usize].as_slice())
        })
        .map_err(stage("carvers"))?;
        worker.timings.carvers += lap(&mut clock);
        if !mask.is_empty() {
            let mut resolver = self.source.resolver(
                &self.climate,
                self.router.get(RouterField::Erosion).as_ref(),
            );
            let mut biome_context = SamplerContext::default();
            apply_carving_mask(
                &mut chunk,
                &mask,
                self.rule.clone(),
                TopMaterialInputs::<R, _> {
                    generation,
                    root: self.root.clone(),
                    surface: SurfaceQueries {
                        pipeline: self,
                        temperatures: &mut worker.temperatures,
                        biome: Biomes::Resolver(&mut resolver, &mut biome_context),
                        possible: None,
                    },
                    preliminary_surface: self.preliminary.clone(),
                    density_context: &mut context,
                },
                aquifer.as_mut(),
                &data.carving_blocks,
                |_| true,
            )
            .map_err(stage("carvers"))?;
        }
        worker.timings.carvers += lap(&mut clock);
        chunk
            .prime_heightmaps(&[
                HeightmapType::MotionBlocking,
                HeightmapType::MotionBlockingNoLeaves,
                HeightmapType::OceanFloor,
                HeightmapType::WorldSurface,
            ])
            .map_err(stage("heightmaps"))?;
        chunk.set_generation_stage(PreLightChunkStage::Terrain);
        let _ = sea;
        Ok(ProtoChunk {
            terrain: chunk,
            biomes: center,
            data: data.clone(),
            decorated: false,
            block_entities: Vec::new(),
            structure_starts: references,
            entities: Vec::new(),
        })
    }
}

impl<R: RandomSource> Pipeline<R>
where
    R::Positional: Clone + Send + Sync,
{
    /// ChunkGenerator.getBaseHeight: the first free Y above the noise column
    /// (with aquifers, without terrain adaptation) for a heightmap type.
    fn base_height(&self, x: i32, z: i32, kind: HeightmapType) -> i32 {
        let min = self.settings.noise_min_y;
        let mut found = min;
        self.noise_column(x, z, &mut |y, state| {
            if kind.is_opaque(self.data.states.get(state)) {
                found = y + 1;
                return true;
            }
            false
        });
        found
    }
    /// ChunkGenerator.getBaseColumn: opacity of each noise-column state for
    /// a heightmap type, indexed from the minimum Y.
    fn base_column(&self, x: i32, z: i32, kind: HeightmapType) -> Vec<bool> {
        self.base_columns(x, z, &[kind]).pop().unwrap_or_default()
    }
    /// One noise-column pass, as opacities for each heightmap kind.
    fn base_columns(&self, x: i32, z: i32, kinds: &[HeightmapType]) -> Vec<Vec<bool>> {
        let (min, height) = (self.settings.noise_min_y, self.settings.noise_height);
        let mut columns = vec![vec![false; height as usize]; kinds.len()];
        self.noise_column(x, z, &mut |y, state| {
            let state = self.data.states.get(state);
            for (column, kind) in columns.iter_mut().zip(kinds) {
                column[(y - min) as usize] = kind.is_opaque(state);
            }
            false
        });
        columns
    }
    /// iterateNoiseColumn from the top down until `visit` returns true.
    fn noise_column(&self, x: i32, z: i32, visit: &mut dyn FnMut(i32, BlockStateId) -> bool) {
        let (min, height) = (self.settings.noise_min_y, self.settings.noise_height);
        let Ok(volume) = DensityVolume::new([1, height, 1], [x, min, z], [1; 3]) else {
            return;
        };
        let mut context = SamplerContext::cached();
        let mut aquifer: Box<dyn TerrainAquifer> = match &self.aquifer {
            Some(samplers) => match RegistryAquiferBlockStates::new(self.data.registry)
                .ok()
                .and_then(|blocks| {
                    NoiseBasedAquifer::new(
                        AquiferSamplers::new(samplers.clone()),
                        self.picker,
                        self.root
                            .from_hash_of("minecraft:aquifer")
                            .fork_positional(),
                        blocks,
                        volume,
                        &mut context,
                    )
                    .ok()
                }) {
                Some(aquifer) => Box::new(aquifer),
                None => return,
            },
            None => Box::new(DisabledAquifer::new(
                self.picker,
                self.data.registry.default_state("minecraft:air").unwrap(),
            )),
        };
        let mut density = vec![0.; height as usize];
        if self
            .router
            .get(RouterField::FinalDensity)
            .sample_volume(&mut context, &mut density, volume)
            .is_err()
        {
            return;
        }
        for y in (0..height).rev() {
            let state = aquifer
                .compute_substance(
                    &mut context,
                    [x, min + y, z],
                    f64::from(density[y as usize]),
                )
                .ok()
                .flatten()
                .unwrap_or(self.settings.default_block);
            if visit(min + y, state) {
                return;
            }
        }
    }
}
/// Biome and height queries for structure generation on one worker.
pub(super) struct StructureQueries<'a, R: RandomSource> {
    pipeline: &'a Pipeline<R>,
    worker: &'a mut WorkerState,
    valid: BTreeSet<BiomeId>,
}
impl<R: RandomSource> super::structure::jigsaw::JigsawContext for StructureQueries<'_, R>
where
    R::Positional: Clone + Send + Sync,
{
    // Structure.GenerationContext.heightAccessor: the chunk, so the level's range.
    fn min_y(&self) -> i32 {
        self.pipeline.settings.level_min_y
    }
    fn max_y(&self) -> i32 {
        self.pipeline.settings.level_min_y + self.pipeline.settings.level_height - 1
    }
    fn base_height(&mut self, x: i32, z: i32, kind: HeightmapType) -> i32 {
        if let Some(h) = self.worker.heights.get(&(x, z, kind as u8)) {
            return *h;
        }
        let h = self.pipeline.base_height(x, z, kind);
        if self.worker.heights.len() > 65536 {
            self.worker.heights.clear();
        }
        self.worker.heights.insert((x, z, kind as u8), h);
        h
    }
    fn column_has_valid_biome(&mut self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool {
        use super::structure::start::StartContext;
        ((min_y >> 2)..=(max_y >> 2)).any(|qy| {
            let id = self.noise_biome([x >> 2, qy, z >> 2]);
            self.valid.contains(&id)
        })
    }
}
impl<R: RandomSource> super::structure::start::StartContext for StructureQueries<'_, R>
where
    R::Positional: Clone + Send + Sync,
{
    fn seed(&self) -> i64 {
        self.pipeline.seed
    }
    fn sea_level(&self) -> i32 {
        self.pipeline.settings.sea_level
    }
    fn base_column(&mut self, x: i32, z: i32, kind: HeightmapType) -> Vec<bool> {
        self.pipeline.base_column(x, z, kind)
    }
    fn base_columns(&mut self, x: i32, z: i32, kinds: &[HeightmapType]) -> Vec<Vec<bool>> {
        self.pipeline.base_columns(x, z, kinds)
    }
    fn cold_enough_to_snow(&mut self, biome: BiomeId, pos: [i32; 3]) -> bool {
        let sea = self.pipeline.settings.sea_level;
        self.worker.temperatures[biome.0 as usize]
            .cold_enough_to_snow(BlockPos::new(pos[0], pos[1], pos[2]), sea)
    }
    fn generation(&self) -> WorldGenerationContext {
        self.pipeline.generation()
    }
    fn noise_biome(&mut self, quart: [i32; 3]) -> BiomeId {
        let mut resolver = self.pipeline.source.resolver(
            &self.pipeline.climate,
            self.pipeline.router.get(RouterField::Erosion).as_ref(),
        );
        resolver
            .sample(&mut self.worker.carver_context, quart)
            .ok()
            .and_then(|name| self.pipeline.data.biome_id(name))
            .unwrap_or(BiomeId(0))
    }
    fn set_valid_biomes(&mut self, biomes: &BTreeSet<BiomeId>) {
        self.valid = biomes.clone();
    }
}

enum Biomes<'a, 'b> {
    Region(&'a BiomeRegion<BiomeId>),
    Resolver(
        &'a mut super::biome_source::BiomeSourceResolver<'b>,
        &'a mut SamplerContext,
    ),
}
/// Material-rule world queries for one chunk: shared seed-bound noises, the
/// worker's temperature caches and either stored or raw-resolver biomes.
struct SurfaceQueries<'a, 'b, R: RandomSource> {
    pipeline: &'a Pipeline<R>,
    temperatures: &'a mut [BiomeTemperature],
    biome: Biomes<'a, 'b>,
    possible: Option<BTreeSet<String>>,
}
impl<R: RandomSource> SurfaceQueries<'_, '_, R> {
    fn temperature(&mut self, name: &str) -> Result<&mut BiomeTemperature, MaterialRuleError> {
        let id = self
            .pipeline
            .data
            .biome_id(name)
            .ok_or(MaterialRuleError::MissingBiome)?;
        Ok(&mut self.temperatures[id.0 as usize])
    }
}
impl<R: RandomSource> MaterialSurfaceQueries for SurfaceQueries<'_, '_, R> {
    fn surface_depth(&mut self, x: i32, z: i32) -> i32 {
        self.pipeline.surface.surface_depth(x, z)
    }
    fn surface_secondary(&mut self, x: i32, z: i32) -> f64 {
        self.pipeline.surface.surface_secondary(x, z)
    }
    fn band(&mut self, p: BlockPos) -> Result<BlockStateId, MaterialRuleError> {
        self.pipeline
            .surface
            .band([p.x, p.y, p.z])
            .ok_or(MaterialRuleError::InvalidBandIndex)
    }
    fn bind_noise(&mut self, name: &str) -> Result<Arc<dyn Noise>, DecodeError> {
        self.pipeline
            .noises
            .get(&super::tags::identifier(name)?)
            .cloned()
            .ok_or(DecodeError::Field("unknown material noise"))
    }
    fn possible_biomes(&self) -> Option<BTreeSet<String>> {
        self.possible.clone()
    }
    fn biome(&mut self, p: BlockPos) -> Result<Option<Arc<str>>, MaterialRuleError> {
        let data = &self.pipeline.data;
        let id = match &mut self.biome {
            Biomes::Region(region) => *region
                .biome(p)
                .map_err(|_| MaterialRuleError::MissingBiome)?,
            Biomes::Resolver(resolver, context) => {
                let name = resolver
                    .sample(context, self.pipeline.zoom.quart_at_block(p))
                    .map_err(|_| MaterialRuleError::BiomeQueryFailed)?;
                data.biome_id(name).ok_or(MaterialRuleError::MissingBiome)?
            }
        };
        Ok(Some(data.biome_names[id.0 as usize].clone()))
    }
    fn cold_enough_to_snow(
        &mut self,
        name: &str,
        p: BlockPos,
        sea: i32,
    ) -> Result<bool, MaterialRuleError> {
        Ok(self.temperature(name)?.cold_enough_to_snow(p, sea))
    }
    fn melts_frozen_ocean(
        &mut self,
        name: &str,
        p: BlockPos,
        sea: i32,
    ) -> Result<bool, MaterialRuleError> {
        Ok(self
            .temperature(name)?
            .should_melt_frozen_ocean_iceberg_slightly(p, sea))
    }
}

fn lap(clock: &mut std::time::Instant) -> std::time::Duration {
    let now = std::time::Instant::now();
    let elapsed = now - *clock;
    *clock = now;
    elapsed
}
/// Cumulative wall time spent by one worker in each proto-chunk stage.
#[derive(Clone, Copy, Debug, Default)]
pub struct StageTimings {
    pub chunks: u64,
    pub biomes: std::time::Duration,
    /// Noise fill including aquifer setup.
    pub noise: std::time::Duration,
    pub surface: std::time::Duration,
    /// Structure starts/references and terrain adaptation.
    pub structures: std::time::Duration,
    /// Carving mask generation/application and final heightmap priming.
    pub carvers: std::time::Duration,
}
/// Worker-owned mutable caches. Biome temperature caches mirror vanilla's
/// thread-local caches and never change results.
pub(super) struct WorkerState {
    pub(super) temperatures: Vec<BiomeTemperature>,
    biome_context: SamplerContext,
    carver_context: SamplerContext,
    timings: StageTimings,
    heights: HashMap<(i32, i32, u8), i32>,
    /// Decorate with structure pieces only (for structure parity checks).
    pub(super) structures_only: bool,
}

trait SeededPipeline: Send + Sync {
    fn generate(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Result<ProtoChunk, GeneratorError>;
    fn decorate(
        &self,
        worker: &mut WorkerState,
        region: &mut [ProtoChunk; 9],
    ) -> Result<DecorateStats, GeneratorError>;
    fn data(&self) -> &Arc<GeneratorData>;
    fn sea_level(&self) -> i32;
    fn structure_starts(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Vec<Arc<super::structure::start::StructureStart>>;
    fn unsupported_features(&self) -> Vec<(String, String)>;
    fn forget_structure_starts(&self);
    fn ring_positions(&self, set: &str) -> Option<Vec<ChunkPos>>;
    fn locate(
        &self,
        worker: &mut WorkerState,
        names: &[String],
        from: [i32; 3],
        radius: i32,
    ) -> Option<(String, [i32; 3])>;
}
impl<R: RandomSource + 'static> SeededPipeline for Pipeline<R>
where
    R::Positional: Clone + Send + Sync,
{
    fn generate(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Result<ProtoChunk, GeneratorError> {
        Pipeline::generate(self, worker, pos)
    }
    fn decorate(
        &self,
        worker: &mut WorkerState,
        region: &mut [ProtoChunk; 9],
    ) -> Result<DecorateStats, GeneratorError> {
        match &self.decoration {
            Some(decoration) => {
                let center = region[4].position();
                // WorldGenRegion's level random, positioned at the center chunk.
                let mut random = self
                    .root
                    .from_hash_of("minecraft:worldgen_region_random")
                    .fork_positional()
                    .at(BlockPos::new(
                        center.x.wrapping_shl(4),
                        0,
                        center.z.wrapping_shl(4),
                    ));
                decoration.decorate(
                    &self.data,
                    self.seed,
                    self.generation(),
                    worker,
                    region,
                    &mut random,
                    self.structures.as_ref(),
                )
            }
            None => Err(resource("decoration resources were not loaded")),
        }
    }
    fn data(&self) -> &Arc<GeneratorData> {
        &self.data
    }
    fn sea_level(&self) -> i32 {
        self.settings.sea_level
    }
    fn structure_starts(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Vec<Arc<super::structure::start::StructureStart>> {
        match &self.structures {
            Some(structures) => structures
                .starts_at(
                    &mut StructureQueries {
                        pipeline: self,
                        worker,
                        valid: BTreeSet::new(),
                    },
                    pos,
                )
                .to_vec(),
            None => Vec::new(),
        }
    }
    fn unsupported_features(&self) -> Vec<(String, String)> {
        self.decoration
            .as_ref()
            .map(super::generator_decoration::Decoration::unsupported)
            .unwrap_or_default()
    }
    fn forget_structure_starts(&self) {
        if let Some(structures) = &self.structures {
            structures.forget_starts();
        }
    }
    fn ring_positions(&self, set: &str) -> Option<Vec<ChunkPos>> {
        self.structures.as_ref()?.cached_rings(set)
    }
    fn locate(
        &self,
        worker: &mut WorkerState,
        names: &[String],
        from: [i32; 3],
        radius: i32,
    ) -> Option<(String, [i32; 3])> {
        self.structures.as_ref()?.locate(
            &mut StructureQueries {
                pipeline: self,
                worker,
                valid: BTreeSet::new(),
            },
            names,
            from,
            radius,
        )
    }
}

/// Counters from one FEATURES pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecorateStats {
    /// Placed-feature roots executed (including those that placed nothing).
    pub placed: usize,
    /// Roots skipped because their algorithm or bindings are not implemented.
    pub unsupported: usize,
    /// Roots that started but failed on an unavailable world query or error.
    /// Their partial writes are kept, as each root reseeds its own random.
    pub failed: usize,
    /// Identifiers of skipped or failed roots, for diagnostics.
    pub skipped: BTreeSet<String>,
}

/// A seed-bound generator for one dimension, shareable across worker threads.
#[derive(Clone)]
pub struct OverworldGenerator {
    pipeline: Arc<dyn SeededPipeline>,
}
impl OverworldGenerator {
    /// The `minecraft:overworld` dimension of the `minecraft:normal` world preset.
    pub fn new(data: Arc<GeneratorData>, seed: i64) -> Result<Self, GeneratorError> {
        Self::for_dimension(data, seed, "minecraft:normal", "minecraft:overworld")
    }
    /// The overworld without structures (no starts, terrain adaptation or
    /// structure pieces), as in a world with structure generation disabled.
    pub fn without_structures(data: Arc<GeneratorData>, seed: i64) -> Result<Self, GeneratorError> {
        Self::build(data, seed, "minecraft:normal", "minecraft:overworld", false)
    }
    /// Any noise-generator dimension of a bundled world preset.
    pub fn for_dimension(
        data: Arc<GeneratorData>,
        seed: i64,
        preset: &str,
        dimension: &str,
    ) -> Result<Self, GeneratorError> {
        Self::build(data, seed, preset, dimension, true)
    }
    fn build(
        data: Arc<GeneratorData>,
        seed: i64,
        preset: &str,
        dimension: &str,
        with_structures: bool,
    ) -> Result<Self, GeneratorError> {
        let preset_json: Value = serde_json::from_str(
            data.world_presets
                .get(preset)
                .ok_or_else(|| resource(format!("missing world preset {preset}")))?,
        )?;
        let generator = preset_json["dimensions"][dimension]["generator"].clone();
        let level = preset_json["dimensions"][dimension]["type"]
            .as_str()
            .and_then(dimension_type_height);
        if generator["type"].as_str() != Some("minecraft:noise") {
            return Err(resource(format!("{dimension} is not a noise generator")));
        }
        let settings_json: Value = match &generator["settings"] {
            Value::String(name) => serde_json::from_str(
                data.settings
                    .get(name)
                    .ok_or_else(|| resource(format!("missing noise settings {name}")))?,
            )?,
            inline => inline.clone(),
        };
        let pipeline: Arc<dyn SeededPipeline> = if settings_json["legacy_random_source"]
            .as_bool()
            .unwrap_or(false)
        {
            Arc::new(Pipeline::new(
                data,
                seed,
                LegacyRandom::new(seed),
                &generator,
                &settings_json,
                with_structures,
                level,
            )?)
        } else {
            Arc::new(Pipeline::new(
                data,
                seed,
                XoroshiroRandom::new(seed),
                &generator,
                &settings_json,
                with_structures,
                level,
            )?)
        };
        Ok(Self { pipeline })
    }
    pub fn data(&self) -> &Arc<GeneratorData> {
        self.pipeline.data()
    }
    pub fn sea_level(&self) -> i32 {
        self.pipeline.sea_level()
    }
    /// Placed features of this dimension's biomes that decoration skips because
    /// their algorithm or bindings are not implemented, with the decode error.
    pub fn unsupported_features(&self) -> Vec<(String, String)> {
        self.pipeline.unsupported_features()
    }
    /// ConcentricRingsStructurePlacement chunks of a structure set (such as
    /// `minecraft:strongholds`), available once structure starts were looked up.
    pub fn ring_positions(&self, set: &str) -> Option<Vec<ChunkPos>> {
        self.pipeline.ring_positions(set)
    }
    /// Create per-thread state. Workers are cheap; keep one per thread.
    pub fn worker(&self) -> GeneratorWorker {
        let data = self.pipeline.data();
        GeneratorWorker {
            pipeline: self.pipeline.clone(),
            state: WorkerState {
                temperatures: data
                    .temperatures
                    .iter()
                    .map(|s| BiomeTemperature::new(*s))
                    .collect(),
                biome_context: SamplerContext::cached(),
                carver_context: SamplerContext::default(),
                timings: StageTimings::default(),
                structures_only: false,
                heights: HashMap::new(),
            },
        }
    }
}

/// Per-thread generation state.
pub struct GeneratorWorker {
    pipeline: Arc<dyn SeededPipeline>,
    state: WorkerState,
}
impl GeneratorWorker {
    /// Biomes, noise fill and aquifers, material surface and carvers: the
    /// vanilla statuses up to TERRAIN. Independent of neighbor chunks' state.
    pub fn generate_proto(&mut self, pos: ChunkPos) -> Result<ProtoChunk, GeneratorError> {
        let result = self.pipeline.generate(&mut self.state, pos);
        self.state.timings.chunks += 1;
        result
    }
    /// `/locate structure`: the nearest start of one of `names`.
    pub fn locate(
        &mut self,
        names: &[String],
        from: [i32; 3],
        radius: i32,
    ) -> Option<(String, [i32; 3])> {
        self.pipeline.locate(&mut self.state, names, from, radius)
    }
    /// STRUCTURE_STARTS: the structure starts owned by a chunk (cached).
    pub fn structure_starts(
        &mut self,
        pos: ChunkPos,
    ) -> Vec<Arc<super::structure::start::StructureStart>> {
        self.pipeline.structure_starts(&mut self.state, pos)
    }
    /// Forget the cached structure starts: chunks generated afterwards get
    /// fresh starts, as in a new level that has not loaded the old chunks.
    pub fn forget_structure_starts(&mut self) {
        self.pipeline.forget_structure_starts();
    }
    pub fn stage_timings(&self) -> StageTimings {
        self.state.timings
    }
    /// FEATURES status for `region[4]`. The region is a 3×3 neighborhood in
    /// row-major order: index `(dz + 1) * 3 + (dx + 1)` around the center.
    /// Writes may reach the neighbors, as in vanilla. Unsupported features are
    /// skipped and counted. On error, chunks keep partial writes.
    pub fn decorate(
        &mut self,
        region: &mut [ProtoChunk; 9],
    ) -> Result<DecorateStats, GeneratorError> {
        self.pipeline.decorate(&mut self.state, region)
    }
    /// Skip placed features when decorating and keep structure pieces only
    /// (for structure placement parity checks).
    pub fn set_structures_only(&mut self, only: bool) {
        self.state.structures_only = only;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_reports_quantize_like_vanilla() {
        let report = r#"{"biomes":[{"biome":"minecraft:plains","parameters":{
            "temperature":[-0.45,-0.15],"humidity":[-1.0,-0.35],"continentalness":[-0.11,0.03],
            "erosion":[0.45,0.55],"depth":0.0,"weirdness":[-0.2633,-0.05],"offset":0.0}}]}"#;
        let table: Value =
            serde_json::from_str(&quantize_parameter_report(report).unwrap()).unwrap();
        assert_eq!(
            table,
            serde_json::json!([{"coordinates":[[-4500,-1500],[-10000,-3500],[-1100,300],
                [4500,5500],[0,0],[-2633,-500]],"offset":0,"biome":"minecraft:plains"}])
        );
        assert!(
            quantize_parameter_report(r#"{"biomes":[{"parameters":{"temperature":"x"}}]}"#)
                .is_err()
        );
    }
}
