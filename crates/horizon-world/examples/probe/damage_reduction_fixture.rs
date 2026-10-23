use crate::support::common::f32_bits;
use horizon_world::{
    combat_rules::ArmorWorld,
    damage_reduction::{self, Bypass, DamageReductionWorld},
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
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    changed: bool,
    slots: &'a [Value],
}
impl World<'_> {
    fn state(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn query(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected damage reduction failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
        }
        Ok(())
    }
}
impl ArmorWorld for World<'_> {
    type Error = io::Error;
    type Weapon = ();
    type ServerLevel = ();
    fn weapon(&mut self) -> io::Result<Option<()>> {
        self.query("weapon")?;
        Ok(self.state()["weapon"].as_bool().unwrap().then_some(()))
    }
    fn server_level(&mut self) -> io::Result<Option<()>> {
        self.query("level")?;
        Ok(self.state()["server"].as_bool().unwrap().then_some(()))
    }
    fn modify_armor_effectiveness(&mut self, _: (), _: (), v: f32) -> io::Result<f32> {
        Ok(v)
    }
}
impl DamageReductionWorld for World<'_> {
    fn bypasses(&mut self, b: Bypass) -> io::Result<bool> {
        let n = match b {
            Bypass::Armor => "armor",
            Bypass::Effects => "effects",
            Bypass::Resistance => "resistance",
            Bypass::Enchantments => "enchantments",
        };
        self.query(&format!("bypass {n}"))?;
        Ok(self.state()[format!("bypass_{n}")].as_bool().unwrap())
    }
    fn hurt_armor(&mut self, v: f32) -> io::Result<()> {
        self.query(&format!("hurt-armor {:08x}", f32_bits(v)))
    }
    fn armor_value(&mut self) -> io::Result<i32> {
        self.query("armor")?;
        Ok(damage_reduction::armor_from_attribute(number(
            &self.state()["armor"],
        )))
    }
    fn toughness_attribute(&mut self) -> io::Result<f64> {
        self.query("toughness")?;
        Ok(number(&self.state()["toughness"]))
    }
    fn has_resistance(&mut self) -> io::Result<bool> {
        self.query("has-resistance")?;
        Ok(self.state()["resistance"].as_bool().unwrap())
    }
    fn resistance_amplifier(&mut self) -> io::Result<i32> {
        self.query("effect")?;
        Ok(self.state()["amplifier"].as_i64().unwrap() as i32)
    }
    fn victim_is_server_player(&self) -> bool {
        self.f["player"].as_bool().unwrap()
    }
    fn award_resisted(&mut self, v: i32) -> io::Result<()> {
        self.query(&format!("stat self resisted {v}"))
    }
    fn attacker_is_server_player(&mut self) -> io::Result<bool> {
        self.query("attacker")?;
        Ok(self.f["attacker"] == 2)
    }
    fn award_dealt_resisted(&mut self, v: i32) -> io::Result<()> {
        self.query("attacker")?;
        self.query(&format!("stat attacker dealt-resisted {v}"))
    }
    fn damage_protection(&mut self, _: ()) -> io::Result<f32> {
        for i in 0..self.slots.len() {
            let slot = self.slots[i].as_i64().unwrap();
            self.query(&format!("slot {slot}"))?;
        }
        Ok(0.0)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let slots: Value = serde_json::from_str(&fs::read_to_string(dir.join("slots.json"))?)?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            changed: false,
            slots: slots.as_array().unwrap(),
        };
        let mode = f["mode"].as_str().unwrap();
        let result = if mode == "armor-value" {
            w.armor_value().and_then(|v| writeln!(w.out, "int {v}"))
        } else {
            let d = single(&f["damage"]);
            let r = if mode == "armor" {
                damage_reduction::after_armor(&mut w, d)
            } else {
                damage_reduction::after_magic(&mut w, d)
            };
            r.and_then(|v| writeln!(w.out, "float {:08x}", f32_bits(v)))
        };
        writeln!(
            w.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
