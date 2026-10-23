//! Chunk generation throughput and sanity map.
//!
//! chunkgen_bench <client resource dir> [--seed S] [--size N] [--threads T]
//!     [--center CX CZ] [--verify <material-stage parity dir>] [--no-decorate]
//!     [--dimension overworld|the_nether|the_end]
//!     [--warmup] [--json] [--no-map] [--snapshot PATH]
use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
    heightmap::HeightmapType,
    nbt::Tag,
    registry::BlockRegistry,
    worldgen::generator::{
        DecorateStats, GeneratorData, GeneratorWorker, OverworldGenerator, ProtoChunk, StageTimings,
    },
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

type Error = Box<dyn std::error::Error + Send + Sync>;

fn tag(v: &Value) -> Tag {
    match v {
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Number(n) => Tag::Int(n.as_i64().unwrap() as i32),
        Value::Array(a) => Tag::List(a.iter().map(tag).collect()),
        Value::Object(o) => {
            Tag::Compound(o.iter().map(|(k, v)| (k.as_str().into(), tag(v))).collect())
        }
        _ => panic!("invalid registry metadata"),
    }
}
fn registry(resources: &Path) -> Result<&'static BlockRegistry, Error> {
    let facts: Value = serde_json::from_str(&fs::read_to_string(resources.join("facts.json"))?)?;
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
    Ok(Box::leak(Box::new(registry)))
}

/// The material-stage fixture's chunk dump format.
fn dump(chunk: &ProtoChunk) -> Vec<String> {
    let terrain = &chunk.terrain;
    let mut out = Vec::new();
    for section in 0..terrain.height() as usize / 16 {
        out.push(format!("section {section}"));
        let data = terrain.section(section).unwrap();
        for y in 0..16 {
            for z in 0..16 {
                let mut line = String::from("blocks");
                for x in 0..16 {
                    line += &format!(" {}", data.get((y << 8) | (z << 4) | x).unwrap().0);
                }
                out.push(line);
            }
        }
    }
    for kind in [HeightmapType::WorldSurfaceWg, HeightmapType::OceanFloorWg] {
        out.push(format!("heightmap {}", kind.name()));
        for word in terrain.heightmap(kind).unwrap().raw_data() {
            out.push(format!("{word:x}"));
        }
    }
    for (section, offsets) in terrain.postprocessing().iter().enumerate() {
        out.push(format!("post {section} {}", offsets.len()));
        out.extend(offsets.iter().map(u16::to_string));
    }
    out
}

/// Compare generated chunks with vanilla's post-carver dumps in server.txt.
fn verify(data: &std::sync::Arc<GeneratorData>, parity: &Path) -> Result<(), Error> {
    let cases = [
        (-1i64, -2, 3),
        (0x0123_4567_89ab_cdef, 1_875_000, -1_875_000),
    ];
    let mut expected = Vec::new();
    for (seed, x, z) in cases {
        let generator = OverworldGenerator::without_structures(data.clone(), seed)?;
        let lines = dump(&generator.worker().generate_proto(ChunkPos::new(x, z))?);
        if let Ok(path) = std::env::var("HORIZON_VERIFY_DUMP") {
            fs::write(format!("{path}{seed}.txt"), lines.join("\n") + "\n")?;
        }
        expected.push((seed, lines));
    }
    let mut matched = vec![false; expected.len()];
    let mut lines = BufReader::new(fs::File::open(parity.join("server.txt"))?).lines();
    let mut candidates = 0;
    while let Some(line) = lines.next() {
        if line? != "generatedcarving ok" {
            continue;
        }
        candidates += 1;
        // Each candidate dump is followed by other output; compare per case.
        let mut active = (0..expected.len()).collect::<Vec<_>>();
        let length = expected.iter().map(|(_, e)| e.len()).max().unwrap_or(0);
        for i in 0..length {
            let Some(line) = lines.next() else { break };
            let line = line?;
            active.retain(|&c| {
                let expected = &expected[c].1;
                if i == expected.len() {
                    matched[c] = true;
                }
                i < expected.len() && expected[i] == line
            });
            if active.is_empty() {
                break;
            }
        }
        for c in active {
            matched[c] = true;
        }
    }
    for ((seed, _), ok) in expected.iter().zip(&matched) {
        println!(
            "verify seed {seed}: {}",
            if *ok { "MATCH" } else { "MISMATCH" }
        );
    }
    println!("({candidates} vanilla generated-terrain dumps scanned)");
    if matched.iter().all(|m| *m) {
        Ok(())
    } else {
        Err("generated chunks differ from vanilla".into())
    }
}

