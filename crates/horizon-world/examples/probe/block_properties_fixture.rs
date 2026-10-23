use horizon_core::geometry::{Aabb, Axis, Direction, ShapeOp, Vec3, VoxelShape};
use horizon_world::{
    block_properties::{BlockDefinitionProperties, BlockPropertiesRegistry, BlockProperty},
    collision::CachedCollisionRegistry,
    nbt::{self, Limits, Tag},
    registry::{BlockRegistry, BlockStateId},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn bbox(v: &Value) -> Aabb {
    let v = v
        .as_array()
        .unwrap()
        .iter()
        .map(|v| f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap()))
        .collect::<Vec<_>>();
    Aabb::new(Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5]))
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let read = |name: &str| -> Result<Tag, Box<dyn std::error::Error>> {
        Ok(
            nbt::read_named(&fs::read(dir.join(name))?, Limits::default())?
                .0
                .tag,
        )
    };
    let registry = BlockRegistry::from_nbt(&read("block-registry.nbt")?)?;
    let collisions = CachedCollisionRegistry::from_nbt(&read("cached-collision.nbt")?, &registry)?;
    let data = read("block-properties.nbt")?;
    let properties = BlockPropertiesRegistry::from_nbt(&data, &collisions)?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    let solid = properties.bind(BlockProperty::Solid);
    let replaceable = properties.bind(BlockProperty::Replaceable);
    let faces = properties.bind_occlusion_faces();
    assert!(
        faces
            .is_full(BlockStateId(registry.len() as u32), Direction::Up)
            .is_err()
    );
    for id in 0..registry.len() {
        let state = BlockStateId(id as u32);
        let definition = properties
            .definition(&registry.state(state).unwrap().name)
            .unwrap();
        let s = properties.property(state, BlockProperty::Solid)?;
        let r = properties.property(state, BlockProperty::Replaceable)?;
        assert_eq!(s, solid.contains(state));
        assert_eq!(r, replaceable.contains(state));
        writeln!(
            out,
            "state {id} {} {} {r} {s}",
            definition.force_solid_on, definition.force_solid_off
        )?;
        writeln!(
            out,
            "render {} {} {}",
            properties.property(state, BlockProperty::Liquid)?,
            definition.can_occlude,
            properties.property(state, BlockProperty::SolidRender)?
        )?;
        assert_eq!(
            properties.property(state, BlockProperty::Liquid)?,
            properties.bind(BlockProperty::Liquid).contains(state)
        );
        assert_eq!(
            properties.property(state, BlockProperty::SolidRender)?,
            properties.bind(BlockProperty::SolidRender).contains(state)
        );
        let mut mask = 0;
        for direction in Direction::ALL {
            if faces.is_full(state, direction)? {
                mask |= 1 << direction as u8;
            }
        }
        writeln!(out, "occlusion {mask}")?;
        match collisions.cached_shape(state)? {
            None => writeln!(out, "cache none")?,
            Some(shape) => match shape.bounds() {
                None => writeln!(out, "cache empty")?,
                Some(b) => {
                    let v = [
                        b.min(Axis::X),
                        b.min(Axis::Y),
                        b.min(Axis::Z),
                        b.max(Axis::X),
                        b.max(Axis::Y),
                        b.max(Axis::Z),
                    ]
                    .map(f64::to_bits);
                    writeln!(
                        out,
                        "cache {:016x} {:016x} {:016x} {:016x} {:016x} {:016x}",
                        v[0], v[1], v[2], v[3], v[4], v[5]
                    )?;
                }
            },
        }
    }
    assert!(
        properties
            .property(BlockStateId(registry.len() as u32), BlockProperty::Solid)
            .is_err()
    );
    for (name, value) in [
        ("DataVersion", Tag::Int(0)),
        ("states", Tag::List(vec![])),
        ("blocks", Tag::compound([] as [(&str, Tag); 0])),
        ("occlusion", Tag::compound([] as [(&str, Tag); 0])),
    ] {
        let mut bad = data.clone();
        *bad.get_mut(name).unwrap() = value;
        assert!(BlockPropertiesRegistry::from_nbt(&bad, &collisions).is_err());
    }
    for flags in [-1, 32] {
        let mut bad = data.clone();
        *bad.get_mut("blocks")
            .unwrap()
            .get_mut("minecraft:stone")
            .unwrap() = Tag::Byte(flags);
        assert!(BlockPropertiesRegistry::from_nbt(&bad, &collisions).is_err());
    }
    let mut bad = data.clone();
    let Some(Tag::List(states)) = bad.get_mut("states") else {
        unreachable!()
    };
    states.swap(0, 1);
    assert!(BlockPropertiesRegistry::from_nbt(&bad, &collisions).is_err());
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut shape = VoxelShape::empty();
        for v in f["shape"].as_array().unwrap() {
            shape = shape.join(&VoxelShape::from_box(bbox(v))?, ShapeOp::OR)?;
        }
        let properties = BlockDefinitionProperties {
            force_solid_on: f["on"].as_bool().unwrap(),
            force_solid_off: f["off"].as_bool().unwrap(),
            replaceable: false,
            liquid: false,
            can_occlude: f["occludes"].as_bool().unwrap(),
        };
        writeln!(
            out,
            "solid {}",
            properties.calculate_solid(f["cached"].as_bool().unwrap().then_some(&shape))
        )?;
        writeln!(out, "render {}", properties.calculate_solid_render(&shape)?)?;
        writeln!(
            out,
            "occlusion {}",
            properties.calculate_full_occlusion_faces(&shape)?
        )?;
    }
    Ok(())
}
