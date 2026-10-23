use crate::support::common::f64_bits;
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::fall_damage::{FallDamageWorld, check_fall_damage};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn support(v: &Value) -> Option<BlockPos> {
    if v.is_null() {
        None
    } else {
        Some(BlockPos::new(
            v[0].as_i64().unwrap() as i32,
            v[1].as_i64().unwrap() as i32,
            v[2].as_i64().unwrap() as i32,
        ))
    }
}
struct World<'a> {
    input: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    fall: f64,
    position: Vec3,
    support: Option<BlockPos>,
}
impl World<'_> {
    fn query(&mut self, text: String) -> io::Result<()> {
        writeln!(self.out, "{text}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.input["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected fall query failure"))
        } else {
            Ok(())
        }
    }
}
impl FallDamageWorld for World<'_> {
    type Error = io::Error;
    type State = u8;
    fn is_in_water(&mut self) -> io::Result<bool> {
        self.query("water".into())?;
        Ok(self.input["water"].as_bool().unwrap())
    }
    fn fall_distance(&self) -> f64 {
        self.fall
    }
    fn set_fall_distance(&mut self, v: f64) {
        self.fall = v;
    }
    fn fall_on(&mut self, _: u8, p: BlockPos, distance: f64) -> io::Result<()> {
        self.query(format!(
            "fall-on {} {} {} {:016x}",
            p.x,
            p.y,
            p.z,
            f64_bits(distance)
        ))?;
        if self.input["mutate"].as_bool().unwrap() {
            self.fall = 17.25;
            self.position = Vec3::new(17.0, 18.0, 19.0);
            self.support = support(&self.input["after_support"]);
        }
        Ok(())
    }
    fn position(&self) -> Vec3 {
        self.position
    }
    fn supporting_block(&self) -> Option<BlockPos> {
        self.support
    }
    fn block_state(&mut self, p: BlockPos) -> io::Result<u8> {
        self.query(format!("state {} {} {}", p.x, p.y, p.z))?;
        Ok(1)
    }
    fn hit_ground(&mut self, p: Vec3, s: u8) -> io::Result<()> {
        self.query(format!(
            "hit-ground {:016x} {:016x} {:016x} {}",
            f64_bits(p.x),
            f64_bits(p.y),
            f64_bits(p.z),
            if s == 1 { "queried" } else { "input" }
        ))
    }
    fn reset_fall_distance(&mut self) -> io::Result<()> {
        self.query("reset".into())?;
        self.fall = 0.0;
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
            input: &f,
            out: &mut out,
            calls: 0,
            fall: number(&f["fall"]),
            position: Vec3::new(0.3, 1.5, -0.3),
            support: support(&f["support"]),
        };
        let result = check_fall_damage(
            &mut world,
            number(&f["dy"]),
            f["ground"].as_bool().unwrap(),
            0,
            BlockPos::new(3, 4, 5),
        );
        writeln!(
            world.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        let p = world.position;
        writeln!(
            world.out,
            "final {:016x} {:016x} {:016x} {:016x}",
            f64_bits(world.fall),
            f64_bits(p.x),
            f64_bits(p.y),
            f64_bits(p.z)
        )?;
        if let Some(p) = world.support {
            writeln!(world.out, "support {} {} {}", p.x, p.y, p.z)?;
        } else {
            writeln!(world.out, "support none")?;
        }
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
