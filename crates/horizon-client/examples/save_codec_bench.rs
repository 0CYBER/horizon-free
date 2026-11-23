//! Where a chunk save and load spend their time, what `SaveDir` does with whole
//! batches, and what pure-Rust codecs already in the tree would cost or save:
//!
//!     cargo run --profile fast -p horizon-client-lib --example save_codec_bench -- \
//!         [--chunks 4096] [--rounds 5] [--threads N] [--scratch DIR] [--world DIR]
//!
//! The chunks are deterministic synthetic overworld columns (no game files):
//! rolling terrain over deepslate and bedrock, stone variants, ores, caves, sea
//! level water, sand and grass, a few biomes and sky light. `--world DIR` reads
//! the NBT records of a real world's `region/` instead, for the NBT rows only
//! (decoding them needs that world's registry).
//!
//! Every timed figure is the median of `--rounds` rounds and their range; within
//! a round each codec and stage runs over every chunk on this thread, in turn,
//! and the CPU figures are this thread's CPU time. The `SaveDir` figures are wall
//! time on the scratch folder's filesystem (`SaveDir::save` syncs each region,
//! as an autosave does), so they depend on its disk.
//!
//! Codecs: `nbt` is the chunk as vanilla saves it (`encode_with`, NBT bytes);
//! `wire` is each section's resident palette and words (`to_wire`) and light,
//! the shape of the native record of #19801 without its other fields (block
//! entities, ticks, structures), checked to read back equal. `zlib-N` is
//! flate2's zlib-rs at level N (vanilla's default is 6), `lz4` the lz4_flex
//! block in lz4-java's framing (vanilla's `region-file-compression=lz4`).
extern crate horizon_client_lib as horizon_client;
use flate2::{Compression as Level, write::ZlibEncoder};
use horizon_client::{
    biome::Biomes,
    chunk::{BIOME_STRATEGY, Chunk, ColumnLayout, LevelShape, Light, Section, block_strategy},
    save::{self, SaveDir},
    spans::thread_cpu,
};
use horizon_core::{
    nbt::{self, Limits, NamedTag, Tag},
    paletted::PalettedContainer,
};
use horizon_world::{
    DATA_VERSION,
    compression::Compression,
    region::{RegionFile, position_from_path},
    registry::BlockRegistry,
};
use std::{
    hint::black_box,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

type Error = Box<dyn std::error::Error>;

/// A folder this run made under `parent`, removed with everything in it
/// when dropped. A name already taken is skipped, never cleared, and
/// `parent` (made if missing) is never removed.
struct Scratch(PathBuf);
impl Scratch {
    fn new(parent: &Path, name: &str) -> std::io::Result<Self> {
        std::fs::create_dir_all(parent)?;
        let mut n = 0u32;
        loop {
            let dir = parent.join(format!("{name}-{}-{n}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => return Err(e),
            }
        }
    }
}
impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const STATES: &str = "air stone deepslate dirt grass_block water sand gravel bedrock granite diorite \
    andesite tuff coal_ore iron_ore copper_ore deepslate_iron_ore deepslate_diamond_ore lava cave_air";
const BIOMES: [&str; 4] = ["plains", "forest", "river", "ocean"];

fn hash(x: i32, y: i32, z: i32, salt: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27D4_EB2D) ^ (y as u32).wrapping_mul(0x1656_67B1);
    h ^= (z as u32).wrapping_mul(0x9E37_79B1) ^ salt.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ h >> 12
}

fn surface(x: i32, z: i32) -> i32 {
    let (fx, fz) = (x as f32, z as f32);
    64 + ((fx / 37.0).sin() * 9.0 + (fz / 29.0).cos() * 7.0 + ((fx + fz) / 11.0).sin() * 2.0) as i32
}

/// One synthetic overworld column; state IDs index [`STATES`].
fn column(cx: i32, cz: i32) -> Chunk {
    let id = |name| STATES.split_whitespace().position(|s| s == name).unwrap() as u32;
    let mut chunk = Chunk::empty(cx, cz);
    chunk.lit = true;
    for (si, section) in chunk.sections.iter_mut().enumerate() {
        let base = -64 + si as i32 * 16;
        let mut blocks = [0u32; 4096];
        let mut light = Box::new([0u8; 4096]);
        for (i, (block, light)) in blocks.iter_mut().zip(light.iter_mut()).enumerate() {
            let (x, y, z) = (
                cx * 16 + (i & 15) as i32,
                base + (i >> 8) as i32,
                cz * 16 + (i >> 4 & 15) as i32,
            );
            let h = surface(x, z) + (hash(x, 0, z, 5) % 3) as i32 - 1;
            let r = hash(x, y, z, 1) % 1000;
            let blob = hash(x >> 1, y >> 1, z >> 1, 2) % 100;
            let cave = hash(x >> 2, y >> 1, z >> 2, 3) % 100 < 9 && y < h - 6 && y > -58;
            *block = match y {
                _ if y < -60 && r < 600 - (y + 64) as u32 * 150 => id("bedrock"),
                _ if cave => id(if y < -54 { "lava" } else { "cave_air" }),
                _ if y < 0 => match (blob, r) {
                    (0..8, _) => id("tuff"),
                    (_, 0..12) => id("deepslate_iron_ore"),
                    (_, 12..16) => id("deepslate_diamond_ore"),
                    _ => id("deepslate"),
                },
                _ if y < h - 3 => match (blob, r) {
                    (0..5, _) => id("granite"),
                    (5..9, _) => id("diorite"),
                    (9..14, _) => id("andesite"),
                    (14..16, _) => id("gravel"),
                    (_, 0..15) => id("coal_ore"),
                    (_, 15..25) => id("iron_ore"),
                    (_, 25..33) => id("copper_ore"),
                    _ => id("stone"),
                },
                _ if y < h && h < 63 => id("sand"),
                _ if y < h => id("dirt"),
                _ if y == h => id(if h < 63 { "sand" } else { "grass_block" }),
                _ if y <= 62 => id("water"),
                _ => id("air"),
            };
            // Sky light fading under the ground and the sea, block light in caves (lava, torches).
            let sky = 15 - (h - y).clamp(0, 15) - (62 - y).clamp(0, 15).min(i32::from(y > h) * 15);
            let block = if cave {
                hash(x >> 1, y, z >> 1, 6) % 12
            } else {
                0
            };
            *light = (sky.max(0) as u8) << 4 | block as u8;
        }
        let biome = |i: usize| {
            let (x, z) = (cx * 4 + (i & 3) as i32, cz * 4 + (i >> 2 & 3) as i32);
            (hash(x >> 4, 0, z >> 4, 4) % BIOMES.len() as u32 + u32::from(i >= 48 && si > 6))
                % BIOMES.len() as u32
        };
        let biomes: Vec<u32> = (0..64).map(biome).collect();
        let section = Arc::make_mut(section);
        section.blocks = PalettedContainer::from_values(block_strategy(), &blocks);
        section.biomes = PalettedContainer::from_values(BIOME_STRATEGY, &biomes);
        section.light = Light::packed(light);
    }
    chunk
}

fn fixture() -> Result<(&'static BlockRegistry, Arc<Biomes>), Error> {
    let names: Vec<String> = STATES
        .split_whitespace()
        .map(|n| format!("minecraft:{n}"))
        .collect();
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(
                names
                    .iter()
                    .map(|n| Tag::String(n.as_str().into()))
                    .collect(),
            ),
        ),
        (
            "defaults",
            Tag::compound(
                names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.as_str(), Tag::Int(i as i32))),
            ),
        ),
    ]))?;
    let temp = Scratch::new(&std::env::temp_dir(), "horizon-save-biomes")?;
    let textures = temp.join("assets/minecraft/textures/colormap");
    let biomes = temp.join("data/minecraft/worldgen/biome");
    std::fs::create_dir_all(&textures)?;
    std::fs::create_dir_all(&biomes)?;
    for name in ["grass", "foliage", "dry_foliage"] {
        let mut encoder = png::Encoder::new(
            std::fs::File::create(textures.join(format!("{name}.png")))?,
            1,
            1,
        );
        encoder.set_color(png::ColorType::Rgb);
        encoder.write_header()?.write_image_data(&[0, 128, 0])?;
    }
    for name in BIOMES {
        std::fs::write(biomes.join(format!("{name}.json")), b"{}")?;
    }
    let loaded = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        temp.to_path_buf(),
    ));
    drop(temp);
    Ok((Box::leak(Box::new(registry)), Arc::new(loaded?)))
}

