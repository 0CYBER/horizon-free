use horizon_core::geometry::{Aabb, Axis, Direction, VoxelShape};
use horizon_world::{
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
fn values(
    out: &mut impl Write,
    label: &str,
    values: impl IntoIterator<Item = f64>,
) -> io::Result<()> {
    write!(out, "{label}")?;
    for v in values {
        let bits = if v.is_nan() {
            0x7ff8_0000_0000_0000
        } else {
            v.to_bits()
        };
        write!(out, " {bits:016x}")?;
    }
    writeln!(out)
}
fn box_values(out: &mut impl Write, label: &str, b: Aabb) -> io::Result<()> {
    values(
        out,
        label,
        [
            b.min(Axis::X),
            b.min(Axis::Y),
            b.min(Axis::Z),
            b.max(Axis::X),
            b.max(Axis::Y),
            b.max(Axis::Z),
        ],
    )
}
fn emit(out: &mut impl Write, shape: &VoxelShape) -> io::Result<()> {
    writeln!(
        out,
        "shape {} {} {}",
        shape.is_empty(),
        shape.boxes().len(),
        shape.is_cube()
    )?;
    for (axis, name) in Axis::ALL.into_iter().zip(["x", "y", "z"]) {
        values(out, name, shape.coordinates(axis).iter().copied())?;
    }
    let axes = shape.cube_coordinate_axes();
    writeln!(
        out,
        "axes {}",
        u8::from(axes[0]) + 2 * u8::from(axes[1]) + 4 * u8::from(axes[2])
    )?;
    values(
        out,
        "extrema",
        Axis::ALL
            .map(|a| shape.min(a))
            .into_iter()
            .chain(Axis::ALL.map(|a| shape.max(a))),
    )?;
    match shape.bounds() {
        Some(b) => box_values(out, "bounds", b)?,
        None => writeln!(out, "bounds none")?,
    }
    for b in shape.boxes() {
        box_values(out, "box", *b)?;
    }
    Ok(())
}
struct World<'a, 'r, W> {
    collisions: &'a CachedCollisionRegistry<'r>,
    dynamic: &'a [Option<DynamicSupport>],
    out: &'a mut W,
    fail: i32,
    calls: i32,
}
impl<W: Write> World<'_, '_, W> {
    fn query(&mut self, p: [i32; 3]) -> io::Result<()> {
        writeln!(self.out, "entity {} {} {}", p[0], p[1], p[2])?;
        let count = self.calls;
        self.calls += 1;
        if count == self.fail {
            Err(io::Error::other("injected block entity failure"))
        } else {
            Ok(())
        }
    }
}
impl<W: Write> DynamicSupportWorld for World<'_, '_, W> {
    type Error = io::Error;
    fn shulker_is_closed(&mut self, p: [i32; 3]) -> io::Result<Option<bool>> {
        self.query(p)?;
        Ok(None)
    }
    fn moving_piston_shape(&mut self, p: [i32; 3]) -> io::Result<Option<VoxelShape>> {
        self.query(p)?;
        Ok(None)
    }
}
impl<W: Write> PistonCollisionWorld for World<'_, '_, W> {
    type Error = io::Error;
    fn collision_shape(&mut self, state: BlockStateId, p: [i32; 3]) -> io::Result<VoxelShape> {
        if let Some(shape) = self.collisions.cached_shape(state)? {
            return Ok(shape.clone());
        }
        let dynamic = self.dynamic[state.0 as usize]
            .as_ref()
            .expect("known empty-context dynamic collision")
            .clone();
        dynamic.shape(self, p).map_err(io::Error::other)
    }
}
fn number(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn float_bits(v: f32) -> u32 {
    if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).expect("private fixture directory");
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
    let bindings = PistonBlockBindings::new(&registry)?;
    let dynamic = (0..registry.len())
        .map(|id| DynamicSupport::from_state(registry.state(BlockStateId(id as u32)).unwrap()))
        .collect::<io::Result<Vec<_>>>()?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let p = std::array::from_fn(|i| f["position"][i].as_i64().unwrap() as i32);
        let piston = MovingPistonState {
            moved_state: BlockStateId(f["state"].as_u64().unwrap() as u32),
            direction: Direction::ALL[f["direction"].as_u64().unwrap() as usize],
            extending: f["extending"].as_bool().unwrap(),
            source: f["source"].as_bool().unwrap(),
            progress: number(&f["progress"]),
            previous_progress: number(&f["previous"]),
        };
        let partial = number(&f["partial"]);
        let offset = piston.interpolated_offset(partial).map(float_bits);
        writeln!(
            out,
            "motion {} {:08x} {:08x} {:08x} {:08x}",
            piston.movement_direction() as u8,
            float_bits(piston.interpolated_progress(partial)),
            offset[0],
            offset[1],
            offset[2]
        )?;
        writeln!(
            out,
            "related {}",
            bindings.collision_related_state(piston)?.0
        )?;
        let mut world = World {
            collisions: &collisions,
            dynamic: &dynamic,
            out: &mut out,
            fail: f["fail"].as_i64().unwrap() as i32,
            calls: 0,
        };
        let clip = f["noclip"].as_i64().unwrap();
        let noclip = (clip >= 0).then(|| Direction::ALL[clip as usize]);
        if !f["kinematic"].as_bool().unwrap() {
            match bindings.collision_shape(&mut world, p, piston, noclip) {
                Ok(shape) => emit(world.out, &shape)?,
                Err(_) => writeln!(world.out, "shape error")?,
            }
        }
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
