use crate::support::common::f64_bits;
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::player_travel::*;
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
fn vec(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn fbits(v: f32) -> u32 {
    if v.is_nan() { 0x7fc00000 } else { v.to_bits() }
}
fn vector(v: Vec3) -> String {
    format!(
        "{:016x} {:016x} {:016x}",
        f64_bits(v.x),
        f64_bits(v.y),
        f64_bits(v.z)
    )
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    velocity: Vec3,
    position: Vec3,
    flying: bool,
}
impl World<'_> {
    fn flag(&self, k: &str) -> bool {
        self.f[k].as_bool().unwrap()
    }
    fn query(&mut self, s: impl AsRef<str>) -> io::Result<()> {
        writeln!(self.out, "{}", s.as_ref())?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected player travel failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.velocity = vec(&self.f["after_velocity"]);
            self.position = vec(&self.f["after_position"]);
            self.flying = self.flag("after_flying");
        }
        Ok(())
    }
}
impl PlayerTravelWorld for World<'_> {
    type Error = io::Error;
    fn is_passenger(&mut self) -> io::Result<bool> {
        self.query("passenger")?;
        Ok(self.flag("passenger"))
    }
    fn is_swimming(&mut self) -> io::Result<bool> {
        self.query("swimming")?;
        Ok(self.flag("swimming"))
    }
    fn look_angle(&mut self) -> io::Result<Vec3> {
        self.query("look")?;
        Ok(vec(&self.f["look"]))
    }
    fn jumping(&self) -> bool {
        self.flag("jumping")
    }
    fn position(&self) -> Vec3 {
        self.position
    }
    fn fluid_is_empty(&mut self, p: BlockPos) -> io::Result<bool> {
        self.query(format!("fluid {} {} {}", p.x, p.y, p.z))?;
        Ok(self.flag("empty"))
    }
    fn velocity(&self) -> Vec3 {
        self.velocity
    }
    fn set_velocity(&mut self, v: Vec3) -> io::Result<()> {
        self.query(format!("set {}", vector(v)))?;
        self.velocity = v;
        Ok(())
    }
    fn query_flying_ability(&mut self) -> io::Result<bool> {
        self.query("abilities")?;
        Ok(self.flying)
    }
    fn flying_ability(&self) -> bool {
        self.flying
    }
    fn ability_flying_speed(&mut self) -> io::Result<f32> {
        self.query("ability-speed")?;
        Ok(single(&self.f["speed"]))
    }
    fn sprinting(&mut self) -> io::Result<bool> {
        self.query("sprint")?;
        Ok(self.flag("sprint"))
    }
    fn living_travel(&mut self, input: Vec3) -> io::Result<()> {
        // Native parent dispatch is exercised through a controlled fluid branch.
        self.query("block-position")?;
        self.query("fluid 17 -64 -33")?;
        self.query("should-fluid")?;
        self.query(format!("base {}", vector(input)))?;
        if self.flag("base_mutate") {
            self.velocity = vec(&self.f["base_velocity"]);
            self.flying = self.flag("base_flying");
        }
        Ok(())
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            velocity: vec(&f["velocity"]),
            position: vec(&f["position"]),
            flying: f["flying"].as_bool().unwrap(),
        };
        let result: io::Result<()> = match f["mode"].as_str().unwrap() {
            "travel" => travel(&mut w, vec(&f["input"])),
            "speed" => flying_speed(&mut w).and_then(|v| writeln!(w.out, "float {:08x}", fbits(v))),
            "affected" => {
                let b = affected_by_fluids(&w);
                writeln!(w.out, "bool {b}")
            }
            _ => panic!("mode"),
        };
        writeln!(
            w.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        writeln!(
            w.out,
            "final {} {} {}",
            vector(w.velocity),
            vector(w.position),
            w.flying
        )?;
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
