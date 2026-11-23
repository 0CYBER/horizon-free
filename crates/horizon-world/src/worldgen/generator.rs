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
    biome_chunks::BiomeChunks,
    biome_source::{
        BiomeSourceCatalog, BiomeSourceDefinition, ResolvedBiomeSource, regions::RegionLayout,
    },
    biome_temperature::{BiomeTemperature, BiomeTemperatureSettings},
    carver::{CarverCatalog, WorldCarver, generate_carving_mask},
    carving::{CarvingBlocks, apply_carving_mask_shared},
    climate::{ClimateSampler, decode_parameter, quantize},
    decode_noise,
    density::DensityRegistry,
    generator_structures::Starts,
    material_condition::{
        CompiledSurfaceRule, MaterialConditionRegistry, MaterialConditions, MaterialDensitySources,
        MaterialRule, MaterialRuleError, MaterialRuleLibrary, MaterialSurfaceQueries,
    },
    material_extensions::{ExtensionBlocks, ExtensionNoises, MaterialExtensions},
    material_stage::{SurfaceInputs, TopMaterialInputs, build_surface_shared},
    material_surface::{ClayBandBlocks, MaterialSurface},
    overrides::{self, WorldgenOverrides},
    router::{NoiseRouter, RouterField, RouterSamplers},
    settings::NoiseSettings,
    spawn_finder::{SpawnTargetPoint, find_spawn_position},
    tags::TagDefinitions,
    terrain::{PreLightChunkStage, TerrainAquifer, TerrainBlockStates, TerrainChunk, fill_noise},
};
use crate::{
    heightmap::{HeightmapState, HeightmapType},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{
    biome::{BiomeZoom, obfuscate_seed},
    density::{DensitySampler, LatticeSamples, SamplerContext},
    noise::{DensityVolume, Noise},
    position::{BlockPos, ChunkPos},
    random::{LegacyRandom, PositionalRandom, RandomSource, XoroshiroRandom},
};
use serde_json::Value;
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet, HashMap},
    fs, io,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
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

/// The seed-independent resources, at the path the internals list names.
pub mod data;
pub use data::GeneratorData;
#[cfg(test)]
use data::{RING_CACHE_LIMIT, build_tags};
mod noise_settings;
use noise_settings::DecodedNoiseSettings;
/// Convert a data-generator `biome_parameters` report into the quantized
/// preset table format, using vanilla's float `quantizeCoord`.
pub(crate) fn quantize_parameter_report(json: &str) -> Result<String, GeneratorError> {
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
/// `load` on a thread of its own ("Worldgen loader").
fn loading<T: Send + 'static>(
    load: impl FnOnce() -> T + Send + 'static,
) -> io::Result<std::thread::JoinHandle<T>> {
    std::thread::Builder::new()
        .name("Worldgen loader".into())
        .spawn(load)
}
/// A loading thread's result; its panic goes on in this thread.
fn joined<T>(thread: std::thread::JoinHandle<T>) -> T {
    thread
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
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
    pub block_entities: Vec<([i32; 3], horizon_core::nbt::Compound)>,
    /// Structure starts referencing this chunk (STRUCTURE_REFERENCES).
    pub(super) structure_starts: Vec<Arc<super::structure::start::StructureStart>>,
    /// The starts made in this chunk (STRUCTURE_STARTS), also those none of
    /// whose pieces reach it (a giant ruined portal next door).
    own_starts: Starts,
    /// Entities placed by structures (`ProtoChunk.addEntity`), as NBT with
    /// absolute `Pos`.
    pub entities: Vec<horizon_core::nbt::Compound>,
    /// Unmodelled root keys retained from a saved proto chunk.
    pub saved_root: horizon_core::nbt::Compound,
    full: bool,
}
impl ProtoChunk {
    /// Its own heap bytes by part (`horizon-ctl memory`): packed blocks,
    /// section palettes and lists, heightmaps, other terrain data, biomes,
    /// and the box with its structure lists (the starts it shares not counted).
    pub fn heap_bytes(&self) -> [usize; 6] {
        let [storage, lists, heightmaps, other] = self.terrain.heap_bytes();
        [
            storage,
            lists,
            heightmaps,
            other,
            self.biomes.heap_bytes(),
            std::mem::size_of::<Self>()
                + self.block_entities.capacity()
                    * std::mem::size_of::<([i32; 3], horizon_core::nbt::Compound)>()
                + self.structure_starts.capacity() * std::mem::size_of::<usize>()
                + self.entities.capacity() * std::mem::size_of::<horizon_core::nbt::Compound>(),
        ]
    }
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
    /// The structure starts this chunk holds itself (`structures.starts`).
    pub fn own_starts(&self) -> Vec<Arc<super::structure::start::StructureStart>> {
        self.own_starts.to_vec()
    }
    /// Also restores a saved proto chunk that was decorated before it was saved.
    pub fn set_decorated(&mut self) {
        self.decorated = true;
    }
    /// Whether this stands for a full chunk in a decoration region ([`Self::set_full`]).
    pub fn is_full(&self) -> bool {
        self.full
    }
    /// Marks this as a full chunk standing in a decoration region (`ImposterProtoChunk`
    /// with `allowWrites` false): ordinary feature writes leave it as it is.
    pub fn set_full(&mut self) {
        self.full = true;
    }
    /// Highest non-air block + 1 in the column (WORLD_SURFACE).
    pub fn surface_height(&self, x: i32, z: i32) -> i32 {
        self.terrain
            .heightmap(HeightmapType::WorldSurface)
            .and_then(|map| map.first_available(x & 15, z & 15).ok())
            .unwrap_or(self.terrain.min_y())
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
    /// `NoiseGeneratorSettings.disableMobGeneration`: whether the generator
    /// spawns the creature packs it would otherwise add to every chunk it
    /// builds (`spawnOriginalMobs`). The End's preset is the only shipped one
    /// that turns it on.
    disable_mob_generation: bool,
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
    /// The terrain density's cell lattice each biome chunk's climate volume
    /// sampled, for the noise fill of this chunk and its -X/-Z neighbours.
    lattice_cache: LatticeCache,
    biome_work: ChunkWork,
    carver_cache: Mutex<HashMap<(i32, i32), BiomeId>>,
    pub(super) decoration: Option<Arc<super::generator_decoration::Decoration>>,
    pub(super) structures: Option<super::generator_structures::StructureState>,
    /// Definitions for explicit placement when automatic structure starts are off.
    manual_structures: std::sync::OnceLock<Option<super::generator_structures::StructureState>>,
    /// The noise settings' `spawn_target` points.
    spawn_target: Vec<SpawnTargetPoint>,
    /// The chunks an older version generated, which new chunks blend with.
    old_world: std::sync::OnceLock<Arc<dyn super::blending::OldWorld>>,
    /// The noise settings' `debug_functions`: label and sampler.
    debug_density: Vec<(String, Arc<dyn DensitySampler>)>,
    spawn_position: std::sync::OnceLock<[i32; 2]>,
    /// A superflat world: its layers stand where the noise terrain would.
    flat: Option<FlatLayers>,
    /// A mod's chunk generator: its terrain stands where the noise would.
    mod_terrain: Option<mod_terrain::ModTerrain>,
}
/// Carver origins whose biome is kept.
const CARVER_CACHE_LIMIT: usize = 65536;
/// What a generator's shared caches hold ([`OverworldGenerator::cache_census`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct CacheCensus {
    /// Chunks in the biome cache, and their bytes.
    pub biome_chunks: usize,
    pub biome_bytes: usize,
    /// Carver origins whose biome is cached.
    pub carver_entries: usize,
    /// Chunks in the structure start cache, and how many of them hold starts.
    pub start_chunks: usize,
    pub start_lists: usize,
}
/// Coordinates only equal missing neighbour keys. Weak entries retain no
/// computed values across eviction or explicit invalidation. Only one work
/// lock may be held at a time; finish shared prerequisites before acquiring it.
/// Sharded by key: every worker asks for the locks of the same neighbourhoods.
pub(super) struct ChunkWork(Box<[Mutex<HashMap<(i32, i32), std::sync::Weak<Mutex<()>>>>]>);
const WORK_SHARDS: usize = 64;
/// The shard of `key` among `shards` (a power of two).
pub(super) fn shard_of(key: (i32, i32), shards: usize) -> usize {
    let hash = (key.0 as u32).wrapping_mul(0x9E37_79B9) ^ (key.1 as u32).wrapping_mul(0x85EB_CA6B);
    (hash >> (32 - shards.trailing_zeros())) as usize
}
impl ChunkWork {
    pub(super) fn new() -> Self {
        Self((0..WORK_SHARDS).map(|_| Mutex::default()).collect())
    }
    pub(super) fn at(&self, key: (i32, i32)) -> Arc<Mutex<()>> {
        let mut work = self.0[shard_of(key, WORK_SHARDS)].lock().unwrap();
        if let Some(lock) = work.get(&key).and_then(std::sync::Weak::upgrade) {
            return lock;
        }
        if work.len() >= 4096 / WORK_SHARDS {
            // Keep every active builder/waiter on its original lock, including
            // when the result cache was cleared while it was in flight.
            work.retain(|_, lock| lock.strong_count() > 0);
        }
        let lock = Arc::new(Mutex::new(()));
        work.insert(key, Arc::downgrade(&lock));
        lock
    }
}

type BiomeCache = Mutex<BiomeChunks>;
type LatticeCache = Mutex<HashMap<(i32, i32), Arc<Vec<LatticeSamples>>>>;
/// ~33 KB a chunk in To The Max; misses only resample.
const LATTICE_CACHE_LIMIT: usize = 1024;

/// `NoiseGeneratorSettings`' `default_block` and `default_fluid`: a block state
/// in 26.3's form, the id alone or `{"id", "properties"}`.
fn state(
    codec: &crate::registry::BlockStateJsonCodec<'_>,
    raw: &Value,
) -> Result<BlockStateId, GeneratorError> {
    codec
        .decode(&raw.to_string(), false)
        .map_err(|e| resource(format!("block state {raw}: {e}")))
}

impl<R: RandomSource + 'static> Pipeline<R>
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
        regions: Option<&RegionLayout>,
    ) -> Result<Self, GeneratorError> {
        let registry = data.registry;
        let root = random.fork_positional();
        let DecodedNoiseSettings {
            settings,
            router,
            debug_density,
            aquifer,
            rule,
            spawn_target,
        } = data.decode_noise_settings(settings_json, seed, level)?;
        let preliminary = router.get(RouterField::ChunkSurfaceLevel).clone();
        let mut source_json = generator["biome_source"].to_string();
        let definition = data.source_catalog.decode(&source_json)?;
        let source = definition
            .resolve(&data.presets, seed)
            .map_err(|e| resource(format!("biome source: {e}")))?;
        // Only the multi-noise presets (Overworld, Nether) take regions, not inline point lists.
        let source = if matches!(definition, BiomeSourceDefinition::Preset { .. }) {
            source
                .with_regions(regions, &root)
                .map_err(|e| resource(format!("biome regions: {e}")))?
        } else {
            source
        };
        // Regions add biomes, so their decoration is cached apart from the bare preset's.
        if let ResolvedBiomeSource::Regions(layered) = &source {
            for biome in layered.biomes() {
                source_json.push(' ');
                source_json.push_str(biome);
            }
        }
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
            biome_cache: Mutex::new(BiomeChunks::default()),
            lattice_cache: Mutex::new(HashMap::new()),
            biome_work: ChunkWork::new(),
            carver_cache: Mutex::new(HashMap::new()),
            decoration: None,
            structures: None,
            manual_structures: std::sync::OnceLock::new(),
            spawn_target,
            old_world: std::sync::OnceLock::new(),
            debug_density,
            spawn_position: std::sync::OnceLock::new(),
            flat: None,
            mod_terrain: None,
        };
        pipeline.decoration = pipeline
            .data
            .decoration_for(&source_json, &pipeline.source)?;
        if with_structures {
            let possible = pipeline
                .source
                .possible_biomes()
                .into_iter()
                .filter_map(|name| pipeline.data.biome_id(name))
                .collect();
            let rings = pipeline.data.rings_for(seed, generator);
            pipeline.structures =
                super::generator_structures::StructureState::new(&pipeline.data, &possible, rings)?;
            if let Some(state) = pipeline.structures.as_mut() {
                state.fixed_biome = generator["biome_source"]["type"] == "minecraft:fixed";
            }
            // ChunkGenerator.createState reads getOrigin: a noise generator
            // answers the chunk of its NoiseSpawnFinder position, others
            // chunk 0 0. Only a dimension-origin structure set reads it.
            if pipeline
                .structures
                .as_ref()
                .is_some_and(|state| state.needs_dimension_origin())
                && let Ok([x, z]) = pipeline.spawn_column()
            {
                let origin = ChunkPos::new(x >> 4, z >> 4);
                if let Some(state) = pipeline.structures.as_mut() {
                    state.set_dimension_origin(origin);
                }
            }
        }
        Ok(pipeline)
    }
    /// `NoiseBasedChunkGenerator.getOrigin`'s spawn column: the
    /// NoiseSpawnFinder position, or 0 0 when the noise settings have no
    /// spawn target (`ChunkGenerator.getOrigin`). Cached.
    fn spawn_column(&self) -> Result<[i32; 2], GeneratorError> {
        if let Some(position) = self.spawn_position.get() {
            return Ok(*position);
        }
        // ChunkGenerator.getOrigin is 0 0 without a spawn target.
        let position = if self.spawn_target.is_empty() {
            [0, 0]
        } else {
            find_spawn_position(&self.spawn_target, &mut SamplerContext::cached())
                .map_err(|e| GeneratorError::Stage("spawn search", e.to_string()))?
        };
        Ok(*self.spawn_position.get_or_init(|| position))
    }
    /// Manual placement reads the registry without enabling the automatic
    /// structure state, its placement sets, starts or locate searches.
    fn placement_structures(&self) -> Option<&super::generator_structures::StructureState> {
        self.structures.as_ref().or_else(|| {
            self.manual_structures
                .get_or_init(|| {
                    super::generator_structures::StructureState::new(
                        &self.data,
                        &BTreeSet::new(),
                        Arc::new(std::sync::OnceLock::new()),
                    )
                    .ok()
                    .flatten()
                })
                .as_ref()
        })
    }
    fn generation(&self) -> WorldGenerationContext {
        // `new WorldGenerationContext(generator, level)`: the larger minimum Y and the smaller
        // height of the generator and the level (`FlatLevelSource`: 0 and 384).
        if self.flat.is_some() {
            return WorldGenerationContext::new(
                self.settings.level_min_y.max(0),
                self.settings.level_height.min(384),
                self.settings.sea_level,
            );
        }
        WorldGenerationContext::new(
            self.settings.noise_min_y,
            self.settings.noise_height,
            self.settings.sea_level,
        )
    }
    /// `Blender.of(region)` for the chunk at `pos`.
    fn blender_at(&self, pos: ChunkPos) -> Arc<super::blending::Blender> {
        fn empty_blender() -> Arc<super::blending::Blender> {
            static EMPTY: OnceLock<Arc<super::blending::Blender>> = OnceLock::new();
            EMPTY
                .get_or_init(|| Arc::new(super::blending::Blender::empty()))
                .clone()
        }
        // `SharedConstants.DEBUG_DISABLE_BLENDING`.
        static DISABLED: OnceLock<bool> = OnceLock::new();
        if *DISABLED.get_or_init(|| std::env::var_os("HORIZON_DISABLE_BLENDING").is_some()) {
            return empty_blender();
        }
        let Some(world) = self.old_world.get() else {
            return empty_blender();
        };
        let blender = super::blending::Blender::of(world.as_ref(), pos);
        if blender.is_empty() {
            empty_blender()
        } else {
            Arc::new(blender)
        }
    }
    fn biome_chunk(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Result<Arc<BiomeChunk<BiomeId>>, GeneratorError> {
        self.biome_chunk_or_busy(worker, pos, true)
            .map(|chunk| chunk.expect("waited for the builder"))
    }
    /// The chunk's biomes, built here if no other thread is building them.
    /// With `wait` false, None when another thread is building them now.
    fn biome_chunk_or_busy(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
        wait: bool,
    ) -> Result<Option<Arc<BiomeChunk<BiomeId>>>, GeneratorError> {
        if let Some(chunk) = self.biome_cache.lock().unwrap().get(&(pos.x, pos.z)) {
            return Ok(Some(chunk.clone()));
        }
        // An old chunk keeps the biomes it was saved with.
        if let Some(old) = self
            .old_world
            .get()
            .and_then(|world| world.old_biomes(pos.x, pos.z))
        {
            return Ok(Some(old));
        }
        let work = self.biome_work.at((pos.x, pos.z));
        let _work = if wait {
            work.lock().unwrap()
        } else {
            match work.try_lock() {
                Ok(guard) => guard,
                Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
                Err(std::sync::TryLockError::Poisoned(error)) => panic!("{error}"),
            }
        };
        if let Some(chunk) = self.biome_cache.lock().unwrap().get(&(pos.x, pos.z)) {
            return Ok(Some(chunk.clone()));
        }
        let (min, height) = (self.settings.level_min_y, self.settings.level_height);
        let mut chunk = BiomeChunk::new(pos, min, height, BiomeId(0)).map_err(stage("biomes"))?;
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        let context = &mut worker.biome_context;
        context.capture_lattice();
        let volume = resolver.prepare_volume(
            context,
            [
                pos.x.wrapping_shl(4) >> 2,
                min >> 2,
                pos.z.wrapping_shl(4) >> 2,
            ],
            [4, height >> 2, 4],
        );
        let lattice = context.take_lattice_capture();
        let volume = volume.map_err(stage("biomes"))?;
        if !lattice.is_empty() {
            let mut cache = self.lattice_cache.lock().unwrap();
            if cache.len() >= LATTICE_CACHE_LIMIT {
                cache.clear();
            }
            cache.insert((pos.x, pos.z), Arc::new(lattice));
        }
        let ids = &self.data.biome_ids;
        let blender = self.blender_at(pos);
        // Names point into the source's parameter list: neighbouring quarts
        // mostly resolve to the same entry, so skip the map lookup for it.
        let mut last: Option<(*const str, BiomeId)> = None;
        chunk
            .fill(min, min + height - 1, |quart| -> Result<BiomeId, String> {
                // `Blender.getBiomeResolver`: an old chunk's biome near it.
                if !blender.is_empty()
                    && let Some(old) = blender.blend_biome(quart[0], quart[1], quart[2])
                {
                    return Ok(old);
                }
                let name = resolver
                    .sample_prepared(context, volume.as_ref(), quart)
                    .map_err(|e| e.to_string())?;
                if let Some((pointer, id)) = last
                    && std::ptr::eq(pointer, name)
                {
                    return Ok(id);
                }
                let id = ids.get(name).copied().ok_or_else(|| name.to_owned())?;
                last = Some((name as *const str, id));
                Ok(id)
            })
            .map_err(stage("biomes"))?;
        let chunk = Arc::new(chunk);
        let mut cache = self.biome_cache.lock().unwrap();
        cache.insert((pos.x, pos.z), chunk.clone());
        Ok(Some(chunk))
    }
    fn carver_biome(&self, worker: &mut WorkerState, origin: ChunkPos) -> Result<BiomeId, String> {
        let key = (origin.x, origin.z);
        if let Some(id) = worker.carver_biomes.get(&key) {
            return Ok(*id);
        }
        if worker.carver_biomes.len() >= 4096 {
            worker.carver_biomes.clear();
        }
        if let Some(id) = self.carver_cache.lock().unwrap().get(&key) {
            worker.carver_biomes.insert(key, *id);
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
        if cache.len() >= CARVER_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, id);
        worker.carver_biomes.insert(key, id);
        Ok(id)
    }
    fn generate(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Result<ProtoChunk, GeneratorError> {
        let mut context = std::mem::take(&mut worker.terrain_context);
        let result = self.generate_with_context(worker, pos, &mut context);
        // Release chunk-specific bindings and caches even after a sampling error.
        context.reset();
        worker.terrain_context = context;
        result
    }
    /// With the native End source there are no badlands/iceberg extensions.
    /// Disabled aquifers with air as their default fluid can only add lava or
    /// air to the default solid, so this constant rule would rewrite each solid
    /// with the state it already has. Check their imported material flags too.
    /// Noise filling already built the WG maps; these writes add no fluid marks.
    fn identity_end_surface(&self, pos: ChunkPos) -> bool {
        // A mod's terrain is not the noise's default block.
        if self.mod_terrain.is_some()
            || !matches!(self.source, ResolvedBiomeSource::End(_))
            // At wrapped block coordinates the ordinary biome-region lookup
            // can fail; retain that lookup and its error outside this range.
            || ![pos.x, pos.z].into_iter().all(|c| {
                c.checked_mul(16).is_some_and(|b| {
                    b.checked_sub(16).is_some() && b.checked_add(31).is_some()
                })
            })
            || self.aquifer.is_some()
            || Some(self.settings.default_block)
                != self.data.registry.default_state("minecraft:end_stone")
            || Some(self.settings.default_fluid)
                != self.data.registry.default_state("minecraft:air")
            || !self.data.states.get(self.settings.default_fluid).air
            || !self
                .data
                .registry
                .default_state("minecraft:lava")
                .is_some_and(|lava| self.data.states.get(lava).fluid)
        {
            return false;
        }
        let mut rule = &self.rule;
        // Restrict the shortcut to shallow block references; other rules still
        // run their normal compilation, queries and error handling.
        for _ in 0..32 {
            match rule {
                MaterialRule::Reference(next) => rule = next,
                MaterialRule::Block(state) => return *state == self.settings.default_block,
                _ => return false,
            }
        }
        false
    }
    fn generate_with_context(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
        context: &mut SamplerContext,
    ) -> Result<ProtoChunk, GeneratorError> {
        let data = &self.data;
        let (min, height, sea) = (
            self.settings.noise_min_y,
            self.settings.noise_height,
            self.settings.sea_level,
        );
        let mut clock = std::time::Instant::now();
        let mut neighbors = Vec::with_capacity(9);
        // The biome ids present, as a bitset: ascending like a sorted set. The
        // registry's ordinary ids fit in the stack array; retain a spill only
        // for unusually large modded registries.
        let mut seen = [0u64; 128];
        let mut seen_spill = Vec::new();
        // Build the neighbours no other thread is building first, then wait
        // for the rest (each is built once, by whichever thread gets it).
        let around =
            |i: i32| ChunkPos::new(pos.x.wrapping_add(i % 3 - 1), pos.z.wrapping_add(i / 3 - 1));
        let mut ready: [Option<Arc<BiomeChunk<BiomeId>>>; 9] = Default::default();
        for (i, slot) in ready.iter_mut().enumerate() {
            *slot = self.biome_chunk_or_busy(worker, around(i as i32), false)?;
        }
        for (i, slot) in ready.iter_mut().enumerate() {
            let chunk = match slot.take() {
                Some(chunk) => chunk,
                None => self.biome_chunk(worker, around(i as i32))?,
            };
            {
                let mut section = 0;
                let mut last = None;
                while let Some(biomes) = chunk.section(section) {
                    for &id in biomes {
                        if last == Some(id) {
                            continue;
                        }
                        last = Some(id);
                        let word = id.0 as usize / 64;
                        if let Some(bits) = seen.get_mut(word) {
                            *bits |= 1 << (id.0 % 64);
                        } else {
                            let spill_word = word - seen.len();
                            if seen_spill.len() <= spill_word {
                                seen_spill.resize(spill_word + 1, 0);
                            }
                            seen_spill[spill_word] |= 1 << (id.0 % 64);
                        }
                    }
                    section += 1;
                }
                neighbors.push((*chunk).clone());
            }
        }
        let possible =
            seen.iter()
                .chain(seen_spill.iter())
                .enumerate()
                .flat_map(|(word, &bits)| {
                    let mut bits = bits;
                    std::iter::from_fn(move || {
                        (bits != 0).then(|| {
                            let bit = bits.trailing_zeros();
                            bits &= bits - 1;
                            BiomeId(word as u32 * 64 + bit)
                        })
                    })
                });
        let center = neighbors[4].clone();
        let biomes =
            BiomeRegion::new(neighbors, obfuscate_seed(self.seed)).map_err(stage("biomes"))?;
        let possible = possible.map(|id| data.biome_name(id).to_owned()).collect();
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
        let (references, own_starts) = match &self.structures {
            Some(structures) => {
                let mut queries = StructureQueries {
                    pipeline: self,
                    worker,
                    valid: BTreeSet::new(),
                };
                (
                    structures.references(&mut queries, pos),
                    structures.starts_at(&mut queries, pos),
                )
            }
            None => (Vec::new(), Starts::default()),
        };
        // `FlatLevelSource.buildTerrain`: the layers, and nothing else (no noise, surface or carvers).
        if let Some(flat) = &self.flat {
            for (i, &state) in flat.states.iter().enumerate() {
                let y = flat.min_y + i as i32;
                if y >= self.settings.level_min_y + self.settings.level_height {
                    break;
                }
                if flat.null[i] || data.states.get(state).is_air() {
                    continue;
                }
                for z in 0..16 {
                    for x in 0..16 {
                        chunk
                            .set_block_raw(
                                pos.x.wrapping_shl(4) + x,
                                y,
                                pos.z.wrapping_shl(4) + z,
                                state,
                            )
                            .map_err(stage("flat"))?;
                    }
                }
            }
            chunk
                .prime_heightmaps(&[
                    HeightmapType::WorldSurfaceWg,
                    HeightmapType::OceanFloorWg,
                    HeightmapType::MotionBlocking,
                    HeightmapType::MotionBlockingNoLeaves,
                    HeightmapType::OceanFloor,
                    HeightmapType::WorldSurface,
                ])
                .map_err(stage("heightmaps"))?;
            chunk.set_generation_stage(PreLightChunkStage::Terrain);
            return Ok(ProtoChunk {
                terrain: chunk,
                biomes: center,
                data: data.clone(),
                decorated: false,
                block_entities: Vec::new(),
                structure_starts: references,
                own_starts,
                entities: Vec::new(),
                saved_root: Default::default(),
                full: false,
            });
        }
        let volume = DensityVolume::new(
            [16, height, 16],
            [pos.x.wrapping_shl(4), min, pos.z.wrapping_shl(4)],
            [1; 3],
        )
        .map_err(stage("noise"))?;
        let blender = self.blender_at(pos);
        let beard = self
            .structures
            .as_ref()
            .and_then(|s| s.beardifier(&references, pos));
        if beard.is_some() || !blender.is_empty() {
            let mut fields = horizon_core::density::WorldFields::default();
            if let Some(beard) = beard {
                fields = fields.with_field(
                    horizon_core::density::WorldField::Beardifier,
                    Arc::new(beard),
                );
            }
            if !blender.is_empty() {
                let buffer = Arc::new(blender.output_buffer(volume));
                for (field, offset) in [
                    (horizon_core::density::WorldField::BlendAlpha, false),
                    (horizon_core::density::WorldField::BlendOffset, true),
                ] {
                    fields = fields.with_field(
                        field,
                        Arc::new(super::blending::BlendFieldSampler {
                            buffer: buffer.clone(),
                            offset,
                        }),
                    );
                }
                fields = fields.with_blender(blender.clone());
            }
            *context = std::mem::take(context).with_world_fields(fields);
        }
        worker.timings.structures += lap(&mut clock);
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
                    context,
                )
                .map_err(stage("aquifer"))?,
            ),
            None => Box::new(DisabledAquifer::new(
                self.picker,
                data.registry.default_state("minecraft:air").unwrap(),
            )),
        };
        worker.timings.noise += lap(&mut clock);
        // The density cells this fill needs are mostly in the climate volumes
        // of this chunk and its +X/+Z neighbours, sampled for their biomes.
        let lattice = {
            let cache = self.lattice_cache.lock().unwrap();
            [(0, 0), (1, 0), (0, 1), (1, 1)]
                .into_iter()
                .filter_map(|(dx, dz)| cache.get(&(pos.x.wrapping_add(dx), pos.z.wrapping_add(dz))))
                .flat_map(|samples| samples.iter().cloned())
                .collect()
        };
        context.set_lattice(lattice);
        let filled = match &self.mod_terrain {
            Some(terrain) => terrain.fill(&mut chunk, data.registry, self.seed, [min, height]),
            None => fill_noise(
                &mut chunk,
                volume,
                self.router.get(RouterField::FinalDensity).as_ref(),
                context,
                aquifer.as_mut(),
                self.settings.default_block,
            ),
        };
        context.clear_lattice();
        filled.map_err(stage("noise"))?;
        let generation = self.generation();
        worker.timings.noise += lap(&mut clock);
        // The surface rule, compiled by this worker's first chunk.
        let mut shared = worker
            .surface_rule
            .take()
            .and_then(|cache| cache.downcast::<CompiledSurfaceRule<R>>().ok())
            .map(|cache| *cache);
        let stages = self.mod_terrain.as_ref().map(|terrain| terrain.stages);
        if stages.is_none_or(|stages| stages.surface) && !self.identity_end_surface(pos) {
            build_surface_shared(
                &mut chunk,
                &self.rule,
                &mut shared,
                SurfaceInputs::<R, _> {
                    generation,
                    noise_volume: volume,
                    root: self.root.clone(),
                    surface: SurfaceQueries {
                        pipeline: self,
                        temperatures: &mut worker.temperatures,
                        biome: Biomes::Region(&biomes),
                        uniform_biomes: Some(UniformSurfaceBiomes::new(
                            pos,
                            self.settings.level_min_y,
                            self.settings.level_height,
                        )),
                        possible: Some(possible),
                    },
                    preliminary_surface: self.preliminary.clone(),
                    density_context: context,
                    write_min_y: min,
                    write_max_y: min + height - 1,
                },
                &self.extensions,
            )
            .map_err(stage("surface"))?;
        }
        worker.timings.surface += lap(&mut clock);
        let carvers = &data.biome_carvers;
        let carves = stages.is_none_or(|stages| stages.carvers);
        let mask = generate_carving_mask(self.seed, pos, generation, false, |origin| {
            if !carves {
                return Ok(&[][..]);
            }
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
            apply_carving_mask_shared(
                &mut chunk,
                &mask,
                &self.rule,
                &mut shared,
                TopMaterialInputs::<R, _> {
                    generation,
                    root: self.root.clone(),
                    surface: SurfaceQueries {
                        pipeline: self,
                        temperatures: &mut worker.temperatures,
                        biome: Biomes::Resolver(&mut resolver, &mut biome_context),
                        uniform_biomes: None,
                        possible: None,
                    },
                    preliminary_surface: self.preliminary.clone(),
                    density_context: context,
                },
                aquifer.as_mut(),
                &data.carving_blocks,
                |[x, y, z]| blender.carving_filter(x, y, z).unwrap_or(true),
            )
            .map_err(stage("carvers"))?;
        }
        worker.surface_rule = shared.map(|rule| Box::new(rule) as Box<dyn Any + Send>);
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
            own_starts,
            entities: Vec::new(),
            saved_root: Default::default(),
            full: false,
        })
    }
}