fn put(out: &mut Vec<u8>, container: &PalettedContainer) {
    let (bits, palette, words) = container.to_wire();
    out.push(bits);
    out.extend((palette.len() as u16).to_le_bytes());
    palette.iter().for_each(|p| out.extend(p.to_le_bytes()));
    out.extend((words.len() as u16).to_le_bytes());
    words.iter().for_each(|w| out.extend(w.to_le_bytes()));
}

fn wire(chunk: &Chunk) -> Vec<u8> {
    let mut out = Vec::new();
    for section in &chunk.sections {
        put(&mut out, &section.blocks);
        put(&mut out, &section.biomes);
        match &section.light {
            Light::Uniform(v) => out.extend([0, *v]),
            Light::Sky(layer, v) => (out.extend([1, *v]), out.extend_from_slice(&layer[..])).1,
            Light::Block(layer, v) => (out.extend([2, *v]), out.extend_from_slice(&layer[..])).1,
            Light::Full(all) => (out.push(3), out.extend_from_slice(&all[..])).1,
        }
    }
    out
}

fn unwire(mut bytes: &[u8]) -> Option<Vec<Section>> {
    let mut take = |n: usize| -> Option<&[u8]> {
        let (head, rest) = bytes.split_at_checked(n)?;
        bytes = rest;
        Some(head)
    };
    let mut sections = Vec::with_capacity(24);
    for _ in 0..24 {
        let mut containers =
            [block_strategy(), BIOME_STRATEGY].map(|strategy| -> Option<PalettedContainer> {
                let bits = take(1)?[0];
                let n = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
                let palette = take(n * 4)?
                    .chunks(4)
                    .map(|p| u32::from_le_bytes(p.try_into().unwrap()))
                    .collect();
                let n = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
                let words = take(n * 8)?
                    .chunks(8)
                    .map(|w| u64::from_le_bytes(w.try_into().unwrap()))
                    .collect();
                PalettedContainer::from_wire(strategy, bits, palette, words).ok()
            });
        let layer = |b: &[u8]| -> Option<Box<[u8; 2048]>> { Some(Box::new(b.try_into().ok()?)) };
        let head = take(1)?[0];
        let light = match head {
            0 => Light::Uniform(take(1)?[0]),
            1 | 2 => {
                let v = take(1)?[0];
                let l = layer(take(2048)?)?;
                if head == 1 {
                    Light::Sky(l, v)
                } else {
                    Light::Block(l, v)
                }
            }
            _ => Light::Full(Box::new(take(4096)?.try_into().ok()?)),
        };
        let [blocks, biomes] = [containers[0].take()?, containers[1].take()?];
        sections.push(Section {
            blocks,
            biomes,
            light,
        });
    }
    Some(sections)
}

