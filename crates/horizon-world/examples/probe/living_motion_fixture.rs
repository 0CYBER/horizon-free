use crate::support::common::f64_bits;
use horizon_core::geometry::Vec3;
use horizon_world::living_motion::*;
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
    sync: bool,
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
            return Err(io::Error::other("injected motion failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.velocity = vec(&self.f["after_velocity"]);
        }
        Ok(())
    }
}
impl LivingMotionWorld for World<'_> {
    type Error = io::Error;
    fn velocity(&self) -> Vec3 {
        self.velocity
    }
    fn set_velocity(&mut self, v: Vec3) -> io::Result<()> {
        self.query(format!("set {}", vector(v)))?;
        self.velocity = v;
        Ok(())
    }
    fn jump_strength(&mut self) -> io::Result<f64> {
        self.query("strength")?;
        Ok(number(&self.f["strength"]))
    }
    fn block_jump_factor(&mut self) -> io::Result<f32> {
        self.query("block-factor")?;
        Ok(single(&self.f["block_factor"]))
    }
    fn has_jump_boost(&mut self) -> io::Result<bool> {
        self.query("has-boost")?;
        Ok(self.flag("boost"))
    }
    fn jump_boost_amplifier(&mut self) -> io::Result<i32> {
        self.query("get-boost")?;
        Ok(self.f["amplifier"].as_i64().unwrap() as i32)
    }
    fn sprinting(&mut self) -> io::Result<bool> {
        self.query("sprint")?;
        Ok(self.flag("sprint"))
    }
    fn yaw(&mut self) -> io::Result<f32> {
        self.query("yaw")?;
        Ok(single(&self.f["yaw"]))
    }
    fn mark_needs_sync(&mut self) {
        self.sync = true;
    }
    fn no_gravity(&mut self) -> io::Result<bool> {
        self.query("no-gravity")?;
        Ok(self.flag("no_gravity"))
    }
    fn gravity_attribute(&mut self) -> io::Result<f64> {
        self.query("gravity")?;
        Ok(number(&self.f["gravity"]))
    }
    fn has_slow_falling(&mut self) -> io::Result<bool> {
        self.query("slow-falling")?;
        Ok(self.flag("slow_falling"))
    }
    fn controlling_passenger_is_player(&mut self) -> io::Result<bool> {
        self.query("passenger")?;
        Ok(self.f["passenger"].as_i64().unwrap() == 2)
    }
    fn speed(&mut self) -> io::Result<f32> {
        self.query("speed")?;
        Ok(single(&self.f["speed"]))
    }
    fn omnidirectional_air_mover(&mut self) -> io::Result<bool> {
        self.query("omni")?;
        Ok(self.flag("omni"))
    }
    fn air_drag_modifier(&mut self) -> io::Result<f64> {
        self.query("drag-modifier")?;
        Ok(number(&self.f["drag_modifier"]))
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
            sync: f["sync"].as_bool().unwrap(),
        };
        let result: io::Result<()> = match f["mode"].as_str().unwrap() {
            "boost" => {
                jump_boost_power(&mut w).and_then(|v| writeln!(w.out, "float {:08x}", fbits(v)))
            }
            "power" => jump_power(&mut w, single(&f["scale"]))
                .and_then(|v| writeln!(w.out, "float {:08x}", fbits(v))),
            "jump" => jump_from_ground(&mut w),
            "gravity" => {
                gravity(&mut w).and_then(|v| writeln!(w.out, "double {:016x}", f64_bits(v)))
            }
            "effective" => effective_gravity(&mut w)
                .and_then(|v| writeln!(w.out, "double {:016x}", f64_bits(v))),
            "apply" => apply_gravity(&mut w),
            "up" => swim_vertical(&mut w, true),
            "down" => swim_vertical(&mut w, false),
            "speed" => {
                default_flying_speed(&mut w).and_then(|v| writeln!(w.out, "float {:08x}", fbits(v)))
            }
            "drag" => air_drag(&mut w).and_then(|v| writeln!(w.out, "float {:08x}", fbits(v))),
            "add" => add_delta_movement(&mut w, vec(&f["delta"])),
            _ => panic!("mode"),
        };
        writeln!(
            w.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        writeln!(w.out, "final {} {}", vector(w.velocity), w.sync)?;
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
