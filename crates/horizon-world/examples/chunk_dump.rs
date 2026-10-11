//! Whole-chunk dump in the format of the vanilla chunk-generation oracle.
//!
//! chunk_dump <client resource dir> <seed> <dimension> <x0,z0,x1,z1>...
//! With HORIZON_SCAN set, one rectangle lists the structure starts in it.
//!
//! Chunks are generated through biomes, terrain and carvers, then decorated in
//! row-major order over the target rectangle grown by one chunk, each with its
//! eight neighbours so feature spill persists, exactly as the oracle does.
//! For every target chunk this prints `chunk X Z`, one `q QY NAME...` line per
//! quart layer (16 columns, z-major), one `y Y ID...` line per block row and
//! one `be X Y Z CANONICAL_NBT` line per block entity.
use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
    nbt::Tag,
    registry::BlockRegistry,
    worldgen::generator::{GeneratorData, OverworldGenerator, ProtoChunk},
};
use serde_json::Value;
use std::{collections::HashMap, fmt::Write as _, fs, io::Write as _, path::Path};

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
    let states = facts["states"].as_array().ok_or("facts states")?;
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(states.iter().map(|v| tag(&v[4])).collect()),
        ),
        ("defaults", tag(&facts["defaults"])),
    ]))?;
    Ok(Box::leak(Box::new(registry)))
}

/// Type-prefixed scalars with sorted compound keys, as the oracle prints them.
fn canonical(tag: &Tag, out: &mut String) {
    match tag {
        Tag::End => {}
        Tag::Byte(v) => write!(out, "b{v}").unwrap(),
        Tag::Short(v) => write!(out, "s{v}").unwrap(),
        Tag::Int(v) => write!(out, "i{v}").unwrap(),
        Tag::Long(v) => write!(out, "l{v}").unwrap(),
        Tag::Float(v) => write!(out, "f{}", *v as i32).unwrap(),
        Tag::Double(v) => write!(out, "d{}", *v as i64).unwrap(),
        Tag::String(s) => {
            let text = s
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            write!(out, "\"{text}\"").unwrap();
        }
        Tag::ByteArray(v) => {
            out.push_str("B[");
            for b in v {
                write!(out, "{},", *b as i8).unwrap();
            }
            out.push(']');
        }
        Tag::IntArray(v) => {
            out.push_str("I[");
            for b in v {
                write!(out, "{b},").unwrap();
            }
            out.push(']');
        }
        Tag::LongArray(v) => {
            out.push_str("L[");
            for b in v {
                write!(out, "{b},").unwrap();
            }
            out.push(']');
        }
        Tag::List(v) => {
            out.push('[');
            for item in v {
                canonical(item, out);
                out.push(',');
            }
            out.push(']');
        }
        Tag::Compound(c) => {
            out.push('{');
            for (k, v) in c {
                write!(out, "{}=", k.to_string_lossy()).unwrap();
                canonical(v, out);
                out.push(',');
            }
            out.push('}');
        }
    }
}

