use horizon_core::geometry::Direction;
use horizon_world::{
    block_render::{FaceVisibility, RenderDefinition, SkipRendering},
    collision::CachedCollisionRegistry,
    nbt::{self, Limits, Tag},
    registry::{BlockRegistry, BlockStateId},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = crate::args().nth(1).ok_or("private fixture directory")?;
    let dir = Path::new(&path);
    let read = |name: &str| -> Result<Tag, Box<dyn std::error::Error>> {
        Ok(
            nbt::read_named(&fs::read(dir.join(name))?, Limits::default())?
                .0
                .tag,
        )
    };
    let registry = BlockRegistry::from_nbt(&read("block-registry.nbt")?)?;
    let occlusion = CachedCollisionRegistry::from_nbt(&read("occlusion.nbt")?, &registry)?;
    let definitions: BTreeMap<String, Value> =
        serde_json::from_reader(fs::File::open(dir.join("definitions.json"))?)?;
    let definitions = definitions
        .into_iter()
        .map(|(name, v)| {
            let skip = match v["rule"].as_str().ok_or("missing rule")? {
                "never" => SkipRendering::Never,
                "same_block" => SkipRendering::SameBlock,
                "bars" => SkipRendering::Bars,
                "leaves" => SkipRendering::Leaves,
                "liquid" => SkipRendering::Liquid,
                "mangrove_roots" => SkipRendering::MangroveRoots,
                _ => return Err("unrecognized rendering rule"),
            };
            Ok((
                name,
                RenderDefinition {
                    skip,
                    can_occlude: v["can_occlude"].as_bool().ok_or("missing occlusion flag")?,
                    bars_tag: v["bars_tag"].as_bool().ok_or("missing bars tag")?,
                },
            ))
        })
        .collect::<Result<_, &str>>()?;
    let mut visibility = FaceVisibility::new(&occlusion, &definitions)?;
    for invalid in [registry.len() as u32, u32::MAX] {
        assert!(
            visibility
                .should_render_face(BlockStateId(invalid), BlockStateId(0), Direction::Up, false)
                .is_err()
        );
        assert!(
            visibility
                .skip_rendering(BlockStateId(0), BlockStateId(invalid), Direction::Up, false)
                .is_err()
        );
    }
    let mut output = BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in BufReader::new(fs::File::open(dir.join("pairs.txt"))?).lines() {
        let line = line?;
        let (a, b) = line.split_once(' ').ok_or("invalid pair")?;
        let a = BlockStateId(a.parse()?);
        let b = BlockStateId(b.parse()?);
        for cutout in [false, true] {
            let mut skip = 0u8;
            let mut visible = 0u8;
            for direction in Direction::ALL {
                if visibility.skip_rendering(a, b, direction, cutout)? {
                    skip |= 1 << direction as u8;
                }
                if visibility.should_render_face(a, b, direction, cutout)? {
                    visible |= 1 << direction as u8;
                }
            }
            writeln!(output, "{} {} {cutout} {skip} {visible}", a.0, b.0)?;
        }
    }
    Ok(())
}
