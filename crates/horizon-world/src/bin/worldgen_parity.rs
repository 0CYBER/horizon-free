//! Original transport for version-specific JSON decoding comparisons.
use horizon_core::{
    density::{
        SamplerContext,
        graph::{DensityCompiler, FunctionId},
    },
    noise::DensityVolume,
    noise::{Noise, NormalNoise},
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::worldgen::{
    decode_noise,
    density::DensityRegistry,
    router::{NoiseRouter, RouterField},
};
use std::io::{self, BufRead, Write};

fn bits(value: f32) -> u32 {
    if value.is_nan() {
        0x7fc0_0000
    } else {
        value.to_bits()
    }
}
fn probe(
    noise: &NormalNoise,
    random: &mut impl RandomSource,
    nether: bool,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let sampler = match if nether {
        noise.create_for_legacy_nether_biome(random)
    } else {
        noise.create(random)
    } {
        Ok(sampler) => sampler,
        Err(_) => {
            writeln!(out, "sample error {}", random.next_long())?;
            return Ok(());
        }
    };
    for [x, y, z] in [
        [0., 0., 0.],
        [-17.25, 64.125, 32.5],
        [30e6, -64., -30e6],
        [1.25, -0.5, -3.75],
        [33554432., -2048., -33554433.],
    ] {
        writeln!(
            out,
            "query {} {}",
            bits(sampler.get2(x, z)),
            bits(sampler.get3(x, y, z))
        )?;
    }
    writeln!(out, "parent {}", random.next_long())?;
    Ok(())
}
fn registry_probe(
    path: &str,
    seed: i64,
    legacy: bool,
    optimized: bool,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let bundle: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>> =
        serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let registry = DensityRegistry::from_json(&bundle["noises"], &bundle["densities"])?;
    let entries: Vec<_> = registry
        .entries()
        .map(|(name, id)| (name.to_owned(), id))
        .collect();
    let mut compiler = DensityCompiler::new_seeded(registry.into_graph(), seed, legacy);
    writeln!(out, "registry {}", entries.len())?;
    for (name, id) in entries {
        sample_root(
            &mut compiler,
            id,
            &name,
            optimized,
            false,
            &mut SamplerContext::cached(),
            out,
        )?;
    }
    Ok(())
}
fn sample_root(
    compiler: &mut DensityCompiler,
    id: FunctionId,
    name: &str,
    optimized: bool,
    router: bool,
    context: &mut SamplerContext,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let range = compiler.graph().range(id)?;
    writeln!(
        out,
        "entry {name} {} {} {}",
        bits(range.min()),
        bits(range.max()),
        compiler.graph().domain_axes(id)?
    )?;
    let sampler = match if optimized {
        compiler.get_sampler(id)
    } else {
        compiler.compile_raw(id)
    } {
        Ok(sampler) => sampler,
        Err(_) => {
            writeln!(out, "compile error")?;
            return Ok(());
        }
    };
    writeln!(out, "compile ok {}", compiler.cache_count())?;
    for position in [
        [0, 0, 0],
        [-17, -64, 32],
        [30000000, 319, -30000000],
        [1, -1, -1],
    ] {
        writeln!(
            out,
            "density {}",
            bits(sampler.sample_value(context, position)?)
        )?;
    }
    let mut volumes = vec![
        ([2, 9, 2], [-17, -64, 32], [1, 1, 1]),
        ([2, 9, 2], [-16, -64, 32], [4, 8, 4]),
    ];
    if router {
        volumes.push(([3, 1, 2], [-17, 0, 32], [4, 1, 4]));
    }
    for (extents, minimum, steps) in volumes {
        let volume = DensityVolume::new(extents, minimum, steps)?;
        let mut buffer = vec![0.; volume.size()];
        for _ in 0..2 {
            let result = sampler.sample_volume(context, &mut buffer, volume);
            writeln!(
                out,
                "volume {}",
                if result.is_ok() { "ok" } else { "error" }
            )?;
            for value in &buffer {
                writeln!(out, "cell {}", bits(*value))?;
            }
        }
    }
    Ok(())
}
fn router_probe(
    path: &str,
    setting: &str,
    seed: i64,
    legacy: bool,
    optimized: bool,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let bundle: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>> =
        serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let mut registry = DensityRegistry::from_json(&bundle["noises"], &bundle["densities"])?;
    let settings: std::collections::BTreeMap<String, Box<serde_json::value::RawValue>> =
        serde_json::from_str(&bundle["settings"][setting])?;
    let router = NoiseRouter::from_json(settings["noise_router"].get(), &mut registry)?;
    let mut compiler = DensityCompiler::new_seeded(registry.into_graph(), seed, legacy);
    let mut context = SamplerContext::cached();
    writeln!(out, "router {setting} 8")?;
    for field in RouterField::ALL {
        sample_root(
            &mut compiler,
            router.function(field),
            field.name(),
            optimized,
            true,
            &mut context,
            out,
        )?;
    }
    let climate = router.compile_climate(&mut compiler)?;
    writeln!(out, "climate caches {}", compiler.cache_count())?;
    for position in [
        [0, 0, 0],
        [-5, -16, 8],
        [7500000, 79, -7500000],
        [i32::MIN, i32::MAX, -1],
        [536870912, -536870913, 17],
        [-4, 0, 8],
    ] {
        write!(out, "climate")?;
        for value in climate.sample(&mut context, position)?.parameters() {
            write!(out, " {value}")?;
        }
        writeln!(out)?;
    }
    for (preset, json) in &bundle["presets"] {
        let source =
            horizon_world::worldgen::biome::MultiNoiseBiomeSource::from_quantized_json(json)?;
        writeln!(
            out,
            "biomes {preset} {}",
            source.parameters().entries().len()
        )?;
        let mut resolver = source.resolver(&climate);
        for position in [
            [0, 0, 0],
            [-5, -16, 8],
            [7500000, 79, -7500000],
            [i32::MIN, i32::MAX, -1],
            [536870912, -536870913, 17],
            [-4, 0, 8],
        ] {
            writeln!(out, "biome {}", resolver.sample(&mut context, position)?)?;
        }
        for (minimum, extents) in [
            ([-5, -16, 8], [3, 4, 2]),
            ([-4, -16, 8], [3, 4, 2]),
            ([i32::MAX - 1, i32::MIN, -1], [2, 2, 2]),
        ] {
            let volume = resolver.prepare_volume(&mut context, minimum, extents)?;
            let mut positions = Vec::new();
            for z in 0..extents[2] {
                for x in 0..extents[0] {
                    for y in 0..extents[1] {
                        positions.push([
                            minimum[0].wrapping_add(x),
                            minimum[1].wrapping_add(y),
                            minimum[2].wrapping_add(z),
                        ]);
                    }
                }
            }
            for position in positions
                .iter()
                .chain(positions.iter().rev())
                .chain(positions.iter().take(3))
            {
                writeln!(
                    out,
                    "biomecell {}",
                    resolver.sample_prepared(&volume, *position)?
                )?;
            }
        }
    }
    Ok(())
}
fn index_probe(
    branching: usize,
    json: &str,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    use horizon_world::worldgen::climate::{ClimateIndex, Parameter, ParameterPoint, TargetPoint};
    let input: serde_json::Value = serde_json::from_str(json)?;
    let mut entries = Vec::new();
    for (id, entry) in input["entries"]
        .as_array()
        .ok_or("entries")?
        .iter()
        .enumerate()
    {
        let coords = entry["coordinates"].as_array().ok_or("coordinates")?;
        let mut coordinates = [Parameter::new(0, 0); 6];
        for (coordinate, value) in coordinates.iter_mut().zip(coords) {
            *coordinate = Parameter::new(
                value[0].as_i64().ok_or("minimum")?,
                value[1].as_i64().ok_or("maximum")?,
            );
        }
        entries.push((
            ParameterPoint {
                coordinates,
                offset: entry["offset"].as_i64().ok_or("offset")?,
            },
            id,
        ));
    }
    let index = match ClimateIndex::with_branching(entries, branching) {
        Ok(index) => index,
        Err(_) => {
            writeln!(out, "index error")?;
            return Ok(());
        }
    };
    writeln!(out, "index ok {}", index.entries().len())?;
    for id in index.leaf_order() {
        writeln!(out, "leaf {id}")?;
    }
    let mut cached = index.searcher();
    for target in input["targets"].as_array().ok_or("targets")? {
        let values = target.as_array().ok_or("target")?;
        let point = TargetPoint {
            temperature: values[0].as_i64().ok_or("temperature")?,
            humidity: values[1].as_i64().ok_or("humidity")?,
            continentalness: values[2].as_i64().ok_or("continentalness")?,
            erosion: values[3].as_i64().ok_or("erosion")?,
            depth: values[4].as_i64().ok_or("depth")?,
            weirdness: values[5].as_i64().ok_or("weirdness")?,
        };
        let brute = index.brute_force(point);
        writeln!(
            out,
            "brute {brute} {}",
            index.entries()[*brute].0.fitness(point)
        )?;
        for (label, result) in [
            ("cached", cached.find(point)),
            ("fresh", index.searcher().find(point)),
        ] {
            match result {
                Ok(id) => writeln!(
                    out,
                    "{label} {id} {}",
                    index.entries()[*id].0.fitness(point)
                )?,
                Err(_) => writeln!(out, "{label} error")?,
            }
        }
    }
    Ok(())
}
fn settings_probe(
    settings: horizon_world::worldgen::settings::NoiseSettings,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    writeln!(out, "settings {} {}", settings.min_y, settings.height)?;
    for (minimum, height) in [
        (-64, 384),
        (0, 128),
        (-2032, 4064),
        (17, 15),
        (2048, 16),
        (i32::MAX, 1),
        (i32::MIN, i32::MAX),
    ] {
        let clamped = settings.clamp_to_height(minimum, height);
        writeln!(out, "clamped {} {}", clamped.min_y, clamped.height)?;
    }
    Ok(())
}
fn aquifer_probe(line: &str, out: &mut impl Write) -> Result<(), Box<dyn std::error::Error>> {
    use horizon_world::{
        registry::BlockStateId,
        worldgen::aquifer::{DisabledAquifer, FluidPicker, FluidStatus, GlobalFluidPicker},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Debug)]
    struct Counted {
        picker: GlobalFluidPicker,
        calls: AtomicUsize,
    }
    impl FluidPicker for Counted {
        fn compute(&self, position: [i32; 3]) -> FluidStatus {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.picker.compute(position)
        }
    }
    let args: Vec<_> = line.split_whitespace().collect();
    let sea = args[1].parse::<i32>()?;
    let fluid = BlockStateId(args[2].parse::<u32>()?);
    let position = [args[3].parse()?, args[4].parse()?, args[5].parse()?];
    let density = f64::from_bits(u64::from_str_radix(args[6], 16)?);
    // Local labels correspond to actual reference default block states. They
    // exercise identity-based selection independently of numeric registry IDs.
    let picker = GlobalFluidPicker::new(sea, fluid, BlockStateId(1), BlockStateId(2));
    let status = picker.compute(position);
    writeln!(
        out,
        "fluid {} {} {}",
        status.level,
        status.state.0,
        status.at(position[1], BlockStateId(2)).0
    )?;
    let counted = Counted {
        picker,
        calls: AtomicUsize::new(0),
    };
    let aquifer = DisabledAquifer::new(&counted, BlockStateId(2));
    let substance = aquifer.compute_substance(position, density);
    let label = substance.map_or_else(|| "default_solid".to_string(), |id| id.0.to_string());
    writeln!(
        out,
        "substance {label} {} {}",
        aquifer.should_schedule_fluid_update(),
        counted.calls.load(Ordering::Relaxed)
    )?;
    Ok(())
}
fn noise_aquifer_probe(line: &str, out: &mut impl Write) -> Result<(), Box<dyn std::error::Error>> {
    use horizon_core::{
        density::{ConstantSampler, DensitySampler, SampleError},
        random::PositionalRandom,
    };
    use horizon_world::{
        registry::BlockStateId,
        worldgen::aquifer::{
            AquiferBlockStates, AquiferConfig, AquiferField, AquiferSamplers, FluidPicker,
            FluidStatus, GlobalFluidPicker, NoiseBasedAquifer,
        },
    };
    use std::sync::{Arc, Mutex};
    #[derive(Debug)]
    struct Traced {
        sampler: Arc<dyn DensitySampler>,
        field: usize,
        events: Arc<Mutex<Vec<String>>>,
        calls: Mutex<usize>,
        fail: Option<usize>,
    }
    impl Traced {
        fn enter(&self, event: String) -> Result<(), SampleError> {
            self.events.lock().unwrap().push(event);
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            if self.fail == Some(*calls) {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("sample failure {}", self.field));
                Err(SampleError::ZeroGradientPeriod)
            } else {
                Ok(())
            }
        }
    }
    impl DensitySampler for Traced {
        fn sample_value(
            &self,
            context: &mut SamplerContext,
            position: [i32; 3],
        ) -> Result<f32, SampleError> {
            self.enter(format!(
                "sample {} value {} {} {}",
                self.field, position[0], position[1], position[2]
            ))?;
            let result = self.sampler.sample_value(context, position);
            self.events.lock().unwrap().push(match result {
                Ok(value) => format!("value {}", bits(value)),
                Err(_) => "value error".into(),
            });
            result
        }
        fn sample_volume(
            &self,
            context: &mut SamplerContext,
            buffer: &mut [f32],
            volume: DensityVolume,
        ) -> Result<(), SampleError> {
            let size = volume.sizes();
            let min = volume.minimum();
            let steps = volume.steps();
            self.enter(format!(
                "sample {} volume {} {} {} {} {} {} {} {} {}",
                self.field,
                size[0],
                size[1],
                size[2],
                min[0],
                min[1],
                min[2],
                steps[0],
                steps[1],
                steps[2]
            ))?;
            let result = self.sampler.sample_volume(context, buffer, volume);
            let mut events = self.events.lock().unwrap();
            events.push(format!(
                "sample volume {}",
                if result.is_ok() { "ok" } else { "error" }
            ));
            events.extend(
                buffer
                    .iter()
                    .map(|value| format!("sample cell {}", bits(*value))),
            );
            result
        }
    }
    #[derive(Debug)]
    struct Picker {
        inner: GlobalFluidPicker,
        events: Arc<Mutex<Vec<String>>>,
    }
    impl FluidPicker for Picker {
        fn compute(&self, position: [i32; 3]) -> FluidStatus {
            let status = self.inner.compute(position);
            self.events.lock().unwrap().push(format!(
                "pick {} {} {} {} {}",
                position[0], position[1], position[2], status.level, status.state.0
            ));
            status
        }
    }
    struct Blocks;
    impl AquiferBlockStates for Blocks {
        fn air(&self) -> BlockStateId {
            BlockStateId(2)
        }
        fn lava(&self) -> BlockStateId {
            BlockStateId(1)
        }
        fn is_air(&self, s: BlockStateId) -> bool {
            matches!(s.0, 2 | 4 | 5)
        }
        fn is_lava(&self, s: BlockStateId) -> bool {
            matches!(s.0, 1 | 7)
        }
        fn is_water(&self, s: BlockStateId) -> bool {
            matches!(s.0, 0 | 6)
        }
    }
    fn flush(events: &Mutex<Vec<String>>, out: &mut impl Write) -> io::Result<()> {
        for event in events.lock().unwrap().drain(..) {
            writeln!(out, "{event}")?;
        }
        Ok(())
    }
    fn run<F: PositionalRandom>(
        samplers: AquiferSamplers,
        picker: Picker,
        random: F,
        volume: DensityVolume,
        queries: &serde_json::Value,
        events: Arc<Mutex<Vec<String>>>,
        out: &mut impl Write,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut context = SamplerContext::cached();
        let created =
            NoiseBasedAquifer::new(samplers, picker, random, Blocks, volume, &mut context);
        flush(&events, out)?;
        let mut aquifer = match created {
            Ok(value) => value,
            Err(_) => {
                writeln!(out, "aquifer create error")?;
                return Ok(());
            }
        };
        writeln!(out, "aquifer ready {}", aquifer.skip_sampling_above_y())?;
        for query in queries.as_array().ok_or("queries")? {
            let position = [
                query[0].as_i64().ok_or("x")? as i32,
                query[1].as_i64().ok_or("y")? as i32,
                query[2].as_i64().ok_or("z")? as i32,
            ];
            let density = f64::from_bits(u64::from_str_radix(
                query[3].as_str().ok_or("density")?,
                16,
            )?);
            let result = aquifer.compute_substance(&mut context, position, density);
            flush(&events, out)?;
            let result = match result {
                Ok(Some(state)) => state.0.to_string(),
                Ok(None) => "default_solid".into(),
                Err(_) => "error".into(),
            };
            writeln!(
                out,
                "aquifer result {result} {}",
                aquifer.should_schedule_fluid_update()
            )?;
        }
        for (index, (location, status)) in aquifer.cached_cells().enumerate() {
            let status = status.map_or("none".into(), |s| format!("{} {}", s.level, s.state.0));
            writeln!(out, "aquifer cell {index} {location} {status}")?;
        }
        let mut surfaces: Vec<_> = aquifer.cached_surfaces().collect();
        surfaces.sort_by_key(|entry| entry.0);
        for (key, value) in surfaces {
            writeln!(out, "aquifer surface {key} {value}")?;
        }
        Ok(())
    }
    let fields: Vec<_> = line.splitn(6, ' ').collect();
    let seed: i64 = fields[1].parse()?;
    let legacy = fields[2] == "1";
    let bundle: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>> =
        serde_json::from_str(&std::fs::read_to_string(fields[3])?)?;
    let fixture: serde_json::Value = serde_json::from_str(fields[5])?;
    let mut registry = DensityRegistry::from_json(&bundle["noises"], &bundle["densities"])?;
    let settings: std::collections::BTreeMap<String, Box<serde_json::value::RawValue>> =
        serde_json::from_str(&bundle["settings"][fields[4]])?;
    let config_text = fixture
        .get("config")
        .map(|c| c.as_str().ok_or("config string"))
        .transpose()?
        .unwrap_or_else(|| settings.get("aquifers").map_or("null", |v| v.get()));
    let config = match AquiferConfig::from_json(config_text, &mut registry) {
        Ok(config) => config,
        Err(_) => {
            writeln!(out, "aquifer config error")?;
            return Ok(());
        }
    };
    let mut compiler = DensityCompiler::new_seeded(registry.into_graph(), seed, legacy);
    let samplers = match config.compile(&mut compiler) {
        Ok(s) => s,
        Err(_) => {
            writeln!(out, "aquifer compile error")?;
            return Ok(());
        }
    };
    writeln!(out, "aquifer compile ok {}", compiler.cache_count())?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut traced = Vec::new();
    for field in AquiferField::ALL {
        let i = field as usize;
        let sampler = if let Some(value) = fixture
            .get("overrides")
            .and_then(|v| v.get(i))
            .and_then(|v| v.as_str())
        {
            Arc::new(ConstantSampler(f32::from_bits(u32::from_str_radix(
                value, 16,
            )?))) as Arc<dyn DensitySampler>
        } else {
            samplers.get(field).clone()
        };
        let fail = fixture
            .get("fail")
            .filter(|f| f[0].as_u64() == Some(i as u64))
            .and_then(|f| f[1].as_u64())
            .map(|n| n as usize);
        traced.push(Arc::new(Traced {
            sampler,
            field: i,
            events: events.clone(),
            calls: Mutex::new(0),
            fail,
        }) as Arc<dyn DensitySampler>);
    }
    let samplers = AquiferSamplers::new(traced.try_into().unwrap());
    let array = |name: &str| -> Result<[i32; 3], Box<dyn std::error::Error>> {
        Ok(std::array::from_fn(|i| {
            fixture[name][i].as_i64().unwrap() as i32
        }))
    };
    let volume = DensityVolume::new(array("sizes")?, array("minimum")?, array("steps")?)?;
    let picker = Picker {
        inner: GlobalFluidPicker::new(
            fixture["sea"].as_i64().ok_or("sea")? as i32,
            BlockStateId(fixture["fluid"].as_u64().ok_or("fluid")? as u32),
            BlockStateId(1),
            BlockStateId(2),
        ),
        events: events.clone(),
    };
    if legacy {
        let random = LegacyRandom::new(seed)
            .fork_positional()
            .from_hash_of("minecraft:aquifer")
            .fork_positional();
        run(
            samplers,
            picker,
            random,
            volume,
            &fixture["queries"],
            events,
            out,
        )
    } else {
        let random = XoroshiroRandom::new(seed)
            .fork_positional()
            .from_hash_of("minecraft:aquifer")
            .fork_positional();
        run(
            samplers,
            picker,
            random,
            volume,
            &fixture["queries"],
            events,
            out,
        )
    }
}

