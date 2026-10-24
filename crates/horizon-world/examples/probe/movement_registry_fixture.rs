use crate::support::common::position;
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::movement_support::{self as support, MovementLocation};
use horizon_world::{
    movement_properties::{
        MovementBlockProvider, MovementProperties, MovementTags, RegistryMovementWorld,
    },
    nbt::{self, Limits, Tag},
    registry::{BlockRegistry, BlockStateId},
    worldgen::tags::TagDefinitions,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
struct World<'a> {
    input: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
}
impl MovementBlockProvider for World<'_> {
    type Error = io::Error;
    fn movement_block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        writeln!(self.out, "state {} {} {}", p.x, p.y, p.z)?;
        let call = self.calls;
        self.calls += 1;
        if call == self.input["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected block read failure"));
        }
        let states = self.input["states"].as_array().unwrap();
        Ok(BlockStateId(
            states[call as usize % states.len()].as_u64().unwrap() as u32,
        ))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    let read = |name: &str| -> Result<Tag, Box<dyn std::error::Error>> {
        Ok(
            nbt::read_named(&fs::read(dir.join(name))?, Limits::default())?
                .0
                .tag,
        )
    };
    let registry = BlockRegistry::from_nbt(&read("block-registry.nbt")?)?;
    let data = read("movement-properties.nbt")?;
    let properties = MovementProperties::from_nbt(&data, &registry)?;
    let resources: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("tags.json"))?)?;
    let mut tags = TagDefinitions::default();
    for (name, json) in resources {
        tags.apply_json(&name, "vanilla", &json)?;
    }
    let built = tags.build(|name, _| registry.default_state(name).map(|_| name.to_owned()))?;
    assert!(built.missing.is_empty());
    let members = |name: &str| -> BTreeSet<String> {
        built.tags.get(name).unwrap().iter().cloned().collect()
    };
    let natural = MovementTags {
        fences: members("minecraft:fences"),
        walls: members("minecraft:walls"),
        suppresses_bounce: members("minecraft:suppresses_bounce"),
    };
    let all: BTreeSet<String> = (0..registry.len())
        .map(|id| {
            registry
                .state(BlockStateId(id as u32))
                .unwrap()
                .name
                .clone()
        })
        .collect();
    let bindings = [
        properties.bind(&natural)?,
        properties.bind(&MovementTags::default())?,
        properties.bind(&MovementTags {
            fences: all.clone(),
            walls: all.clone(),
            suppresses_bounce: all,
        })?,
    ];
    assert!(
        bindings[0]
            .get(BlockStateId(registry.len() as u32))
            .is_err()
    );
    for id in 0..registry.len() {
        let s = bindings[0].get(BlockStateId(id as u32))?;
        let c = s.coefficients;
        writeln!(
            out,
            "properties {id} {:08x} {:08x} {:08x} {:08x} {:08x} {} {} {} {} {} {}",
            c.friction.to_bits(),
            c.speed_factor.to_bits(),
            c.jump_factor.to_bits(),
            c.bounce_restitution.to_bits(),
            c.fall_distance_reduction.to_bits(),
            c.fence_gate,
            s.fence,
            s.wall,
            s.suppresses_bounce,
            s.water,
            s.bubble_column
        )?;
    }
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let p = &f["position"];
        let location = MovementLocation {
            position: Vec3::new(number(&p[0]), number(&p[1]), number(&p[2])),
            block_position: position(&f["cached"]),
            supporting_block: if f["support"].is_null() {
                None
            } else {
                Some(position(&f["support"]))
            },
        };
        let mut world = World {
            input: &f,
            out: &mut out,
            calls: 0,
        };
        let mut bound = RegistryMovementWorld {
            properties: &bindings[f["tags"].as_u64().unwrap() as usize],
            world: &mut world,
        };
        let mode = f["mode"].as_u64().unwrap();
        if mode < 4 {
            let offset = match mode {
                0 => single(&f["offset"]),
                1 => support::LEGACY_SUPPORT_OFFSET,
                2 => support::ON_SUPPORT_OFFSET,
                3 => support::MOVEMENT_SUPPORT_OFFSET,
                _ => unreachable!(),
            };
            match support::support_position(
                &mut bound,
                location.position,
                location.supporting_block,
                offset,
            ) {
                Ok(p) => writeln!(world.out, "position {} {} {}", p.x, p.y, p.z)?,
                Err(_) => writeln!(world.out, "error")?,
            }
        } else {
            let result = if mode == 4 {
                support::block_jump_factor(&mut bound, location)
            } else {
                support::block_speed_factor(&mut bound, location)
            };
            match result {
                Ok(factor) => writeln!(
                    world.out,
                    "factor {:08x}",
                    if factor.is_nan() {
                        0x7fc00000
                    } else {
                        factor.to_bits()
                    }
                )?,
                Err(_) => writeln!(world.out, "error")?,
            }
        }
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
