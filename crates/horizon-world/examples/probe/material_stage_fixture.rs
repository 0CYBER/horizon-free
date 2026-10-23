//! Original transport for complete surface-stage chunk comparison.
use crate::support::common::tag;
use horizon_core::{
    biome::{BiomeZoom, obfuscate_seed},
    carving::CarvingMask,
    density::{ConstantSampler, SamplerContext},
    noise::{DensityVolume, Noise},
    position::{BlockPos, ChunkPos},
    random::{LegacyRandom, PositionalRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::{
    DATA_VERSION,
    heightmap::{HeightmapState, HeightmapType},
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId, BlockStateJsonCodec},
    worldgen::{
        DecodeError,
        anchor::WorldGenerationContext,
        aquifer::{
            AquiferConfig, AquiferError, DisabledAquifer, GlobalFluidPicker, NoiseBasedAquifer,
            RegistryAquiferBlockStates,
        },
        biome_chunk::BiomeChunk,
        biome_source::{BiomeSourceCatalog, BiomeSourceResolver},
        biome_temperature::{BiomeTemperature, BiomeTemperatureSettings},
        carver::{CarverCatalog, WorldCarver, generate_carving_mask},
        carving::{CarvingBlocks, apply_carving_mask},
        decode_noise,
        density::DensityRegistry,
        material_condition::{
            MaterialConditionRegistry, MaterialNoiseSources, MaterialRuleError,
            MaterialRuleLibrary, MaterialSurfaceQueries,
        },
        material_extensions::{ExtensionBlocks, ExtensionNoises, MaterialExtensions},
        material_stage::{SurfaceInputs, TopMaterialInputs, build_surface, top_material},
        material_surface::{ClayBandBlocks, MaterialSurface},
        router::{NoiseRouter, RouterField},
        tags::TagDefinitions,
        terrain::{TerrainAquifer, TerrainBlockStates, TerrainChunk, fill_noise},
    },
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
};
#[derive(Clone, Copy)]
struct State {
    air: bool,
    air_block: bool,
    fluid: bool,
    motion: bool,
    no_leaves: bool,
}
impl HeightmapState for State {
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
struct Properties<'a>(&'a [State]);
impl TerrainBlockStates for Properties<'_> {
    type State = State;
    fn heightmap_state(&self, id: BlockStateId) -> State {
        self.0[id.0 as usize]
    }
}
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
type BiomeNeighborhood = BTreeMap<(i32, i32), BiomeChunk<Arc<str>>>;
type Temperatures = Arc<Mutex<BTreeMap<String, BiomeTemperature>>>;
struct Surface<P: PositionalRandom> {
    noises: MaterialNoiseSources<P, MaterialSurface<P>>,
    choices: Vec<Arc<str>>,
    zoom: Option<BiomeZoom>,
    neighborhood: Option<BiomeNeighborhood>,
    possible: Option<BTreeSet<String>>,
    temperatures: Temperatures,
    trace: Arc<Mutex<Vec<String>>>,
}
impl<P: PositionalRandom> MaterialSurfaceQueries for Surface<P> {
    fn surface_depth(&mut self, x: i32, z: i32) -> i32 {
        self.noises.surface_depth(x, z)
    }
    fn surface_secondary(&mut self, x: i32, z: i32) -> f64 {
        self.noises.surface_secondary(x, z)
    }
    fn band(&mut self, p: BlockPos) -> Result<BlockStateId, MaterialRuleError> {
        self.noises.band(p)
    }
    fn bind_noise(&mut self, name: &str) -> Result<Arc<dyn Noise>, DecodeError> {
        self.noises.bind_noise(name)
    }
    fn possible_biomes(&self) -> Option<BTreeSet<String>> {
        self.possible.clone()
    }
    fn biome(&mut self, p: BlockPos) -> Result<Option<Arc<str>>, MaterialRuleError> {
        let [x, y, z] = self
            .zoom
            .map_or([p.x, p.y, p.z], |zoom| zoom.quart_at_block(p));
        let hash = x
            .wrapping_mul(31)
            .wrapping_add(y.wrapping_mul(7))
            .wrapping_add(z.wrapping_mul(17));
        let name = if let Some(chunks) = &self.neighborhood {
            chunks
                .get(&(x >> 2, z >> 2))
                .ok_or(MaterialRuleError::MissingBiome)?
                .noise_biome([x, y, z])
                .clone()
        } else {
            self.choices[hash.rem_euclid(self.choices.len() as i32) as usize].clone()
        };
        if self.zoom.is_some() {
            self.trace
                .lock()
                .unwrap()
                .push(format!("quart {x} {y} {z} {name}"));
        }
        self.trace
            .lock()
            .unwrap()
            .push(format!("biome {} {} {} {name}", p.x, p.y, p.z));
        Ok(Some(name))
    }
    fn cold_enough_to_snow(
        &mut self,
        name: &str,
        p: BlockPos,
        sea: i32,
    ) -> Result<bool, MaterialRuleError> {
        Ok(self
            .temperatures
            .lock()
            .unwrap()
            .get_mut(name)
            .unwrap()
            .cold_enough_to_snow(p, sea))
    }
    fn melts_frozen_ocean(
        &mut self,
        name: &str,
        p: BlockPos,
        sea: i32,
    ) -> Result<bool, MaterialRuleError> {
        Ok(self
            .temperatures
            .lock()
            .unwrap()
            .get_mut(name)
            .unwrap()
            .should_melt_frozen_ocean_iceberg_slightly(p, sea))
    }
}
struct Data {
    registry: BlockRegistry,
    integers: BTreeSet<(String, String)>,
    properties: Vec<State>,
    carving_blocks: CarvingBlocks,
    biome_carvers: BTreeMap<String, Vec<Arc<WorldCarver>>>,
    conditions: MaterialConditionRegistry,
    noises: BTreeMap<String, String>,
    densities: BTreeMap<String, String>,
    rules: BTreeMap<String, String>,
    presets: BTreeMap<String, String>,
    source_catalog: BiomeSourceCatalog,
    temperatures: Temperatures,
}
fn biome_neighborhood(
    f: &Value,
    resolver: Option<&mut BiomeSourceResolver<'_>>,
    out: &mut impl Write,
) -> Result<Option<BiomeNeighborhood>, Box<dyn std::error::Error>> {
    let Some(resolver) = resolver else {
        return Ok(None);
    };
    let (cx, cz, min, height) = (n(&f["cx"]), n(&f["cz"]), n(&f["min"]), n(&f["height"]));
    let mut chunks = BTreeMap::new();
    for dx in -1..=1 {
        for dz in -1..=1 {
            let (x, z) = (cx.wrapping_add(dx), cz.wrapping_add(dz));
            let mut chunk = BiomeChunk::new(
                ChunkPos::new(x, z),
                min,
                height,
                Arc::<str>::from("minecraft:plains"),
            )?;
            let mut context = SamplerContext::cached();
            let volume = resolver.prepare_volume(
                &mut context,
                [x.wrapping_shl(4) >> 2, min >> 2, z.wrapping_shl(4) >> 2],
                [4, height >> 2, 4],
            )?;
            chunk.fill(min, min + height - 1, |quart| {
                resolver
                    .sample_prepared(&mut context, volume.as_ref(), quart)
                    .map(Arc::from)
            })?;
            writeln!(out, "biomechunk {x} {z}")?;
            for section in 0..height as usize / 16 {
                for name in chunk.section(section).unwrap() {
                    writeln!(out, "stored {name}")?;
                }
            }
            chunks.insert((x, z), chunk);
        }
    }
    Ok(Some(chunks))
}
fn fixture<R: RandomSource>(
    f: &Value,
    data: &Data,
    mut random: R,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>>
where
    R::Positional: Clone,
{
    let root = random.fork_positional();
    let seed = f["seed"].as_i64().unwrap();
    let legacy = f["legacy"].as_bool().unwrap();
    let codec = BlockStateJsonCodec::new(&data.registry, data.integers.clone())?;
    let mut densities = DensityRegistry::from_json(&data.noises, &data.densities)?;
    let router = f
        .get("router")
        .map(|value| NoiseRouter::from_json(&value.to_string(), &mut densities))
        .transpose()?;
    let aquifers = f
        .get("aquifers")
        .filter(|value| !value.is_null())
        .map(|value| AquiferConfig::from_json(&value.to_string(), &mut densities))
        .transpose()?;
    let mut library = MaterialRuleLibrary::from_json(
        &data.rules,
        &data.conditions,
        &codec,
        densities,
        seed,
        legacy,
    )?;
    let router = router
        .map(|value| library.compile_router(value))
        .transpose()?;
    let aquifers = aquifers
        .map(|value| library.compile_aquifer(value))
        .transpose()?;
    let rule = library.rule(f["rule"].as_str().unwrap())?;
    let preliminary = if let Some(ref router) = router {
        router.get(RouterField::ChunkSurfaceLevel).clone()
    } else if let Some(name) = f["preliminary"].as_str() {
        library.named_density(name)?
    } else {
        Arc::new(ConstantSampler(f["preliminary"].as_f64().unwrap() as f32))
    };
    let source = f
        .get("biome_source_json")
        .map(|json| {
            data.source_catalog
                .decode(&json.to_string())?
                .resolve(&data.presets)
        })
        .transpose()?;
    let climate = router.as_ref().map(|r| r.climate_sampler());
    let mut biome_resolver = source.as_ref().map(|source| {
        source.resolver(
            climate.as_ref().unwrap(),
            router.as_ref().unwrap().get(RouterField::Erosion).as_ref(),
        )
    });
    let neighborhood = biome_neighborhood(f, biome_resolver.as_mut(), out)?;
    let sampler = |name: &str| -> Result<Arc<dyn Noise>, DecodeError> {
        let id = format!("minecraft:{name}");
        Ok(Arc::new(
            decode_noise(&data.noises[&id])?.create(&mut root.from_hash_of(&id))?,
        ))
    };
    let surface = MaterialSurface::new(
        root.clone(),
        ClayBandBlocks::from_registry(&data.registry).unwrap(),
        sampler("clay_bands_offset")?,
        sampler("surface")?,
        sampler("surface_secondary")?,
    );
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mut surface = Surface {
        noises: MaterialNoiseSources::from_json(root.clone(), surface, &data.noises)?,
        neighborhood,
        zoom: f["biome_zoom"]
            .as_bool()
            .unwrap_or(false)
            .then(|| BiomeZoom::new(obfuscate_seed(seed))),
        choices: f["biome_choices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Arc::from(v.as_str().unwrap()))
            .collect(),
        possible: f["possible"]
            .as_array()
            .map(|v| v.iter().map(|v| v.as_str().unwrap().to_owned()).collect()),
        temperatures: data.temperatures.clone(),
        trace: trace.clone(),
    };
    let extensions = MaterialExtensions::new(
        root.clone(),
        ExtensionBlocks {
            default: BlockStateId(f["default"].as_u64().unwrap() as u32),
            water: data.registry.default_state("minecraft:water").unwrap(),
            packed_ice: data.registry.default_state("minecraft:packed_ice").unwrap(),
            snow: data.registry.default_state("minecraft:snow_block").unwrap(),
        },
        ExtensionNoises {
            badlands_surface: sampler("badlands_surface")?,
            badlands_pillar: sampler("badlands_pillar")?,
            badlands_roof: sampler("badlands_pillar_roof")?,
            iceberg_surface: sampler("iceberg_surface")?,
            iceberg_pillar: sampler("iceberg_pillar")?,
            iceberg_roof: sampler("iceberg_pillar_roof")?,
        },
        n(&f["sea"]),
    );
    let (cx, cz, min, height) = (n(&f["cx"]), n(&f["cz"]), n(&f["min"]), n(&f["height"]));
    let mut chunk = TerrainChunk::new(
        ChunkPos::new(cx, cz),
        min,
        height,
        &data.registry,
        Properties(&data.properties),
    )?;
    for span in f["initial"].as_array().unwrap() {
        for y in n(&span[1])..=n(&span[3]) {
            chunk.set_block_raw(
                n(&span[0]),
                y,
                n(&span[2]),
                BlockStateId(span[4].as_u64().unwrap() as u32),
            )?;
        }
    }
    if f["prime"].as_bool().unwrap() {
        chunk.prime_heightmaps(&[HeightmapType::WorldSurfaceWg, HeightmapType::OceanFloorWg])?;
    }
    let mut context = SamplerContext::cached();
    let volume = DensityVolume::new(
        [16, n(&f["noise_height"]), 16],
        [cx.wrapping_shl(4), n(&f["noise_min"]), cz.wrapping_shl(4)],
        [1; 3],
    )?;
    let air = data.registry.default_state("minecraft:air").unwrap();
    let picker = GlobalFluidPicker::new(
        n(&f["sea"]),
        f.get("fluid").map_or_else(
            || data.registry.default_state("minecraft:water").unwrap(),
            |v| BlockStateId(v.as_u64().unwrap() as u32),
        ),
        data.registry.default_state("minecraft:lava").unwrap(),
        air,
    );
    let mut aquifer: Box<dyn TerrainAquifer> = if let Some(samplers) = aquifers {
        Box::new(NoiseBasedAquifer::new(
            samplers,
            picker,
            root.from_hash_of("minecraft:aquifer").fork_positional(),
            RegistryAquiferBlockStates::new(&data.registry)?,
            volume,
            &mut context,
        )?)
    } else {
        Box::new(DisabledAquifer::new(picker, air))
    };
    if let Some(ref router) = router {
        fill_noise(
            &mut chunk,
            volume,
            router.get(RouterField::FinalDensity).as_ref(),
            &mut context,
            aquifer.as_mut(),
            BlockStateId(f["default"].as_u64().unwrap() as u32),
        )?;
        writeln!(out, "terrain {}", aquifer.should_schedule_fluid_update())?;
        dump(&mut chunk, height, out)?;
    }
    for repeat in 0..n(&f["repeats"]) {
        let inputs = SurfaceInputs::<R, _> {
            generation: WorldGenerationContext::new(min, height, n(&f["sea"])),
            noise_volume: volume,
            root: root.clone(),
            surface: &mut surface,
            preliminary_surface: preliminary.clone(),
            density_context: &mut context,
            write_min_y: n(&f["write_min"]),
            write_max_y: n(&f["write_max"]),
        };
        let result = build_surface(&mut chunk, rule.clone(), inputs, &extensions);
        for event in trace.lock().unwrap().drain(..) {
            writeln!(out, "{event}")?;
        }
        writeln!(
            out,
            "surface {repeat} {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        if let Err(error) = result {
            eprintln!("surface failure: {error}");
        }
        dump(&mut chunk, height, out)?;
    }
    for query in f["top_queries"].as_array().unwrap() {
        let position = BlockPos::new(n(&query[0]), n(&query[1]), n(&query[2]));
        let result = top_material(
            &mut chunk,
            rule.clone(),
            TopMaterialInputs::<R, _> {
                generation: WorldGenerationContext::new(min, height, n(&f["sea"])),
                root: root.clone(),
                surface: &mut surface,
                preliminary_surface: preliminary.clone(),
                density_context: &mut context,
            },
            position,
            query[3].as_bool().unwrap(),
        );
        for event in trace.lock().unwrap().drain(..) {
            writeln!(out, "{event}")?;
        }
        match result {
            Ok(Some(state)) => writeln!(out, "top {}", state.0)?,
            Ok(None) => writeln!(out, "top none")?,
            Err(error) => {
                writeln!(out, "top error")?;
                eprintln!("top failure: {error}");
            }
        }
    }
    if let Some(resolver) = biome_resolver.as_mut() {
        let mut biome_context = SamplerContext::default();
        let generation = WorldGenerationContext::new(min, height, n(&f["sea"]));
        let generated =
            generate_carving_mask(seed, ChunkPos::new(cx, cz), generation, false, |origin| {
                let quart = [
                    origin.x.wrapping_shl(4) >> 2,
                    0,
                    origin.z.wrapping_shl(4) >> 2,
                ];
                let name = raw_biome(resolver, &mut biome_context, quart, &trace)?;
                data.biome_carvers
                    .get(name.as_ref())
                    .map(Vec::as_slice)
                    .ok_or(MaterialRuleError::MissingBiome)
            })?;
        let mut traced = TracedAquifer {
            inner: aquifer.as_mut(),
            trace: trace.clone(),
        };
        let result = if generated.is_empty() {
            Ok(())
        } else {
            apply_carving_mask(
                &mut chunk,
                &generated,
                rule.clone(),
                TopMaterialInputs::<R, _> {
                    generation,
                    root: root.clone(),
                    surface: CarverSurface {
                        surface: &mut surface,
                        resolver,
                        context: &mut biome_context,
                        zoom: BiomeZoom::new(obfuscate_seed(seed)),
                        trace: trace.clone(),
                    },
                    preliminary_surface: preliminary.clone(),
                    density_context: &mut context,
                },
                &mut traced,
                &data.carving_blocks,
                |_| true,
            )
        };
        for event in trace.lock().unwrap().drain(..) {
            writeln!(out, "{event}")?;
        }
        writeln!(
            out,
            "generatedcarving {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        if let Err(error) = result {
            eprintln!("generated carving failure: {error}");
        }
        dump(&mut chunk, height, out)?;
    }
    let carving = &f["carving"];
    for q in carving["initial"].as_array().unwrap() {
        chunk.set_block_worldgen(
            n(&q[0]),
            n(&q[1]),
            n(&q[2]),
            BlockStateId(q[3].as_u64().unwrap() as u32),
        )?;
    }
    let mut mask = CarvingMask::new(min + 1, min + height - 8)?;
    for q in carving["spans"].as_array().unwrap() {
        for y in n(&q[2])..=n(&q[3]) {
            mask.carve(n(&q[0]), y, n(&q[1]))?;
        }
    }
    let mut traced = TracedAquifer {
        inner: aquifer.as_mut(),
        trace: trace.clone(),
    };
    for mode in carving["filters"].as_array().unwrap() {
        let mode = n(mode);
        let result = apply_carving_mask(
            &mut chunk,
            &mask,
            rule.clone(),
            TopMaterialInputs::<R, _> {
                generation: WorldGenerationContext::new(min, height, n(&f["sea"])),
                root: root.clone(),
                surface: &mut surface,
                preliminary_surface: preliminary.clone(),
                density_context: &mut context,
            },
            &mut traced,
            &data.carving_blocks,
            |[x, y, z]| {
                if mode == 0 {
                    return true;
                }
                let keep = mode == 1
                    && x.wrapping_mul(31)
                        .wrapping_add(y.wrapping_mul(7))
                        .wrapping_add(z.wrapping_mul(17))
                        .rem_euclid(5)
                        != 0;
                trace
                    .lock()
                    .unwrap()
                    .push(format!("filter {x} {y} {z} {keep}"));
                keep
            },
        );
        for event in trace.lock().unwrap().drain(..) {
            writeln!(out, "{event}")?;
        }
        writeln!(
            out,
            "carving {mode} {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        if let Err(error) = result {
            eprintln!("carving failure: {error}");
        }
        dump(&mut chunk, height, out)?;
    }
    Ok(())
}
fn raw_biome(
    resolver: &mut BiomeSourceResolver<'_>,
    context: &mut SamplerContext,
    [x, y, z]: [i32; 3],
    trace: &Arc<Mutex<Vec<String>>>,
) -> Result<Arc<str>, MaterialRuleError> {
    let name = resolver
        .sample(context, [x, y, z])
        .map_err(|_| MaterialRuleError::BiomeQueryFailed)?;
    trace
        .lock()
        .unwrap()
        .push(format!("rawquart {x} {y} {z} {name}"));
    Ok(Arc::from(name))
}
struct CarverSurface<'a, 'b, P: PositionalRandom> {
    surface: &'a mut Surface<P>,
    resolver: &'a mut BiomeSourceResolver<'b>,
    context: &'a mut SamplerContext,
    zoom: BiomeZoom,
    trace: Arc<Mutex<Vec<String>>>,
}
impl<P: PositionalRandom> MaterialSurfaceQueries for CarverSurface<'_, '_, P> {
    fn surface_depth(&mut self, x: i32, z: i32) -> i32 {
        self.surface.surface_depth(x, z)
    }
    fn surface_secondary(&mut self, x: i32, z: i32) -> f64 {
        self.surface.surface_secondary(x, z)
    }
    fn band(&mut self, p: BlockPos) -> Result<BlockStateId, MaterialRuleError> {
        self.surface.band(p)
    }
    fn bind_noise(&mut self, name: &str) -> Result<Arc<dyn Noise>, DecodeError> {
        self.surface.bind_noise(name)
    }
    fn biome(&mut self, p: BlockPos) -> Result<Option<Arc<str>>, MaterialRuleError> {
        raw_biome(
            self.resolver,
            self.context,
            self.zoom.quart_at_block(p),
            &self.trace,
        )
        .map(Some)
    }
    fn cold_enough_to_snow(
        &mut self,
        name: &str,
        p: BlockPos,
        sea: i32,
    ) -> Result<bool, MaterialRuleError> {
        self.surface.cold_enough_to_snow(name, p, sea)
    }
    fn melts_frozen_ocean(
        &mut self,
        name: &str,
        p: BlockPos,
        sea: i32,
    ) -> Result<bool, MaterialRuleError> {
        self.surface.melts_frozen_ocean(name, p, sea)
    }
}
struct TracedAquifer<'a> {
    inner: &'a mut dyn TerrainAquifer,
    trace: Arc<Mutex<Vec<String>>>,
}
impl TerrainAquifer for TracedAquifer<'_> {
    fn compute_substance(
        &mut self,
        context: &mut SamplerContext,
        [x, y, z]: [i32; 3],
        density: f64,
    ) -> Result<Option<BlockStateId>, AquiferError> {
        self.trace
            .lock()
            .unwrap()
            .push(format!("aquifer {x} {y} {z} {:x}", density.to_bits()));
        let result = self.inner.compute_substance(context, [x, y, z], density)?;
        self.trace.lock().unwrap().push(format!(
            "substance {}",
            result.map_or_else(|| "none".to_owned(), |v| v.0.to_string())
        ));
        Ok(result)
    }
    fn should_schedule_fluid_update(&self) -> bool {
        let result = self.inner.should_schedule_fluid_update();
        self.trace
            .lock()
            .unwrap()
            .push(format!("fluid_schedule {result}"));
        result
    }
}
fn dump<B: TerrainBlockStates>(
    chunk: &mut TerrainChunk<'_, B>,
    height: i32,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    for section in 0..height as usize / 16 {
        writeln!(out, "section {section}")?;
        let data = chunk.section(section).unwrap();
        for y in 0..16 {
            for z in 0..16 {
                write!(out, "blocks")?;
                for x in 0..16 {
                    write!(out, " {}", data.get((y << 8) | (z << 4) | x)?.0)?;
                }
                writeln!(out)?;
            }
        }
    }
    for kind in [HeightmapType::WorldSurfaceWg, HeightmapType::OceanFloorWg] {
        writeln!(out, "heightmap {}", kind.name())?;
        for word in chunk.ensure_heightmap(kind).raw_data() {
            writeln!(out, "{word:x}")?;
        }
    }
    for (section, offsets) in chunk.postprocessing().iter().enumerate() {
        writeln!(out, "post {section} {}", offsets.len())?;
        for offset in offsets {
            writeln!(out, "{offset}")?;
        }
    }
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let read = |name: &str| -> Result<BTreeMap<String, String>, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(&fs::read_to_string(
            dir.join(format!("{name}.json")),
        )?)?)
    };
    let facts: Value = serde_json::from_str(&fs::read_to_string(dir.join("facts-server.json"))?)?;
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(
                facts["states"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| tag(&v[4]))
                    .collect(),
            ),
        ),
        ("defaults", tag(&facts["defaults"])),
    ]))?;
    let biomes = read("biomes")?;
    let mut temperatures = BTreeMap::new();
    for (name, raw) in &biomes {
        temperatures.insert(
            name.clone(),
            BiomeTemperature::new(BiomeTemperatureSettings::from_json(raw)?),
        );
    }
    let mut biome_tags = TagDefinitions::default();
    for (name, raw) in read("biome_tags")? {
        biome_tags.apply_json(&name, "vanilla", &raw)?;
    }
    let biome_tags = biome_tags.build(|id, _| biomes.contains_key(id).then(|| id.to_owned()))?;
    assert!(biome_tags.missing.is_empty());
    let conditions = MaterialConditionRegistry::from_json_with_biome_tags(
        &read("conditions")?,
        &biomes.keys().cloned().collect(),
        &biome_tags.tags,
    )?;
    let mut block_tags = TagDefinitions::default();
    for (name, raw) in read("block_tags")? {
        block_tags.apply_json(&name, "vanilla", &raw)?;
    }
    let block_tags = block_tags.build(|id, _| facts["defaults"].get(id).map(|_| id.to_owned()))?;
    assert!(block_tags.missing.is_empty());
    let motion: BTreeSet<_> = block_tags.tags["minecraft:blocks_motion_in_heightmap"]
        .iter()
        .map(String::as_str)
        .collect();
    let no_leaves: BTreeSet<_> = block_tags.tags["minecraft:blocks_motion_in_heightmap_no_leaves"]
        .iter()
        .map(String::as_str)
        .collect();
    let properties = facts["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let name = v[0].as_str().unwrap();
            State {
                air: v[1].as_bool().unwrap(),
                air_block: name == "minecraft:air",
                fluid: v[2].as_bool().unwrap(),
                motion: motion.contains(name),
                no_leaves: no_leaves.contains(name),
            }
        })
        .collect();
    let integers = facts["integers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v[0].as_str().unwrap().to_owned(),
                v[1].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let presets = read("presets")?;
    let source_catalog = BiomeSourceCatalog::new(
        &biomes.keys().cloned().collect(),
        &biome_tags.tags,
        &presets.keys().cloned().collect(),
    )?;
    let carving_blocks = CarvingBlocks::from_registry(
        &registry,
        &block_tags.tags["minecraft:uncarvable"]
            .iter()
            .cloned()
            .collect(),
    )?;
    let carvers = CarverCatalog::from_json(&read("carvers")?)?;
    let mut biome_carvers = BTreeMap::new();
    for (name, json) in &biomes {
        let value: Value = serde_json::from_str(json)?;
        biome_carvers.insert(
            name.clone(),
            carvers.decode_set(&value["carvers"].to_string())?,
        );
    }
    let data = Data {
        biome_carvers,
        carving_blocks,
        registry,
        integers,
        properties,
        conditions,
        noises: read("noises")?,
        densities: read("densities")?,
        rules: read("rules")?,
        presets,
        source_catalog,
        temperatures: Arc::new(Mutex::new(temperatures)),
    };
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let seed = f["seed"].as_i64().unwrap();
        if f["legacy"].as_bool().unwrap() {
            fixture(&f, &data, LegacyRandom::new(seed), &mut out)?;
        } else {
            fixture(&f, &data, XoroshiroRandom::new(seed), &mut out)?;
        }
    }
    Ok(())
}