/// Structure starts in the oracle's line format (VanillaStructureStartOracle).
fn dump_starts(
    generator: &OverworldGenerator,
    seed: i64,
    [x0, z0, x1, z1]: [i32; 4],
    out: &Path,
) -> Result<(), Error> {
    use horizon_world::worldgen::structure::{Rotation, start::Piece};
    let _ = seed;
    let mut worker = generator.worker();
    let mut lines = Vec::new();
    let start = Instant::now();
    for cx in x0..=x1 {
        for cz in z0..=z1 {
            for s in worker.structure_starts(ChunkPos::new(cx, cz)) {
                let b = s.bounding_box;
                lines.push(format!(
                    "start {cx} {cz} {} {} {} {} {} {} {} {}",
                    s.name,
                    b.min[0],
                    b.min[1],
                    b.min[2],
                    b.max[0],
                    b.max[1],
                    b.max[2],
                    s.pieces.len()
                ));
                for piece in &s.pieces {
                    let Piece::Pool(p) = piece else {
                        let b = piece.bounding_box();
                        lines.push(format!(
                            "piece {} {} {} {} {} {} {}",
                            piece.type_name(),
                            b.min[0],
                            b.min[1],
                            b.min[2],
                            b.max[0],
                            b.max[1],
                            b.max[2]
                        ));
                        continue;
                    };
                    let b = p.bounding_box;
                    let rotation = match p.rotation {
                        Rotation::None => "NONE",
                        Rotation::Clockwise90 => "CLOCKWISE_90",
                        Rotation::Clockwise180 => "CLOCKWISE_180",
                        Rotation::Counterclockwise90 => "COUNTERCLOCKWISE_90",
                    };
                    lines.push(format!(
                        "piece minecraft:jigsaw {} {} {} {} {} {} {} {} {} {rotation} {} {}",
                        b.min[0],
                        b.min[1],
                        b.min[2],
                        b.max[0],
                        b.max[1],
                        b.max[2],
                        p.position[0],
                        p.position[1],
                        p.position[2],
                        p.ground_level_delta,
                        p.junctions.len()
                    ));
                }
            }
        }
    }
    println!(
        "structure starts for {} chunks in {:.2?}",
        (x1 - x0 + 1) * (z1 - z0 + 1),
        start.elapsed()
    );
    fs::write(out, lines.join("\n") + "\n")?;
    Ok(())
}

/// Process CPU seconds (Linux), robust against other load on shared machines.
fn cpu_seconds() -> f64 {
    fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|stat| {
            let fields = stat
                .rsplit_once(')')?
                .1
                .split_whitespace()
                .collect::<Vec<_>>();
            Some((fields[11].parse::<f64>().ok()? + fields[12].parse::<f64>().ok()?) / 100.)
        })
        .unwrap_or(0.)
}
fn glyph(name: &str) -> char {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    match name {
        "water" => '~',
        "lava" => '!',
        "dirt_path" => '_',
        "cobblestone" | "mossy_cobblestone" => 'o',
        "hay_block" => 'h',
        _ if name.ends_with("planks") || name.ends_with("stairs") || name.ends_with("slab") => 'P',
        _ if name.contains("glass") => 'g',
        "grass_block" => '"',
        "sand" | "suspicious_sand" => '.',
        "red_sand" => ',',
        "sandstone" | "red_sandstone" => '=',
        "stone" | "deepslate" | "andesite" | "diorite" | "granite" | "tuff" => '#',
        "gravel" => ':',
        "dirt" | "coarse_dirt" | "rooted_dirt" => 'd',
        "podzol" => 'p',
        "mycelium" => 'm',
        "mud" => 'u',
        "snow" | "snow_block" | "powder_snow" => 's',
        "ice" | "packed_ice" | "blue_ice" => 'i',
        "clay" => 'c',
        "calcite" => 'C',
        _ if name.ends_with("terracotta") => 't',
        _ if name.ends_with("leaves") => '*',
        _ if name.ends_with("log") || name.ends_with("wood") => '|',
        _ if name.contains("grass") || name.contains("fern") || name.contains("flower") => '\'',
        _ => '?',
    }
}

