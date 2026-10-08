//! Differential check of decorated chunks against a vanilla world.
//!
//! vanilla_diff <client resource dir> <vanilla world dir> --seed S
//!     [--dimension minecraft:overworld] [--from CX CZ] [--size N]
//!     [--threads T] [--show FROM->TO] [--limit N] [--reverse] [--shuffle SEED]
//!     [--other <second vanilla world dir>]
//!
//! Generates an (N+4)² chunk area with the Rust pipeline, decorates the inner
//! (N+2)² chunks (vanilla FEATURES writes reach one neighbor) and compares the
//! N² chunks starting at (CX, CZ) block by block with the region files of a
//! vanilla world generated for the same seed (with ticks frozen, see
//! docs/parity/worldgen.md). Prints mismatches grouped by vanilla → Rust block.
use horizon_core::position::{BlockPos, ChunkPos};
use horizon_world::{
    DATA_VERSION,
    chunk::{BlockSection, BlockState, ChunkData},
    nbt::{Limits, Tag},
    region::RegionFile,
    registry::BlockRegistry,
    worldgen::generator::{GeneratorData, OverworldGenerator, ProtoChunk},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
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

fn describe(state: &BlockState) -> String {
    if state.properties.is_empty() {
        return state.name.clone();
    }
    let props: Vec<String> = state
        .properties
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    format!("{}[{}]", state.name, props.join(","))
}

fn region_dir(world: &Path, dimension: &str) -> PathBuf {
    let modern = world
        .join("dimensions")
        .join(dimension.replace(':', "/"))
        .join("region");
    if modern.is_dir() {
        return modern;
    }
    match dimension {
        "minecraft:the_nether" => world.join("DIM-1/region"),
        "minecraft:the_end" => world.join("DIM1/region"),
        _ => world.join("region"),
    }
}

/// Terrain-stage chunks of the `full`² area at `origin`, row-major.
fn generate_area(
    generator: &OverworldGenerator,
    origin: (i32, i32),
    full: i32,
    threads: usize,
) -> Result<Vec<ProtoChunk>, Error> {
    let count = (full * full) as usize;
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<ProtoChunk>>> = (0..count).map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| -> Result<(), Error> {
        let handles = (0..threads)
            .map(|_| {
                scope.spawn(|| -> Result<(), Error> {
                    let mut worker = generator.worker();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= count {
                            return Ok(());
                        }
                        let pos = ChunkPos::new(
                            origin.0 + (i as i32 % full),
                            origin.1 + (i as i32 / full),
                        );
                        *slots[i].lock().unwrap() = Some(worker.generate_proto(pos)?);
                    }
                })
            })
            .collect::<Vec<_>>();
        handles.into_iter().try_for_each(|h| h.join().unwrap())
    })?;
    Ok(slots
        .into_iter()
        .map(|s| s.into_inner().unwrap().unwrap())
        .collect())
}

/// Decorates the chunks at `order` (indices into the `full`² area, each with
/// all eight neighbors present) in that order, one FEATURES pass each.
fn decorate_area(
    generator: &OverworldGenerator,
    protos: Vec<ProtoChunk>,
    full: i32,
    order: &[(i32, i32)],
) -> Result<Vec<ProtoChunk>, Error> {
    let mut chunks: Vec<Option<ProtoChunk>> = protos.into_iter().map(Some).collect();
    let mut worker = generator.worker();
    let (mut unsupported_roots, mut failed) = (0, 0);
    let mut skipped = std::collections::BTreeSet::new();
    for &(x, z) in order {
        let mut region = Vec::with_capacity(9);
        for dz in -1..=1 {
            for dx in -1..=1 {
                region.push(chunks[((z + dz) * full + x + dx) as usize].take().unwrap());
            }
        }
        let mut region: [ProtoChunk; 9] = region.try_into().map_err(|_| "region")?;
        let stats = worker.decorate(&mut region)?;
        unsupported_roots += stats.unsupported;
        failed += stats.failed;
        skipped.extend(stats.skipped);
        for (i, chunk) in region.into_iter().enumerate() {
            let (dx, dz) = (i as i32 % 3 - 1, i as i32 / 3 - 1);
            chunks[((z + dz) * full + x + dx) as usize] = Some(chunk);
        }
    }
    println!("decoration: unsupported roots {unsupported_roots}, failed {failed}");
    if !skipped.is_empty() {
        println!(
            "skipped: {}",
            skipped.into_iter().collect::<Vec<_>>().join(" ")
        );
    }
    Ok(chunks.into_iter().map(Option::unwrap).collect())
}