impl<R: RandomSource> Pipeline<R>
where
    R::Positional: Clone + Send + Sync,
{
    /// WorldGenRegion's level random, positioned at the center chunk.
    fn region_random_at(&self, center: ChunkPos) -> R {
        self.root
            .from_hash_of("minecraft:worldgen_region_random")
            .fork_positional()
            .at(BlockPos::new(
                center.x.wrapping_shl(4),
                0,
                center.z.wrapping_shl(4),
            ))
    }
    /// The Y range `getBaseHeight` and `getBaseColumn` answer for: the noise's, or a superflat
    /// world's whole level (its layers are not bound by any noise settings).
    fn column_range(&self) -> (i32, i32) {
        if self.flat.is_some() {
            (self.settings.level_min_y, self.settings.level_height)
        } else {
            (self.settings.noise_min_y, self.settings.noise_height)
        }
    }
    /// ChunkGenerator.getBaseHeight: the first free Y above the noise column
    /// (with aquifers, without terrain adaptation) for a heightmap type.
    fn base_height(&self, x: i32, z: i32, kind: HeightmapType) -> i32 {
        let (min, _) = self.column_range();
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
        self.base_columns_by(x, z, 1, &|_| kind)
            .pop()
            .unwrap_or_default()
    }
    /// One noise-column pass, as opacities for each heightmap kind.
    fn base_columns_by(
        &self,
        x: i32,
        z: i32,
        count: usize,
        kind: &dyn Fn(usize) -> HeightmapType,
    ) -> Vec<Vec<bool>> {
        let (min, height) = self.column_range();
        let mut columns = vec![vec![false; height as usize]; count];
        self.noise_column(x, z, &mut |y, state| {
            let state = self.data.states.get(state);
            for (index, column) in columns.iter_mut().enumerate() {
                column[(y - min) as usize] = kind(index).is_opaque(state);
            }
            false
        });
        columns
    }
    /// ChunkGenerator.getBaseColumn's blocks, by what structures test
    /// (BlockState.isSolid is blocksMotion for the noise's blocks, which
    /// are never cobwebs or bamboo saplings).
    fn noise_blocks_into(
        &self,
        x: i32,
        z: i32,
        column: &mut dyn super::structure::start::NoiseColumnTarget,
    ) {
        use super::structure::start::NoiseBlock;
        let (min, height) = self.column_range();
        column.resize(min, height as usize);
        let registry = self.data.registry;
        self.noise_column(x, z, &mut |y, state| {
            let props = self.data.states.get(state);
            let block = if props.is_air() {
                NoiseBlock::Air
            } else {
                match registry.state(state).map(|s| s.name.as_str()) {
                    Some("minecraft:water") => NoiseBlock::Water,
                    Some("minecraft:lava") => NoiseBlock::Lava,
                    _ if props.blocks_motion_in_heightmap() => NoiseBlock::Solid,
                    _ => NoiseBlock::Other,
                }
            };
            column.set((y - min) as usize, block);
            false
        });
    }
    /// iterateNoiseColumn from the top down until `visit` returns true.
    fn noise_column(&self, x: i32, z: i32, visit: &mut dyn FnMut(i32, BlockStateId) -> bool) {
        // `FlatLevelSource.getBaseHeight` and `getBaseColumn`: the layers, and air above them.
        if let Some(flat) = &self.flat {
            let air = self.data.registry.default_state("minecraft:air");
            let top = self.settings.level_min_y + self.settings.level_height;
            for y in (self.settings.level_min_y..top).rev() {
                let i = (y - flat.min_y) as usize;
                let state = match flat.states.get(i) {
                    Some(&state) if !flat.null[i] => state,
                    _ => match air {
                        Some(air) => air,
                        None => return,
                    },
                };
                if visit(y, state) {
                    return;
                }
            }
            return;
        }
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
    /// JigsawPlacement.couldStructureExistInColumn: one resolver for the
    /// column (createResolverForChunk, a bulk climate volume of 1 x n x 1
    /// quarts, as the biome stage samples a chunk), then each quart from the
    /// bottom. One volume costs about what a few single climate samples do;
    /// a column without a valid biome took one sample per quart (~500 in To
    /// The Max).
    fn column_has_valid_biome(&mut self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool {
        let (qx, qz, qmin, qmax) = (x >> 2, z >> 2, min_y >> 2, max_y >> 2);
        if qmin > qmax {
            return false;
        }
        let pipeline = self.pipeline;
        let mut resolver = pipeline.source.resolver(
            &pipeline.climate,
            pipeline.router.get(RouterField::Erosion).as_ref(),
        );
        let context = &mut self.worker.biome_context;
        let volume = resolver
            .prepare_volume(context, [qx, qmin, qz], [1, qmax - qmin + 1, 1])
            .ok()
            .flatten();
        (qmin..=qmax).any(|qy| {
            let id = resolver
                .sample_prepared(context, volume.as_ref(), [qx, qy, qz])
                .ok()
                .and_then(|name| pipeline.data.biome_id(name))
                .unwrap_or(BiomeId(0));
            self.valid.contains(&id)
        })
    }
}
impl<R: RandomSource + 'static> super::structure::start::StartContext for StructureQueries<'_, R>
where
    R::Positional: Clone + Send + Sync,
{
    fn is_old_chunk(&self, chunk: ChunkPos) -> bool {
        self.pipeline
            .old_world
            .get()
            .is_some_and(|world| world.is_old_chunk(chunk.x, chunk.z))
    }
    fn saved_starts(&self, chunk: ChunkPos) -> Vec<super::blending::SavedStart> {
        self.pipeline
            .old_world
            .get()
            .map(|world| world.saved_starts(chunk.x, chunk.z))
            .unwrap_or_default()
    }
    fn seed(&self) -> i64 {
        self.pipeline.seed
    }
    fn sea_level(&self) -> i32 {
        self.pipeline.settings.sea_level
    }
    fn generator_min_y(&self) -> i32 {
        if self.pipeline.flat.is_some() {
            0
        } else {
            self.pipeline.settings.noise_min_y
        }
    }
    fn base_column(&mut self, x: i32, z: i32, kind: HeightmapType) -> Vec<bool> {
        self.pipeline.base_column(x, z, kind)
    }
    fn base_columns_by(
        &mut self,
        x: i32,
        z: i32,
        count: usize,
        kind: &dyn Fn(usize) -> HeightmapType,
    ) -> Vec<Vec<bool>> {
        self.pipeline.base_columns_by(x, z, count, kind)
    }
    fn noise_column_into(
        &mut self,
        x: i32,
        z: i32,
        column: &mut dyn super::structure::start::NoiseColumnTarget,
    ) {
        self.pipeline.noise_blocks_into(x, z, column)
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
        if let Some(&id) = self.worker.structure_biomes.get(&quart) {
            return id;
        }
        let id = self.noise_biomes(&[quart])[0];
        if self.worker.structure_biomes.len() >= 1 << 16 {
            self.worker.structure_biomes.clear();
        }
        self.worker.structure_biomes.insert(quart, id);
        id
    }
    fn noise_biomes(&mut self, quarts: &[[i32; 3]]) -> Vec<BiomeId> {
        let mut resolver = self.pipeline.source.resolver(
            &self.pipeline.climate,
            self.pipeline.router.get(RouterField::Erosion).as_ref(),
        );
        quarts
            .iter()
            .map(|&quart| {
                resolver
                    .sample(&mut self.worker.carver_context, quart)
                    .ok()
                    .and_then(|name| self.pipeline.data.biome_id(name))
                    .unwrap_or(BiomeId(0))
            })
            .collect()
    }
    /// `None` is `/place structure`'s `biome -> true`: every biome the bundle
    /// knows counts as valid.
    fn set_valid_biomes(&mut self, biomes: Option<&BTreeSet<BiomeId>>) {
        self.valid = match biomes {
            Some(biomes) => biomes.clone(),
            None => (0..self.pipeline.data.biome_names().len() as u32)
                .map(BiomeId)
                .collect(),
        };
    }
}

enum Biomes<'a, 'b> {
    Region(&'a BiomeRegion<BiomeId>),
    Resolver(
        &'a mut super::biome_source::BiomeSourceResolver<'b>,
        &'a mut SamplerContext,
    ),
    /// A resolver whose answers are kept per quart: far sampling paints eight
    /// depths of every column, and neighbouring columns share quarts, so most
    /// queries repeat one (each is a full climate sample and parameter search).
    Memo(
        &'a mut super::biome_source::BiomeSourceResolver<'b>,
        &'a mut SamplerContext,
        &'a mut BiomeMemo,
    ),
}
/// Resolved biomes by quart (`Biomes::Memo`).
struct BiomeMemo {
    biomes: HashMap<[i32; 3], BiomeId>,
    /// Quarts per side of the columns sharing one biome, the first one asked
    /// (its surface's); 0 for one per quart. Far cells wider than a quart
    /// don't show a biome change a few blocks down, and climate changes over
    /// hundreds of blocks: a far tile shares one per 4x4 of its columns, in
    /// cells up to 64 blocks wide.
    /// (Painting asks through the biome zoom, which strays into neighbouring
    /// quarts: per quart, those strays were most of the climate samples.)
    cell: i32,
}
/// The biome at `quart`, resolved once per quart (`Biomes::Memo`).
fn memo_biome(
    data: &GeneratorData,
    resolver: &mut super::biome_source::BiomeSourceResolver<'_>,
    context: &mut SamplerContext,
    memo: &mut BiomeMemo,
    quart: [i32; 3],
) -> Result<BiomeId, MaterialRuleError> {
    let key = if memo.cell > 0 {
        [
            quart[0].div_euclid(memo.cell),
            0,
            quart[2].div_euclid(memo.cell),
        ]
    } else {
        quart
    };
    if let Some(&id) = memo.biomes.get(&key) {
        return Ok(id);
    }
    let name = resolver
        .sample(context, quart)
        .map_err(|_| MaterialRuleError::BiomeQueryFailed)?;
    let id = data.biome_id(name).ok_or(MaterialRuleError::MissingBiome)?;
    memo.biomes.insert(key, id);
    Ok(id)
}
// Per quart cell of a block-biome lookup: a lazy equality proof for its
// eight corners, or (when they differ) their jitter and biomes, so the blocks
// of the cell only redo the distance comparison.
#[derive(Clone, Copy, PartialEq)]
enum SurfaceCell {
    Unknown,
    /// A corner lookup failed: only the original lookup may report errors.
    Lookup,
    Uniform(BiomeId),
    Mixed(u32),
}
struct MixedCell {
    jitter: [[f64; 3]; 8],
    biomes: [BiomeId; 8],
}
struct UniformSurfaceBiomes {
    origin: [i32; 3],
    height: i32,
    cells: Vec<SurfaceCell>,
    mixed: Vec<MixedCell>,
    /// The cell of the previous lookup (columns are walked down in y).
    last: ([i32; 3], SurfaceCell),
}
impl UniformSurfaceBiomes {
    fn new(pos: ChunkPos, min_y: i32, height: i32) -> Self {
        let height = (height >> 2) + 1;
        Self {
            origin: [
                pos.x.wrapping_shl(4).wrapping_sub(2) >> 2,
                min_y.wrapping_sub(2) >> 2,
                pos.z.wrapping_shl(4).wrapping_sub(2) >> 2,
            ],
            height,
            cells: vec![SurfaceCell::Unknown; 25 * height as usize],
            mixed: Vec::new(),
            last: ([0; 3], SurfaceCell::Unknown),
        }
    }
    #[inline]
    fn get(&mut self, region: &BiomeRegion<BiomeId>, p: BlockPos) -> Option<BiomeId> {
        let base = [p.x, p.y, p.z].map(|v| v.wrapping_sub(2) >> 2);
        let cell = if self.last.0 == base && self.last.1 != SurfaceCell::Unknown {
            self.last.1
        } else {
            let cell = self.cell(region, base);
            self.last = (base, cell);
            cell
        };
        match cell {
            SurfaceCell::Uniform(id) => Some(id),
            SurfaceCell::Mixed(index) => {
                let mixed = &self.mixed[index as usize];
                Some(mixed.biomes[BiomeZoom::select_corner(p, &mixed.jitter)])
            }
            SurfaceCell::Unknown | SurfaceCell::Lookup => None,
        }
    }
    fn cell(&mut self, region: &BiomeRegion<BiomeId>, base: [i32; 3]) -> SurfaceCell {
        let d: [i32; 3] = std::array::from_fn(|i| base[i].wrapping_sub(self.origin[i]));
        if !(0..5).contains(&d[0]) || !(0..self.height).contains(&d[1]) || !(0..5).contains(&d[2]) {
            return SurfaceCell::Lookup;
        }
        let slot = &mut self.cells[((d[0] * 5 + d[2]) * self.height + d[1]) as usize];
        if *slot != SurfaceCell::Unknown {
            return *slot;
        }
        // These are immutable stored lookups, in quart_at_block's corner order.
        let mut biomes = [BiomeId(0); 8];
        for (corner, biome) in biomes.iter_mut().enumerate() {
            let offsets = [(corner >> 2) & 1, (corner >> 1) & 1, corner & 1];
            match region.noise_biome(std::array::from_fn(|i| {
                base[i].wrapping_add(offsets[i] as i32)
            })) {
                Ok(id) => *biome = *id,
                Err(_) => {
                    *slot = SurfaceCell::Lookup;
                    return SurfaceCell::Lookup;
                }
            }
        }
        let value = if biomes.iter().all(|id| *id == biomes[0]) {
            SurfaceCell::Uniform(biomes[0])
        } else {
            self.mixed.push(MixedCell {
                jitter: region.zoom().corner_jitter(base),
                biomes,
            });
            SurfaceCell::Mixed(self.mixed.len() as u32 - 1)
        };
        *slot = value;
        value
    }
}
/// Material-rule world queries for one chunk: shared seed-bound noises, the
/// worker's temperature caches and either stored or raw-resolver biomes.
struct SurfaceQueries<'a, 'b, R: RandomSource> {
    pipeline: &'a Pipeline<R>,
    temperatures: &'a mut [BiomeTemperature],
    biome: Biomes<'a, 'b>,
    possible: Option<BTreeSet<String>>,
    uniform_biomes: Option<UniformSurfaceBiomes>,
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
        Ok(self
            .biome_id(p)?
            .map(|id| self.pipeline.data.biome_names[id as usize].clone()))
    }
    fn biome_index(&self, name: &str) -> Option<Option<u32>> {
        Some(self.pipeline.data.biome_id(name).map(|id| id.0))
    }
    fn biome_id(&mut self, p: BlockPos) -> Result<Option<u32>, MaterialRuleError> {
        let data = &self.pipeline.data;
        let id = match &mut self.biome {
            Biomes::Region(region) => match self
                .uniform_biomes
                .as_mut()
                .and_then(|cache| cache.get(region, p))
            {
                Some(id) => id,
                None => *region
                    .biome(p)
                    .map_err(|_| MaterialRuleError::MissingBiome)?,
            },
            Biomes::Resolver(resolver, context) => {
                let name = resolver
                    .sample(context, self.pipeline.zoom.quart_at_block(p))
                    .map_err(|_| MaterialRuleError::BiomeQueryFailed)?;
                data.biome_id(name).ok_or(MaterialRuleError::MissingBiome)?
            }
            Biomes::Memo(resolver, context, memo) => memo_biome(
                data,
                resolver,
                context,
                memo,
                self.pipeline.zoom.quart_at_block(p),
            )?,
        };
        Ok(Some(id.0))
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
    terrain_context: SamplerContext,
    biome_context: SamplerContext,
    carver_context: SamplerContext,
    /// Seed/dimension-bound lookups avoid the shared mutex for overlapping
    /// carver neighborhoods. Bounded independently of the shared cache.
    carver_biomes: HashMap<(i32, i32), BiomeId>,
    /// Noise biomes asked one at a time by structure placement checks
    /// (`StartContext::noise_biome`): several structures test the same
    /// quarts, column checks whole columns, and each is a full climate sample.
    structure_biomes: HashMap<[i32; 3], BiomeId>,
    timings: StageTimings,
    heights: HashMap<(i32, i32, u8), i32>,
    /// Decorate with structure pieces only (for structure parity checks).
    pub(super) structures_only: bool,
    pub(super) structure_mobs: Option<Arc<dyn StructureMobs>>,
    /// The world's surface rule compiled once for this worker's chunks: a
    /// `CompiledSurfaceRule` of the pipeline's random.
    surface_rule: Option<Box<dyn Any + Send>>,
    /// Empty stand-in chunks recycled across decorated chunks so decoration
    /// does not allocate 9 throwaway `TerrainChunk`s per chunk.
    pub(super) decor_standins: Vec<TerrainChunk<'static, GenBlockStates>>,
    /// Uniform-biome section cache recycled across decorated chunks.
    pub(super) decor_uniform_biomes: Vec<u32>,
}
/// `Mob.finalizeSpawn(level, difficulty, STRUCTURE, null)` of the entities a
/// structure places with finalizeEntities, run by the game, which owns mobs.
pub trait StructureMobs: Send + Sync {
    /// Finalizes the entity in `nbt` if it is a mob, drawing from the
    /// region's random. `biome` is the biome at its block, `structures` the
    /// structures with a piece there. False when it could not: the entity
    /// keeps its `horizon:finalize` for the game to finish on load.
    fn finalize(
        &self,
        nbt: &mut horizon_core::nbt::Compound,
        biome: &str,
        structures: &[String],
        random: &mut dyn super::structure::legacy::PieceRandom,
    ) -> bool;
}

/// A start chunk's saved `structures.starts`, including pending writes.
pub type SavedStructureStarts =
    dyn Fn(ChunkPos) -> Option<horizon_core::nbt::Compound> + Send + Sync;

trait SeededPipeline: Send + Sync {
    fn set_saved_starts(&self, source: Arc<SavedStructureStarts>);
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
    /// What the shared caches hold, for `horizon-ctl memory`.
    fn cache_census(&self) -> CacheCensus {
        CacheCensus::default()
    }
    /// Empty the caches that only spare work ([`OverworldGenerator::release_caches`]).
    fn release_caches(&self) {}
    /// The exact `end_island_decorated` origins in a chunk, without terrain.
    fn end_island_origins(&self, chunk: ChunkPos) -> Vec<[i32; 3]>;
    fn sea_level(&self) -> i32;
    /// `NoiseGeneratorSettings.disableMobGeneration`, which
    /// `NoiseBasedChunkGenerator.spawnOriginalMobs` reads.
    fn disable_mob_generation(&self) -> bool;
    /// The level's build range (minimum Y, height).
    fn level(&self) -> (i32, i32);
    fn surface_sample(
        &self,
        worker: &mut WorkerState,
        x: i32,
        z: i32,
        spans: Option<&mut Vec<(i32, i32, BlockStateId)>>,
    ) -> Option<SurfaceSample>;
    fn lod_grid(
        &self,
        worker: &mut WorkerState,
        x: i32,
        z: i32,
        width: i32,
        side: usize,
        approximate: bool,
    ) -> Result<Vec<LodColumn>, GeneratorError>;
    fn structure_starts(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Vec<Arc<super::structure::start::StructureStart>>;
    /// `structure_starts` from the cache alone: None when they aren't cached.
    fn cached_structure_starts(
        &self,
        pos: ChunkPos,
    ) -> Option<Vec<Arc<super::structure::start::StructureStart>>>;
    /// `Structure.generate` for one structure in one chunk (`/place
    /// structure`), outside any structure set's placement.
    fn structure_start(
        &self,
        worker: &mut WorkerState,
        structure: &str,
        chunk: ChunkPos,
    ) -> Option<Arc<super::structure::start::StructureStart>>;
    fn unsupported_features(&self) -> Vec<(String, String)>;
    fn use_saved_start(
        &self,
        worker: &mut WorkerState,
        chunk: ChunkPos,
        structure: &str,
        pieces: &[serde_json::Value],
    ) -> Result<(), String>;
    fn forget_structure_starts(&self);
    fn ring_positions(&self, worker: &mut WorkerState, set: &str) -> Option<Vec<ChunkPos>>;
    /// The structures whose random-spread set may start in `chunk`, each with whether a
    /// search's start check lets it start there.
    #[cfg(test)]
    fn may_start_answers(
        &self,
        _worker: &mut WorkerState,
        _chunk: ChunkPos,
    ) -> Vec<(String, bool)> {
        Vec::new()
    }
    fn spawn_position(&self) -> Result<[i32; 2], GeneratorError>;
    fn region_random(&self, center: ChunkPos) -> Box<dyn super::structure::legacy::PieceRandom>;
    fn locate(
        &self,
        worker: &mut WorkerState,
        names: &[String],
        from: [i32; 3],
        radius: i32,
        skip: &dyn Fn(&str, ChunkPos) -> bool,
    ) -> Option<(String, [i32; 3])>;
    /// The chunks an older version generated, for `Blender.of`.
    fn set_old_world(&self, world: Arc<dyn super::blending::OldWorld>);
    /// `NoiseBasedChunkGenerator.addDebugScreenInfo`.
    fn debug_density_line(&self, feet: [i32; 3]) -> Option<String>;
    fn find_closest_biome(
        &self,
        worker: &mut WorkerState,
        search: &BiomeSearch<'_>,
    ) -> Option<(String, [i32; 3])>;
    /// `BiomeSource.getNoiseBiome` at each quart position (unloaded chunks).
    fn noise_biomes(&self, worker: &mut WorkerState, quarts: &[[i32; 3]]) -> Vec<Option<String>>;
    fn jigsaw_pieces(
        &self,
        worker: &mut WorkerState,
        query: &JigsawQuery<'_>,
    ) -> Option<JigsawPieces>;
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
                let mut random = self.region_random_at(region[4].position());
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
            // A mod's level that asks for neither structures nor features has nothing to place.
            None if self.mod_terrain.is_some() => {
                region[4]
                    .terrain
                    .set_generation_stage(PreLightChunkStage::Features);
                region[4].set_decorated();
                Ok(DecorateStats::default())
            }
            None => Err(resource("decoration resources were not loaded")),
        }
    }
    fn data(&self) -> &Arc<GeneratorData> {
        &self.data
    }
    fn release_caches(&self) {
        // Taken out under the lock, dropped after it.
        let biomes = std::mem::take(&mut *self.biome_cache.lock().unwrap());
        let carvers = std::mem::take(&mut *self.carver_cache.lock().unwrap());
        drop((biomes, carvers));
    }
    fn cache_census(&self) -> CacheCensus {
        let biomes = self.biome_cache.lock().unwrap();
        let (start_chunks, start_lists) = self
            .structures
            .as_ref()
            .map_or((0, 0), |s| s.start_census());
        CacheCensus {
            biome_chunks: biomes.len(),
            biome_bytes: biomes
                .values()
                .map(|b| std::mem::size_of::<BiomeChunk<BiomeId>>() + b.heap_bytes())
                .sum::<usize>()
                + biomes.capacity() * std::mem::size_of::<((i32, i32), usize)>(),
            carver_entries: self.carver_cache.lock().unwrap().len(),
            start_chunks,
            start_lists,
        }
    }
    fn end_island_origins(&self, chunk: ChunkPos) -> Vec<[i32; 3]> {
        let Some(decoration) = &self.decoration else {
            return Vec::new();
        };
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        let mut context = SamplerContext::cached();
        decoration.origins(self.seed, chunk, |position| {
            let quart = [position[0] >> 2, position[1] >> 2, position[2] >> 2];
            resolver.sample(&mut context, quart).ok() == Some("minecraft:small_end_islands")
        })
    }
    fn sea_level(&self) -> i32 {
        self.settings.sea_level
    }
    fn disable_mob_generation(&self) -> bool {
        self.settings.disable_mob_generation
    }
    fn level(&self) -> (i32, i32) {
        (self.settings.level_min_y, self.settings.level_height)
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
    fn cached_structure_starts(
        &self,
        pos: ChunkPos,
    ) -> Option<Vec<Arc<super::structure::start::StructureStart>>> {
        match &self.structures {
            Some(structures) => structures
                .cached_starts_at(pos)
                .map(|starts| starts.to_vec()),
            None => Some(Vec::new()),
        }
    }
    fn structure_start(
        &self,
        worker: &mut WorkerState,
        structure: &str,
        chunk: ChunkPos,
    ) -> Option<Arc<super::structure::start::StructureStart>> {
        self.placement_structures()?
            .structure_start(
                &mut StructureQueries {
                    pipeline: self,
                    worker,
                    valid: BTreeSet::new(),
                },
                structure,
                chunk,
            )
            .map(Arc::new)
    }
    fn unsupported_features(&self) -> Vec<(String, String)> {
        self.decoration
            .as_ref()
            .map(|decoration| decoration.unsupported())
            .unwrap_or_default()
    }
    fn use_saved_start(
        &self,
        worker: &mut WorkerState,
        chunk: ChunkPos,
        structure: &str,
        pieces: &[serde_json::Value],
    ) -> Result<(), String> {
        let structures = (self.structures.as_ref()).ok_or("structures were not loaded")?;
        let mut queries = StructureQueries {
            pipeline: self,
            worker,
            valid: BTreeSet::new(),
        };
        structures.use_saved_start(&mut queries, chunk, structure, pieces)
    }
    fn set_saved_starts(&self, source: Arc<SavedStructureStarts>) {
        if let Some(structures) = &self.structures {
            structures.set_saved_starts(source);
        }
    }
    fn forget_structure_starts(&self) {
        if let Some(structures) = &self.structures {
            structures.forget_starts();
        }
    }
    #[cfg(test)]
    fn may_start_answers(&self, worker: &mut WorkerState, chunk: ChunkPos) -> Vec<(String, bool)> {
        match &self.structures {
            Some(structures) => structures.may_start_answers(
                &mut StructureQueries {
                    pipeline: self,
                    worker,
                    valid: BTreeSet::new(),
                },
                chunk,
            ),
            None => Vec::new(),
        }
    }
    fn ring_positions(&self, worker: &mut WorkerState, set: &str) -> Option<Vec<ChunkPos>> {
        self.structures.as_ref()?.ring_positions(
            &mut StructureQueries {
                pipeline: self,
                worker,
                valid: BTreeSet::new(),
            },
            set,
        )
    }
    fn region_random(&self, center: ChunkPos) -> Box<dyn super::structure::legacy::PieceRandom> {
        Box::new(self.region_random_at(center))
    }
    fn spawn_position(&self) -> Result<[i32; 2], GeneratorError> {
        self.spawn_column()
    }
    fn locate(
        &self,
        worker: &mut WorkerState,
        names: &[String],
        from: [i32; 3],
        radius: i32,
        skip: &dyn Fn(&str, ChunkPos) -> bool,
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
            skip,
        )
    }
    fn set_old_world(&self, world: Arc<dyn super::blending::OldWorld>) {
        let _ = self.old_world.set(world);
    }
    /// `NoiseBasedChunkGenerator.addDebugScreenInfo`: the labelled values of
    /// the settings' `debug_functions` at `feet`, three decimals each.
    fn debug_density_line(&self, feet: [i32; 3]) -> Option<String> {
        if self.debug_density.is_empty() {
            return None;
        }
        let mut context = SamplerContext::cached();
        let mut line = String::from("Density ");
        for (label, sampler) in &self.debug_density {
            let value = sampler.sample_value(&mut context, feet).ok()?;
            line.push_str(&format!("{label}: {:.3} ", f64::from(value)));
        }
        line.pop();
        Some(line)
    }
    fn find_closest_biome(
        &self,
        worker: &mut WorkerState,
        search: &BiomeSearch<'_>,
    ) -> Option<(String, [i32; 3])> {
        // BiomeSource.findClosestBiome3d.
        let candidates: BTreeSet<&str> = self
            .source
            .possible_biomes()
            .into_iter()
            .filter(|name| (search.allowed)(name))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        let ys = out_from_origin(
            search.origin[1],
            search.min_y + 1,
            search.max_y + 1,
            search.vertical,
        );
        let radius = search.radius.div_euclid(search.horizontal);
        for [sx, sz] in spiral_around(radius) {
            let x = search.origin[0].wrapping_add(sx.wrapping_mul(search.horizontal));
            let z = search.origin[2].wrapping_add(sz.wrapping_mul(search.horizontal));
            for &y in &ys {
                let quart = [x >> 2, y >> 2, z >> 2];
                let Ok(name) = resolver.sample(&mut worker.carver_context, quart) else {
                    continue;
                };
                if candidates.contains(name) {
                    return Some((name.to_owned(), [x, y, z]));
                }
            }
        }
        None
    }
    fn surface_sample(
        &self,
        worker: &mut WorkerState,
        x: i32,
        z: i32,
        mut spans: Option<&mut Vec<(i32, i32, BlockStateId)>>,
    ) -> Option<SurfaceSample> {
        let mut top = None;
        let mut water = None;
        let mut below = 0;
        let mut surface_complete = false;
        let mut visit = |y, state| {
            let properties = self.data.states.get(state);
            if let Some(spans) = spans.as_mut()
                && !properties.is_air()
            {
                if let Some((min, _, previous)) = spans.last_mut()
                    && *min == y + 1
                    && *previous == state
                {
                    *min = y;
                } else {
                    spans.push((y, y + 1, state));
                }
            }
            if surface_complete {
                return false;
            }
            if properties.is_air() || properties.has_fluid() {
                if top.is_some() {
                    surface_complete = true;
                    return spans.is_none();
                }
                if properties.has_fluid() {
                    water.get_or_insert((y + 1, state));
                } else {
                    // Match the material traversal's water reset across air gaps.
                    water = None;
                }
            } else {
                top.get_or_insert((y + 1, state));
                below += 1;
            }
            false
        };
        self.noise_column(x, z, &mut visit);
        let (height, state) = top?;
        // A superflat surface is its top layer, with its one biome.
        if self.flat.is_some() {
            let biome = self
                .source
                .possible_biomes()
                .into_iter()
                .find_map(|name| self.data.biome_id(name))?;
            return Some(SurfaceSample {
                height,
                state,
                water,
                biome,
            });
        }
        let y = height - 1;
        let position = BlockPos::new(x, y, z);
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        // Use a fresh bounded context: far samples must not grow the chunk/structure caches.
        let mut biome_context = SamplerContext::default();
        let biome = resolver
            .sample(&mut biome_context, self.zoom.quart_at_block(position))
            .ok()?;
        let biome = self.data.biome_id(biome)?;
        let volume = DensityVolume::new([1; 3], [x, y, z], [1; 3]).ok()?;
        let queries = SurfaceQueries {
            pipeline: self,
            temperatures: &mut worker.temperatures,
            biome: Biomes::Resolver(&mut resolver, &mut biome_context),
            uniform_biomes: None,
            possible: None,
        };
        let sources = MaterialDensitySources::new(
            queries,
            volume,
            self.preliminary.clone(),
            SamplerContext::default(),
        )
        .ok()?;
        let mut material =
            MaterialConditions::<R, _>::new(self.generation(), self.root.clone(), sources);
        let rule = material.add_rule(self.rule.clone()).ok()?;
        // shortcut: distant surfaces omit slope rules and column extensions; nearby chunks provide full terrain.
        material.update_xz(x, z, 0, 0);
        material.update_y(1, below, water.map_or(i32::MIN, |(height, _)| height), y);
        let state = material.try_apply(rule, position).ok()?.unwrap_or(state);
        if let Some(spans) = spans
            && let Some(index) = spans.iter().position(|&(min, max, _)| min <= y && y < max)
            && spans[index].2 != state
        {
            let (min, max, _) = spans[index];
            if min == y {
                spans[index].2 = state;
            } else {
                spans[index].1 = y;
                spans.insert(index, (y, max, state));
            }
        }
        Some(SurfaceSample {
            height,
            state,
            water,
            biome,
        })
    }
    fn lod_grid(
        &self,
        worker: &mut WorkerState,
        x: i32,
        z: i32,
        width: i32,
        side: usize,
        approximate: bool,
    ) -> Result<Vec<LodColumn>, GeneratorError> {
        if width <= 0 || side == 0 || side > 64 {
            return Err(resource("far sample dimensions"));
        }
        // A superflat world's columns are its layers: the surface sample, and its spans.
        if self.flat.is_some() {
            let mut columns = Vec::with_capacity(side * side);
            for dz in 0..side {
                for dx in 0..side {
                    let (wx, wz) = (x + dx as i32 * width, z + dz as i32 * width);
                    let mut spans = Vec::new();
                    let sample =
                        SeededPipeline::surface_sample(self, worker, wx, wz, Some(&mut spans));
                    columns.push((sample, spans));
                }
            }
            return Ok(columns);
        }
        let mut geometry = Vec::with_capacity(side * side);
        // The coarse and band passes below share the 2D (x/z-only) parts.
        let mut context = if approximate {
            SamplerContext::sparse().with_planes()
        } else {
            SamplerContext::sparse()
        };
        let (min, height) = (self.settings.noise_min_y, self.settings.noise_height);
        let step_xz = width.max(4);
        let ox = x.div_euclid(4) * 4;
        let oz = z.div_euclid(4) * 4;
        let nx = ((x - ox + (side as i32 - 1) * width + step_xz - 1) / step_xz) + 1;
        let nz = ((z - oz + (side as i32 - 1) * width + step_xz - 1) / step_xz) + 1;
        // Reduced far detail: how many blocks down surface rules paint a far
        // column. Vanilla's rules reach eight blocks (dirt under grass, sand
        // under a beach, powder snow over a peak), but a tile this wide draws
        // at most a block or two of each column's vertical face, so painting
        // the six below it is work no view resolves. Depth 1 is always painted,
        // so the surface the tile shows -- its height, its top state, its
        // biome -- is exactly what Full detail gives; only the interior of a
        // span, seen on a cliff, loses its vanilla material. Narrow tiles keep
        // all eight: their faces are tall enough in view to show them.
        let material_depth = if approximate && width >= 16 { 2 } else { 8 };
        // Approximate: cells many blocks wide keep their interpolated shape on
        // a coarser vertical lattice, for a fraction of the density work.
        let step_y = match width {
            _ if !approximate => 8,
            ..32 => 8,
            32..128 => 16,
            _ => 32,
        };
        let final_density = self.router.get(RouterField::FinalDensity);
        // Approximate: far tiles show the surface, so find its band on a
        // coarse lattice (its points are the fine lattice's), then sample the
        // fine lattice only inside it. Most of a tall world's column is sky or
        // deep rock. Parts thinner than the coarse step above an air sample
        // (floating islands, overhang lips) can be missed; below the band, a
        // column is one span of the default block.
        let (band_min, band_max) = if approximate {
            const COARSE: i32 = 64;
            let coarse_layers = (height + COARSE - 1) / COARSE + 1;
            let coarse = DensityVolume::new(
                [nx, coarse_layers, nz],
                [ox, min, oz],
                [step_xz, COARSE, step_xz],
            )
            .map_err(stage("far density"))?;
            let mut coarse_densities = vec![0.; coarse.size()];
            final_density
                .sample_volume(&mut context, &mut coarse_densities, coarse)
                .map_err(stage("far density"))?;
            let (mut lowest, mut highest) = (i32::MAX, i32::MIN);
            for iz in 0..nz as usize {
                for ix in 0..nx as usize {
                    let solid = (0..coarse_layers as usize)
                        .rev()
                        .find(|&iy| coarse_densities[coarse.index([ix, iy, iz]).unwrap()] > 0.);
                    if let Some(iy) = solid {
                        let y = min + iy as i32 * COARSE;
                        lowest = lowest.min(y - COARSE);
                        highest = highest.max(y + 2 * COARSE);
                    }
                }
            }
            if lowest > highest {
                (min, min + height)
            } else {
                let lowest = lowest.max(min);
                (
                    min + (lowest - min) / step_y * step_y,
                    highest
                        .max(self.settings.sea_level + COARSE)
                        .min(min + height),
                )
            }
        } else {
            (min, min + height)
        };
        let ny = (band_max - band_min + step_y - 1) / step_y + 1;
        let volume =
            DensityVolume::new([nx, ny, nz], [ox, band_min, oz], [step_xz, step_y, step_xz])
                .map_err(stage("far density"))?;
        let mut densities = vec![0.; volume.size()];
        final_density
            .sample_volume(&mut context, &mut densities, volume)
            .map_err(stage("far density"))?;
        let aquifer = DisabledAquifer::new(
            self.picker,
            self.data.registry.default_state("minecraft:air").unwrap(),
        );
        // Interpolate the supplied lattice, never the physical bounding volume.
        let value = |wx: i32, iy: usize, wz: i32| {
            let rx = (wx - ox) as f32 / step_xz as f32;
            let rz = (wz - oz) as f32 / step_xz as f32;
            let ix = rx.floor() as usize;
            let iz = rz.floor() as usize;
            let at = |dx: usize, dz: usize| {
                densities[volume
                    .index([
                        (ix + dx).min(nx as usize - 1),
                        iy,
                        (iz + dz).min(nz as usize - 1),
                    ])
                    .unwrap()]
            };
            let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
            mix(
                mix(at(0, 0), at(1, 0), rx - ix as f32),
                mix(at(0, 1), at(1, 1), rx - ix as f32),
                rz - iz as f32,
            )
        };
        let mut profile = vec![0.; ny as usize];
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        let mut biome_context = SamplerContext::cached();
        let mut biome_memo = BiomeMemo {
            biomes: HashMap::new(),
            // Columns `width` blocks apart are width / 4 quarts apart; cells
            // stop at 64 blocks (wider ones moved a third of level 6's biomes).
            cell: if approximate && width >= 4 {
                width.min(16)
            } else {
                0
            },
        };
        for dz in 0..side {
            for dx in 0..side {
                let (wx, wz) = (x + dx as i32 * width, z + dz as i32 * width);
                let mut top = None;
                let mut water = None;
                let mut below = 0;
                let mut complete = false;
                let mut spans: Vec<(i32, i32, BlockStateId)> = Vec::new();
                for (iy, density) in profile.iter_mut().enumerate() {
                    *density = value(wx, iy, wz);
                }
                for y in (band_min..band_max).rev() {
                    let iy = ((y - band_min) / step_y) as usize;
                    let alpha = ((y - band_min) % step_y) as f32 / step_y as f32;
                    let density = profile[iy] + (profile[iy + 1] - profile[iy]) * alpha;
                    let state = aquifer
                        .compute_substance([wx, y, wz], f64::from(density))
                        .unwrap_or(self.settings.default_block);
                    let properties = self.data.states.get(state);
                    if !properties.is_air() {
                        if let Some((min, _, old)) = spans.last_mut()
                            && *min == y + 1
                            && *old == state
                        {
                            *min = y;
                        } else {
                            spans.push((y, y + 1, state));
                        }
                    }
                    if !complete {
                        if properties.is_air() || properties.has_fluid() {
                            if top.is_some() {
                                complete = true;
                            } else if properties.has_fluid() {
                                water.get_or_insert((y + 1, state));
                            } else {
                                water = None;
                            }
                        } else {
                            top.get_or_insert((y + 1, state));
                            below += 1;
                        }
                    }
                }
                if band_min > min {
                    let fill = self.settings.default_block;
                    if top.is_some() && !complete {
                        below += band_min - min;
                    }
                    match spans.last_mut() {
                        Some((low, _, state)) if *low == band_min && *state == fill => *low = min,
                        _ => spans.push((min, band_min, fill)),
                    }
                }
                // Void cells still need a biome: the End's small islands are
                // features over the void, and their biome is 2D there.
                let biome = if let Some((height, _)) = top {
                    memo_biome(
                        &self.data,
                        &mut resolver,
                        &mut biome_context,
                        &mut biome_memo,
                        self.zoom.quart_at_block(BlockPos::new(wx, height - 1, wz)),
                    )
                    .map_err(stage("far biome"))?
                } else {
                    BiomeId(0)
                };
                geometry.push((top, water, below, spans, biome));
            }
        }
        if geometry.iter().all(|column| column.0.is_none()) {
            return Ok(geometry
                .into_iter()
                .map(|(_, _, _, spans, _)| (None, spans))
                .collect());
        }
        let volume =
            DensityVolume::new([1; 3], [x, 0, z], [1; 3]).map_err(stage("far material"))?;
        let queries = SurfaceQueries {
            pipeline: self,
            temperatures: &mut worker.temperatures,
            biome: Biomes::Memo(&mut resolver, &mut biome_context, &mut biome_memo),
            uniform_biomes: None,
            possible: None,
        };
        let sources = MaterialDensitySources::new(
            queries,
            volume,
            self.preliminary.clone(),
            SamplerContext::cached(),
        )
        .map_err(stage("far material"))?;
        let mut material =
            MaterialConditions::<R, _>::new(self.generation(), self.root.clone(), sources);
        let rule = material
            .add_rule(self.rule.clone())
            .map_err(stage("far material"))?;
        let mut result = Vec::with_capacity(side * side);
        for (index, (top, water, below, mut spans, biome)) in geometry.into_iter().enumerate() {
            let (wx, wz) = (
                x + (index % side) as i32 * width,
                z + (index / side) as i32 * width,
            );
            let Some((height, mut state)) = top else {
                result.push((None, spans));
                continue;
            };
            material.update_xz(wx, wz, 0, 0);
            for depth in 1..=below.min(material_depth) {
                let y = height - depth;
                material.update_y(
                    depth,
                    below - depth + 1,
                    water.map_or(i32::MIN, |(height, _)| height),
                    y,
                );
                let Some(index) = spans.iter().position(|&(min, max, _)| min <= y && y < max)
                else {
                    continue;
                };
                let (min, max, old) = spans[index];
                let painted = material
                    .try_apply(rule, BlockPos::new(wx, y, wz))
                    .map_err(stage("far material"))?
                    .unwrap_or(old);
                if depth == 1 {
                    state = painted;
                }
                if old == painted {
                    continue;
                }
                let mut replacement = Vec::with_capacity(3);
                if y + 1 < max {
                    replacement.push((y + 1, max, old));
                }
                replacement.push((y, y + 1, painted));
                if min < y {
                    replacement.push((min, y, old));
                }
                spans.splice(index..=index, replacement);
            }
            result.push((
                Some(SurfaceSample {
                    height,
                    state,
                    water,
                    biome,
                }),
                spans,
            ));
        }
        Ok(result)
    }
    fn noise_biomes(&self, worker: &mut WorkerState, quarts: &[[i32; 3]]) -> Vec<Option<String>> {
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        quarts
            .iter()
            .map(|&quart| {
                resolver
                    .sample(&mut worker.carver_context, quart)
                    .ok()
                    .map(str::to_owned)
            })
            .collect()
    }
    fn jigsaw_pieces(
        &self,
        worker: &mut WorkerState,
        query: &JigsawQuery<'_>,
    ) -> Option<JigsawPieces> {
        // JigsawPlacement.generateJigsaw: addPieces with no heightmap
        // projection, MaxDistance(128), no aliases, no padding and
        // waterlogging applied. The pieces draw from the GenerationContext's
        // random, seeded from the level seed and the position's chunk, not
        // from the level random.
        let structures = self.placement_structures()?;
        let mut random = horizon_core::random::WorldgenRandom::new(LegacyRandom::new(0));
        random.set_large_feature_seed(self.seed, query.position[0] >> 4, query.position[2] >> 4);
        let aliases = BTreeMap::new();
        let config = super::structure::jigsaw::JigsawConfig {
            start_pool: query.pool,
            start_jigsaw: Some(query.target),
            max_depth: query.max_depth,
            use_expansion_hack: false,
            project_to: None,
            max_distance: [128, 128],
            aliases: &aliases,
            padding: [0, 0],
            waterlogging: true,
        };
        let (_, pieces) = super::structure::jigsaw::assemble(
            &config,
            &mut StructureQueries {
                pipeline: self,
                worker,
                valid: BTreeSet::new(),
            },
            &structures.registry.pools,
            &structures.resources,
            &structures.bindings,
            &mut random,
            query.position,
        )?;
        Some(JigsawPieces {
            pieces,
            resources: structures.resources.clone(),
        })
    }
}

mod flat;
mod mod_terrain;
mod queries;
use flat::FlatLayers;
pub use flat::FlatSettings;
pub use queries::{
    BiomeSearch, DecorateStats, JigsawPieces, JigsawQuery, out_from_origin, spiral_around,
};

/// A seed-bound generator for one dimension, shareable across worker threads.
#[derive(Clone)]
pub struct OverworldGenerator {
    pipeline: Arc<dyn SeededPipeline>,
    /// The noise range the settings asked for and the build height's part
    /// of it that generates, when they differ.
    noise_clamped: Option<(NoiseSettings, NoiseSettings)>,
}
impl OverworldGenerator {
    /// The structure starts a chunk owns when they are cached already, else None: no save is
    /// read and nothing is built, for reads that may load no chunk.
    pub fn cached_structure_starts(
        &self,
        pos: ChunkPos,
    ) -> Option<Vec<Arc<super::structure::start::StructureStart>>> {
        self.pipeline.cached_structure_starts(pos)
    }
    /// Install before workers run, so every cache miss can consult the save before generation.
    pub fn set_saved_starts(&self, source: Arc<SavedStructureStarts>) {
        self.pipeline.set_saved_starts(source);
    }
    /// What the shared caches hold, for `horizon-ctl memory`.
    pub fn cache_census(&self) -> CacheCensus {
        self.pipeline.cache_census()
    }
    /// Empty the chunk biome and carver biome caches, while nothing generates:
    /// they keep every chunk generated (up to 16384, 6 KB each) and only spare
    /// computing a chunk's biomes again. The biomes come out the same: the
    /// caches are emptied whole at their limit too.
    pub fn release_caches(&self) {
        self.pipeline.release_caches();
    }
    /// The chunks an older version generated, which new chunks blend with
    /// (`Blender.of`); set once, before chunks are generated.
    pub fn set_old_world(&self, world: Arc<dyn super::blending::OldWorld>) {
        self.pipeline.set_old_world(world);
    }
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
    /// A dimension of a world preset, with or without structures (the
    /// world's `generate_structures`: without, no starts, terrain adaptation,
    /// structure pieces or structure locating).
    pub fn for_world(
        data: Arc<GeneratorData>,
        seed: i64,
        preset: &str,
        dimension: &str,
        structures: bool,
    ) -> Result<Self, GeneratorError> {
        Self::build(data, seed, preset, dimension, structures)
    }
    fn build(
        data: Arc<GeneratorData>,
        seed: i64,
        preset: &str,
        dimension: &str,
        with_structures: bool,
    ) -> Result<Self, GeneratorError> {
        Self::for_world_with_settings(data, seed, preset, dimension, with_structures, None, None)
    }
    /// A saved level stem supplies the generator and dimension type when no data
    /// pack replaces that dimension. Worlds without one use the preset's stem.
    /// `regions` is the dimension's saved region layout, if it has one.
    pub fn for_world_with_settings(
        data: Arc<GeneratorData>,
        seed: i64,
        preset: &str,
        dimension: &str,
        with_structures: bool,
        saved_stem: Option<&Value>,
        regions: Option<Arc<RegionLayout>>,
    ) -> Result<Self, GeneratorError> {
        let stem = if let Some(json) = data.dimensions.get(dimension) {
            serde_json::from_str(json)?
        } else if let Some(stem) = saved_stem {
            stem.clone()
        } else {
            let preset_json: Value = serde_json::from_str(
                data.world_presets
                    .get(preset)
                    .ok_or_else(|| resource(format!("missing world preset {preset}")))?,
            )?;
            preset_json["dimensions"][dimension].clone()
        };
        if stem["generator"]["type"].as_str() != Some("minecraft:noise") {
            return Err(resource(format!("{dimension} is not a noise generator")));
        }
        Self::from_stem(data, seed, &stem, with_structures, regions, None)
    }
    /// The pipeline of level `stem` over its noise settings, with a mod's `terrain` in place of
    /// the noise when it has one.
    fn from_stem(
        data: Arc<GeneratorData>,
        seed: i64,
        stem: &Value,
        with_structures: bool,
        regions: Option<Arc<RegionLayout>>,
        terrain: Option<mod_terrain::ModTerrain>,
    ) -> Result<Self, GeneratorError> {
        let generator = &stem["generator"];
        // The level's build range: its dimension type's, a data pack's
        // over the vanilla one.
        let level = overrides::LevelHeight::of_type(&stem["type"], &data.dimension_types)
            .map(|h| (h.min_y, h.height));
        let mut settings_json: Value = match &generator["settings"] {
            Value::String(name) => serde_json::from_str(
                data.settings
                    .get(name)
                    .ok_or_else(|| resource(format!("missing noise settings {name}")))?,
            )?,
            inline => inline.clone(),
        };
        // NoiseSettings.clampToHeightAccessor: noise past the build height
        // (a data pack's taller terrain over a dimension type it left as it
        // was) fills only the part inside it.
        let mut noise_clamped = None;
        if let Some((level_min_y, level_height)) = level
            && let (Some(min_y), Some(height)) = (
                settings_json["noise"]["min_y"].as_i64(),
                settings_json["noise"]["height"].as_i64(),
            )
        {
            let noise = NoiseSettings::new(min_y as i32, height as i32);
            let clamped = noise.clamp_to_height(level_min_y, level_height);
            if clamped != noise {
                settings_json["noise"]["min_y"] = clamped.min_y.into();
                settings_json["noise"]["height"] = clamped.height.into();
                noise_clamped = Some((noise, clamped));
            }
        }
        let pipeline: Arc<dyn SeededPipeline> = if settings_json["legacy_random_source"]
            .as_bool()
            .unwrap_or(false)
        {
            Arc::new(
                Pipeline::new(
                    data,
                    seed,
                    LegacyRandom::new(seed),
                    generator,
                    &settings_json,
                    with_structures,
                    level,
                    regions.as_deref(),
                )?
                .with_mod_terrain(terrain)?,
            )
        } else {
            Arc::new(
                Pipeline::new(
                    data,
                    seed,
                    XoroshiroRandom::new(seed),
                    generator,
                    &settings_json,
                    with_structures,
                    level,
                    regions.as_deref(),
                )?
                .with_mod_terrain(terrain)?,
            )
        };
        Ok(Self {
            pipeline,
            noise_clamped,
        })
    }
    pub fn data(&self) -> &Arc<GeneratorData> {
        self.pipeline.data()
    }
    /// The noise range the noise settings asked for and the part of it
    /// inside the dimension's build height, when the settings reach past it.
    pub fn noise_clamped(&self) -> Option<(NoiseSettings, NoiseSettings)> {
        self.noise_clamped
    }
    /// The exact `end_island_decorated` origins in a chunk, without terrain.
    pub fn end_island_origins(&self, chunk: ChunkPos) -> Vec<[i32; 3]> {
        self.pipeline.end_island_origins(chunk)
    }
    pub fn sea_level(&self) -> i32 {
        self.pipeline.sea_level()
    }
    /// `NoiseGeneratorSettings.disableMobGeneration`: the End's preset asks for
    /// no creature packs at generation, every other shipped one does.
    pub fn disable_mob_generation(&self) -> bool {
        self.pipeline.disable_mob_generation()
    }
    /// The dimension's build range (minimum Y, height): what its chunks span.
    pub fn level(&self) -> (i32, i32) {
        self.pipeline.level()
    }
    /// Placed features of this dimension's biomes that decoration skips because
    /// their algorithm or bindings are not implemented, with the decode error.
    pub fn unsupported_features(&self) -> Vec<(String, String)> {
        self.pipeline.unsupported_features()
    }
    /// NoiseBasedChunkGenerator.getOrigin: the chunk a new world's spawn
    /// search starts from (NoiseSpawnFinder over the `spawn_target` points).
    pub fn origin(&self) -> Result<ChunkPos, GeneratorError> {
        let [x, z] = self.spawn_position()?;
        Ok(ChunkPos::new(x >> 4, z >> 4))
    }
    /// NoiseSpawnFinder.findSpawnPosition: the column `origin` is the chunk of.
    pub fn spawn_position(&self) -> Result<[i32; 2], GeneratorError> {
        self.pipeline.spawn_position()
    }
    /// A new `WorldGenRegion` random centred on the chunk: what its generation
    /// steps draw from (`level.getRandom()`).
    pub fn region_random(
        &self,
        center: ChunkPos,
    ) -> Box<dyn super::structure::legacy::PieceRandom> {
        self.pipeline.region_random(center)
    }
    /// `NoiseBasedChunkGenerator.addDebugScreenInfo`: the `Density` line of
    /// the noise settings' `debug_functions` at the feet (none without them).
    /// It reads the pipeline's samplers only, so it needs no worker.
    pub fn debug_density_line(&self, feet: [i32; 3]) -> Option<String> {
        self.pipeline.debug_density_line(feet)
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
                    .map(|s| BiomeTemperature::for_generation(*s))
                    .collect(),
                terrain_context: SamplerContext::cached(),
                biome_context: SamplerContext::cached(),
                carver_context: SamplerContext::default(),
                carver_biomes: HashMap::new(),
                structure_biomes: HashMap::new(),
                timings: StageTimings::default(),
                structures_only: false,
                structure_mobs: None,
                heights: HashMap::new(),
                surface_rule: None,
                decor_standins: Vec::new(),
                decor_uniform_biomes: Vec::new(),
            },
        }
    }
}