fn main() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let resources = PathBuf::from(args.next().expect("client resource directory"));
    let (mut seed, mut size, mut threads, mut center) = (0i64, 16i32, 2usize, (0, 0));
    let mut parity = None;
    let mut snapshot_path = None;
    let mut decorate = true;
    let (mut warmup, mut json, mut map) = (false, false, true);
    let mut dimension = String::from("minecraft:overworld");
    let mut starts: Option<(i32, i32, i32, i32, PathBuf)> = None;
    while let Some(arg) = args.next() {
        let mut next = || args.next().expect("argument value");
        match arg.as_str() {
            "--seed" => seed = next().parse()?,
            "--size" => size = next().parse()?,
            "--threads" => threads = next().parse()?,
            "--center" => center = (next().parse()?, next().parse()?),
            "--verify" => parity = Some(PathBuf::from(next())),
            "--snapshot" => snapshot_path = Some(PathBuf::from(next())),
            "--no-decorate" => decorate = false,
            "--warmup" => warmup = true,
            "--json" => json = true,
            "--no-map" => map = false,
            "--dimension" => {
                let d = next();
                dimension = if d.contains(':') {
                    d
                } else {
                    format!("minecraft:{d}")
                };
            }
            "--starts" => {
                starts = Some((
                    next().parse()?,
                    next().parse()?,
                    next().parse()?,
                    next().parse()?,
                    PathBuf::from(next()),
                ))
            }
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    let start = Instant::now();
    let registry = registry(&resources)?;
    let data = GeneratorData::load(&resources.join("worldgen"), registry)?;
    println!("loaded resources in {:.2?}", start.elapsed());
    if let Some(parity) = parity {
        verify(&data, &parity)?;
    }
    let start = Instant::now();
    let generator =
        OverworldGenerator::for_dimension(data.clone(), seed, "minecraft:normal", &dimension)?;
    if std::env::var_os("HORIZON_LIST_UNSUPPORTED").is_some() {
        for (feature, error) in generator.unsupported_features() {
            println!("unsupported {feature}: {error}");
        }
    }
    println!("seeded generator in {:.2?}", start.elapsed());
    if let Some((x0, z0, x1, z1, out)) = starts {
        dump_starts(&generator, seed, [x0, z0, x1, z1], &out)?;
        return Ok(());
    }
    if warmup {
        let mut worker = generator.worker();
        for z in -4..=4 {
            for x in -4..=4 {
                std::hint::black_box(
                    worker.generate_proto(ChunkPos::new(center.0 + 64 + x, center.1 + 64 + z))?,
                );
            }
        }
    }
    let (x0, z0) = (center.0 - size / 2, center.1 - size / 2);
    let count = (size * size) as usize;
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<ProtoChunk>>> = (0..count).map(|_| Mutex::new(None)).collect();
    let timings = Mutex::new(Vec::new());
    let cpu = cpu_seconds();
    let start = Instant::now();
    std::thread::scope(|scope| -> Result<(), Error> {
        let handles = (0..threads)
            .map(|_| {
                scope.spawn(|| -> Result<(), Error> {
                    let mut worker = generator.worker();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= count {
                            timings.lock().unwrap().push(worker.stage_timings());
                            return Ok(());
                        }
                        let pos = ChunkPos::new(x0 + (i as i32 % size), z0 + (i as i32 / size));
                        *slots[i].lock().unwrap() = Some(worker.generate_proto(pos)?);
                    }
                })
            })
            .collect::<Vec<_>>();
        handles.into_iter().try_for_each(|h| h.join().unwrap())
    })?;
    let elapsed = start.elapsed();
    let cpu_elapsed = cpu_seconds() - cpu;
    println!(
        "generated {count} proto chunks with {threads} threads in {elapsed:.2?}: {:.1} chunks/s ({:.1} ms/chunk/thread)",
        count as f64 / elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000. * threads as f64 / count as f64
    );
    println!(
        "  cpu time {:.1} ms/chunk",
        cpu_elapsed * 1000. / count as f64
    );
    let timings = timings.into_inner().unwrap();
    let per_chunk = |f: fn(&StageTimings) -> std::time::Duration| {
        timings
            .iter()
            .map(f)
            .sum::<std::time::Duration>()
            .as_secs_f64()
            * 1000.
            / count as f64
    };
    println!(
        "  per chunk: biomes {:.2} ms, noise {:.2} ms, surface {:.2} ms, carvers {:.2} ms",
        per_chunk(|t| t.biomes),
        per_chunk(|t| t.noise),
        per_chunk(|t| t.surface),
        per_chunk(|t| t.carvers)
    );
    if json {
        println!(
            "BENCH {}",
            serde_json::json!({"phase": "proto", "chunks": count,
            "threads": threads, "wall_s": elapsed.as_secs_f64(), "cpu_s": cpu_elapsed,
            "biomes_ms_chunk": per_chunk(|t| t.biomes), "noise_ms_chunk": per_chunk(|t| t.noise),
            "structures_ms_chunk": per_chunk(|t| t.structures),
            "surface_ms_chunk": per_chunk(|t| t.surface), "carvers_ms_chunk": per_chunk(|t| t.carvers)})
        );
    }
    let mut chunks: Vec<Option<ProtoChunk>> = slots
        .into_iter()
        .map(|slot| slot.into_inner().unwrap())
        .collect();
    if let Some(path) = &snapshot_path {
        snapshot(&chunks, path, "terrain")?;
    }
    if decorate && size >= 3 {
        let totals = Mutex::new(DecorateStats::default());
        let start = Instant::now();
        let mut decorated = 0;
        // Disjoint 3×3 regions per phase can be decorated concurrently.
        for phase in 0..9 {
            let mut regions = Vec::new();
            for z in 1..size - 1 {
                for x in 1..size - 1 {
                    if (x % 3) * 3 + z % 3 != phase {
                        continue;
                    }
                    let mut region = Vec::with_capacity(9);
                    for dz in -1..=1 {
                        for dx in -1..=1 {
                            region
                                .push(chunks[((z + dz) * size + x + dx) as usize].take().unwrap());
                        }
                    }
                    regions.push(((x, z), region));
                }
            }
            decorated += regions.len();
            let queue = Mutex::new(regions);
            let done = Mutex::new(Vec::new());
            std::thread::scope(|scope| -> Result<(), Error> {
                let handles = (0..threads)
                    .map(|_| {
                        scope.spawn(|| -> Result<(), Error> {
                            let mut worker: GeneratorWorker = generator.worker();
                            loop {
                                let Some((at, region)) = queue.lock().unwrap().pop() else {
                                    return Ok(());
                                };
                                let mut region: [ProtoChunk; 9] =
                                    region.try_into().map_err(|_| "region size")?;
                                let stats = worker.decorate(&mut region)?;
                                let mut totals = totals.lock().unwrap();
                                totals.placed += stats.placed;
                                totals.unsupported += stats.unsupported;
                                totals.failed += stats.failed;
                                totals.skipped.extend(stats.skipped);
                                drop(totals);
                                done.lock().unwrap().push((at, region));
                            }
                        })
                    })
                    .collect::<Vec<_>>();
                handles.into_iter().try_for_each(|h| h.join().unwrap())
            })?;
            for ((x, z), region) in done.into_inner().unwrap() {
                for (i, chunk) in region.into_iter().enumerate() {
                    let (dx, dz) = (i as i32 % 3 - 1, i as i32 / 3 - 1);
                    chunks[((z + dz) * size + x + dx) as usize] = Some(chunk);
                }
            }
        }
        let elapsed = start.elapsed();
        if json {
            println!(
                "BENCH {}",
                serde_json::json!({"phase": "decorate", "chunks": decorated,
                "wall_s": elapsed.as_secs_f64()})
            );
        }
        let totals = totals.into_inner().unwrap();
        println!(
            "decorated {decorated} chunks in {elapsed:.2?}: {:.1} chunks/s; feature roots placed {}, unsupported {}, failed {}",
            decorated as f64 / elapsed.as_secs_f64(),
            totals.placed,
            totals.unsupported,
            totals.failed
        );
        if std::env::var_os("HORIZON_FEATURE_ERRORS").is_some() {
            for (name, error) in generator.unsupported_features() {
                println!("unsupported {name}: {error}");
            }
        }
        if !totals.skipped.is_empty() {
            println!(
                "skipped roots: {}",
                totals.skipped.into_iter().collect::<Vec<_>>().join(" ")
            );
        }
    }
    if let Some(path) = &snapshot_path {
        snapshot(&chunks, path, "decorated")?;
    }
    let mut structures = BTreeMap::<(String, i32, i32), [[i32; 3]; 2]>::new();
    for chunk in chunks.iter().flatten() {
        for (name, start, min, max) in chunk.structures() {
            structures.insert((name.to_owned(), start.x, start.z), [min, max]);
        }
    }
    for ((name, x, z), [min, max]) in &structures {
        println!("structure {name} start chunk ({x}, {z}) box {min:?}..{max:?}");
    }
    let block_entities: usize = chunks
        .iter()
        .flatten()
        .map(|c| c.block_entities.len())
        .sum();
    println!("block entities: {block_entities}");
    if std::env::var_os("HORIZON_BLOCK_ENTITIES").is_some() {
        for chunk in chunks.iter().flatten() {
            for ([x, y, z], nbt) in &chunk.block_entities {
                let text = |key: &str| match nbt.get(&key.into()) {
                    Some(Tag::String(s)) => s.to_string_lossy(),
                    _ => "-".into(),
                };
                println!(
                    "block_entity {x} {y} {z} {} {} {}",
                    chunk.block_name(*x, *y, *z),
                    text("id"),
                    text("LootTable")
                );
            }
        }
    }
    if !map {
        return Ok(());
    }
    let mut legend = BTreeMap::<char, BTreeMap<String, usize>>::new();
    let step = (size * 16 / 96).max(1);
    for bz in (0..size * 16).step_by(step as usize) {
        let mut row = String::new();
        for bx in (0..size * 16).step_by(step as usize) {
            let chunk = chunks[((bz / 16) * size + bx / 16) as usize]
                .as_ref()
                .unwrap();
            let (x, z) = (
                chunk.position().x * 16 + bx % 16,
                chunk.position().z * 16 + bz % 16,
            );
            let y = chunk.surface_height(x, z) - 1;
            let name = chunk.block_name(x, y, z).to_owned();
            let c = glyph(&name);
            *legend.entry(c).or_default().entry(name).or_default() += 1;
            row.push(c);
        }
        println!("{row}");
    }
    for (c, names) in legend {
        let mut names = names.into_iter().collect::<Vec<_>>();
        names.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let names = names
            .into_iter()
            .take(4)
            .map(|(n, _)| n)
            .collect::<Vec<_>>();
        println!("  {c} {}", names.join(" "));
    }
    let middle = chunks[(size / 2 * size + size / 2) as usize]
        .as_ref()
        .unwrap();
    let p = middle.position();
    println!(
        "biome at center ({}, 64, {}): {}",
        p.x * 16,
        p.z * 16,
        middle.biome_at(p.x * 16, 64, p.z * 16)
    );
    Ok(())
}

