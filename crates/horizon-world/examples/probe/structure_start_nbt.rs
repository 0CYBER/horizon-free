//! The structure starts a chunk owns, as `StructureStart.createTag` saves them.
//!
//! structure_start_nbt <client resource dir> <seed> <dimension> <cx,cz>...
//! prints `nbt CX CZ TAG` for every start whose chunk is a target, the tag
//! with sorted keys and typed numbers (VanillaStructureStartOracle's
//! `canonical`), or `unsaved CX CZ ID` when Horizon cannot save it yet.
use horizon_core::position::ChunkPos;
use horizon_world::{
    nbt::Tag,
    worldgen::generator::{GeneratorData, OverworldGenerator},
};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// `VanillaStructureStartOracle.canonical`.
fn canonical(tag: &Tag) -> String {
    let join = |parts: Vec<String>| parts.join(",");
    match tag {
        Tag::Compound(c) => {
            let mut keys: Vec<_> = c.keys().map(|k| (k.to_string_lossy(), k)).collect();
            keys.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            let parts = keys
                .into_iter()
                .map(|(name, key)| format!("{name}:{}", canonical(&c[key])))
                .collect();
            format!("{{{}}}", join(parts))
        }
        Tag::List(l) => format!("[{}]", join(l.iter().map(canonical).collect())),
        Tag::Byte(v) => format!("{v}b"),
        Tag::Short(v) => format!("{v}s"),
        Tag::Int(v) => v.to_string(),
        Tag::Long(v) => format!("{v}L"),
        Tag::Float(bits) => format!("{}f", *bits as i32),
        Tag::Double(bits) => format!("{}d", *bits as i64),
        Tag::String(s) => format!(
            "\"{}\"",
            s.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        ),
        Tag::IntArray(a) => format!("[I;{}]", join(a.iter().map(i32::to_string).collect())),
        Tag::LongArray(a) => format!("[L;{}]", join(a.iter().map(i64::to_string).collect())),
        Tag::ByteArray(a) => format!(
            "[B;{}]",
            join(a.iter().map(|b| (*b as i8).to_string()).collect())
        ),
        Tag::End => "END".into(),
    }
}

pub fn main() -> Result<(), Error> {
    let mut args = crate::args().skip(1);
    let resources = std::path::PathBuf::from(args.next().ok_or("client resource directory")?);
    let seed: i64 = args.next().ok_or("seed")?.parse()?;
    let dimension = format!("minecraft:{}", args.next().ok_or("dimension")?);
    let registry = crate::structure_pieces::registry(&resources)?;
    let data = GeneratorData::load(&resources.join("worldgen"), registry)?;
    let generator = OverworldGenerator::for_dimension(data, seed, "minecraft:normal", &dimension)?;
    let mut worker = generator.worker();
    for target in args {
        let (x, z) = target.split_once(',').ok_or("target cx,cz")?;
        let chunk = ChunkPos::new(x.parse()?, z.parse()?);
        for start in worker.structure_starts(chunk) {
            if start.chunk != chunk {
                continue;
            }
            match start.save() {
                Some(tag) => println!("nbt {} {} {}", chunk.x, chunk.z, canonical(&tag)),
                None => println!("unsaved {} {} {}", chunk.x, chunk.z, start.name),
            }
        }
    }
    Ok(())
}