fn inflate(bytes: &[u8]) -> Vec<u8> {
    Compression::Zlib.decode(bytes, 64 << 20).unwrap()
}

fn zlib(level: u32, bytes: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Level::new(level));
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// This thread's CPU µs per item of `f` over `items`.
fn cpu_us<T, U>(items: &[T], mut f: impl FnMut(&T) -> U) -> f64 {
    let start = thread_cpu().unwrap_or_default();
    items.iter().for_each(|i| drop(black_box(f(i))));
    (thread_cpu().unwrap_or_default() - start).as_secs_f64() * 1e6 / items.len().max(1) as f64
}

/// "median [min–max]" of `samples`.
fn spread(samples: &mut [f64]) -> String {
    samples.sort_by(f64::total_cmp);
    let digits = if samples[0] < 10.0 { 1 } else { 0 };
    format!(
        "{:.*} [{:.*}–{:.*}]",
        digits,
        samples[samples.len() / 2],
        digits,
        samples[0],
        digits,
        samples[samples.len() - 1]
    )
}

/// The value after `name`, `None` when the flag is absent; a flag with no
/// value (the next argument another flag) or one that doesn't parse is an
/// error, not the default.
fn flag<T: std::str::FromStr>(args: &[String], name: &str) -> Result<Option<T>, Error> {
    let Some(i) = args.iter().position(|a| a == name) else {
        return Ok(None);
    };
    match args
        .get(i + 1)
        .filter(|v| !v.starts_with("--"))
        .map(|v| v.parse())
    {
        Some(Ok(value)) => Ok(Some(value)),
        _ => Err(format!("{name} needs a valid value").into()),
    }
}

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().collect();
    let count: usize = flag(&args, "--chunks")?.unwrap_or(4096);
    let rounds: usize = flag(&args, "--rounds")?.unwrap_or(5);
    let threads: usize = flag(&args, "--threads")?
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));
    for (name, value) in [
        ("--chunks", count),
        ("--rounds", rounds),
        ("--threads", threads),
    ] {
        if value == 0 {
            return Err(format!("{name} must be at least 1").into());
        }
    }
    let scratch = Scratch::new(
        &flag::<PathBuf>(&args, "--scratch")?.unwrap_or_else(std::env::temp_dir),
        "horizon-save-codec",
    )?;
    let (registry, biomes) = fixture()?;
    let limits = Limits::default();
    let encode = |c: &Chunk| save::encode_with(c, registry, &biomes, None, LevelShape::OVERWORLD);
    let decode = |root: &NamedTag| {
        let mut chunk =
            save::decode_with(root, registry, &biomes, None, ColumnLayout::VANILLA).unwrap();
        save::load_light(&root.tag, &mut chunk, true);
        chunk
    };

    let (chunks, nbt_bytes): (Vec<Arc<Chunk>>, Vec<Vec<u8>>) =
        if let Some(world) = flag::<PathBuf>(&args, "--world")? {
            // Region files in name order, so the same world gives the same
            // sample whatever order the folder lists them in; reading stops
            // at `--chunks` records. Only `r.X.Z.mca` files: the `.mcc`
            // files of oversized chunks are read through their region.
            let mut paths = Vec::new();
            for entry in std::fs::read_dir(world.join("region"))? {
                let path = entry?.path();
                if path.is_file() && position_from_path(&path).is_ok() {
                    paths.push(path);
                }
            }
            paths.sort();
            let mut records = Vec::new();
            'regions: for path in paths {
                let mut region = RegionFile::open(path)?;
                let positions: Vec<_> = region.chunks().map(|(p, _)| p).collect();
                for pos in positions {
                    if records.len() == count {
                        break 'regions;
                    }
                    if let Some(record) = region.read(pos)? {
                        records.push(nbt::write_named(&record.nbt(limits)?, limits)?);
                    }
                }
            }
            if records.is_empty() {
                return Err("the world's region folder holds no chunk records".into());
            }
            (Vec::new(), records)
        } else {
            let side = (count as f64).sqrt().ceil() as i32;
            let chunks: Vec<_> = (0..count as i32)
                .map(|i| Arc::new(column(i % side, i / side)))
                .collect();
            for c in &chunks {
                let back = unwire(&wire(c)).ok_or("wire payload did not read back")?;
                let same = back.iter().zip(&c.sections).all(|(a, b)| {
                    a.blocks.to_wire() == b.blocks.to_wire()
                        && a.biomes.to_wire() == b.biomes.to_wire()
                        && (0..4096).all(|i| a.light.get(i) == b.light.get(i))
                });
                assert!(
                    same,
                    "wire payload of chunk {},{} reads back different",
                    c.x, c.z
                );
                let root = encode(c);
                assert_eq!(
                    nbt::write_named(&encode(&decode(&root)), limits)?,
                    nbt::write_named(&root, limits)?
                );
            }
            let bytes = chunks
                .iter()
                .map(|c| nbt::write_named(&encode(c), limits).unwrap())
                .collect();
            (chunks, bytes)
        };
    let n = nbt_bytes.len();
    println!(
        "{n} chunks, {rounds} rounds, profile and machine as built; CPU µs/chunk on this thread, median [range]"
    );

    // Stage breakdown of one chunk's save and load.
    let roots: Vec<NamedTag> = nbt_bytes
        .iter()
        .map(|b| nbt::read_named(b, limits).unwrap().0)
        .collect();
    let deflated: Vec<Vec<u8>> = nbt_bytes
        .iter()
        .map(|b| Compression::Zlib.encode(b).unwrap())
        .collect();
    let names =
        "encode_with,write NBT,deflate (zlib-6),inflate,parse NBT,decode + light".split(',');
    let mut stages: Vec<(&str, Vec<f64>)> = names.map(|s| (s, Vec::new())).collect();
    let have_chunks = !chunks.is_empty();
    // Without chunks, `encode_with` and `decode + light` don't run.
    let ran = |name: &str| have_chunks || !matches!(name, "encode_with" | "decode + light");
    // Without chunks (`--world`), the roots aren't decoded either.
    let decoded = if have_chunks { &roots[..] } else { &[] };
    for _ in 0..rounds {
        let times = [
            cpu_us(&chunks, |c| encode(c)),
            cpu_us(&roots, |r| nbt::write_named(r, limits).unwrap()),
            cpu_us(&nbt_bytes, |b| Compression::Zlib.encode(b).unwrap()),
            cpu_us(&deflated, |d| inflate(d)),
            cpu_us(&nbt_bytes, |b| nbt::read_named(b, limits).unwrap()),
            cpu_us(decoded, |r| decode(r)),
        ];
        stages
            .iter_mut()
            .zip(times)
            .for_each(|((_, s), t)| s.push(t));
    }
    println!("\n| Stage | CPU µs/chunk |\n|---|---:|");
    for (name, mut samples) in stages.into_iter().filter(|(name, _)| ran(name)) {
        println!("| {name} | {} |", spread(&mut samples));
    }

    // Codec matrix. Each row: payload, codec, and its bytes once compressed.
    let wires: Vec<Vec<u8>> = chunks.iter().map(|c| wire(c)).collect();
    type Codec = (&'static str, fn(&[u8]) -> Vec<u8>, fn(&[u8]) -> Vec<u8>);
    let lz4: Codec = (
        "lz4",
        |b| Compression::Lz4.encode(b).unwrap(),
        |b| Compression::Lz4.decode(b, 64 << 20).unwrap(),
    );
    let codecs: [Codec; 5] = [
        ("none", |b| b.to_vec(), |b| b.to_vec()),
        lz4,
        ("zlib-1", |b| zlib(1, b), inflate),
        ("zlib-3", |b| zlib(3, b), inflate),
        ("zlib-6", |b| zlib(6, b), inflate),
    ];
    let rows: Vec<(&str, &Codec, &Vec<Vec<u8>>)> = [
        ("nbt", 4),
        ("nbt", 1),
        ("wire", 0),
        ("wire", 1),
        ("wire", 2),
        ("wire", 3),
        ("wire", 4),
    ]
    .into_iter()
    .filter(|(payload, _)| *payload == "nbt" || have_chunks)
    .map(|(payload, c)| {
        (
            payload,
            &codecs[c],
            if payload == "nbt" { &nbt_bytes } else { &wires },
        )
    })
    .collect();
    let packed: Vec<Vec<Vec<u8>>> = rows
        .iter()
        .map(|(_, (_, enc, _), src)| src.iter().map(|b| enc(b)).collect())
        .collect();
    let mut times = vec![(Vec::new(), Vec::new()); rows.len()];
    for _ in 0..rounds {
        for (i, (_, (_, enc, dec), src)) in rows.iter().enumerate() {
            times[i].0.push(cpu_us(src, |b| enc(b)));
            times[i].1.push(cpu_us(&packed[i], |b| dec(b)));
        }
    }
    let reference = packed[0].iter().map(Vec::len).sum::<usize>() as f64 / n as f64;
    println!(
        "\n| Payload + codec | Bytes/chunk | vs nbt + zlib-6 | In 4 KiB sectors | Compress µs | Decompress µs |"
    );
    println!("|---|---:|---:|---:|---:|---:|");
    for (i, ((payload, (codec, _, _), _), (enc, dec))) in rows.iter().zip(&mut times).enumerate() {
        let bytes = packed[i].iter().map(Vec::len).sum::<usize>() as f64 / n as f64;
        let sectors = packed[i]
            .iter()
            .map(|b| (b.len() + 5).div_ceil(4096) * 4096)
            .sum::<usize>() as f64
            / n as f64;
        println!(
            "| {payload} + {codec} | {bytes:.0} | {:.1}% | {sectors:.0} | {} | {} |",
            bytes / reference * 100.0,
            spread(enc),
            spread(dec)
        );
    }
    if !have_chunks {
        return Ok(());
    }

    // SaveDir workloads: durable autosaves, warm loads, small-edit rounds.
    let mut results: Vec<(String, Vec<f64>)> = Vec::new();
    let mut record = |name: String, value: f64| match results.iter_mut().find(|(n, _)| *n == name) {
        Some((_, v)) => v.push(value),
        None => results.push((name, vec![value])),
    };
    let mut disk = 0.0;
    let largest = [1024, 4096].into_iter().rfind(|&b| b <= chunks.len());
    for round in 0..rounds {
        for batch in [1024, 4096].into_iter().filter(|&b| b <= chunks.len()) {
            let dir = scratch.join(format!("autosave-{batch}-{round}"));
            let saves = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
            for pass in ["first", "again"] {
                let start = Instant::now();
                saves.save(&chunks[..batch], registry, &biomes)?;
                record(
                    format!("autosave {batch}, {pass} (ms)"),
                    start.elapsed().as_secs_f64() * 1e3,
                );
            }
            drop(saves);
            if Some(batch) == largest {
                let files = std::fs::read_dir(dir.join("region"))?;
                disk = files
                    .map(|f| f.unwrap().metadata().unwrap().len())
                    .sum::<u64>() as f64
                    / batch as f64;
                let saves = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
                let keys: Vec<(i32, i32)> = chunks[..batch].iter().map(|c| (c.x, c.z)).collect();
                // One untimed pass: region files open and in the page cache.
                keys.iter()
                    .for_each(|&(x, z)| assert!(saves.load(x, z, registry, &biomes).is_some()));
                for t in [1, threads] {
                    let start = Instant::now();
                    std::thread::scope(|s| {
                        for part in keys.chunks(keys.len().div_ceil(t)) {
                            let (saves, biomes) = (&saves, &biomes);
                            s.spawn(move || {
                                part.iter().for_each(|&(x, z)| {
                                    drop(black_box(saves.load(x, z, registry, biomes)))
                                })
                            });
                        }
                    });
                    record(
                        format!("warm load, {t} thread(s) (chunks/s)"),
                        batch as f64 / start.elapsed().as_secs_f64(),
                    );
                }
            }
            std::fs::remove_dir_all(&dir)?;
        }
        // 256 chunks saved, then 20 rounds that each change one block in one section of each.
        let dir = scratch.join(format!("edits-{round}"));
        let saves = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        let mut edited: Vec<Arc<Chunk>> = chunks[..256.min(chunks.len())].to_vec();
        saves.save(&edited, registry, &biomes)?;
        let mut per_round = Vec::new();
        for edit in 0..20 {
            for c in &mut edited {
                Arc::make_mut(c).set(edit % 16, 40 + edit, 7, 1 + (edit as u32 % 3));
            }
            let start = Instant::now();
            saves.save(&edited, registry, &biomes)?;
            per_round.push(start.elapsed().as_secs_f64() * 1e3);
        }
        per_round.sort_by(f64::total_cmp);
        record("edit round, 256 chunks, p50 (ms)".into(), per_round[10]);
        record(
            "edit round, 256 chunks, worst of 20 (ms)".into(),
            per_round[19],
        );
        drop(saves);
        std::fs::remove_dir_all(&dir)?;
    }
    println!(
        "\n| SaveDir workload ({}) | Median [range] |\n|---|---:|",
        scratch.display()
    );
    for (name, mut samples) in results {
        println!("| {name} | {} |", spread(&mut samples));
    }
    println!("| region bytes on disk per chunk | {disk:.0} |");
    drop(scratch);
    Ok(())
}
