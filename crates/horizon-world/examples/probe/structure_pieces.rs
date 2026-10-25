//! The structure starts a chunk owns, piece by piece, as a saved start lists them.
//!
//! structure_pieces <client resource dir> <seed> <dimension> <cx,cz>...
//! prints, for every start whose chunk is a target, `start ID CX CZ` and then
//! one `piece TYPE ELEMENT ROTATION X Y Z GROUND_LEVEL_DELTA X0 Y0 Z0 X1 Y1 Z1`
//! line per piece in placement order (jigsaw pieces: the element's template
//! location; other pieces: `-` for element, rotation and position).
use crate::support::common::tag;
use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
    nbt::Tag,
    registry::BlockRegistry,
    worldgen::{
        generator::{GeneratorData, OverworldGenerator},
        structure::{Rotation, jigsaw::PoolElement, start::Piece},
    },
};
use serde_json::Value;
use std::{fs, path::Path};

type Error = Box<dyn std::error::Error + Send + Sync>;

pub fn registry(resources: &Path) -> Result<&'static BlockRegistry, Error> {
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

fn element(element: &PoolElement) -> String {
    match element {
        PoolElement::Empty => "empty".into(),
        PoolElement::Single { template, .. } => template.location.to_string(),
        PoolElement::Feature { feature, .. } => format!("feature:{feature}"),
        PoolElement::List { elements, .. } => {
            let parts: Vec<_> = elements.iter().map(self::element).collect();
            format!("list:{}", parts.join("+"))
        }
    }
}

fn rotation(rotation: Rotation) -> &'static str {
    match rotation {
        Rotation::None => "NONE",
        Rotation::Clockwise90 => "CLOCKWISE_90",
        Rotation::Clockwise180 => "CLOCKWISE_180",
        Rotation::Counterclockwise90 => "COUNTERCLOCKWISE_90",
    }
}

pub fn main() -> Result<(), Error> {
    let mut args = crate::args().skip(1);
    let resources = std::path::PathBuf::from(args.next().ok_or("client resource directory")?);
    let seed: i64 = args.next().ok_or("seed")?.parse()?;
    let dimension = format!("minecraft:{}", args.next().ok_or("dimension")?);
    let data = GeneratorData::load(&resources.join("worldgen"), registry(&resources)?)?;
    let generator = OverworldGenerator::for_dimension(data, seed, "minecraft:normal", &dimension)?;
    let mut worker = generator.worker();
    for target in args {
        let (x, z) = target.split_once(',').ok_or("target cx,cz")?;
        let chunk = ChunkPos::new(x.parse()?, z.parse()?);
        for start in worker.structure_starts(chunk) {
            if start.chunk != chunk {
                continue;
            }
            println!("start {} {} {}", start.name, chunk.x, chunk.z);
            for piece in &start.pieces {
                let b = piece.bounding_box();
                let placed = match piece {
                    Piece::Pool(p) => format!(
                        "{} {} {} {} {} {}",
                        element(&p.element),
                        rotation(p.rotation),
                        p.position[0],
                        p.position[1],
                        p.position[2],
                        p.ground_level_delta
                    ),
                    Piece::Legacy(_) => "- - - - - -".into(),
                };
                println!(
                    "piece {} {placed} {} {} {} {} {} {}",
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
    }
    Ok(())
}