/// Lazily opened region files of one dimension.
struct Regions {
    dir: PathBuf,
    files: HashMap<(i32, i32), Option<RegionFile>>,
}
impl Regions {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            files: HashMap::new(),
        }
    }
    fn chunk(&mut self, pos: ChunkPos) -> Result<Option<ChunkData>, Error> {
        let key = (pos.x >> 5, pos.z >> 5);
        let dir = &self.dir;
        let file = self.files.entry(key).or_insert_with(|| {
            RegionFile::open(dir.join(format!("r.{}.{}.mca", key.0, key.1))).ok()
        });
        let Some(file) = file else { return Ok(None) };
        let Some(record) = file.read(pos)? else {
            return Ok(None);
        };
        Ok(Some(ChunkData::new(record.nbt(Limits::default())?)?))
    }
}

fn main() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let resources = PathBuf::from(args.next().expect("client resource directory"));
    let world = PathBuf::from(args.next().expect("vanilla world directory"));
    let (mut seed, mut from, mut size, mut threads) = (0i64, (0, 0), 8i32, 4usize);
    let mut dimension = String::from("minecraft:overworld");
    let mut show: Option<String> = None;
    let mut limit = 40usize;
    let mut other: Option<PathBuf> = None;
    let (mut reverse, mut shuffle) = (false, 0i64);
    while let Some(arg) = args.next() {
        let mut next = || args.next().expect("argument value");
        match arg.as_str() {
            "--seed" => seed = next().parse()?,
            "--from" => from = (next().parse()?, next().parse()?),
            "--size" => size = next().parse()?,
            "--threads" => threads = next().parse()?,
            "--dimension" => dimension = next(),
            "--show" => show = Some(next()),
            "--limit" => limit = next().parse()?,
            "--other" => other = Some(PathBuf::from(next())),
            "--reverse" => reverse = true,
            "--shuffle" => shuffle = next().parse()?,
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    let registry = registry(&resources)?;
    let data = GeneratorData::load(&resources.join("worldgen"), registry)?;
    let generator = OverworldGenerator::for_dimension(data, seed, "minecraft:normal", &dimension)?;
    let unsupported = generator.unsupported_features();
    println!("unsupported placed features: {}", unsupported.len());

    // Rust: terrain for (size+4)², decoration for the inner (size+2)².
    let full = size + 4;
    let (x0, z0) = (from.0 - 2, from.1 - 2);
    let mut order: Vec<(i32, i32)> = (1..full - 1)
        .flat_map(|z| (1..full - 1).map(move |x| (x, z)))
        .collect();
    if reverse {
        let mut reversed = order.clone();
        reversed.reverse();
        let alternate = decorate_area(
            &generator,
            generate_area(&generator, (x0, z0), full, threads)?,
            full,
            &reversed,
        )?;
        let chunks = decorate_area(
            &generator,
            generate_area(&generator, (x0, z0), full, threads)?,
            full,
            &order,
        )?;
        let mut pairs: BTreeMap<(String, String), usize> = BTreeMap::new();
        let mut total = 0;
        let name = |id| {
            registry
                .state(id)
                .map(|s: &BlockState| s.name.trim_start_matches("minecraft:").to_owned())
                .unwrap_or_default()
        };
        for (a, b) in chunks.iter().zip(&alternate) {
            let p = a.position();
            if p.x < from.0 || p.z < from.1 || p.x >= from.0 + size || p.z >= from.1 + size {
                continue;
            }
            for section in 0..a.terrain.height() as usize / 16 {
                let (sa, sb) = (
                    a.terrain.section(section).unwrap(),
                    b.terrain.section(section).unwrap(),
                );
                for index in 0..4096 {
                    let (x, y) = (sa.get(index)?, sb.get(index)?);
                    if x != y {
                        total += 1;
                        *pairs.entry((name(x), name(y))).or_default() += 1;
                    }
                }
            }
        }
        println!("decoration order sensitivity (raster vs reverse): {total} blocks differ");
        let mut sorted: Vec<_> = pairs.into_iter().collect();
        sorted.sort_by_key(|p| std::cmp::Reverse(p.1));
        for ((v, r), n) in sorted.iter().take(limit) {
            println!("{n:>9}  {v} <-> {r}");
        }
        return Ok(());
    }
    if shuffle != 0 {
        // Deterministic Fisher-Yates with a 64-bit LCG.
        let mut state = shuffle as u64;
        for i in (1..order.len()).rev() {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            order.swap(i, (state >> 33) as usize % (i + 1));
        }
    }
    let chunks = match other {
        Some(_) => Vec::new(),
        None => decorate_area(
            &generator,
            generate_area(&generator, (x0, z0), full, threads)?,
            full,
            &order,
        )?,
    };
    let (min_y, height) = match dimension.as_str() {
        "minecraft:overworld" => (-64, 384),
        _ => (0, 256),
    };

    // Vanilla region files.
    let mut vanilla = Regions::new(region_dir(&world, &dimension));
    let mut second = other.map(|w| Regions::new(region_dir(&w, &dimension)));
    let air = registry.default_state("minecraft:air").unwrap();
    let mut pairs: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut examples: Vec<[i32; 3]> = Vec::new();
    let (mut compared, mut differ, mut missing) = (0usize, 0usize, 0usize);
    let mut per_chunk = Vec::new();
    for cz in from.1..from.1 + size {
        for cx in from.0..from.0 + size {
            let pos = ChunkPos::new(cx, cz);
            let Some(theirs) = vanilla.chunk(pos)? else {
                missing += 1;
                continue;
            };
            let ours = match &mut second {
                Some(second) => match second.chunk(pos)? {
                    Some(chunk) => Some(chunk),
                    None => {
                        missing += 1;
                        continue;
                    }
                },
                None => None,
            };
            let status = match theirs.root().tag.get("Status") {
                Some(Tag::String(s)) => s.to_string().unwrap_or_default(),
                _ => String::new(),
            };
            let rust = chunks.get(((cz - z0) * full + cx - x0) as usize);
            if size == 1 {
                println!(
                    "vanilla structures: {:?}",
                    theirs.root().tag.get("structures")
                );
                if let Some(rust) = rust {
                    println!("rust structures: {:?}", rust.structures());
                }
            }
            if status != "minecraft:full" {
                println!("chunk {cx} {cz}: vanilla status {status}");
            }
            let mut chunk_differ = 0;
            for section in 0..height as usize / 16 {
                let base_y = min_y + section as i32 * 16;
                let at = BlockPos::new(cx * 16, base_y, cz * 16);
                let ours_loaded;
                let ours: Option<&BlockSection> = match (&ours, rust) {
                    (Some(chunk), _) => {
                        ours_loaded = chunk.load_block_section(at, registry)?;
                        ours_loaded.as_ref()
                    }
                    (None, Some(rust)) => rust.terrain.section(section),
                    (None, None) => None,
                };
                let theirs = theirs.load_block_section(at, registry)?;
                for index in 0..4096 {
                    let a = ours.map_or(Ok(air), |s| s.get(index))?;
                    let b = theirs.as_ref().map_or(Ok(air), |s| s.get(index))?;
                    compared += 1;
                    if a == b {
                        continue;
                    }
                    differ += 1;
                    chunk_differ += 1;
                    let (va, ru) = (
                        registry.state(b).map(describe).unwrap_or_default(),
                        registry.state(a).map(describe).unwrap_or_default(),
                    );
                    let short = |s: &str| {
                        s.split('[')
                            .next()
                            .unwrap()
                            .trim_start_matches("minecraft:")
                            .to_owned()
                    };
                    let pair = (short(&va), short(&ru));
                    let p = [
                        cx * 16 + (index & 15) as i32,
                        base_y + (index >> 8) as i32,
                        cz * 16 + ((index >> 4) & 15) as i32,
                    ];
                    if let Some(filter) = &show {
                        let label = format!("{}->{}", pair.0, pair.1);
                        if &label == filter && examples.len() < limit {
                            examples.push(p);
                            println!("  {p:?}: vanilla {va} rust {ru}");
                        }
                    }
                    *pairs.entry(pair).or_default() += 1;
                }
            }
            per_chunk.push(((cx, cz), chunk_differ));
        }
    }
    println!(
        "compared {compared} blocks in {} chunks ({missing} missing): {differ} differ ({:.4}%)",
        per_chunk.len(),
        differ as f64 * 100.0 / compared.max(1) as f64
    );
    let mut sorted: Vec<_> = pairs.into_iter().collect();
    sorted.sort_by_key(|p| std::cmp::Reverse(p.1));
    for ((v, r), n) in sorted.iter().take(limit) {
        println!("{n:>9}  vanilla {v} -> rust {r}");
    }
    // Decoration order at chunk borders differs between runs (vanilla's own
    // runs disagree), which mostly swaps blocks symmetrically. One-sided
    // differences point at real generation differences.
    let counts: BTreeMap<_, _> = sorted.iter().cloned().collect();
    let mut net: Vec<(i64, &(String, String))> = counts
        .iter()
        .map(|(pair, &n)| {
            let back = counts
                .get(&(pair.1.clone(), pair.0.clone()))
                .copied()
                .unwrap_or(0);
            (n as i64 - back as i64, pair)
        })
        .filter(|(n, _)| *n > 0)
        .collect();
    net.sort_by_key(|p| std::cmp::Reverse(p.0));
    println!("one-sided differences:");
    for (n, (v, r)) in net.iter().take(limit) {
        println!("{n:>9}  vanilla {v} -> rust {r}");
    }
    let clean = per_chunk.iter().filter(|(_, n)| *n == 0).count();
    println!("chunks identical: {clean}/{}", per_chunk.len());
    let mut worst = per_chunk.clone();
    worst.sort_by_key(|p| std::cmp::Reverse(p.1));
    for ((x, z), n) in worst.iter().take(8).filter(|(_, n)| *n > 0) {
        println!("  chunk {x} {z}: {n}");
    }
    Ok(())
}