// Diagnostic only: full ordered chunk state, outside performance measurements.
fn snapshot(chunks: &[Option<ProtoChunk>], path: &Path, phase: &str) -> Result<(), Error> {
    use std::io::Write;
    let mut out =
        std::io::BufWriter::new(fs::File::create(format!("{}-{phase}.bin", path.display()))?);
    for chunk in chunks.iter().flatten() {
        let terrain = &chunk.terrain;
        writeln!(
            out,
            "chunk {:?} {:?} {} {:?} {:?}",
            chunk.position(),
            terrain.generation_stage(),
            chunk.is_decorated(),
            chunk.structures(),
            terrain.below_zero_retrogen().map(|v| v.to_nbt())
        )?;
        for section in 0..terrain.height() as usize / 16 {
            let data = terrain.section(section).unwrap();
            writeln!(out, "section {section} {:?}", data.to_nbt()?)?;
            for index in 0..4096 {
                out.write_all(&data.get(index)?.0.to_le_bytes())?;
            }
        }
        // Compare public reads too, including all-air section behavior.
        for y in terrain.min_y()..terrain.min_y() + terrain.height() {
            for z in 0..16 {
                for x in 0..16 {
                    out.write_all(&terrain.block(x, y, z).0.to_le_bytes())?;
                }
            }
        }
        for y in (terrain.min_y() >> 2)..((terrain.min_y() + terrain.height()) >> 2) {
            for z in 0..4 {
                for x in 0..4 {
                    writeln!(
                        out,
                        "biome {}",
                        chunk.biome_at(
                            chunk.position().x * 16 + x * 4,
                            y * 4,
                            chunk.position().z * 16 + z * 4
                        )
                    )?;
                }
            }
        }
        for kind in HeightmapType::ALL {
            writeln!(
                out,
                "heightmap {kind:?} {:?}",
                terrain.heightmap(kind).map(|m| m.raw_data())
            )?;
        }
        writeln!(out, "post {:?}", terrain.postprocessing())?;
        writeln!(out, "blockticks {:?}", terrain.block_ticks().pack(0))?;
        writeln!(out, "fluidticks {:?}", terrain.fluid_ticks().pack(0))?;
        for (position, tag) in &chunk.block_entities {
            writeln!(out, "blockentity {position:?}")?;
            let bytes =
                horizon_world::nbt::write_network(&Tag::Compound(tag.clone()), Default::default())?;
            out.write_all(&(bytes.len() as u64).to_le_bytes())?;
            out.write_all(&bytes)?;
        }
        let entities = Tag::List(chunk.entities.iter().cloned().map(Tag::Compound).collect());
        let bytes = horizon_world::nbt::write_network(&entities, Default::default())?;
        out.write_all(&(bytes.len() as u64).to_le_bytes())?;
        out.write_all(&bytes)?;
    }
    out.flush()?;
    Ok(())
}
