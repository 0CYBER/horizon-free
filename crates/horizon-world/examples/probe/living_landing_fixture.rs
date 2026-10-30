use crate::support::common::f64_bits;
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::fall_damage::FallDamageWorld;
use horizon_world::living_landing::{LivingLandingWorld, check_living_fall_damage};
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
    water: bool,
    climbable: bool,
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
        Ok(self.water)
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
impl LivingLandingWorld for World<'_> {
    fn update_fluid_interaction(&mut self) -> io::Result<bool> {
        self.query("fluid-update".into())?;
        self.water = self.input["after_water"].as_bool().unwrap();
        if self.input["fluid_mutate"].as_bool().unwrap() {
            self.fall = number(&self.input["after_fall"]);
        }
        Ok(self.input["fluid_result"].as_bool().unwrap())
    }
    fn is_server_level(&self) -> bool {
        self.input["server"].as_bool().unwrap()
    }
    fn on_changed_block(&mut self, p: BlockPos) -> io::Result<()> {
        self.query(format!("changed {} {} {}", p.x, p.y, p.z))?;
        if self.input["changed_mutate"].as_bool().unwrap() {
            self.fall = number(&self.input["after_fall"]);
            self.position = Vec3::new(17.0, 18.0, 19.0);
        }
        Ok(())
    }
    fn safe_fall_distance(&mut self) -> io::Result<f64> {
        self.query("safe".into())?;
        Ok(number(&self.input["safe"]))
    }
    fn state_is_air(&mut self, _: u8) -> io::Result<bool> {
        self.query("air".into())?;
        Ok(self.input["air"].as_bool().unwrap())
    }
    fn block_position(&mut self) -> io::Result<BlockPos> {
        self.query("block-position".into())?;
        if self.input["block_mutate"].as_bool().unwrap() {
            self.position = Vec3::new(31.0, 32.0, 33.0);
        }
        Ok(support(&self.input["block_position"]).unwrap())
    }
    fn landing_particles(&mut self, s: u8, p: Vec3, count: i32, speed: f64) -> io::Result<()> {
        assert_eq!(s, 0);
        self.query(format!(
            "particles {:016x} {:016x} {:016x} {} {:016x}",
            f64_bits(p.x),
            f64_bits(p.y),
            f64_bits(p.z),
            count,
            f64_bits(speed)
        ))?;
        if self.input["particle_mutate"].as_bool().unwrap() {
            self.fall = number(&self.input["after_fall"]);
            self.water = !self.water;
        }
        Ok(())
    }
    fn clear_last_climbable_position(&mut self) {
        self.climbable = false;
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
            position: Vec3::new(
                number(&f["position"][0]),
                number(&f["position"][1]),
                number(&f["position"][2]),
            ),
            water: f["water"].as_bool().unwrap(),
            climbable: true,
            support: support(&f["support"]),
        };
        let result = check_living_fall_damage(
            &mut world,
            number(&f["dy"]),
            f["ground"].as_bool().unwrap(),
            0,
            support(&f["landing"]).unwrap(),
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
        writeln!(
            world.out,
            "climbable {} water {}",
            world.climbable, world.water
        )?;
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
