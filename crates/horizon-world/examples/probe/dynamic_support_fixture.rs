use crate::support::shapes::{emit, values};
use horizon_core::geometry::{Direction, VoxelShape};
use horizon_world::{
    block_collision::{DynamicCollisionWorld, ShulkerCollisionState},
    block_support::SupportType,
    chunk::BlockState,
    collision::CachedCollisionRegistry,
    dynamic_support::{DynamicSupport, DynamicSupportWorld},
    nbt::{self, Limits, Tag},
    piston_shape::{MovingPistonState, PistonBlockBindings, PistonCollisionWorld},
    registry::{BlockRegistry, BlockStateId},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
struct World<'a, W> {
    collisions: &'a CachedCollisionRegistry<'a>,
    pistons: &'a PistonBlockBindings<'a>,
    dynamic: &'a [Option<DynamicSupport>],
    active: Option<MovingPistonState>,
    noclip: Option<Direction>,
    out: &'a mut W,
    entity: &'a str,
    shulker: ShulkerCollisionState,
    fail: i32,
    calls: i32,
}
impl<W: Write> World<'_, W> {
    fn query(&mut self, p: [i32; 3]) -> io::Result<()> {
        writeln!(self.out, "entity {} {} {}", p[0], p[1], p[2])?;
        let count = self.calls;
        self.calls += 1;
        if count == self.fail {
            return Err(io::Error::other("injected block entity failure"));
        }
        Ok(())
    }
}
impl<W: Write> DynamicSupportWorld for World<'_, W> {
    type Error = io::Error;
    fn shulker_is_closed(&mut self, p: [i32; 3]) -> io::Result<Option<bool>> {
        self.query(p)?;
        Ok(match self.entity {
            "none" | "other" | "piston" => None,
            "closed" => Some(true),
            _ => Some(false),
        })
    }
    fn moving_piston_shape(&mut self, p: [i32; 3]) -> io::Result<Option<VoxelShape>> {
        self.query(p)?;
        match self.active {
            None => Ok(None),
            Some(active) => {
                let bindings = self.pistons;
                bindings
                    .collision_shape(self, p, active, self.noclip)
                    .map(Some)
                    .map_err(io::Error::other)
            }
        }
    }
}
impl<W: Write> DynamicCollisionWorld for World<'_, W> {
    fn shulker_collision_state(
        &mut self,
        p: [i32; 3],
    ) -> io::Result<Option<ShulkerCollisionState>> {
        self.query(p)?;
        Ok(match self.entity {
            "none" | "other" | "piston" => None,
            _ => Some(self.shulker),
        })
    }
}
impl<W: Write> PistonCollisionWorld for World<'_, W> {
    type Error = io::Error;
    fn collision_shape(&mut self, state: BlockStateId, p: [i32; 3]) -> io::Result<VoxelShape> {
        if let Some(shape) = self.collisions.cached_shape(state)? {
            return Ok(shape.clone());
        }
        self.dynamic[state.0 as usize]
            .as_ref()
            .expect("known empty-context dynamic collision")
            .clone()
            .collision_shape(self, p)
            .map_err(io::Error::other)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let facts: Value = serde_json::from_str(&fs::read_to_string(dir.join("facts.json"))?)?;
    let states = facts["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| BlockState {
            name: value["id"].as_str().unwrap().to_owned(),
            properties: value
                .get("properties")
                .and_then(Value::as_object)
                .map(|p| {
                    p.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let read = |name: &str| -> Result<Tag, Box<dyn std::error::Error>> {
        Ok(
            nbt::read_named(&fs::read(dir.join(name))?, Limits::default())?
                .0
                .tag,
        )
    };
    let registry = BlockRegistry::from_nbt(&read("block-registry.nbt")?)?;
    let collisions = CachedCollisionRegistry::from_nbt(&read("cached-collision.nbt")?, &registry)?;
    let pistons = PistonBlockBindings::new(&registry)?;
    let dynamic = states
        .iter()
        .map(DynamicSupport::from_state)
        .collect::<io::Result<Vec<_>>>()?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let support = DynamicSupport::from_state(&states[f["state"].as_u64().unwrap() as usize])?
            .ok_or("missing dynamic support selection")?;
        let p = std::array::from_fn(|i| f["position"][i].as_i64().unwrap() as i32);
        values(&mut out, "offset", support.offset(p).components())?;
        let active = f.get("piston").map(|v| MovingPistonState {
            moved_state: BlockStateId(v["state"].as_u64().unwrap() as u32),
            direction: Direction::ALL[v["direction"].as_u64().unwrap() as usize],
            extending: v["extending"].as_bool().unwrap(),
            source: v["source"].as_bool().unwrap(),
            progress: f32::from_bits(
                u32::from_str_radix(v["progress"].as_str().unwrap(), 16).unwrap(),
            ),
            previous_progress: 0.0,
        });
        let clip = f
            .get("piston")
            .map(|v| v["noclip"].as_i64().unwrap())
            .unwrap_or(-1);
        let mut world = World {
            collisions: &collisions,
            pistons: &pistons,
            dynamic: &dynamic,
            active,
            noclip: (clip >= 0).then(|| Direction::ALL[clip as usize]),
            out: &mut out,
            entity: f["entity"].as_str().unwrap(),
            shulker: ShulkerCollisionState {
                previous_progress: f.get("shulker_progress").map_or(0.0, |v| {
                    f32::from_bits(u32::from_str_radix(v[0].as_str().unwrap(), 16).unwrap())
                }),
                progress: f.get("shulker_progress").map_or(0.0, |v| {
                    f32::from_bits(u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
                }),
            },
            calls: 0,
            fail: f["fail"].as_i64().unwrap() as i32,
        };
        match if f.get("collision").and_then(Value::as_bool).unwrap_or(false) {
            support.collision_shape(&mut world, p)
        } else {
            support.shape(&mut world, p)
        } {
            Ok(shape) => emit(world.out, &shape)?,
            Err(_) => writeln!(world.out, "shape error")?,
        }
        for direction in Direction::ALL {
            for (i, kind) in [SupportType::Full, SupportType::Center, SupportType::Rigid]
                .into_iter()
                .enumerate()
            {
                match support.is_face_sturdy(&mut world, p, direction, kind) {
                    Ok(value) => writeln!(world.out, "support {} {i} {value}", direction as u8)?,
                    Err(_) => writeln!(world.out, "support {} {i} error", direction as u8)?,
                }
            }
        }
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
