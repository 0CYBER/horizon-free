use crate::support::common::f32_bits;
use horizon_world::{
    living_damage::{self, DamageStat, DamageWorld},
    living_health::{self, HealthWorld},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn stat(s: DamageStat) -> &'static str {
    match s {
        DamageStat::Absorbed => "absorbed",
        DamageStat::DealtAbsorbed => "dealt-absorbed",
        DamageStat::Taken => "taken",
    }
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    changed: bool,
    health: f32,
    absorption: f32,
    exhaustion: f32,
    recorded: f32,
}
impl World<'_> {
    fn state(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn reset(&mut self) {
        self.health = single(&self.state()["health"]);
        self.absorption = single(&self.state()["absorption"]);
        self.exhaustion = single(&self.state()["exhaustion"]);
    }
    fn query(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected damage callback failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
            self.reset();
        }
        Ok(())
    }
}
impl DamageWorld for World<'_> {
    type Error = io::Error;
    type Attacker = ();
    fn invulnerable(&mut self) -> io::Result<bool> {
        self.query("invulnerable")?;
        Ok(self.state()["invulnerable"].as_bool().unwrap())
    }
    fn after_armor(&mut self, v: f32) -> io::Result<f32> {
        self.query(&format!("armor {:08x}", f32_bits(v)))?;
        Ok(single(&self.state()["armor"]))
    }
    fn after_magic(&mut self, v: f32) -> io::Result<f32> {
        self.query(&format!("magic {:08x}", f32_bits(v)))?;
        Ok(single(&self.state()["magic"]))
    }
    fn absorption(&mut self) -> io::Result<f32> {
        self.query("absorption")?;
        Ok(self.absorption)
    }
    fn set_absorption(&mut self, v: f32) -> io::Result<()> {
        living_health::set_absorption(self, v)
    }
    fn server_player_attacker(&mut self) -> io::Result<Option<()>> {
        self.query("attacker")?;
        Ok((self.state()["attacker"] == 2).then_some(()))
    }
    fn award_attacker(&mut self, _: (), s: DamageStat, v: i32) -> io::Result<()> {
        self.query(&format!("stat attacker {} {v}", stat(s)))
    }
    fn award_self(&mut self, s: DamageStat, v: i32) -> io::Result<()> {
        self.query(&format!("stat self {} {v}", stat(s)))
    }
    fn source_food_exhaustion(&mut self) -> io::Result<f32> {
        self.query("food-exhaustion")?;
        Ok(single(&self.state()["food_exhaustion"]))
    }
    fn cause_food_exhaustion(&mut self, v: f32) -> io::Result<()> {
        self.query(&format!("exhaust {:08x}", f32_bits(v)))?;
        self.exhaustion += v;
        Ok(())
    }
    fn record_damage(&mut self, v: f32) -> io::Result<()> {
        self.query("tracker")?;
        self.query(&format!("record {:08x}", f32_bits(v)))?;
        self.recorded = v;
        Ok(())
    }
    fn health(&mut self) -> io::Result<f32> {
        self.query("health")?;
        Ok(self.health)
    }
    fn set_health(&mut self, v: f32) -> io::Result<()> {
        self.query(&format!("set-health {:08x}", f32_bits(v)))?;
        self.health = v;
        Ok(())
    }
    fn damage_event(&mut self) -> io::Result<()> {
        self.query("damage-event")
    }
}
// Reuse the separately verified final absorption setter for native clamping.
impl HealthWorld for World<'_> {
    type Error = io::Error;
    type Data = ();
    fn health(&mut self) -> io::Result<f32> {
        DamageWorld::health(self)
    }
    fn health_data(&self) {}
    fn max_health_attribute(&mut self) -> io::Result<f64> {
        unreachable!("fixture only binds absorption")
    }
    fn max_absorption_attribute(&mut self) -> io::Result<f64> {
        self.query("max-absorption")?;
        Ok(number(&self.state()["max_absorption"]))
    }
    fn store_health(&mut self, _: (), v: f32) -> io::Result<()> {
        DamageWorld::set_health(self, v)
    }
    fn store_absorption(&mut self, v: f32) -> io::Result<()> {
        self.query(&format!("set-absorption {:08x}", f32_bits(v)))?;
        self.absorption = v;
        Ok(())
    }
    fn dead_flag(&self) -> bool {
        false
    }
    fn dispatch_set_health(&mut self, v: f32) -> io::Result<()> {
        DamageWorld::set_health(self, v)
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
            changed: false,
            health: 0.,
            absorption: 0.,
            exhaustion: 0.,
            recorded: 0.,
        };
        w.reset();
        let result = living_damage::actually_hurt(
            &mut w,
            single(&f["damage"]),
            f["player"].as_bool().unwrap(),
        );
        writeln!(
            w.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        writeln!(
            w.out,
            "final {:08x} {:08x} {:08x} {:08x}",
            f32_bits(w.health),
            f32_bits(w.absorption),
            f32_bits(w.exhaustion),
            f32_bits(w.recorded)
        )?;
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