/// A surface-only noise column for far terrain. Heights are exclusive top faces.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceSample {
    pub height: i32,
    pub state: BlockStateId,
    pub water: Option<(i32, BlockStateId)>,
    pub biome: BiomeId,
}

pub type LodColumn = (Option<SurfaceSample>, Vec<(i32, i32, BlockStateId)>);

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
    /// Noise/aquifer surface and its material rule, without chunks, structures or features.
    pub fn surface_sample(&mut self, x: i32, z: i32) -> Option<SurfaceSample> {
        self.pipeline.surface_sample(&mut self.state, x, z, None)
    }
    /// Noise/aquifer material runs, excluding air; bounds are half-open.
    /// Features and carvers still require generated chunks.
    pub fn surface_column(
        &mut self,
        x: i32,
        z: i32,
    ) -> (Option<SurfaceSample>, Vec<(i32, i32, BlockStateId)>) {
        let mut spans = Vec::new();
        let surface = self
            .pipeline
            .surface_sample(&mut self.state, x, z, Some(&mut spans));
        (surface, spans)
    }
    /// `/locate structure`: the nearest start of one of `names`.
    pub fn locate(
        &mut self,
        names: &[String],
        from: [i32; 3],
        radius: i32,
    ) -> Option<(String, [i32; 3])> {
        self.locate_skipping(names, from, radius, &|_, _| false)
    }
    /// `ChunkGenerator.findNearestMapStructure`: like `locate`, past the
    /// starts `skip` says can't be referenced again (an explorer map that
    /// skips known structures).
    pub fn locate_skipping(
        &mut self,
        names: &[String],
        from: [i32; 3],
        radius: i32,
        skip: &dyn Fn(&str, ChunkPos) -> bool,
    ) -> Option<(String, [i32; 3])> {
        self.pipeline
            .locate(&mut self.state, names, from, radius, skip)
    }
    /// Direct far sampling: work scales with rendered vertical resolution,
    /// without allocating chunks, carving masks or propagated aquifers.
    /// `approximate` (Reduced far detail) trades the exact tile for speed in
    /// wide cells: one biome per 4x4 columns, a coarser vertical lattice, and
    /// density only around the surface.
    pub fn lod_grid(
        &mut self,
        x: i32,
        z: i32,
        width: i32,
        side: usize,
        approximate: bool,
    ) -> Result<Vec<LodColumn>, GeneratorError> {
        self.pipeline
            .lod_grid(&mut self.state, x, z, width, side, approximate)
    }
    pub fn noise_biomes(&mut self, quarts: &[[i32; 3]]) -> Vec<Option<String>> {
        self.pipeline.noise_biomes(&mut self.state, quarts)
    }
    /// `NoiseBasedChunkGenerator.addDebugScreenInfo`: the `Density` line of
    /// the noise settings' `debug_functions` at the feet (none without them).
    pub fn debug_density_line(&self, feet: [i32; 3]) -> Option<String> {
        self.pipeline.debug_density_line(feet)
    }
    /// `BiomeSource.findClosestBiome3d`: the first sampled position whose
    /// noise biome passes `allowed`, spiralling out from the origin.
    pub fn find_closest_biome(&mut self, search: &BiomeSearch<'_>) -> Option<(String, [i32; 3])> {
        self.pipeline.find_closest_biome(&mut self.state, search)
    }
    /// `JigsawPlacement.addPieces` for `/place jigsaw`, drawing from the
    /// generation context's random (level seed and the position's chunk).
    pub fn jigsaw_pieces(&mut self, query: &JigsawQuery<'_>) -> Option<JigsawPieces> {
        self.pipeline.jigsaw_pieces(&mut self.state, query)
    }
    /// ConcentricRingsStructurePlacement chunks of a structure set (such as
    /// `minecraft:strongholds`), searched for now if nothing needed them yet.
    pub fn ring_positions(&mut self, set: &str) -> Option<Vec<ChunkPos>> {
        self.pipeline.ring_positions(&mut self.state, set)
    }
    /// STRUCTURE_STARTS: the structure starts owned by a chunk (cached).
    pub fn structure_starts(
        &mut self,
        pos: ChunkPos,
    ) -> Vec<Arc<super::structure::start::StructureStart>> {
        self.pipeline.structure_starts(&mut self.state, pos)
    }
    /// `Structure.generate` for `/place structure`: the start `structure`
    /// builds in `chunk`, whatever its placement set says about that chunk.
    pub fn structure_start(
        &mut self,
        structure: &str,
        chunk: ChunkPos,
    ) -> Option<Arc<super::structure::start::StructureStart>> {
        self.pipeline
            .structure_start(&mut self.state, structure, chunk)
    }
    /// Puts a saved start of `structure` (its pieces as a save lists them, in
    /// JSON) in `chunk`, in place of the one generation builds there, until
    /// the starts are forgotten. Harnesses place another game's layout.
    pub fn use_saved_start(
        &mut self,
        chunk: ChunkPos,
        structure: &str,
        pieces: &[serde_json::Value],
    ) -> Result<(), String> {
        (self.pipeline).use_saved_start(&mut self.state, chunk, structure, pieces)
    }
    /// Forget the cached structure starts: chunks generated afterwards get
    /// fresh starts, as in a new level that has not loaded the old chunks.
    pub fn forget_structure_starts(&mut self) {
        self.pipeline.forget_structure_starts();
    }
    #[cfg(test)]
    fn may_start_answers(&mut self, chunk: ChunkPos) -> Vec<(String, bool)> {
        self.pipeline.may_start_answers(&mut self.state, chunk)
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
    pub fn set_structure_mobs(&mut self, mobs: Arc<dyn StructureMobs>) {
        self.state.structure_mobs = Some(mobs);
    }
    /// Give back the density scratch buffers and the lookup caches, which only
    /// speed up the next chunks (a generating worker holds megabytes of them).
    /// Results do not change: they are made again on demand.
    pub fn release(&mut self) {
        let state = &mut self.state;
        for context in [
            &mut state.terrain_context,
            &mut state.biome_context,
            &mut state.carver_context,
        ] {
            context.clear_scratch();
        }
        state.carver_biomes = HashMap::new();
        state.structure_biomes = HashMap::new();
        state.heights = HashMap::new();
        state.decor_standins = Vec::new();
        state.decor_uniform_biomes = Vec::new();
    }
}

