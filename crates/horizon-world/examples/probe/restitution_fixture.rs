use crate::support::common::f64_bits;
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::{
    movement_restitution::{RestitutionInput, RestitutionWorld, restitute_movement},
    registry::BlockStateId,
};
use serde_json::Value;
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
fn vector(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn vector_text(prefix: &str, v: Vec3) -> String {
    format!(
        "{prefix} {:016x} {:016x} {:016x}",
        f64_bits(v.x),
        f64_bits(v.y),
        f64_bits(v.z)
    )
}
struct World<'a, W> {
    f: &'a Value,
    out: &'a mut W,
    calls: i32,
    suppress_calls: usize,
    gravity_calls: usize,
    velocity: Vec3,
    sync: bool,
}
impl<W: Write> World<'_, W> {
    fn query(&mut self, text: &str) -> io::Result<()> {
        writeln!(self.out, "{text}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.f["fail"].as_i64().unwrap() as i32 {
            Err(io::Error::other("injected restitution failure"))
        } else {
            Ok(())
        }
    }
}
impl<W: Write> RestitutionWorld for World<'_, W> {
    type Error = io::Error;
    fn suppressing_bounce(&mut self) -> io::Result<bool> {
        self.query("suppress")?;
        let values = self.f["suppress"].as_array().unwrap();
        let result = values[self.suppress_calls.min(values.len() - 1)]
            .as_bool()
            .unwrap();
        self.suppress_calls += 1;
        Ok(result)
    }
    fn entity_bounciness(&mut self) -> io::Result<f64> {
        self.query("entity-bounce")?;
        Ok(number(&self.f["entity"]))
    }
    fn velocity(&mut self) -> io::Result<Vec3> {
        self.query("velocity")?;
        Ok(self.velocity)
    }
    fn effective_gravity(&mut self) -> io::Result<f64> {
        self.query("gravity")?;
        let values = self.f["gravity"].as_array().unwrap();
        let result = number(&values[self.gravity_calls.min(values.len() - 1)]);
        self.gravity_calls += 1;
        Ok(result)
    }
    fn block_suppresses_bounce(&mut self, _: BlockStateId) -> io::Result<bool> {
        self.query("block-suppresses")?;
        Ok(self.f["tag"].as_bool().unwrap())
    }
    fn block_bounce_restitution(&mut self, _: BlockStateId) -> io::Result<f32> {
        self.query("block-bounce")?;
        Ok(single(&self.f["block"]))
    }
    fn air_drag(&mut self) -> io::Result<f32> {
        self.query("drag")?;
        Ok(single(&self.f["drag"]))
    }
    fn movement_support_position(&mut self) -> io::Result<BlockPos> {
        self.query("support-position")?;
        Ok(BlockPos::new(17, -64, -33))
    }
    fn bounce_on(&mut self, _: BlockStateId, p: BlockPos, fall: f64) -> io::Result<()> {
        self.query(&format!(
            "bounce-on {} {} {} {:016x}",
            p.x,
            p.y,
            p.z,
            f64_bits(fall)
        ))?;
        if self.f["mutate"].as_bool().unwrap() {
            self.velocity = Vec3::new(17.0, 18.0, 19.0);
        }
        Ok(())
    }
    fn bounce_event(&mut self) -> io::Result<()> {
        self.query("bounce-event")
    }
    fn mark_position_for_sync(&mut self) {
        self.sync = true;
    }
    fn set_velocity(&mut self, v: Vec3) -> io::Result<()> {
        self.query(&vector_text("set-velocity", v))?;
        self.velocity = v;
        Ok(())
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut world = World {
            f: &f,
            out: &mut out,
            calls: 0,
            suppress_calls: 0,
            gravity_calls: 0,
            velocity: vector(&f["velocity"]),
            sync: f["sync"].as_bool().unwrap(),
        };
        let result = restitute_movement(
            &mut world,
            RestitutionInput {
                block: BlockStateId(1),
                collided_x: f["x"].as_bool().unwrap(),
                collided_z: f["z"].as_bool().unwrap(),
                vertical_collision: f["vertical"].as_bool().unwrap(),
                collision_below: f["below"].as_bool().unwrap(),
                living: f["living"].as_bool().unwrap(),
                resolved_movement: vector(&f["resolved"]),
                fall_distance: number(&f["fall"]),
            },
        );
        writeln!(
            world.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        writeln!(world.out, "{}", vector_text("final", world.velocity))?;
        writeln!(world.out, "sync {}", world.sync)?;
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