fn tags_probe(
    fixture: &serde_json::Value,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    use horizon_world::worldgen::tags::TagDefinitions;
    writeln!(
        out,
        "tags resources {}",
        fixture["files"].as_array().ok_or("files")?.len()
    )?;
    let mut definitions = TagDefinitions::default();
    for file in fixture["files"].as_array().ok_or("files")? {
        let name = file[0].as_str().ok_or("name")?;
        let source = file[1].as_str().ok_or("source")?;
        if definitions
            .apply_json(name, source, file[2].as_str().ok_or("json")?)
            .is_err()
        {
            // Fixture names are valid. Codec errors can introduce empty tags.
            let canonical = definitions
                .definitions()
                .keys()
                .find(|key| key.as_str() == name || key.as_str() == format!("minecraft:{name}"))
                .ok_or("tag name")?;
            writeln!(out, "tag resource error {canonical} {source}")?;
        }
    }
    let mut trace = Vec::new();
    let result = definitions.build(|id, required| {
        trace.push(format!("tag lookup {id} {required}"));
        fixture["elements"]
            .get(id)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    })?;
    for event in trace {
        writeln!(out, "{event}")?;
    }
    writeln!(out, "tags built {}", result.tags.len())?;
    for (name, values) in result.tags {
        writeln!(out, "tag members {name} {}", values.len())?;
        for value in values {
            writeln!(out, "tag member {value}")?;
        }
    }
    for name in result.missing.keys() {
        writeln!(out, "tag missing {name}")?;
    }
    Ok(())
}
fn tag_registry_probe(path: &str, out: &mut impl Write) -> Result<(), Box<dyn std::error::Error>> {
    let bundle: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>> =
        serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let files: Vec<_> = bundle["tags"]
        .iter()
        .map(|(name, json)| serde_json::json!([name, "vanilla", json]))
        .collect();
    tags_probe(
        &serde_json::json!({"files":files,"elements":bundle["block_elements"]}),
        out,
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line?;
        let mut args = line.splitn(5, ' ');
        let command = args.next().ok_or("command")?;
        if command == "tag_entry" {
            match horizon_world::worldgen::tags::TagEntry::from_json(
                line.strip_prefix("tag_entry ").ok_or("entry")?,
            ) {
                Ok(e) => writeln!(out, "tag entry {}", e.display())?,
                Err(_) => writeln!(out, "tag entry error")?,
            }
            continue;
        }
        if command == "tag_file" {
            match horizon_world::worldgen::tags::TagFile::from_json(
                line.strip_prefix("tag_file ").ok_or("file")?,
            ) {
                Ok(file) => {
                    writeln!(out, "tag file {} {}", file.replace, file.entries.len())?;
                    for e in file.entries {
                        writeln!(out, "tag entry {}", e.display())?;
                    }
                }
                Err(_) => writeln!(out, "tag file error")?,
            }
            continue;
        }
        if command == "tag_registry" {
            tag_registry_probe(
                line.strip_prefix("tag_registry ").ok_or("registry")?,
                &mut out,
            )?;
            continue;
        }
        if command == "tags" {
            tags_probe(
                &serde_json::from_str(line.strip_prefix("tags ").ok_or("tags")?)?,
                &mut out,
            )?;
            continue;
        }
        if command == "noise_aquifer" {
            noise_aquifer_probe(&line, &mut out)?;
            continue;
        }
        if command == "aquifer" {
            aquifer_probe(&line, &mut out)?;
            continue;
        }
        if command == "noise_settings" {
            match horizon_world::worldgen::settings::NoiseSettings::from_json(
                line.strip_prefix("noise_settings ")
                    .ok_or("settings json")?,
            ) {
                Ok(settings) => settings_probe(settings, &mut out)?,
                Err(_) => writeln!(out, "settings error")?,
            }
            continue;
        }
        if command == "settings_create" {
            let values: Vec<i32> = line
                .split_whitespace()
                .skip(1)
                .map(str::parse)
                .collect::<Result<_, _>>()?;
            match horizon_world::worldgen::settings::NoiseSettings::create(values[0], values[1]) {
                Ok(settings) => settings_probe(settings, &mut out)?,
                Err(_) => writeln!(out, "settings error")?,
            }
            continue;
        }
        if command == "climate_parameter" {
            match horizon_world::worldgen::climate::decode_parameter(
                line.strip_prefix("climate_parameter ")
                    .ok_or("parameter json")?,
            ) {
                Ok(p) => writeln!(out, "parameter {} {}", p.min, p.max)?,
                Err(_) => writeln!(out, "parameter error")?,
            }
            continue;
        }
        if command == "climate_point" {
            match horizon_world::worldgen::climate::decode_parameter_point(
                line.strip_prefix("climate_point ").ok_or("point json")?,
            ) {
                Ok(p) => {
                    write!(out, "parameterpoint")?;
                    for coordinate in p.coordinates {
                        write!(out, " {} {}", coordinate.min, coordinate.max)?;
                    }
                    writeln!(out, " {}", p.offset)?;
                }
                Err(_) => writeln!(out, "parameterpoint error")?,
            }
            continue;
        }
        if command == "index" {
            let mut args = line.splitn(3, ' ');
            args.next();
            let branching = args.next().ok_or("branching")?.parse::<usize>()?;
            index_probe(branching, args.next().ok_or("index json")?, &mut out)?;
            continue;
        }
        if command == "quantize" {
            let mut values = line.split_whitespace().skip(1);
            write!(out, "quantized")?;
            for _ in 0..6 {
                let value = f32::from_bits(u32::from_str_radix(values.next().ok_or("float")?, 16)?);
                write!(
                    out,
                    " {}",
                    horizon_world::worldgen::climate::quantize(value)
                )?;
            }
            writeln!(out)?;
            let value = values.next().ok_or("long")?.parse::<i64>()?;
            writeln!(
                out,
                "unquantized {}",
                bits(horizon_world::worldgen::climate::unquantize(value))
            )?;
            continue;
        }
        let seed = args.next().ok_or("seed")?.parse::<i64>()?;
        let legacy = args.next().ok_or("stream")? == "1";
        let nether = args.next().ok_or("nether")? == "1";
        let json = args.next().ok_or("json")?;
        if command == "router" {
            let (path, setting) = json.split_once(' ').ok_or("router setting")?;
            router_probe(path, setting, seed, legacy, nether, &mut out)?;
            continue;
        }
        if command == "registry" {
            registry_probe(json, seed, legacy, nether, &mut out)?;
            continue;
        }
        if command != "noise" {
            return Err("unknown command".into());
        }
        let noise = match decode_noise(json) {
            Ok(noise) => noise,
            Err(_) => {
                writeln!(out, "decode error")?;
                continue;
            }
        };
        let [min, max] = noise.range();
        writeln!(
            out,
            "decode {} {} {} {}",
            noise.octaves().len(),
            noise.normalization_factor().to_bits(),
            bits(min),
            bits(max)
        )?;
        for octave in noise.octaves() {
            writeln!(
                out,
                "octave {} {} {}",
                octave.octave_index,
                octave.frequency.to_bits(),
                octave.amplitude.to_bits()
            )?;
        }
        if legacy {
            probe(&noise, &mut LegacyRandom::new(seed), nether, &mut out)?;
        } else {
            probe(&noise, &mut XoroshiroRandom::new(seed), nether, &mut out)?;
        }
    }
    Ok(())
}