#[cfg(test)]
mod mod_density_tests;
#[cfg(test)]
mod region_tests;
#[cfg(test)]
mod tests {
    use super::super::biome_chunks::CACHE_LIMIT;
    use super::*;
    #[test]
    fn out_from_origin_preserves_order_and_handles_extreme_positive_steps() {
        assert_eq!(out_from_origin(4, 0, 8, 2), [4, 6, 2, 8, 0]);
        assert_eq!(out_from_origin(0, 0, 8, 3), [0, 3, 6]);
        assert_eq!(out_from_origin(8, 0, 8, 3), [8, 5, 2]);
        for (lower, upper) in [(-63, 320), (1, 256)] {
            for origin in [i32::MIN, 64, i32::MAX] {
                assert_eq!(
                    out_from_origin(origin, lower, upper, i32::MAX),
                    [origin.clamp(lower, upper)]
                );
            }
        }
        assert_eq!(
            out_from_origin(0, i32::MIN, i32::MAX, i32::MAX),
            [0, i32::MAX, -i32::MAX]
        );
        assert!(std::panic::catch_unwind(|| out_from_origin(0, 0, 8, 0)).is_err());
        assert!(std::panic::catch_unwind(|| out_from_origin(0, 8, 0, 1)).is_err());
    }

    #[test]
    fn settings_default_states_read_the_26_3_block_state_form() {
        let facts = br#"{"states": [
            ["minecraft:stone", false, false, "x", "minecraft:stone"],
            ["minecraft:jigsaw", false, false, "x", {"id": "minecraft:jigsaw", "properties": {"orientation": "north_up"}}],
            ["minecraft:jigsaw", false, false, "x", {"id": "minecraft:jigsaw", "properties": {"orientation": "down_north"}}]
        ], "defaults": {"minecraft:stone": 0, "minecraft:jigsaw": 1}}"#;
        let (registry, _) = BlockRegistry::from_facts_json_bytes(facts).unwrap();
        let codec =
            crate::registry::BlockStateJsonCodec::new(&registry, Default::default()).unwrap();
        let read = |raw: Value| state(&codec, &raw).map(|id| id.0);
        assert_eq!(read(Value::from("minecraft:stone")).unwrap(), 0);
        assert_eq!(
            read(serde_json::json!({"id": "minecraft:jigsaw", "properties": {"orientation": "down_north"}}))
                .unwrap(),
            2
        );
        assert_eq!(
            read(serde_json::json!({"id": "minecraft:jigsaw"})).unwrap(),
            1
        );
        assert!(read(serde_json::json!({"properties": {}})).is_err());
    }
    #[test]
    fn pack_tags_join_the_prepared_ones_and_broken_ones_are_dropped() {
        let raw = BTreeMap::from([(
            "minecraft:hill".to_owned(),
            r#"{"values":["minecraft:a"]}"#.to_owned(),
        )]);
        let known = |id: &str| id != "minecraft:gone";
        let mut ov = WorldgenOverrides::default();
        ov.add_tag(
            "biome_tags",
            "minecraft:hill",
            "file/p",
            r#"{"values":["minecraft:b"]}"#.into(),
        );
        ov.add_tag(
            "biome_tags",
            "p:broken",
            "file/p",
            r#"{"values":["minecraft:gone"]}"#.into(),
        );
        let mut dropped = Vec::new();
        let tags = build_tags(&raw, &ov, "biome_tags", &mut dropped, known).unwrap();
        assert_eq!(tags["minecraft:hill"], ["minecraft:a", "minecraft:b"]);
        assert!(!tags.contains_key("p:broken"));
        assert_eq!(dropped.len(), 1, "{dropped:?}");
        assert!(dropped[0].contains("p:broken"));
        // Without packs the prepared tags must all resolve.
        let broken = BTreeMap::from([(
            "p:broken".to_owned(),
            r#"{"values":["minecraft:gone"]}"#.to_owned(),
        )]);
        let none = WorldgenOverrides::default();
        assert!(build_tags(&broken, &none, "biome_tags", &mut dropped, known).is_err());
    }
    /// A pack's level on a generator nobody registered: opening it is an
    /// error for the caller to report, and the noise levels still build.
    #[test]
    fn a_level_stem_naming_an_unknown_generator_is_an_error() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry = Box::leak(Box::new(crate::level::data::load_registry(&root).unwrap()));
        let mut ov = WorldgenOverrides::default();
        ov.set(
            overrides::DIMENSIONS,
            "test:gone",
            r#"{"type":"minecraft:overworld","generator":{"type":"test:gone"}}"#.to_owned(),
        );
        let data = GeneratorData::load_with(&root.join("worldgen"), registry, &ov).unwrap();
        let open = |dimension| {
            OverworldGenerator::for_world(data.clone(), 0, "minecraft:normal", dimension, false)
        };
        let error = open("test:gone").err().expect("no such generator");
        assert!(
            error
                .to_string()
                .contains("test:gone is not a noise generator")
        );
        assert!(open("minecraft:overworld").is_ok());
    }
    #[test]
    fn a_pack_level_stem_names_taller_noise_which_clamps_to_the_build_height() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry = Box::leak(Box::new(crate::level::data::load_registry(&root).unwrap()));
        let dir = root.join("worldgen");
        let vanilla = GeneratorData::load(&dir, registry).unwrap();
        let mut settings: Value =
            serde_json::from_str(&vanilla.settings["minecraft:overworld"]).unwrap();
        settings["noise"]["height"] = 2096.into();
        let preset: Value =
            serde_json::from_str(&vanilla.world_presets["minecraft:normal"]).unwrap();
        let mut stem = preset["dimensions"]["minecraft:overworld"].clone();
        stem["generator"]["settings"] = "horizon:tall".into();
        let mut ov = WorldgenOverrides::default();
        ov.set("settings", "horizon:tall", settings.to_string());
        ov.set(
            overrides::DIMENSIONS,
            "minecraft:overworld",
            stem.to_string(),
        );
        let packed = GeneratorData::load_with(&dir, registry, &ov).unwrap();
        assert!(packed.warnings().is_empty(), "{:?}", packed.warnings());
        let (asked, kept) = OverworldGenerator::new(packed, 0)
            .unwrap()
            .noise_clamped()
            .expect("the pack's noise reaches past the build height");
        assert_eq!((asked.min_y, asked.height), (-64, 2096));
        assert_eq!((kept.min_y, kept.height), (-64, 384));
        let plain = OverworldGenerator::new(vanilla, 0).unwrap();
        assert!(plain.noise_clamped().is_none());
    }
    #[test]
    fn end_identity_surface_matches_normal_surface_and_rejects_custom_rules() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry = Box::leak(Box::new(crate::level::data::load_registry(&root).unwrap()));
        let data = GeneratorData::load(&root.join("worldgen"), registry).unwrap();
        let preset: Value = serde_json::from_str(&data.world_presets["minecraft:normal"]).unwrap();
        let settings: Value = serde_json::from_str(&data.settings["minecraft:end"]).unwrap();
        let mut pipeline = Pipeline::new(
            data.clone(),
            12345,
            LegacyRandom::new(12345),
            &preset["dimensions"]["minecraft:the_end"]["generator"],
            &settings,
            false,
            None,
            None,
        )
        .unwrap();
        let mut worker = OverworldGenerator::for_world(
            data.clone(),
            12345,
            "minecraft:normal",
            "minecraft:the_end",
            false,
        )
        .unwrap()
        .worker();
        assert!(pipeline.identity_end_surface(ChunkPos::new(0, 0)));
        for pos in [
            ChunkPos::new(0, 0),
            ChunkPos::new(128, 128),
            ChunkPos::new(-128, 192),
            ChunkPos::new(1_875_000, -1_875_000),
        ] {
            let actual = pipeline.generate(&mut worker.state, pos).unwrap();
            let rule = pipeline.rule.clone();
            // A singleton sequence evaluates the same block rule through the full surface path.
            pipeline.rule = MaterialRule::Sequence(vec![rule.clone()]);
            assert!(!pipeline.identity_end_surface(ChunkPos::new(0, 0)));
            let expected = pipeline.generate(&mut worker.state, pos).unwrap();
            pipeline.rule = rule;
            assert_eq!(
                actual.terrain.generation_stage(),
                expected.terrain.generation_stage()
            );
            for section in 0..actual.terrain.height() as usize / 16 {
                assert_eq!(
                    actual.terrain.section(section).unwrap().to_nbt().unwrap(),
                    expected.terrain.section(section).unwrap().to_nbt().unwrap()
                );
            }
            for kind in HeightmapType::ALL {
                assert_eq!(
                    actual.terrain.heightmap(kind).map(|m| m.raw_data()),
                    expected.terrain.heightmap(kind).map(|m| m.raw_data())
                );
            }
            assert_eq!(
                actual.terrain.postprocessing(),
                expected.terrain.postprocessing()
            );
            assert_eq!(
                actual.terrain.block_ticks().pack(0),
                expected.terrain.block_ticks().pack(0)
            );
            assert_eq!(
                actual.terrain.fluid_ticks().pack(0),
                expected.terrain.fluid_ticks().pack(0)
            );
            assert_eq!(actual.block_entities, expected.block_entities);
            assert_eq!(actual.entities, expected.entities);
            assert_eq!(actual.structures(), expected.structures());
        }
        assert!(!pipeline.identity_end_surface(ChunkPos::new(i32::MAX, 0)));
        let old_rule = pipeline.rule.clone();
        pipeline.rule = MaterialRule::Block(registry.default_state("minecraft:stone").unwrap());
        assert!(!pipeline.identity_end_surface(ChunkPos::new(0, 0)));
        pipeline.rule = old_rule;
        let old_fluid = pipeline.settings.default_fluid;
        pipeline.settings.default_fluid = registry.default_state("minecraft:stone").unwrap();
        assert!(!pipeline.identity_end_surface(ChunkPos::new(0, 0)));
        pipeline.settings.default_fluid = old_fluid;
        let old_source = std::mem::replace(
            &mut pipeline.source,
            ResolvedBiomeSource::Fixed(super::super::biome_source::FixedBiomeSource {
                biome: "minecraft:the_end".into(),
            }),
        );
        assert!(!pipeline.identity_end_surface(ChunkPos::new(0, 0)));
        pipeline.source = old_source;
        pipeline.aquifer = Some(std::array::from_fn(|_| {
            pipeline.router.get(RouterField::Erosion).clone()
        }));
        assert!(!pipeline.identity_end_surface(ChunkPos::new(0, 0)));
    }

    #[test]
    #[ignore = "daily"]
    fn uniform_surface_biomes_preserve_zoom_and_missing_corner_errors() {
        use super::super::biome_chunk::BiomeChunk;
        use std::convert::Infallible;

        for pos in [
            ChunkPos::new(0, 0),
            ChunkPos::new(-17, 23),
            ChunkPos::new(i32::MIN >> 4, i32::MAX >> 4),
        ] {
            for seed in [0, -1, i64::MIN] {
                for mixed in [false, true] {
                    let mut chunks = Vec::new();
                    for dz in -1..=1 {
                        for dx in -1..=1 {
                            let mut chunk = BiomeChunk::new(
                                ChunkPos::new(pos.x + dx, pos.z + dz),
                                -64,
                                128,
                                BiomeId(7),
                            )
                            .unwrap();
                            chunk
                                .fill(-64, 63, |[x, y, z]| {
                                    Ok::<_, Infallible>(if mixed {
                                        BiomeId(
                                            (x.div_euclid(3) + y.div_euclid(4) + z.div_euclid(2))
                                                .rem_euclid(5)
                                                as u32,
                                        )
                                    } else {
                                        BiomeId(7)
                                    })
                                })
                                .unwrap();
                            chunks.push(chunk);
                        }
                    }
                    let region = BiomeRegion::new(chunks, seed).unwrap();
                    let mut cache = UniformSurfaceBiomes::new(pos, -64, 128);
                    let mut hits = 0;
                    for x in 0..16 {
                        for z in 0..16 {
                            // Include clamped Y, out-of-cache Y, and revisit each
                            // cached proof in the ordinary descending traversal.
                            for y in (-70..70).rev() {
                                let p = BlockPos::new(pos.x * 16 + x, y, pos.z * 16 + z);
                                let shortcut = cache.get(&region, p);
                                hits += usize::from(shortcut.is_some());
                                let actual = shortcut.map_or_else(|| region.biome(p).copied(), Ok);
                                assert_eq!(actual, region.biome(p).copied());
                            }
                        }
                    }
                    if !mixed && pos.x != i32::MIN >> 4 {
                        assert!(hits > 0);
                    }
                }
            }
        }

        // Some unselected corners are missing. Probing them must neither invent
        // an error nor hide the original selected-corner error.
        let pos = ChunkPos::new(0, 0);
        let region =
            BiomeRegion::new(vec![BiomeChunk::new(pos, 0, 16, BiomeId(7)).unwrap()], -1).unwrap();
        let mut cache = UniformSurfaceBiomes::new(pos, 0, 16);
        let mut successes = 0;
        let mut failures = 0;
        for x in -2..18 {
            for z in -2..18 {
                let p = BlockPos::new(x, 7, z);
                let actual = cache
                    .get(&region, p)
                    .map_or_else(|| region.biome(p).copied(), Ok);
                assert_eq!(actual, region.biome(p).copied());
                successes += usize::from(actual.is_ok());
                failures += usize::from(actual.is_err());
            }
        }
        assert!(successes > 0 && failures > 0);
    }

    /// Reduced far detail paints only the top two blocks of a wide column
    /// (`material_depth`). Vanilla's surface rules reach eight blocks down, so
    /// Full detail layers dirt under grass; a tile 256 blocks across draws at
    /// most a block or two of each column's face, so Reduced leaves the rest as
    /// the default block. The second half of this test samples the same tile at
    /// Full detail to prove the first half is discriminating, not vacuous.
    #[test]
    fn reduced_far_detail_paints_only_the_top_of_a_wide_column() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .or_else(|| std::env::var_os("HORIZON_RESOURCES"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry: &'static BlockRegistry = Box::leak(Box::new(
            crate::level::data::load_registry(&root).expect("registry"),
        ));
        let data = GeneratorData::load(&root.join("worldgen"), registry).expect("worldgen data");
        let generator = OverworldGenerator::without_structures(data, 1234).expect("generator");
        let mut worker = generator.worker();
        // The block a column holds at `y`, where its spans cover it.
        let at = |spans: &[(i32, i32, BlockStateId)], y: i32| {
            spans
                .iter()
                .find(|&&(low, high, _)| low <= y && y < high)
                .map(|&(_, _, state)| state)
        };
        // Two depths inside the eight vanilla rules reach, both below the two
        // Reduced keeps. Only columns whose spans cover both are compared, so a
        // cave or an overhang between them is never read as a painted layer.
        let layered = |grid: &[LodColumn]| {
            let mut compared = 0;
            let mut differing = 0;
            for (surface, spans) in grid {
                let Some(surface) = surface else { continue };
                let (Some(shallow), Some(deep)) =
                    (at(spans, surface.height - 4), at(spans, surface.height - 8))
                else {
                    continue;
                };
                compared += 1;
                differing += usize::from(shallow != deep);
            }
            (compared, differing)
        };
        // 16 columns 16 blocks apart: a level 4 tile, which the gate covers.
        let (compared, differing) = layered(&worker.lod_grid(0, 0, 16, 16, true).expect("reduced"));
        assert!(compared > 64, "only {compared} columns to compare");
        assert_eq!(
            differing, 0,
            "{differing} of {compared} reduced columns still carry a painted layer below depth 2"
        );
        let (compared, differing) = layered(&worker.lod_grid(0, 0, 16, 16, false).expect("full"));
        assert!(compared > 64, "only {compared} columns to compare");
        assert!(
            differing > 0,
            "Full detail painted no layer below depth 2 either, so the check above proves nothing"
        );
    }
    #[test]
    fn far_surface_matches_undecorated_land_and_keeps_end_void() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .or_else(|| std::env::var_os("HORIZON_RESOURCES"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry: &'static BlockRegistry = Box::leak(Box::new(
            crate::level::data::load_registry(&root).expect("registry"),
        ));
        let data = GeneratorData::load(&root.join("worldgen"), registry).expect("worldgen data");
        let generator =
            OverworldGenerator::without_structures(data.clone(), 1234).expect("generator");
        let mut worker = generator.worker();
        let sample = worker.surface_sample(0, 0).expect("land");
        let (column_surface, spans) = worker.surface_column(0, 0);
        let column_surface = column_surface.expect("column land");
        assert_eq!(column_surface.height, sample.height);
        assert_eq!(column_surface.state, sample.state);
        assert_eq!(column_surface.water, sample.water);
        assert_eq!(column_surface.biome, sample.biome);
        assert!(spans.iter().all(|&(min, max, state)| {
            min < max && !generator.data().block_states().get(state).is_air()
        }));
        assert!(spans.windows(2).all(|pair| {
            pair[1].1 <= pair[0].0 && (pair[1].1 != pair[0].0 || pair[1].2 != pair[0].2)
        }));
        assert!(spans.iter().any(|&(min, max, state)| {
            min < sample.height && sample.height - 1 < max && state == sample.state
        }));
        assert!(sample.water.is_some_and(|(height, state)| {
            spans
                .iter()
                .any(|&(min, max, block)| min < height && height - 1 < max && block == state)
        }));
        let chunk = worker.generate_proto(ChunkPos::new(0, 0)).expect("terrain");
        assert_eq!(
            sample.water.map_or(sample.height, |(height, _)| height),
            chunk.surface_height(0, 0)
        );
        assert_eq!(sample.state, chunk.block(0, sample.height - 1, 0));
        assert!(
            sample.water.is_some(),
            "fixture includes underwater terrain"
        );
        let started = std::time::Instant::now();
        let mut exact = Vec::new();
        for z in 0..8 {
            for x in 0..8 {
                exact.push(worker.surface_column(-1024 + x * 32, z * 32));
            }
        }
        let dense_time = started.elapsed();
        let started = std::time::Instant::now();
        let direct = worker.lod_grid(-1024, 0, 32, 8, false).unwrap();
        for z in 0..8 {
            for x in 0..8 {
                let wx = -1024 + x * 32;
                let wz = z * 32;
                let (surface, spans) = &direct[(z * 8 + x) as usize];
                let expected = exact[(z * 8 + x) as usize].0;
                if let (Some(surface), Some(expected)) = (surface, expected) {
                    assert!(
                        (surface.height - expected.height).abs() <= 32,
                        "coarse height exceeds its vertical resolution at {wx} {wz}: {} vs {}",
                        surface.height,
                        expected.height
                    );
                    assert!(spans.iter().any(|&(min, max, state)| min < surface.height
                        && surface.height - 1 < max
                        && state == surface.state));
                }
                assert!(spans.iter().all(|&(min, max, _)| min < max));
                assert!(spans.windows(2).all(|pair| pair[1].1 <= pair[0].0));
            }
        }
        eprintln!(
            "64 far columns: dense={dense_time:?}, direct32={:?}",
            started.elapsed()
        );
        let end =
            OverworldGenerator::for_dimension(data, 1234, "minecraft:normal", "minecraft:the_end")
                .expect("End generator");
        let mut worker = end.worker();
        assert!(worker.surface_sample(0, 0).is_some());
        assert!(worker.surface_sample(256, 0).is_none());
        let (surface, spans) = worker.surface_column(256, 0);
        assert!(surface.is_none());
        assert!(spans.is_empty());
        let (surface, spans) = worker
            .lod_grid(256, 0, 32, 1, false)
            .unwrap()
            .pop()
            .unwrap();
        assert!(
            surface.is_none() && spans.is_empty(),
            "direct sampling retains End void"
        );
    }

    #[test]
    fn local_carver_cache_is_worker_owned_and_eviction_preserves_terrain() {
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry = Box::leak(Box::new(
            crate::level::data::load_registry(&root).expect("registry"),
        ));
        let data = GeneratorData::load(&root.join("worldgen"), registry).unwrap();
        let generator = OverworldGenerator::new(data.clone(), 12345).unwrap();
        let mut first = generator.worker();
        let mut second = generator.worker();
        let pos = ChunkPos::new(0, 0);
        let before = first.generate_proto(pos).unwrap();
        assert!(!first.state.carver_biomes.is_empty());
        assert!(second.state.carver_biomes.is_empty());
        second.generate_proto(pos).unwrap();
        assert_eq!(first.state.carver_biomes, second.state.carver_biomes);
        let expected = first.state.carver_biomes.clone();
        first.state.carver_biomes.clear();
        // Fill unrelated origins to force eviction on the next local miss.
        for x in 0..4096 {
            first.state.carver_biomes.insert((x, 100_000), BiomeId(0));
        }
        let after = first.generate_proto(pos).unwrap();
        assert_eq!(first.state.carver_biomes, expected);
        for y in -64..320 {
            for z in 0..16 {
                for x in 0..16 {
                    assert_eq!(before.block(x, y, z), after.block(x, y, z));
                    assert_eq!(before.biome_id_at(x, y, z), after.biome_id_at(x, y, z));
                }
            }
        }
        assert!(
            OverworldGenerator::new(data, -1)
                .unwrap()
                .worker()
                .state
                .carver_biomes
                .is_empty()
        );
        assert_eq!(second.state.carver_biomes, expected);
    }

    /// The prepared worldgen data, if this checkout has it.
    fn prepared_data() -> Option<Arc<GeneratorData>> {
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return None;
        }
        let registry: &'static BlockRegistry = Box::leak(Box::new(
            crate::level::data::load_registry(&root).expect("registry"),
        ));
        Some(GeneratorData::load(&root.join("worldgen"), registry).expect("worldgen data"))
    }

    /// A superflat world decorates with what `adjustGenerationSettings` keeps of its biome
    /// (26.3 bytecode): the lava lakes of `createLakesList` in the lakes step (0 and 1) when
    /// `lakes` is on; the biome's own features with `features` on, but those of the two
    /// structure steps (plains' monster rooms are underground structures); nothing of the
    /// biome when every layer is air, unless the biome is the_void.
    #[test]
    fn a_superflat_world_keeps_what_adjust_generation_settings_keeps() {
        let Some(data) = prepared_data() else {
            return;
        };
        use super::super::generator_decoration::{Decoration, FlatDecoration};
        let plains = ResolvedBiomeSource::Fixed(super::super::biome_source::FixedBiomeSource {
            biome: "minecraft:plains".into(),
        });
        let kept = |layer: &str, features, lakes, name: &str| {
            let void_gen = layer == "minecraft:air";
            let adjust = FlatDecoration::new(features, lakes, void_gen, false);
            let decoration = Decoration::new_with(&data, &plains, Some(adjust))
                .unwrap()
                .expect("decoration");
            decoration.seed(name).map(|(_, index, step)| (index, step))
        };
        let grass = "minecraft:grass_block";
        // The Overworld preset: lakes and features.
        assert_eq!(
            kept(grass, true, true, "minecraft:lake_lava_underground"),
            Some((0, 1))
        );
        assert_eq!(
            kept(grass, true, true, "minecraft:lake_lava_surface"),
            Some((1, 1))
        );
        assert_eq!(
            kept(grass, true, true, "minecraft:trees_plains").map(|s| s.1),
            Some(9)
        );
        assert_eq!(
            kept(grass, true, true, "minecraft:freeze_top_layer").map(|s| s.1),
            Some(10)
        );
        assert_eq!(kept(grass, true, true, "minecraft:monster_room"), None);
        // Lakes alone: no tree, no flower, no ore.
        assert_eq!(
            kept(grass, false, true, "minecraft:lake_lava_surface"),
            Some((1, 1))
        );
        assert_eq!(kept(grass, false, true, "minecraft:trees_plains"), None);
        assert_eq!(kept(grass, false, true, "minecraft:ore_iron_upper"), None);
        // Features alone: plains' own lakes step stays.
        assert_eq!(
            kept(grass, true, false, "minecraft:lake_lava_surface").map(|s| s.1),
            Some(1)
        );
        assert_eq!(
            kept(grass, true, false, "minecraft:ore_iron_upper").map(|s| s.1),
            Some(6)
        );
        // Neither: the steps run for structures, with no feature in them.
        assert_eq!(
            kept(grass, false, false, "minecraft:lake_lava_surface"),
            None
        );
        // Every layer air (`voidGen`) and a biome other than the_void: no biome features.
        assert_eq!(
            kept("minecraft:air", true, false, "minecraft:trees_plains"),
            None
        );
    }

    /// Seed 1234's strongholds where vanilla 26.3 puts them
    /// (VanillaStructureStartOracle with HORIZON_RINGS). Each search sampled
    /// its quarts through fresh resolvers, which broke climate ties their own
    /// way: three of the 128 were up to 6 chunks off.
    #[test]
    fn stronghold_rings_match_vanilla() {
        let Some(data) = prepared_data() else {
            return;
        };
        let generator = OverworldGenerator::new(data, 1234).expect("generator");
        let rings = generator
            .worker()
            .ring_positions("minecraft:strongholds")
            .expect("ring positions");
        assert_eq!(rings.len(), 128);
        for (i, x, z) in [
            (0, -100, -125),
            (1, 131, -12),
            (111, 1190, -560),
            (113, 1305, -112),
        ] {
            assert_eq!(rings[i], ChunkPos::new(x, z), "stronghold {i}");
        }
    }

    /// Seed 8675309's giant ruined portal starts in chunk (-26, 23) and lies
    /// wholly in (-26, 24): its own chunk still holds the start, as vanilla's
    /// getAllStarts does, though no piece reaches it.
    #[test]
    fn a_chunk_holds_its_starts_whose_pieces_lie_elsewhere() {
        let Some(data) = prepared_data() else {
            return;
        };
        let generator = OverworldGenerator::new(data, 8675309).expect("generator");
        let mut worker = generator.worker();
        let proto = worker
            .generate_proto(ChunkPos::new(-26, 23))
            .expect("proto");
        let own: Vec<_> = proto.own_starts().iter().map(|s| s.name.clone()).collect();
        assert!(
            own.iter().any(|n| n == "minecraft:ruined_portal_mountain"),
            "{own:?}"
        );
    }

    /// Chunks no ring search can end near are decided without the rings,
    /// which stay unsearched, and get the starts they get with them.
    #[test]
    fn chunks_far_from_the_rings_do_not_search_them() {
        let Some(data) = prepared_data() else {
            return;
        };
        let generator = OverworldGenerator::new(data.clone(), 4321).expect("generator");
        let mut worker = generator.worker();
        let summary = |worker: &mut GeneratorWorker, c: ChunkPos| -> Vec<(String, i32, i32)> {
            worker
                .structure_starts(c)
                .iter()
                .map(|s| (s.name.clone(), s.chunk.x, s.chunk.z))
                .collect()
        };
        let chunks: Vec<ChunkPos> = (-24..=24)
            .step_by(3)
            .flat_map(|x| (-24..=24).step_by(3).map(move |z| ChunkPos::new(x, z)))
            .collect();
        let before: Vec<_> = chunks.iter().map(|&c| summary(&mut worker, c)).collect();
        assert!(before.iter().any(|s| !s.is_empty()), "no starts to compare");
        let searched = || data.ring_cache.lock().unwrap()[0].1.get().is_some();
        assert!(!searched());
        let rings = worker
            .ring_positions("minecraft:strongholds")
            .expect("ring positions");
        assert_eq!(rings.len(), 128);
        assert!(searched());
        worker.forget_structure_starts();
        let after: Vec<_> = chunks.iter().map(|&c| summary(&mut worker, c)).collect();
        assert_eq!(before, after);
    }

    /// A map's structure search skips a chunk's starts when no wanted structure's start
    /// piece and biome fit there (#8669). Over the candidate chunks of every random-spread
    /// set around the camp TiesB2007's map went looking from, a skipped structure is never
    /// one the chunk's full starts hold, and some are skipped.
    #[test]
    fn may_start_never_skips_a_start_the_chunk_holds() {
        let Some(data) = prepared_data() else {
            return;
        };
        let generator = OverworldGenerator::new(data, -3292718948513116954).expect("generator");
        let mut worker = generator.worker();
        let (mut held, mut skipped) = (0, 0);
        for x in 212..332 {
            for z in -481..-361 {
                let chunk = ChunkPos::new(x, z);
                let checks = worker.may_start_answers(chunk);
                if checks.is_empty() {
                    continue;
                }
                let starts: Vec<String> = (worker.structure_starts(chunk).iter())
                    .map(|s| s.name.clone())
                    .collect();
                for (name, may_start) in checks {
                    if starts.contains(&name) {
                        assert!(
                            may_start,
                            "{name} starts in {chunk:?}, but its check skips it"
                        );
                        held += 1;
                    } else if !may_start {
                        skipped += 1;
                    }
                }
            }
        }
        assert!(held > 0 && skipped > 0, "held {held}, skipped {skipped}");
    }

    /// Generators of one seed and dimension share their ring positions;
    /// another seed or generator gets its own, and old entries go.
    #[test]
    fn ring_positions_are_shared_by_seed_and_dimension() {
        let Some(data) = prepared_data() else {
            return;
        };
        let first = OverworldGenerator::new(data.clone(), 1234).expect("generator");
        let again = OverworldGenerator::new(data.clone(), 1234).expect("generator");
        {
            let cache = data.ring_cache.lock().unwrap();
            assert_eq!(cache.len(), 1);
            // The cache's handle and the two generators'.
            assert_eq!(Arc::strong_count(&cache[0].1), 3);
        }
        drop((first, again));
        let a = data.rings_for(1, &serde_json::json!({ "settings": "a" }));
        assert!(Arc::ptr_eq(
            &a,
            &data.rings_for(1, &serde_json::json!({ "settings": "a" }))
        ));
        assert!(!Arc::ptr_eq(
            &a,
            &data.rings_for(2, &serde_json::json!({ "settings": "a" }))
        ));
        assert!(!Arc::ptr_eq(
            &a,
            &data.rings_for(1, &serde_json::json!({ "settings": "b" }))
        ));
        for seed in 10..10 + RING_CACHE_LIMIT as i64 {
            data.rings_for(seed, &serde_json::json!({ "settings": "a" }));
        }
        assert_eq!(data.ring_cache.lock().unwrap().len(), RING_CACHE_LIMIT);
        assert!(!Arc::ptr_eq(
            &a,
            &data.rings_for(1, &serde_json::json!({ "settings": "a" }))
        ));
    }

    /// Generators of every seed share their biome source's decoration, the
    /// one the world screens' preload built; another dimension gets its own.
    #[test]
    fn decoration_is_shared_by_biome_source() {
        let Some(data) = prepared_data() else {
            return;
        };
        data.prepare_decoration("minecraft:normal", "minecraft:overworld")
            .expect("decoration");
        let built = |i: usize| {
            let cache = data.decorations.lock().unwrap();
            let slot = cache[i].1.get().expect("built").clone();
            slot.expect("decoration").expect("feature resources")
        };
        let prepared = built(0);
        // The cache's handle and ours.
        assert_eq!(Arc::strong_count(&prepared), 2);
        let first = OverworldGenerator::new(data.clone(), 1234).expect("generator");
        let other = OverworldGenerator::new(data.clone(), -7).expect("generator");
        assert_eq!(Arc::strong_count(&prepared), 4);
        assert_eq!(data.decorations.lock().unwrap().len(), 1);
        let nether = OverworldGenerator::for_dimension(
            data.clone(),
            1234,
            "minecraft:normal",
            "minecraft:the_nether",
        )
        .expect("generator");
        assert_eq!(data.decorations.lock().unwrap().len(), 2);
        assert!(!Arc::ptr_eq(&prepared, &built(1)));
        drop((first, other, nether));
        assert_eq!(Arc::strong_count(&prepared), 2);
    }

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
    #[test]
    fn neighbour_work_cleanup_keeps_active_keys_and_releases_idle_keys() {
        let work = ChunkWork::new();
        let first = work.at((0, 0));
        let held = first.lock().unwrap();
        let same = work.at((0, 0));
        assert!(Arc::ptr_eq(&first, &same));
        assert!(!Arc::ptr_eq(&first, &work.at((1, 0))));
        // Sweep many expired weak entries while a builder and a prospective
        // waiter retain their key. Cleanup must not split their coordination.
        for x in 1..8192 {
            drop(work.at((x, 1)));
        }
        assert!(
            work.0
                .iter()
                .map(|s| s.lock().unwrap().len())
                .sum::<usize>()
                <= 4096
        );
        assert!(Arc::ptr_eq(&same, &work.at((0, 0))));
        let weak = Arc::downgrade(&first);
        drop(held);
        drop((first, same));
        assert!(weak.upgrade().is_none());
        let next = work.at((0, 0));
        assert_eq!(Arc::strong_count(&next), 1);
    }

    /// Emptying the shared caches (every worker idle) changes no chunk: the
    /// chunks generated after it have the same blocks and biomes.
    #[test]
    fn released_caches_generate_the_same_chunks() {
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry = Box::leak(Box::new(
            crate::level::data::load_registry(&root).expect("registry"),
        ));
        let data = GeneratorData::load(&root.join("worldgen"), registry).expect("worldgen data");
        let preset: Value = serde_json::from_str(&data.world_presets["minecraft:normal"]).unwrap();
        let generator_json = &preset["dimensions"]["minecraft:overworld"]["generator"];
        let settings: Value =
            serde_json::from_str(&data.settings[generator_json["settings"].as_str().unwrap()])
                .unwrap();
        let generator = OverworldGenerator {
            pipeline: Arc::new(
                Pipeline::new(
                    data,
                    12345,
                    XoroshiroRandom::new(12345),
                    generator_json,
                    &settings,
                    true,
                    Some((-64, 384)),
                    None,
                )
                .expect("pipeline"),
            ),
            noise_clamped: None,
        };
        let contents = |chunk: &ProtoChunk| {
            let (x0, z0) = (chunk.position().x * 16, chunk.position().z * 16);
            let mut out = Vec::new();
            for y in (-64..320).step_by(3) {
                for z in 0..16 {
                    for x in 0..16 {
                        out.push(chunk.block(x0 + x, y, z0 + z).0);
                        out.push(chunk.biome_id_at(x0 + x, y, z0 + z).0);
                    }
                }
            }
            out
        };
        let mut worker = generator.worker();
        let chunks = [
            ChunkPos::new(11, -11),
            ChunkPos::new(12, -11),
            ChunkPos::new(-3, 7),
        ];
        let before: Vec<_> = chunks
            .iter()
            .map(|&p| contents(&worker.generate_proto(p).unwrap()))
            .collect();
        assert!(generator.cache_census().biome_chunks > 0);
        generator.release_caches();
        worker.release();
        let census = generator.cache_census();
        assert_eq!((census.biome_chunks, census.carver_entries), (0, 0));
        // The other way round, so a neighbour's biomes come from the cache
        // where they were made before, and the reverse.
        for (&p, before) in chunks.iter().zip(&before).rev() {
            assert_eq!(
                &contents(&worker.generate_proto(p).unwrap()),
                before,
                "chunk {p:?}"
            );
        }
    }

    /// Equal concurrent misses share one biome volume and the same mutable
    /// structure pieces. Exercise cold ring startup before work coordination,
    /// then cache eviction and explicit start invalidation with old Arcs alive.
    #[test]
    fn concurrent_neighbours_share_results_and_survive_invalidation() {
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry = Box::leak(Box::new(
            crate::level::data::load_registry(&root).expect("registry"),
        ));
        let data = GeneratorData::load(&root.join("worldgen"), registry).expect("worldgen data");
        let preset: Value = serde_json::from_str(&data.world_presets["minecraft:normal"]).unwrap();
        let generator_json = &preset["dimensions"]["minecraft:overworld"]["generator"];
        let settings: Value =
            serde_json::from_str(&data.settings[generator_json["settings"].as_str().unwrap()])
                .unwrap();
        let pipeline = Arc::new(
            Pipeline::new(
                data,
                12345,
                XoroshiroRandom::new(12345),
                generator_json,
                &settings,
                true,
                Some((-64, 384)),
                None,
            )
            .expect("pipeline"),
        );
        let generator = OverworldGenerator {
            pipeline: pipeline.clone(),
            noise_clamped: None,
        };
        let barrier = std::sync::Barrier::new(6);
        let pos = ChunkPos::new(11, -11); // ancient city for this seed
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..6)
                .map(|_| {
                    scope.spawn(|| {
                        let mut worker = generator.worker();
                        barrier.wait();
                        let biomes = pipeline.biome_chunk(&mut worker.state, pos).unwrap();
                        let starts = pipeline.structure_starts(&mut worker.state, pos);
                        (biomes, starts)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        let (biomes, starts) = &results[0];
        assert!(!starts.is_empty(), "fixture must exercise shared pieces");
        for (b, s) in &results[1..] {
            assert!(Arc::ptr_eq(biomes, b));
            assert_eq!(starts.len(), s.len());
            for (first, other) in starts.iter().zip(s) {
                assert!(Arc::ptr_eq(first, other));
            }
        }
        // Two generations of other chunks push this one out of the cache.
        {
            let mut cache = pipeline.biome_cache.lock().unwrap();
            for x in 0..CACHE_LIMIT as i32 {
                cache.insert((x, 10000), biomes.clone());
            }
            assert!(cache.len() <= CACHE_LIMIT);
        }
        let mut worker = generator.worker();
        let rebuilt = pipeline.biome_chunk(&mut worker.state, pos).unwrap();
        assert!(!Arc::ptr_eq(biomes, &rebuilt));
        assert!(pipeline.biome_cache.lock().unwrap().len() <= CACHE_LIMIT);
        for section in 0..24 {
            assert_eq!(biomes.section(section), rebuilt.section(section));
        }
        worker.forget_structure_starts();
        let fresh = worker.structure_starts(pos);
        assert_eq!(starts.len(), fresh.len());
        for (old, new) in starts.iter().zip(&fresh) {
            assert!(!Arc::ptr_eq(old, new));
            assert_eq!(old.name, new.name);
            assert_eq!(old.bounding_box, new.bounding_box);
        }
        let again = worker.structure_starts(pos);
        for (first, hit) in fresh.iter().zip(&again) {
            assert!(Arc::ptr_eq(first, hit));
        }
    }

    /// The resources the worldgen tests read.
    fn test_root() -> std::path::PathBuf {
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .or_else(|| std::env::var_os("HORIZON_RESOURCES"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            })
    }
    #[test]
    #[ignore = "daily"]
    fn saved_starts_match_generated_layouts() {
        use super::super::structure::start::Piece;
        use horizon_core::nbt::Compound;
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("resources"),
        );
        let registry = Box::leak(Box::new(crate::level::data::load_registry(&root).unwrap()));
        let data = GeneratorData::load(&root.join("worldgen"), registry).unwrap();
        for seed in [12345, 1234] {
            for (dimension, names) in [
                (
                    "minecraft:overworld",
                    &[
                        "village_plains",
                        "village_desert",
                        "village_savanna",
                        "village_snowy",
                        "village_taiga",
                        "pillager_outpost",
                        "ancient_city",
                        "trail_ruins",
                        "trial_chambers",
                        "shipwreck",
                        "shipwreck_beached",
                        "ocean_ruin_warm",
                        "ocean_ruin_cold",
                        "igloo",
                        "ruined_portal",
                        "mansion",
                        "monument",
                        "mineshaft",
                        "stronghold",
                    ][..],
                ),
                (
                    "minecraft:the_nether",
                    &["bastion_remnant", "nether_fossil", "fortress"][..],
                ),
                ("minecraft:the_end", &["end_city"][..]),
            ] {
                let generator = OverworldGenerator::for_dimension(
                    data.clone(),
                    seed,
                    "minecraft:normal",
                    dimension,
                )
                .unwrap();
                let mut worker = generator.worker();
                let mut originals = Vec::new();
                // A naturally generated area, including unsupported starts sharing its chunks.
                if seed == 12345 && dimension == "minecraft:overworld" {
                    let starts = worker.structure_starts(ChunkPos::new(11, -11));
                    assert!(starts.iter().any(|s| s.name == "minecraft:ancient_city"));
                    originals.extend(starts);
                }
                for (i, name) in names.iter().enumerate() {
                    let name = format!("minecraft:{name}");
                    let pos = if dimension == "minecraft:the_end" {
                        ChunkPos::new(-15, -76)
                    } else {
                        ChunkPos::new(i as i32 * 3, 0)
                    };
                    // A monument needs open ocean around it, which this small area may not
                    // have: search a cell per spacing step. `structure_start` generates the
                    // structure wherever its own checks pass, not only in the set's chunk.
                    let search: Vec<ChunkPos> = if name == "minecraft:monument" {
                        (0..1024)
                            .map(|i| {
                                ChunkPos::new(
                                    pos.x + (i % 32 - 16) * 32,
                                    pos.z + (i / 32 - 16) * 32,
                                )
                            })
                            .collect()
                    } else {
                        std::iter::once(pos)
                            .chain((1..64).map(|i| ChunkPos::new(pos.x + i % 8, pos.z + i / 8)))
                            .collect()
                    };
                    originals.push(
                        search
                            .into_iter()
                            .find_map(|pos| worker.structure_start(&name, pos))
                            .unwrap_or_else(|| panic!("{seed} {name}")),
                    );
                }
                let mut saved = HashMap::<(i32, i32), Compound>::new();
                worker.forget_structure_starts();
                // A fortress start's first piece is built through the `BridgeCrossing`
                // constructor, so vanilla saves it as `nebcr`, which the registry reads
                // back as a plain bridge crossing: only the kind's name differs.
                let kinds = |s: String| s.replace("kind: Start,", "kind: BridgeCrossing,");
                for original in &mut originals {
                    // Locating a generated start can increment its count. A nonzero value also
                    // proves that loading did not silently fall back to seed-identical generation.
                    if original.pieces.iter().all(|p| {
                        matches!(p, Piece::Pool(_))
                            || matches!(
                                p.type_name(),
                                "minecraft:shipwreck"
                                    | "minecraft:orp"
                                    | "minecraft:iglu"
                                    | "minecraft:rupo"
                                    | "minecraft:wmp"
                                    | "minecraft:nefos"
                                    | "minecraft:ecp"
                                    | "minecraft:omb"
                            )
                            // The mineshaft, fortress and stronghold pieces read their own tags.
                            || ["minecraft:ms", "minecraft:ne", "minecraft:sh"]
                                .iter()
                                .any(|prefix| p.type_name().starts_with(prefix))
                    }) {
                        Arc::get_mut(original)
                            .expect("uncached placement start")
                            .references = 3;
                    }
                    saved
                        .entry((original.chunk.x, original.chunk.z))
                        .or_default()
                        .insert(original.name.as_str().into(), original.save().unwrap());
                }
                generator
                    .set_saved_starts(Arc::new(move |pos| saved.get(&(pos.x, pos.z)).cloned()));
                for original in &originals {
                    let starts = worker.structure_starts(original.chunk);
                    let loaded = starts
                        .iter()
                        .find(|s| s.name == original.name)
                        .expect("saved or fallback start");
                    assert_eq!(
                        loaded.save(),
                        original.save(),
                        "{seed} {}: every saved field",
                        original.name
                    );
                    assert_eq!(
                        kinds(format!("{loaded:?}")),
                        kinds(format!("{original:?}")),
                        "{seed} {}: every runtime field",
                        original.name
                    );
                    let again = generator.worker().structure_starts(original.chunk);
                    assert!(
                        again.iter().any(|s| Arc::ptr_eq(s, loaded)),
                        "workers share admitted starts"
                    );
                }
            }
        }
    }
    /// The block registry the worldgen bundle needs. `GeneratorData` keeps it
    /// for as long as the generator lives, so it is shared, not freed.
    static TEST_REGISTRY: std::sync::LazyLock<BlockRegistry> = std::sync::LazyLock::new(|| {
        crate::level::data::load_registry(&test_root()).expect("registry")
    });

    /// The world presets with their own noise settings generate the terrain
    /// those settings describe: the amplified and large-biome presets are not
    /// the overworld's terrain under another name.
    #[test]
    fn world_presets_use_their_own_noise_settings() {
        let root = test_root();
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let data =
            GeneratorData::load(&root.join("worldgen"), &TEST_REGISTRY).expect("worldgen data");
        let surface = |preset: &str| {
            let generator = OverworldGenerator::for_world(
                data.clone(),
                1234,
                preset,
                "minecraft:overworld",
                false,
            )
            .expect("generator");
            let mut worker = generator.worker();
            (0..32)
                .map(|i| {
                    worker
                        .surface_sample(i * 137, i * 91)
                        .expect("surface")
                        .height
                })
                .collect::<Vec<_>>()
        };
        let normal = surface("minecraft:normal");
        assert!(normal.iter().all(|height| (0..384).contains(height)));
        assert_ne!(normal, surface("minecraft:amplified"));
        assert_ne!(normal, surface("minecraft:large_biomes"));
    }

    struct ProtoSnapshot {
        position: ChunkPos,
        stage: PreLightChunkStage,
        decorated: bool,
        min_y: i32,
        height: i32,
        sections: Vec<horizon_core::nbt::Tag>,
        biomes: Vec<[BiomeId; 64]>,
        heightmaps: Vec<Option<Vec<u64>>>,
        postprocessing: Vec<Vec<u16>>,
        block_ticks: Vec<horizon_core::ticks::SavedTick>,
        fluid_ticks: Vec<horizon_core::ticks::SavedTick>,
        block_entities: Vec<([i32; 3], horizon_core::nbt::Compound)>,
        entities: Vec<horizon_core::nbt::Compound>,
        structure_starts_len: usize,
    }

    impl ProtoSnapshot {
        fn of(chunk: &ProtoChunk) -> Self {
            let section_count = (chunk.terrain.height() / 16) as usize;
            assert!(
                chunk.terrain.section(section_count).is_none(),
                "unexpected extra terrain section"
            );
            let mut sections = Vec::with_capacity(section_count);
            let mut biomes = Vec::with_capacity(section_count);
            for idx in 0..section_count {
                sections.push(chunk.terrain.section(idx).unwrap().to_nbt().unwrap());
                biomes.push(*chunk.biomes.section(idx).unwrap());
            }
            let heightmaps = HeightmapType::ALL
                .into_iter()
                .map(|kind| chunk.terrain.heightmap(kind).map(|m| m.raw_data().to_vec()))
                .collect();
            Self {
                position: chunk.position(),
                stage: chunk.terrain.generation_stage(),
                decorated: chunk.decorated,
                min_y: chunk.terrain.min_y(),
                height: chunk.terrain.height(),
                sections,
                biomes,
                heightmaps,
                postprocessing: chunk.terrain.postprocessing().to_vec(),
                block_ticks: chunk.terrain.block_ticks().pack(0).to_vec(),
                fluid_ticks: chunk.terrain.fluid_ticks().pack(0).to_vec(),
                block_entities: chunk.block_entities.clone(),
                entities: chunk.entities.clone(),
                structure_starts_len: chunk.structure_starts.len(),
            }
        }
    }

    fn assert_proto_snapshot_equal(a: &ProtoSnapshot, b: &ProtoSnapshot, label: &str) {
        assert_eq!(a.position, b.position, "{label}: chunk position");
        assert_eq!(a.stage, b.stage, "{label}: generation stage");
        assert_eq!(a.decorated, b.decorated, "{label}: decorated flag");
        assert_eq!(a.min_y, b.min_y, "{label}: min_y");
        assert_eq!(a.height, b.height, "{label}: height");
        assert_eq!(a.sections.len(), b.sections.len(), "{label}: section count");
        for (idx, ((sa, sb), (ba, bb))) in a
            .sections
            .iter()
            .zip(&b.sections)
            .zip(a.biomes.iter().zip(&b.biomes))
            .enumerate()
        {
            assert_eq!(sa, sb, "{label}: section {idx} NBT");
            assert_eq!(ba, bb, "{label}: biome section {idx}");
        }
        for ((ha, hb), kind) in a
            .heightmaps
            .iter()
            .zip(&b.heightmaps)
            .zip(HeightmapType::ALL)
        {
            assert_eq!(ha, hb, "{label}: heightmap {kind:?}");
        }
        assert_eq!(
            a.postprocessing, b.postprocessing,
            "{label}: postprocessing"
        );
        assert_eq!(a.block_ticks, b.block_ticks, "{label}: block ticks");
        assert_eq!(a.fluid_ticks, b.fluid_ticks, "{label}: fluid ticks");
        assert_eq!(
            a.block_entities, b.block_entities,
            "{label}: block entities"
        );
        assert_eq!(a.entities, b.entities, "{label}: entities");
        assert_eq!(
            a.structure_starts_len, b.structure_starts_len,
            "{label}: structure starts count"
        );
    }

    fn assert_proto_chunk_byte_equal(a: &ProtoChunk, b: &ProtoChunk, label: &str) {
        assert_proto_snapshot_equal(&ProtoSnapshot::of(a), &ProtoSnapshot::of(b), label);
    }

    /// Private temporary directory created atomically with a non-recursive
    /// `DirBuilder` (`0o700` on Unix) so a test fixture never reuses or deletes
    /// a directory it did not create.
    struct TempDirGuard(std::path::PathBuf);

    impl TempDirGuard {
        fn create(prefix: &str) -> Self {
            use std::collections::hash_map::RandomState;
            use std::hash::BuildHasher;
            use std::sync::atomic::{AtomicU64, Ordering};
            use std::time::{SystemTime, UNIX_EPOCH};

            static SEQ: AtomicU64 = AtomicU64::new(0);

            let base = std::env::temp_dir();
            let pid = std::process::id();
            Self::create_with(&base, |attempt| {
                let seq = SEQ.fetch_add(1, Ordering::Relaxed);
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let nonce = RandomState::new().hash_one((pid, seq, attempt, nanos));
                format!("{prefix}-{pid}-{seq}-{nanos:x}-{nonce:016x}")
            })
        }

        fn create_with(base: &Path, mut next_name: impl FnMut(u64) -> String) -> Self {
            let builder = Self::private_dir_builder();
            for attempt in 0..1024u64 {
                let path = base.join(next_name(attempt));
                match builder.create(&path) {
                    Ok(()) => return Self(path),
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(err) => panic!(
                        "failed to create temporary directory {}: {err}",
                        path.display()
                    ),
                }
            }
            panic!(
                "failed to create a unique temporary directory under {} after 1024 attempts",
                base.display()
            );
        }

        fn private_dir_builder() -> fs::DirBuilder {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = fs::DirBuilder::new();
                builder.mode(0o700);
                builder
            }
            #[cfg(not(unix))]
            {
                fs::DirBuilder::new()
            }
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn create_subdir(&self, name: &str) -> std::path::PathBuf {
            let path = self.0.join(name);
            Self::private_dir_builder()
                .create(&path)
                .unwrap_or_else(|err| {
                    panic!(
                        "failed to create temporary subdirectory {}: {err}",
                        path.display()
                    )
                });
            path
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct SyntheticReuseFixture {
        data: Arc<GeneratorData>,
        registry: &'static BlockRegistry,
        _dir: TempDirGuard,
    }

    fn build_synthetic_reuse_data() -> SyntheticReuseFixture {
        use horizon_core::geometry::{Axis, VoxelShape};
        use horizon_core::nbt::{JavaString, Limits, NamedTag, Tag, write_named};
        static REGISTRY: OnceLock<&'static BlockRegistry> = OnceLock::new();
        static META: OnceLock<(
            Vec<Tag>,
            Vec<(bool, bool)>,
            Vec<i32>,
            Vec<(&'static str, i8)>,
        )> = OnceLock::new();

        let (states_nbt, flags, cached_shapes, block_props) = META.get_or_init(|| {
            let mut states_nbt = Vec::new();
            let mut defaults: Vec<(&'static str, i32)> = Vec::new();
            let mut flags = Vec::new();
            let mut cached_shapes = Vec::new();
            let mut block_props: Vec<(&'static str, i8)> = Vec::new();

            let mut add_simple =
                |name: &'static str, is_air: bool, is_solid: bool, prop_flags: i8| {
                    let idx = states_nbt.len() as i32;
                    states_nbt.push(Tag::compound([("id", Tag::String(name.into()))]));
                    defaults.push((name, idx));
                    flags.push((is_air, false));
                    cached_shapes.push(i32::from(is_solid));
                    block_props.push((name, prop_flags));
                };

            // Bit flags for block-properties.nbt:
            // 4 = replaceable, 8 = liquid, 16 = can_occlude.
            add_simple("minecraft:air", true, false, 4);
            add_simple("minecraft:void_air", true, false, 4);
            add_simple("minecraft:cave_air", true, false, 4);

            for solid_name in [
                "minecraft:stone",
                "minecraft:deepslate",
                "minecraft:bedrock",
                "minecraft:dirt",
                "minecraft:sand",
                "minecraft:gravel",
                "minecraft:coal_ore",
                "minecraft:iron_ore",
                "minecraft:raw_iron_block",
                "minecraft:tuff",
                "minecraft:cobblestone",
                "minecraft:end_stone",
                "minecraft:obsidian",
                "minecraft:ice",
                "minecraft:packed_ice",
                "minecraft:snow_block",
                "minecraft:chest",
                "minecraft:terracotta",
                "minecraft:orange_terracotta",
                "minecraft:yellow_terracotta",
                "minecraft:brown_terracotta",
                "minecraft:red_terracotta",
                "minecraft:white_terracotta",
                "minecraft:light_gray_terracotta",
            ] {
                add_simple(solid_name, false, true, 16);
            }

            add_simple("minecraft:torch", false, false, 0);

            for fluid_name in ["minecraft:water", "minecraft:lava"] {
                let first = states_nbt.len() as i32;
                defaults.push((fluid_name, first));
                for level in 0..16 {
                    states_nbt.push(Tag::compound([
                        ("id", Tag::String(fluid_name.into())),
                        (
                            "properties",
                            Tag::compound([(
                                "level",
                                Tag::String(level.to_string().as_str().into()),
                            )]),
                        ),
                    ]));
                    flags.push((false, true));
                    cached_shapes.push(0);
                }
                block_props.push((fluid_name, 4 | 8));
            }

            for snowy_name in [
                "minecraft:grass_block",
                "minecraft:mycelium",
                "minecraft:podzol",
            ] {
                let first = states_nbt.len() as i32;
                defaults.push((snowy_name, first));
                for snowy in ["false", "true"] {
                    states_nbt.push(Tag::compound([
                        ("id", Tag::String(snowy_name.into())),
                        (
                            "properties",
                            Tag::compound([("snowy", Tag::String(snowy.into()))]),
                        ),
                    ]));
                    flags.push((false, false));
                    cached_shapes.push(1);
                }
                block_props.push((snowy_name, 16));
            }

            // minecraft:snow (layers 1..=8)
            {
                let first = states_nbt.len() as i32;
                defaults.push(("minecraft:snow", first));
                for layers in 1..=8 {
                    states_nbt.push(Tag::compound([
                        ("id", Tag::String("minecraft:snow".into())),
                        (
                            "properties",
                            Tag::compound([(
                                "layers",
                                Tag::String(layers.to_string().as_str().into()),
                            )]),
                        ),
                    ]));
                    flags.push((false, false));
                    cached_shapes.push(0);
                }
                block_props.push(("minecraft:snow", 4));
            }

            // minecraft:pale_moss_carpet (required by BlockPlacementBindings)
            {
                let mut default_idx = states_nbt.len() as i32;
                for bottom in ["false", "true"] {
                    for north in ["none", "low", "tall"] {
                        for east in ["none", "low", "tall"] {
                            for south in ["none", "low", "tall"] {
                                for west in ["none", "low", "tall"] {
                                    if bottom == "true"
                                        && north == "none"
                                        && east == "none"
                                        && south == "none"
                                        && west == "none"
                                    {
                                        default_idx = states_nbt.len() as i32;
                                    }
                                    states_nbt.push(Tag::compound([
                                        ("id", Tag::String("minecraft:pale_moss_carpet".into())),
                                        (
                                            "properties",
                                            Tag::compound([
                                                ("bottom", Tag::String(bottom.into())),
                                                ("east", Tag::String(east.into())),
                                                ("north", Tag::String(north.into())),
                                                ("south", Tag::String(south.into())),
                                                ("west", Tag::String(west.into())),
                                            ]),
                                        ),
                                    ]));
                                    flags.push((false, false));
                                    cached_shapes.push(0);
                                }
                            }
                        }
                    }
                }
                defaults.push(("minecraft:pale_moss_carpet", default_idx));
                block_props.push(("minecraft:pale_moss_carpet", 0));
            }

            let registry = BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                ("states", Tag::List(states_nbt.clone())),
                (
                    "defaults",
                    Tag::compound(defaults.into_iter().map(|(k, v)| (k, Tag::Int(v)))),
                ),
            ]))
            .unwrap();
            let _ = REGISTRY.set(Box::leak(Box::new(registry)));
            (states_nbt, flags, cached_shapes, block_props)
        });
        let registry = *REGISTRY.get().unwrap();

        let guard = TempDirGuard::create("horizon-worker-reuse-test");
        let dir = guard.path();
        let blocks_dir = guard.create_subdir("blocks");

        let write_nbt = |path: &Path, tag: Tag| {
            let out = write_named(
                &NamedTag {
                    name: JavaString::default(),
                    tag,
                },
                Limits::default(),
            )
            .unwrap();
            std::fs::write(path, out).unwrap();
        };
        let shape_nbt = |shape: &VoxelShape, canonical: i8, occupied: Vec<u8>| {
            let (dmin, dmax) = shape.discrete_bounds();
            let axes = shape.cube_coordinate_axes();
            let cube_axes = i8::from(axes[0]) | (i8::from(axes[1]) << 1) | (i8::from(axes[2]) << 2);
            let coords = |axis| {
                Tag::LongArray(
                    shape
                        .coordinates(axis)
                        .iter()
                        .map(|v| v.to_bits() as i64)
                        .collect(),
                )
            };
            Tag::compound([
                ("canonical", Tag::Byte(canonical)),
                ("x", coords(Axis::X)),
                ("y", coords(Axis::Y)),
                ("z", coords(Axis::Z)),
                ("occupied", Tag::ByteArray(occupied)),
                ("cube_axes", Tag::Byte(cube_axes)),
                ("discrete_min", Tag::IntArray(dmin.to_vec())),
                ("discrete_max", Tag::IntArray(dmax.to_vec())),
                ("cube", Tag::Byte(i8::from(shape.is_cube()))),
            ])
        };
        let shapes = vec![
            shape_nbt(&VoxelShape::empty(), 1, Vec::new()),
            shape_nbt(&VoxelShape::block(), 2, vec![1]),
        ];
        let collision_tag = |kind: Option<&str>| {
            let mut entries: Vec<(&str, Tag)> = vec![
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                ("states", Tag::List(states_nbt.clone())),
                ("shapes", Tag::List(shapes.clone())),
                ("cached_shapes", Tag::IntArray(cached_shapes.clone())),
            ];
            if let Some(kind) = kind {
                entries.push(("Kind", Tag::String(kind.into())));
            }
            Tag::compound(entries)
        };
        write_nbt(
            &blocks_dir.join("cached-collision.nbt"),
            collision_tag(None),
        );
        write_nbt(
            &blocks_dir.join("cached-support.nbt"),
            collision_tag(Some("block_support")),
        );
        write_nbt(
            &blocks_dir.join("block-properties.nbt"),
            Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                ("states", Tag::List(states_nbt.clone())),
                (
                    "blocks",
                    Tag::compound(
                        block_props
                            .iter()
                            .map(|&(name, bits)| (name, Tag::Byte(bits))),
                    ),
                ),
                ("occlusion", collision_tag(None)),
            ]),
        );
        std::fs::write(
            blocks_dir.join("survival-rules.json"),
            r#"{"minecraft:snow":"SnowLayer","minecraft:torch":"CenterBelow"}"#,
        )
        .unwrap();
        std::fs::write(
            blocks_dir.join("survival-extra-definitions.json"),
            r#"{"ignition_odds":{},"fence_gates":[]}"#,
        )
        .unwrap();
        std::fs::write(
            blocks_dir.join("placement-definitions.json"),
            r#"{"double_plants":[],"mossy_carpets":[]}"#,
        )
        .unwrap();

        let write_json = |name: &str, value: Value| {
            std::fs::write(dir.join(name), serde_json::to_string(&value).unwrap()).unwrap();
        };
        let write_bundle = |name: &str, entries: &[(&str, Value)]| {
            let mut map = serde_json::Map::new();
            for (k, v) in entries {
                map.insert(
                    (*k).to_owned(),
                    Value::String(serde_json::to_string(v).unwrap()),
                );
            }
            std::fs::write(
                dir.join(name),
                serde_json::to_string(&Value::Object(map)).unwrap(),
            )
            .unwrap();
        };

        write_json("block_flags.json", serde_json::to_value(flags).unwrap());

        let noise_def = |base_octave: i32| {
            serde_json::json!({
                "base_octave": base_octave,
                "octave_count": 2,
                "base_amplitude": 1.0
            })
        };
        write_bundle(
            "noises.json",
            &[
                ("minecraft:clay_bands_offset", noise_def(-4)),
                ("minecraft:surface", noise_def(-4)),
                ("minecraft:surface_secondary", noise_def(-3)),
                ("minecraft:badlands_surface", noise_def(-4)),
                ("minecraft:badlands_pillar", noise_def(-3)),
                ("minecraft:badlands_pillar_roof", noise_def(-4)),
                ("minecraft:iceberg_surface", noise_def(-4)),
                ("minecraft:iceberg_pillar", noise_def(-3)),
                ("minecraft:iceberg_pillar_roof", noise_def(-4)),
                ("minecraft:aquifer_barrier", noise_def(-3)),
                ("minecraft:aquifer_fluid_level_floodedness", noise_def(-4)),
                ("minecraft:aquifer_fluid_level_spread", noise_def(-4)),
                ("minecraft:aquifer_lava", noise_def(-2)),
                ("minecraft:terrain", noise_def(-5)),
                ("minecraft:vein", noise_def(-3)),
            ],
        );
        write_bundle("densities.json", &[]);
        write_bundle("conditions.json", &[]);

        let surface_rule = serde_json::json!({
            "type": "minecraft:sequence",
            "sequence": [
                {
                    "type": "minecraft:condition",
                    "if_true": {
                        "type": "minecraft:vertical_gradient",
                        "random_name": "minecraft:bedrock_floor",
                        "true_at_and_below": {"above_bottom": 0},
                        "false_at_and_above": {"above_bottom": 4}
                    },
                    "then_run": {"type": "minecraft:block", "result_state": "minecraft:bedrock"}
                },
                {
                    "type": "minecraft:condition",
                    "if_true": {"type": "minecraft:above_preliminary_surface"},
                    "then_run": {
                        "type": "minecraft:sequence",
                        "sequence": [
                            {
                                "type": "minecraft:condition",
                                "if_true": {
                                    "type": "minecraft:stone_depth",
                                    "offset": 0,
                                    "add_surface_depth": false,
                                    "secondary_depth_range": 0,
                                    "surface_type": "floor"
                                },
                                "then_run": {
                                    "type": "minecraft:sequence",
                                    "sequence": [
                                        {
                                            "type": "minecraft:condition",
                                            "if_true": {
                                                "type": "minecraft:biome",
                                                "biome_is": ["minecraft:eroded_badlands"]
                                            },
                                            "then_run": {"type": "minecraft:bandlands"}
                                        },
                                        {
                                            "type": "minecraft:condition",
                                            "if_true": {
                                                "type": "minecraft:biome",
                                                "biome_is": ["minecraft:frozen_ocean"]
                                            },
                                            "then_run": {"type": "minecraft:block", "result_state": "minecraft:gravel"}
                                        },
                                        {
                                            "type": "minecraft:condition",
                                            "if_true": {
                                                "type": "minecraft:noise_threshold",
                                                "noise": "minecraft:surface",
                                                "min_threshold": 0.4,
                                                "max_threshold": 1.0
                                            },
                                            "then_run": {"type": "minecraft:block", "result_state": "minecraft:sand"}
                                        },
                                        {"type": "minecraft:block", "result_state": "minecraft:grass_block"}
                                    ]
                                }
                            },
                            {
                                "type": "minecraft:condition",
                                "if_true": {
                                    "type": "minecraft:stone_depth",
                                    "offset": 2,
                                    "add_surface_depth": true,
                                    "secondary_depth_range": 0,
                                    "surface_type": "floor"
                                },
                                "then_run": {"type": "minecraft:block", "result_state": "minecraft:dirt"}
                            }
                        ]
                    }
                },
                {
                    "type": "minecraft:ore_vein",
                    "ore_block": "minecraft:iron_ore",
                    "raw_ore_block": "minecraft:raw_iron_block",
                    "filler_block": "minecraft:tuff",
                    "raw_ore_chance": 0.2,
                    "density": {
                        "type": "minecraft:mul",
                        "left": {
                            "type": "minecraft:gradient",
                            "axis": "y",
                            "from_coordinate": -32,
                            "to_coordinate": 8,
                            "from_value": 0.8,
                            "to_value": -0.2
                        },
                        "right": {
                            "type": "minecraft:noise",
                            "noise": "minecraft:vein",
                            "xz_scale": 1.0,
                            "y_scale": 1.0
                        }
                    },
                    "richness": {
                        "type": "minecraft:noise",
                        "noise": "minecraft:vein",
                        "xz_scale": 2.0,
                        "y_scale": 2.0
                    },
                    "filler_gap": {
                        "type": "minecraft:noise",
                        "noise": "minecraft:surface_secondary",
                        "xz_scale": 1.0,
                        "y_scale": 1.0
                    }
                },
                {
                    "type": "minecraft:condition",
                    "if_true": {
                        "type": "minecraft:vertical_gradient",
                        "random_name": "minecraft:deepslate",
                        "true_at_and_below": {"absolute": -4},
                        "false_at_and_above": {"absolute": 4}
                    },
                    "then_run": {"type": "minecraft:block", "result_state": "minecraft:deepslate"}
                }
            ]
        });
        write_bundle(
            "rules.json",
            &[("minecraft:synthetic_surface", surface_rule)],
        );

        write_bundle(
            "carvers.json",
            &[
                (
                    "minecraft:cave",
                    serde_json::json!({
                        "type": "minecraft:cave",
                        "probability": 0.35,
                        "y": {
                            "type": "minecraft:uniform",
                            "min_inclusive": {"absolute": -24},
                            "max_inclusive": {"absolute": 48}
                        },
                        "count": 2,
                        "thickness": 1.0,
                        "room_vertical_radius_multiplier": 1.0,
                        "horizontal_radius_multiplier": 1.0,
                        "vertical_radius_multiplier": 1.0,
                        "floor_level": -0.7
                    }),
                ),
                (
                    "minecraft:canyon",
                    serde_json::json!({
                        "type": "minecraft:canyon",
                        "probability": 0.25,
                        "y": {
                            "type": "minecraft:uniform",
                            "min_inclusive": {"absolute": -20},
                            "max_inclusive": {"absolute": 44}
                        },
                        "vertical_rotation": 0.0,
                        "shape": {
                            "distance_factor": 1.0,
                            "thickness": 1.0,
                            "width_smoothness": 3,
                            "horizontal_radius_factor": 1.0,
                            "vertical_radius_default_factor": 1.0,
                            "vertical_radius_center_factor": 0.0,
                            "y_scale": 3.0
                        }
                    }),
                ),
            ],
        );

        let biome_features = serde_json::json!([
            [],
            [],
            [],
            [],
            [],
            [],
            ["minecraft:ore_coal"],
            [],
            [],
            ["minecraft:bonus_chest"],
            ["minecraft:freeze_top_layer"]
        ]);
        let biome_carvers = serde_json::json!(["minecraft:cave", "minecraft:canyon"]);
        write_bundle(
            "biomes.json",
            &[
                (
                    "minecraft:plains",
                    serde_json::json!({
                        "temperature": 0.8,
                        "downfall": 0.4,
                        "has_precipitation": true,
                        "carvers": biome_carvers,
                        "features": biome_features
                    }),
                ),
                (
                    "minecraft:snowy_taiga",
                    serde_json::json!({
                        "temperature": -0.5,
                        "downfall": 0.4,
                        "has_precipitation": true,
                        "carvers": biome_carvers,
                        "features": biome_features
                    }),
                ),
                (
                    "minecraft:eroded_badlands",
                    serde_json::json!({
                        "temperature": 2.0,
                        "downfall": 0.0,
                        "has_precipitation": false,
                        "carvers": biome_carvers,
                        "features": biome_features
                    }),
                ),
                (
                    "minecraft:frozen_ocean",
                    serde_json::json!({
                        "temperature": 0.0,
                        "downfall": 0.5,
                        "has_precipitation": true,
                        "temperature_modifier": "frozen",
                        "carvers": biome_carvers,
                        "features": biome_features
                    }),
                ),
            ],
        );
        write_bundle("biome_tags.json", &[]);

        let motion_blocks = serde_json::json!({
            "values": [
                "minecraft:stone",
                "minecraft:deepslate",
                "minecraft:bedrock",
                "minecraft:dirt",
                "minecraft:grass_block",
                "minecraft:mycelium",
                "minecraft:podzol",
                "minecraft:sand",
                "minecraft:gravel",
                "minecraft:coal_ore",
                "minecraft:iron_ore",
                "minecraft:raw_iron_block",
                "minecraft:tuff",
                "minecraft:cobblestone",
                "minecraft:end_stone",
                "minecraft:obsidian",
                "minecraft:ice",
                "minecraft:packed_ice",
                "minecraft:snow_block",
                "minecraft:chest",
                "minecraft:terracotta",
                "minecraft:orange_terracotta",
                "minecraft:yellow_terracotta",
                "minecraft:brown_terracotta",
                "minecraft:red_terracotta",
                "minecraft:white_terracotta",
                "minecraft:light_gray_terracotta"
            ]
        });
        write_bundle(
            "block_tags.json",
            &[
                (
                    "minecraft:blocks_motion_in_heightmap",
                    motion_blocks.clone(),
                ),
                (
                    "minecraft:blocks_motion_in_heightmap_no_leaves",
                    motion_blocks,
                ),
                (
                    "minecraft:uncarvable",
                    serde_json::json!({"values": ["minecraft:bedrock"]}),
                ),
                (
                    "minecraft:cannot_support_snow_layer",
                    serde_json::json!({"values": ["minecraft:ice", "minecraft:packed_ice"]}),
                ),
            ],
        );
        write_bundle(
            "fluid_tags.json",
            &[
                (
                    "minecraft:water",
                    serde_json::json!({"values": ["minecraft:water", "minecraft:flowing_water"]}),
                ),
                (
                    "minecraft:lava",
                    serde_json::json!({"values": ["minecraft:lava", "minecraft:flowing_lava"]}),
                ),
            ],
        );

        write_bundle(
            "features.json",
            &[
                (
                    "minecraft:ore_coal",
                    serde_json::json!({
                        "type": "minecraft:ore",
                        "size": 8,
                        "discard_chance_on_air_exposure": 0.0,
                        "targets": [{
                            "target": {
                                "predicate_type": "minecraft:block_match",
                                "block": "minecraft:stone"
                            },
                            "state": "minecraft:coal_ore"
                        }]
                    }),
                ),
                (
                    "minecraft:bonus_chest",
                    serde_json::json!({
                        "type": "minecraft:bonus_chest",
                        "config": {}
                    }),
                ),
                (
                    "minecraft:freeze_top_layer",
                    serde_json::json!({
                        "type": "minecraft:freeze_top_layer",
                        "config": {}
                    }),
                ),
            ],
        );
        write_bundle(
            "placed_features.json",
            &[
                (
                    "minecraft:ore_coal",
                    serde_json::json!({
                        "feature": "minecraft:ore_coal",
                        "placement": [
                            {"type": "minecraft:count", "count": 6},
                            {"type": "minecraft:in_square"},
                            {
                                "type": "minecraft:height_range",
                                "height": {
                                    "type": "minecraft:uniform",
                                    "min_inclusive": {"absolute": -32},
                                    "max_inclusive": {"absolute": 48}
                                }
                            },
                            {"type": "minecraft:biome"}
                        ]
                    }),
                ),
                (
                    "minecraft:bonus_chest",
                    serde_json::json!({
                        "feature": "minecraft:bonus_chest",
                        "placement": [
                            {"type": "minecraft:in_square"},
                            {"type": "minecraft:heightmap", "heightmap": "MOTION_BLOCKING_NO_LEAVES"},
                            {"type": "minecraft:biome"}
                        ]
                    }),
                ),
                (
                    "minecraft:freeze_top_layer",
                    serde_json::json!({
                        "feature": "minecraft:freeze_top_layer",
                        "placement": [{"type": "minecraft:biome"}]
                    }),
                ),
            ],
        );

        // Terrain oscillates across sea_level (32) and sea_level + 17 (49), so
        // warm/cold lakes, hills, caves, canyons, aquifers, and snow/ice all fire.
        let terrain_density = serde_json::json!({
            "type": "minecraft:add",
            "left": {
                "type": "minecraft:gradient",
                "axis": "y",
                "from_coordinate": 16,
                "to_coordinate": 60,
                "from_value": 1.0,
                "to_value": -1.0
            },
            "right": {
                "type": "minecraft:mul",
                "left": 0.75,
                "right": {
                    "type": "minecraft:noise",
                    "noise": "minecraft:terrain",
                    "xz_scale": 1.0,
                    "y_scale": 0.5
                }
            }
        });
        let surface_level = serde_json::json!({
            "type": "minecraft:add",
            "left": 38.0,
            "right": {
                "type": "minecraft:mul",
                "left": 14.0,
                "right": {
                    "type": "minecraft:noise",
                    "noise": "minecraft:terrain",
                    "xz_scale": 1.0,
                    "y_scale": 0.0
                }
            }
        });
        write_bundle(
            "settings.json",
            &[(
                "minecraft:synthetic_overworld",
                serde_json::json!({
                    "sea_level": 32,
                    "default_block": "minecraft:stone",
                    "default_fluid": "minecraft:water",
                    "legacy_random_source": false,
                    "noise": {"min_y": -32, "height": 96},
                    "noise_router": {
                        "temperature": 0.0,
                        "vegetation": 0.0,
                        "continents": 0.0,
                        "erosion": 0.0,
                        "depth": 0.0,
                        "ridges": 0.0,
                        "chunk_surface_level": surface_level,
                        "final_density": terrain_density
                    },
                    "aquifers": {
                        "barrier": {
                            "type": "minecraft:noise",
                            "noise": "minecraft:aquifer_barrier",
                            "xz_scale": 1.0,
                            "y_scale": 0.5
                        },
                        "fluid_level_floodedness": {
                            "type": "minecraft:noise",
                            "noise": "minecraft:aquifer_fluid_level_floodedness",
                            "xz_scale": 1.0,
                            "y_scale": 0.67
                        },
                        "fluid_level_spread": {
                            "type": "minecraft:noise",
                            "noise": "minecraft:aquifer_fluid_level_spread",
                            "xz_scale": 1.0,
                            "y_scale": 0.714
                        },
                        "lava": {
                            "type": "minecraft:noise",
                            "noise": "minecraft:aquifer_lava",
                            "xz_scale": 1.0,
                            "y_scale": 1.0
                        },
                        "exclusion": 0.0,
                        "surface_level": surface_level
                    },
                    "material_rule": "minecraft:synthetic_surface"
                }),
            )],
        );
        write_bundle(
            "dimension_types.json",
            &[(
                "minecraft:overworld",
                serde_json::json!({"min_y": -32, "height": 96}),
            )],
        );
        // Scale = 2 gives 4-chunk-wide checkerboard tiles: interior centers are
        // uniform single-biome 3×3s while boundary centers span mixed biome bitsets.
        write_bundle(
            "world_presets.json",
            &[(
                "minecraft:normal",
                serde_json::json!({
                    "dimensions": {
                        "minecraft:overworld": {
                            "type": "minecraft:overworld",
                            "generator": {
                                "type": "minecraft:noise",
                                "settings": "minecraft:synthetic_overworld",
                                "biome_source": {
                                    "type": "minecraft:checkerboard",
                                    "scale": 2,
                                    "biomes": [
                                        "minecraft:plains",
                                        "minecraft:snowy_taiga",
                                        "minecraft:eroded_badlands",
                                        "minecraft:frozen_ocean"
                                    ]
                                }
                            }
                        }
                    }
                }),
            )],
        );

        let data = GeneratorData::load(dir, registry).unwrap();
        assert!(data.warnings().is_empty(), "{:?}", data.warnings());
        SyntheticReuseFixture {
            data,
            registry,
            _dir: guard,
        }
    }

    fn neighborhood_positions(center: ChunkPos) -> [ChunkPos; 9] {
        std::array::from_fn(|i| {
            ChunkPos::new(
                center.x.wrapping_add(i as i32 % 3 - 1),
                center.z.wrapping_add(i as i32 / 3 - 1),
            )
        })
    }

    fn verify_worker_reuse_on_centers(
        generator: &OverworldGenerator,
        centers: &[ChunkPos],
        passes: &[&[usize]],
    ) -> (Vec<[ProtoChunk; 9]>, Vec<DecorateStats>) {
        // Baseline: a brand-new GeneratorWorker for every proto chunk and every
        // decorated 3×3 neighborhood. Snapshot each proto chunk before decoration
        // so we verify both proto and decorated equality without generating the
        // baseline twice.
        let mut fresh_proto: BTreeMap<(i32, i32), ProtoSnapshot> = BTreeMap::new();
        let mut fresh_decorated: Vec<[ProtoChunk; 9]> = Vec::with_capacity(centers.len());
        let mut fresh_stats: Vec<DecorateStats> = Vec::with_capacity(centers.len());
        for &center in centers {
            let positions = neighborhood_positions(center);
            let mut region =
                std::array::from_fn(|i| generator.worker().generate_proto(positions[i]).unwrap());
            for (pos, chunk) in positions.iter().zip(region.iter()) {
                fresh_proto.insert((pos.x, pos.z), ProtoSnapshot::of(chunk));
            }
            let stats = generator.worker().decorate(&mut region).unwrap();
            fresh_decorated.push(region);
            fresh_stats.push(stats);
        }

        // Candidate: ONE reused GeneratorWorker across multiple shuffled passes
        // of both proto generation and decoration.
        let mut reused = generator.worker();
        let all_positions: Vec<ChunkPos> = centers
            .iter()
            .flat_map(|&c| neighborhood_positions(c))
            .collect();

        for (pass_idx, &center_order) in passes.iter().enumerate() {
            assert_eq!(center_order.len(), centers.len());
            let mut proto_order: Vec<usize> = (0..all_positions.len()).collect();
            let proto_len = proto_order.len();
            let stride = if pass_idx.is_multiple_of(2) { 7 } else { 11 };
            let offset = (pass_idx * 5 + 3) % proto_len;
            proto_order.sort_by_key(|&i| (i * stride + offset) % proto_len);

            let mut reused_proto: BTreeMap<(i32, i32), ProtoChunk> = BTreeMap::new();
            for idx in proto_order {
                let pos = all_positions[idx];
                let chunk = reused.generate_proto(pos).unwrap();
                assert_proto_snapshot_equal(
                    &fresh_proto[&(pos.x, pos.z)],
                    &ProtoSnapshot::of(&chunk),
                    &format!("pass {pass_idx} proto ({}, {})", pos.x, pos.z),
                );
                reused_proto.insert((pos.x, pos.z), chunk);
            }

            for &center_idx in center_order {
                let center = centers[center_idx];
                let positions = neighborhood_positions(center);
                let mut region = std::array::from_fn(|i| {
                    reused_proto
                        .remove(&(positions[i].x, positions[i].z))
                        .unwrap()
                });
                let stats = reused.decorate(&mut region).unwrap();
                assert_eq!(
                    stats, fresh_stats[center_idx],
                    "pass {pass_idx} decorate stats center ({}, {})",
                    center.x, center.z
                );
                for (slot, (want, got)) in fresh_decorated[center_idx]
                    .iter()
                    .zip(region.iter())
                    .enumerate()
                {
                    assert_proto_chunk_byte_equal(
                        want,
                        got,
                        &format!(
                            "pass {pass_idx} decorated center ({}, {}) slot {slot}",
                            center.x, center.z
                        ),
                    );
                }
            }
            assert_eq!(reused.state.decor_standins.len(), 9);
            assert!(!reused.state.decor_uniform_biomes.is_empty());
        }
        reused.release();
        assert!(reused.state.decor_standins.is_empty());
        assert!(reused.state.decor_uniform_biomes.is_empty());
        (fresh_decorated, fresh_stats)
    }

    #[test]
    fn temp_dir_guard_retries_existing_paths_and_cleans_up_on_drop() {
        let outer = TempDirGuard::create("horizon-temp-guard-test");
        let outer_path = outer.path().to_path_buf();
        assert!(outer_path.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::symlink_metadata(&outer_path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }

        let existing = outer.create_subdir("colliding");
        let sentinel = existing.join("keep.txt");
        fs::write(&sentinel, "sentinel").unwrap();

        let inner = TempDirGuard::create_with(outer.path(), |attempt| {
            if attempt == 0 {
                "colliding".to_owned()
            } else {
                format!("fresh-{attempt}")
            }
        });
        let inner_path = inner.path().to_path_buf();
        assert_eq!(inner_path, outer.path().join("fresh-1"));
        assert!(inner_path.is_dir());

        drop(inner);
        assert!(!inner_path.exists());
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "sentinel");

        drop(outer);
        assert!(!outer_path.exists());
    }

    #[test]
    fn worker_reuse_matches_fresh_workers_for_proto_and_decorated_chunks() {
        let fixture = build_synthetic_reuse_data();
        let generator = OverworldGenerator::for_world(
            Arc::clone(&fixture.data),
            12345,
            "minecraft:normal",
            "minecraft:overworld",
            false,
        )
        .unwrap();

        // 3 disjoint 3×3 neighborhoods (27 proto chunks, 3 decorated regions)
        // covering all 4 biomes: uniform plains (1, 1), uniform snowy_taiga
        // (5, 1), and a mixed eroded_badlands + frozen_ocean boundary (7, 5).
        let centers = [
            ChunkPos::new(1, 1),
            ChunkPos::new(5, 1),
            ChunkPos::new(7, 5),
        ];
        let order_a = [2, 0, 1];
        let order_b = [1, 2, 0];
        let (decorated, decorate_stats) =
            verify_worker_reuse_on_centers(&generator, &centers, &[&order_a, &order_b]);
        for stats in &decorate_stats {
            assert_eq!(
                stats.failed, 0,
                "unexpected failed features: {:?}",
                stats.skipped
            );
            assert_eq!(
                stats.unsupported, 0,
                "unexpected unsupported features: {:?}",
                stats.skipped
            );
        }

        // Verify the synthetic world actually exercised surface rules, ores,
        // freezing/snow, and block entities so the equality test is non-vacuous.
        let coal_ore = fixture
            .registry
            .default_state("minecraft:coal_ore")
            .unwrap();
        let grass = fixture
            .registry
            .default_state("minecraft:grass_block")
            .unwrap();
        let deepslate = fixture
            .registry
            .default_state("minecraft:deepslate")
            .unwrap();
        let bedrock = fixture.registry.default_state("minecraft:bedrock").unwrap();
        let ice = fixture.registry.default_state("minecraft:ice").unwrap();
        let snow = fixture.registry.default_state("minecraft:snow").unwrap();
        let terracotta = fixture
            .registry
            .default_state("minecraft:terracotta")
            .unwrap();

        let mut saw_block_entities = false;
        let mut saw_coal_ore = false;
        let mut saw_grass = false;
        let mut saw_deepslate = false;
        let mut saw_bedrock = false;
        let mut saw_freeze_or_snow = false;
        let mut saw_terracotta = false;
        for region in &decorated {
            for chunk in region {
                if !chunk.block_entities.is_empty() {
                    saw_block_entities = true;
                }
                let ox = chunk.position().x * 16;
                let oz = chunk.position().z * 16;
                for y in -32..64 {
                    for lx in 0..16 {
                        for lz in 0..16 {
                            let b = chunk.terrain.block(ox + lx, y, oz + lz);
                            if b == coal_ore {
                                saw_coal_ore = true;
                            } else if b == grass {
                                saw_grass = true;
                            } else if b == deepslate {
                                saw_deepslate = true;
                            } else if b == bedrock {
                                saw_bedrock = true;
                            } else if b == ice || b == snow {
                                saw_freeze_or_snow = true;
                            } else if b == terracotta {
                                saw_terracotta = true;
                            }
                        }
                    }
                }
            }
        }
        assert!(saw_block_entities, "expected bonus_chest block entities");
        assert!(saw_coal_ore, "expected ore_coal feature blocks");
        assert!(saw_grass, "expected grass_block from surface rule");
        assert!(saw_deepslate, "expected deepslate from surface rule");
        assert!(saw_bedrock, "expected bedrock from surface rule");
        assert!(
            saw_freeze_or_snow,
            "expected ice or snow from freeze_top_layer"
        );
        assert!(
            saw_terracotta,
            "expected terracotta from eroded_badlands bandlands"
        );
    }

    #[test]
    fn decoration_error_preserves_caller_chunks_and_standin_pool() {
        let fixture = build_synthetic_reuse_data();
        let generator = OverworldGenerator::for_world(
            Arc::clone(&fixture.data),
            12345,
            "minecraft:normal",
            "minecraft:overworld",
            false,
        )
        .unwrap();

        // Populate the worker's recycled stand-in pool with 9 stand-ins from (1, 1).
        let mut worker = generator.worker();
        let positions_a = neighborhood_positions(ChunkPos::new(1, 1));
        let mut region_a = std::array::from_fn(|i| worker.generate_proto(positions_a[i]).unwrap());
        let _ = worker.decorate(&mut region_a).unwrap();
        assert_eq!(worker.state.decor_standins.len(), 9);
        assert!(!worker.state.decor_uniform_biomes.is_empty());

        // Build a second 3×3 neighborhood at (5, 1) and replace slot 0's terrain
        // with a terrain-stage chunk at the same position but mismatched height
        // (16 instead of 96). This passes the 3×3 position and stage checks and
        // trips the TerrainRegion::new InvalidChunks precondition check.
        let positions_b = neighborhood_positions(ChunkPos::new(5, 1));
        let mut bad_region =
            std::array::from_fn(|i| worker.generate_proto(positions_b[i]).unwrap());
        let min_y = bad_region[0].terrain.min_y();
        let mut mismatched = TerrainChunk::new(
            bad_region[0].position(),
            min_y,
            16,
            fixture.registry,
            fixture.data.block_states().clone(),
        )
        .unwrap();
        mismatched.set_generation_stage(PreLightChunkStage::Terrain);
        bad_region[0].terrain = mismatched;

        let before: Vec<ProtoSnapshot> = bad_region.iter().map(ProtoSnapshot::of).collect();
        let err = worker
            .decorate(&mut bad_region)
            .expect_err("mismatched chunk height must fail decoration");
        assert!(
            err.to_string().contains("InvalidChunks"),
            "unexpected error: {err}"
        );

        // Every caller chunk must remain byte-identical to its pre-call snapshot
        // (no recycled stand-ins from (1, 1) left in caller chunks).
        for (slot, (want, got)) in before.iter().zip(bad_region.iter()).enumerate() {
            assert_proto_snapshot_equal(
                want,
                &ProtoSnapshot::of(got),
                &format!("failed decorate slot {slot}"),
            );
        }
        assert_eq!(worker.state.decor_standins.len(), 9);

        // Subsequent decoration of a valid neighborhood on the same worker must
        // still match a fresh worker byte-for-byte.
        let mut fresh_region =
            std::array::from_fn(|i| generator.worker().generate_proto(positions_b[i]).unwrap());
        let fresh_stats = generator.worker().decorate(&mut fresh_region).unwrap();

        let mut reused_region =
            std::array::from_fn(|i| worker.generate_proto(positions_b[i]).unwrap());
        let reused_stats = worker.decorate(&mut reused_region).unwrap();
        assert_eq!(reused_stats, fresh_stats);
        for (slot, (want, got)) in fresh_region.iter().zip(reused_region.iter()).enumerate() {
            assert_proto_chunk_byte_equal(want, got, &format!("post-error slot {slot}"));
        }

        worker.release();
        assert!(worker.state.decor_standins.is_empty());
        assert!(worker.state.decor_uniform_biomes.is_empty());
    }

    #[test]
    fn worker_reuse_matches_fresh_workers_on_overworld_bundle() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry = Box::leak(Box::new(crate::level::data::load_registry(&root).unwrap()));
        let data = GeneratorData::load(&root.join("worldgen"), registry).unwrap();
        let generator = OverworldGenerator::new(data, 12345).unwrap();
        let centers = [ChunkPos::new(0, 0), ChunkPos::new(3, 0)];
        let order_a = [1, 0];
        let order_b = [0, 1];
        let _ = verify_worker_reuse_on_centers(&generator, &centers, &[&order_a, &order_b]);
    }
}