fn main() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let resources = std::path::PathBuf::from(args.next().ok_or("client resource directory")?);
    let seed: i64 = args.next().ok_or("seed")?.parse()?;
    let dimension = format!("minecraft:{}", args.next().ok_or("dimension")?);
    let rects = args
        .map(|rect| {
            let v: Vec<i32> = rect.split(',').map(str::parse).collect::<Result<_, _>>()?;
            let v: [i32; 4] = v.try_into().map_err(|_| "rectangle x0,z0,x1,z1")?;
            Ok(v)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let data = GeneratorData::load(&resources.join("worldgen"), registry(&resources)?)?;
    let generator = OverworldGenerator::for_dimension(data, seed, "minecraft:normal", &dimension)?;
    let mut worker = generator.worker();
    // HORIZON_PHASE: terrain (no decoration), structures (pieces only) or full.
    let phase = std::env::var("HORIZON_PHASE").unwrap_or_else(|_| "full".into());
    worker.set_structures_only(phase == "structures");
    if std::env::var_os("HORIZON_SCAN").is_some() {
        let [x0, z0, x1, z1] = rects[0];
        for x in x0..=x1 {
            for z in z0..=z1 {
                for s in worker.structure_starts(ChunkPos::new(x, z)) {
                    if s.chunk == ChunkPos::new(x, z) {
                        if std::env::var_os("HORIZON_PIECES").is_some() {
                            for piece in &s.pieces {
                                let b = piece.bounding_box();
                                println!(
                                    "piece {x} {z} {} {} {} {} {} {} {}",
                                    piece.type_name(),
                                    b.min[0],
                                    b.min[1],
                                    b.min[2],
                                    b.max[0],
                                    b.max[1],
                                    b.max[2]
                                );
                            }
                        }
                        println!(
                            "start {} {x} {z} {} {} {} {} {} {}",
                            s.name,
                            s.bounding_box.min[0],
                            s.bounding_box.min[1],
                            s.bounding_box.min[2],
                            s.bounding_box.max[0],
                            s.bounding_box.max[1],
                            s.bounding_box.max[2]
                        );
                    }
                }
            }
        }
        return Ok(());
    }
    for [x0, z0, x1, z1] in rects {
        // Each rectangle starts from scratch, like the oracle's fresh chunks:
        // structure pieces keep placement state (a shipwreck's adjusted height).
        worker.forget_structure_starts();
        let mut chunks: HashMap<(i32, i32), ProtoChunk> = HashMap::new();
        for z in z0 - 2..=z1 + 2 {
            for x in x0 - 2..=x1 + 2 {
                chunks.insert((x, z), worker.generate_proto(ChunkPos::new(x, z))?);
            }
        }
        let mut failed = 0;
        let margin = i32::from(std::env::var_os("HORIZON_ISOLATED").is_none());
        for z in z0 - margin..=z1 + margin {
            for x in x0 - margin..=x1 + margin {
                let mut region = Vec::with_capacity(9);
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        region.push(chunks.remove(&(x + dx, z + dz)).ok_or("missing chunk")?);
                    }
                }
                let mut region: [ProtoChunk; 9] = region.try_into().map_err(|_| "region size")?;
                let stats = if phase == "terrain" {
                    Default::default()
                } else {
                    worker.decorate(&mut region)?
                };
                if stats.failed > 0 || stats.unsupported > 0 {
                    failed += 1;
                    eprintln!(
                        "chunk {x} {z}: {} failed, {} unsupported {:?}",
                        stats.failed, stats.unsupported, stats.skipped
                    );
                }
                for (i, chunk) in region.into_iter().enumerate() {
                    chunks.insert((x + i as i32 % 3 - 1, z + i as i32 / 3 - 1), chunk);
                }
            }
        }
        if failed > 0 {
            eprintln!("{failed} chunks had failed or unsupported features");
        }
        let stdout = std::io::stdout();
        let mut w = std::io::BufWriter::new(stdout.lock());
        for z in z0..=z1 {
            for x in x0..=x1 {
                let chunk = &chunks[&(x, z)];
                writeln!(w, "chunk {x} {z}")?;
                let (min, height) = (chunk.terrain.min_y(), chunk.terrain.height());
                for qy in min >> 2..=(min + height - 1) >> 2 {
                    let mut line = format!("q {qy}");
                    for qz in 0..4 {
                        for qx in 0..4 {
                            line.push(' ');
                            line.push_str(chunk.biome_at(x * 16 + qx * 4, qy * 4, z * 16 + qz * 4));
                        }
                    }
                    writeln!(w, "{line}")?;
                }
                for y in min..min + height {
                    let mut line = format!("y {y}");
                    for zz in 0..16 {
                        for xx in 0..16 {
                            write!(line, " {}", chunk.block(x * 16 + xx, y, z * 16 + zz).0)?;
                        }
                    }
                    writeln!(w, "{line}")?;
                }
                let mut entities: Vec<_> = chunk.block_entities.iter().collect();
                entities.sort_by_key(|(p, _)| *p);
                for ([bx, by, bz], nbt) in entities {
                    let mut text = String::new();
                    canonical(&Tag::Compound(nbt.clone()), &mut text);
                    writeln!(w, "be {bx} {by} {bz}\t{text}")?;
                }
            }
        }
    }
    Ok(())
}
