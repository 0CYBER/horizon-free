use crate::support::common::f64_bits;
use horizon_core::geometry::Vec3;
use horizon_world::living_fall::{
    ImpulseContext, LivingFallWorld, calculate_fall_damage, cause_living_fall_damage,
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
fn vector(v: &Value) -> Option<Vec3> {
    if v.is_null() {
        None
    } else {
        Some(Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])))
    }
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    immune_calls: usize,
    hurt: usize,
    impulse: ImpulseContext,
}
impl World<'_> {
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.f["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected living fall query failure"))
        } else {
            Ok(())
        }
    }
    fn emit(&mut self) -> io::Result<()> {
        let i = self.impulse;
        writeln!(
            self.out,
            "grace {} {} {}",
            i.reset_grace_time,
            i.is_in_grace_time(),
            i.ignores_fall_damage()
        )?;
        if let Some(p) = i.impact_position {
            writeln!(
                self.out,
                "impact {:016x} {:016x} {:016x}",
                f64_bits(p.x),
                f64_bits(p.y),
                f64_bits(p.z)
            )
        } else {
            writeln!(self.out, "impact none")
        }
    }
}
impl LivingFallWorld for World<'_> {
    type Error = io::Error;
    type Sound = bool;
    fn impact_position(&self) -> Option<Vec3> {
        self.impulse.impact_position
    }
    fn y(&self) -> f64 {
        number(&self.f["y"])
    }
    fn reset_impulse(&mut self) -> io::Result<()> {
        self.query("reset".into())?;
        self.impulse.reset();
        Ok(())
    }
    fn try_reset_impulse(&mut self) -> io::Result<()> {
        self.query("try-reset".into())?;
        if self.impulse.reset_grace_time == 0 {
            self.reset_impulse()?;
        }
        Ok(())
    }
    fn immune_to_fall_damage(&mut self) -> io::Result<bool> {
        self.query("immune".into())?;
        let v = self.f["immune"].as_array().unwrap();
        let value = v[self.immune_calls.min(v.len() - 1)].as_bool().unwrap();
        self.immune_calls += 1;
        Ok(value)
    }
    fn propagate_fall_to_passengers(&mut self, d: f64, m: f32) -> io::Result<()> {
        self.query(format!(
            "passengers {:016x} {:08x}",
            f64_bits(d),
            if m.is_nan() { 0x7fc00000 } else { m.to_bits() }
        ))?;
        if self.f["mutate"].as_bool().unwrap() {
            self.impulse.reset_grace_time = 17;
            self.impulse.impact_position = Some(Vec3::new(17.0, 18.0, 19.0));
        }
        Ok(())
    }
    fn safe_fall_distance(&mut self) -> io::Result<f64> {
        self.query("safe".into())?;
        Ok(number(&self.f["safe"]))
    }
    fn fall_damage_multiplier(&mut self) -> io::Result<f64> {
        self.query("damage-factor".into())?;
        Ok(number(&self.f["damage-factor"]))
    }
    fn fall_sound(&mut self, big: bool) -> io::Result<bool> {
        self.query("sounds".into())?;
        Ok(big)
    }
    fn play_fall_sound(&mut self, big: bool) -> io::Result<()> {
        self.query(format!("sound {}", if big { "big" } else { "small" }))
    }
    fn play_block_fall_sound(&mut self) -> io::Result<()> {
        self.query("block-sound".into())
    }
    fn hurt(&mut self, damage: f32) -> io::Result<()> {
        if self.f["server"].as_bool().unwrap() {
            self.query(format!("hurt {:08x}", damage.to_bits()))?;
            self.hurt += 1;
        }
        Ok(())
    }
}
fn run(world: &mut World<'_>) -> io::Result<()> {
    let f = world.f;
    match f["mode"].as_u64().unwrap() {
        0 => {
            let value =
                calculate_fall_damage(world, number(&f["distance"]), single(&f["multiplier"]))?;
            writeln!(world.out, "damage {value}")?;
        }
        1 => {
            let value =
                cause_living_fall_damage(world, number(&f["distance"]), single(&f["multiplier"]))?;
            writeln!(world.out, "caused {value}")?;
        }
        2 => {
            for op in f["ops"].as_array().unwrap() {
                match op["mode"].as_u64().unwrap() {
                    0 => world.impulse.set_ignore_fall_damage(
                        op["ignore"].as_bool().unwrap(),
                        vector(&op["position"]),
                    ),
                    1 => world
                        .impulse
                        .apply_grace_time(op["ticks"].as_i64().unwrap() as i32),
                    2 => world.try_reset_impulse()?,
                    3 => world.reset_impulse()?,
                    _ => unreachable!(),
                };
                world.emit()?;
            }
        }
        _ => unreachable!(),
    };
    Ok(())
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
            immune_calls: 0,
            hurt: 0,
            impulse: ImpulseContext {
                reset_grace_time: f["grace"].as_i64().unwrap() as i32,
                impact_position: vector(&f["impact"]),
            },
        };
        if run(&mut world).is_err() {
            writeln!(world.out, "error")?;
        }
        world.emit()?;
        writeln!(world.out, "hurt-count {}", world.hurt)?;
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
