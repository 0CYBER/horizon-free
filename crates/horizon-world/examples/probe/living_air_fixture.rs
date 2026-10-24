use crate::support::common::f64_bits;
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::living_air::*;
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
    ground_calls: usize,
    climb_calls: usize,
    velocity: Vec3,
    fall: f64,
    moved: bool,
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
            Err(io::Error::other("injected travel failure"))
        } else {
            Ok(())
        }
    }
    fn sequence(&self, k: &str, n: usize) -> bool {
        let a = self.f[k].as_array().unwrap();
        a[n.min(a.len() - 1)].as_bool().unwrap()
    }
}
impl LivingAirWorld for World<'_> {
    type Error = io::Error;
    fn movement_support_position(&mut self) -> io::Result<BlockPos> {
        self.query("support")?;
        Ok(BlockPos::new(17, -64, -33))
    }
    fn on_ground(&mut self) -> io::Result<bool> {
        self.query("ground")?;
        let b = self.sequence("ground", self.ground_calls);
        self.ground_calls += 1;
        Ok(b)
    }
    fn block_friction(&mut self, p: BlockPos) -> io::Result<f32> {
        self.query(format!("state {} {} {}", p.x, p.y, p.z))?;
        self.query("friction")?;
        Ok(single(&self.f["friction"]))
    }
    fn friction_modifier(&mut self) -> io::Result<f64> {
        self.query("friction-modifier")?;
        Ok(number(&self.f["friction_modifier"]))
    }
    fn speed(&mut self) -> io::Result<f32> {
        self.query("speed")?;
        Ok(single(&self.f["speed"]))
    }
    fn flying_speed(&mut self) -> io::Result<f32> {
        self.query("flying-speed")?;
        Ok(single(&self.f["flying_speed"]))
    }
    fn yaw(&mut self) -> io::Result<f32> {
        self.query("yaw")?;
        if self.flag("yaw_mutate") {
            self.velocity = Vec3::new(17.0, 18.0, 19.0);
        }
        Ok(single(&self.f["yaw"]))
    }
    fn velocity(&self) -> Vec3 {
        self.velocity
    }
    fn set_velocity(&mut self, v: Vec3) -> io::Result<()> {
        self.query(format!("set {}", vector(v)))?;
        self.velocity = v;
        Ok(())
    }
    fn on_climbable(&mut self) -> io::Result<bool> {
        self.query("climb")?;
        let b = self.sequence("climb", self.climb_calls);
        self.climb_calls += 1;
        Ok(b)
    }
    fn reset_fall_distance(&mut self) -> io::Result<()> {
        self.query("reset")?;
        self.fall = 0.0;
        Ok(())
    }
    fn in_scaffolding(&mut self) -> io::Result<bool> {
        self.query("in-block")?;
        Ok(self.flag("scaffolding"))
    }
    fn suppressing_ladder_slide(&mut self) -> io::Result<bool> {
        self.query("suppress")?;
        Ok(self.flag("suppress"))
    }
    fn is_player_class(&self) -> bool {
        self.flag("player")
    }
    fn move_self(&mut self, v: Vec3) -> io::Result<()> {
        self.query(format!("move {}", vector(v)))?;
        self.moved = true;
        if self.flag("move_mutate") {
            self.velocity = vec(&self.f["after_velocity"]);
        }
        Ok(())
    }
    fn horizontal_collision(&self) -> bool {
        self.flag(if self.moved {
            "after_horizontal"
        } else {
            "horizontal"
        })
    }
    fn jumping(&self) -> bool {
        self.flag(if self.moved {
            "after_jumping"
        } else {
            "jumping"
        })
    }
    fn was_in_powder_snow(&self) -> bool {
        self.flag(if self.moved { "after_snow" } else { "snow" })
    }
    fn can_walk_powder_snow(&mut self) -> io::Result<bool> {
        self.query("powder-tag")?;
        if self.flag("powder_tag") {
            Ok(true)
        } else {
            self.query("feet")?;
            Ok(self.flag("boots"))
        }
    }
    fn levitation_amplifier(&mut self) -> io::Result<Option<i32>> {
        self.query("levitation")?;
        Ok(self.f["levitation"].as_i64().map(|v| v as i32))
    }
    fn is_client_side(&mut self) -> io::Result<bool> {
        self.query("client")?;
        Ok(self.flag("client"))
    }
    fn has_chunk_at(&mut self, p: BlockPos) -> io::Result<bool> {
        self.query(format!("chunk {} {} {}", p.x, p.y, p.z))?;
        Ok(self.flag("chunk"))
    }
    fn effective_gravity(&mut self) -> io::Result<f64> {
        self.query("gravity")?;
        Ok(number(&self.f["gravity"]))
    }
    fn y(&self) -> f64 {
        number(&self.f["y"])
    }
    fn minimum_y(&mut self) -> io::Result<i32> {
        self.query("min-y")?;
        Ok(self.f["min_y"].as_i64().unwrap() as i32)
    }
    fn discard_friction(&mut self) -> io::Result<bool> {
        self.query("discard")?;
        Ok(self.flag("discard"))
    }
    fn air_drag_modifier(&mut self) -> io::Result<f64> {
        self.query("drag-modifier")?;
        Ok(number(&self.f["drag_modifier"]))
    }
    fn omnidirectional_air_mover(&mut self) -> io::Result<bool> {
        self.query("omni")?;
        Ok(self.flag("omni"))
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
            ground_calls: 0,
            climb_calls: 0,
            velocity: vec(&f["velocity"]),
            fall: number(&f["fall"]),
            moved: false,
        };
        let input = vec(&f["input"]);
        let friction = single(&f["friction"]);
        let result: io::Result<()> = match f["mode"].as_str().unwrap() {
            "air" => travel_in_air(&mut w, input),
            "relative" => relative_friction_movement(&mut w, input, friction)
                .and_then(|v| writeln!(w.out, "vector {}", vector(v))),
            "climb" => handle_on_climbable(&mut w, input)
                .and_then(|v| writeln!(w.out, "vector {}", vector(v))),
            "speed" => friction_influenced_speed(&mut w, friction)
                .and_then(|v| writeln!(w.out, "float {:08x}", fbits(v))),
            "input" => writeln!(
                w.out,
                "vector {}",
                vector(input_vector(input, single(&f["speed"]), single(&f["yaw"])))
            ),
            "friction" => writeln!(
                w.out,
                "float {:08x}",
                fbits(modified_friction(
                    friction,
                    number(&f["friction_modifier"]) as f32
                ))
            ),
            _ => panic!("unknown mode"),
        };
        writeln!(
            w.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        writeln!(
            w.out,
            "final {} {:016x}",
            vector(w.velocity),
            f64_bits(w.fall)
        )?;
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
