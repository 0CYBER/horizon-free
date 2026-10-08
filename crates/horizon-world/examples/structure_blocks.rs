//! Structure placement dump in the vanilla structure-placement oracle's
//! format: full terrain generation, then decoration with structure pieces
//! only (no placed features) for each target chunk.
//!
//! structure_blocks <client resource dir> <seed> <dimension> <cx,cz>...
//! prints `chunk CX CZ`, one `y Y ID...` line per block row (256 state ids in
//! z-major order) and `be X Y Z ID LOOTTABLE SEED` per block entity.
use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
    nbt::Tag,
    registry::BlockRegistry,
    worldgen::generator::{GeneratorData, OverworldGenerator, ProtoChunk},
};
use serde_json::Value;
use std::{fs, path::Path};

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

fn main() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let resources = std::path::PathBuf::from(args.next().ok_or("client resource directory")?);
    let seed: i64 = args.next().ok_or("seed")?.parse()?;
    let dimension = format!("minecraft:{}", args.next().ok_or("dimension")?);
    let data = GeneratorData::load(&resources.join("worldgen"), registry(&resources)?)?;
    let generator = OverworldGenerator::for_dimension(data, seed, "minecraft:normal", &dimension)?;
    let mut worker = generator.worker();
    worker.set_structures_only(true);
    for target in args {
        let (cx, cz) = target.split_once(',').ok_or("target cx,cz")?;
        let (cx, cz): (i32, i32) = (cx.parse()?, cz.parse()?);
        let mut region = Vec::with_capacity(9);
        for dz in -1..=1 {
            for dx in -1..=1 {
                region.push(worker.generate_proto(ChunkPos::new(cx + dx, cz + dz))?);
            }
        }
        let mut region: [ProtoChunk; 9] = region.try_into().map_err(|_| "region size")?;
        let stats = if std::env::var_os("HORIZON_TERRAIN_ONLY").is_some() {
            Default::default()
        } else {
            worker.decorate(&mut region)?
        };
        if stats.failed > 0 {
            eprintln!(
                "chunk {cx} {cz}: {} structure placements failed",
                stats.failed
            );
        }
        let chunk = &region[4];
        println!("chunk {cx} {cz}");
        let (min, height) = (chunk.terrain.min_y(), chunk.terrain.height());
        for y in min..min + height {
            let mut line = format!("y {y}");
            for z in 0..16 {
                for x in 0..16 {
                    line.push_str(&format!(" {}", chunk.block(cx * 16 + x, y, cz * 16 + z).0));
                }
            }
            println!("{line}");
        }
        let mut entities: Vec<_> = chunk.block_entities.iter().collect();
        entities.sort_by_key(|(p, _)| *p);
        for ([x, y, z], nbt) in entities {
            let text = |key: &str| match nbt.get(&key.into()) {
                Some(Tag::String(s)) => s.to_string_lossy(),
                _ => "-".into(),
            };
            // The seed only matters (and is only reported) with a loot table.
            let seed = match nbt.get(&"LootTableSeed".into()) {
                Some(Tag::Long(v)) if text("LootTable") != "-" => v.to_string(),
                _ => "-".into(),
            };
            println!("be {x} {y} {z} {} {} {seed}", text("id"), text("LootTable"));
        }
    }
    Ok(())
}
