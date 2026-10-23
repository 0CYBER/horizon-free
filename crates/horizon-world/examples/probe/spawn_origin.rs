//! NoiseSpawnFinder for tools/test_world_spawn_parity.py: for each seed, the
//! column the overworld's spawn search starts from and its chunk
//! (NoiseBasedChunkGenerator.getOrigin), as `seed x z chunk_x chunk_z` lines.
//!
//! usage: spawn_origin <client resource directory> <seed>...
use crate::support::common::tag;
use horizon_world::{
    DATA_VERSION,
    nbt::Tag,
    registry::BlockRegistry,
    worldgen::generator::{GeneratorData, OverworldGenerator},
};
use serde_json::Value;
use std::{fs, path::Path};

type Error = Box<dyn std::error::Error + Send + Sync>;

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

pub fn main() -> Result<(), Error> {
    let mut args = crate::args().skip(1);
    let resources = std::path::PathBuf::from(args.next().ok_or("client resource directory")?);
    let data = GeneratorData::load(&resources.join("worldgen"), registry(&resources)?)?;
    for seed in args {
        let seed: i64 = seed.parse()?;
        let generator = OverworldGenerator::new(data.clone(), seed)?;
        let [x, z] = generator.spawn_position()?;
        let origin = generator.origin()?;
        println!("{seed} {x} {z} {} {}", origin.x, origin.z);
    }
    Ok(())
}
