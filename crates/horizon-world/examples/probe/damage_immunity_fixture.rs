use horizon_world::damage_immunity::{self, DamageRule, ImmunityTag, ImmunityWorld};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
struct Env<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    slots: &'a [Value],
    calls: i64,
    changed: bool,
}
impl Env<'_> {
    fn flag(&self, n: &str) -> bool {
        self.f[if self.changed { "after" } else { "initial" }][n]
            .as_bool()
            .unwrap()
    }
    fn event(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected immunity callback failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
        }
        Ok(())
    }
    fn query(&mut self, n: &str, k: &str) -> io::Result<bool> {
        self.event(n)?;
        Ok(self.flag(k))
    }
}
impl ImmunityWorld for Env<'_> {
    type Error = io::Error;
    fn removed(&self) -> bool {
        self.f["initial"]["removed"].as_bool().unwrap()
    }
    fn permanently_invulnerable(&mut self) -> io::Result<bool> {
        self.query("permanent", "permanent")
    }
    fn temporarily_invulnerable(&mut self) -> io::Result<bool> {
        self.query("temporary", "temporary")
    }
    fn tag(&mut self, t: ImmunityTag) -> io::Result<bool> {
        let n = match t {
            ImmunityTag::BypassesInvulnerability => "bypass",
            ImmunityTag::Fire => "fire",
            ImmunityTag::Fall => "fall",
            ImmunityTag::Drowning => "drowning",
            ImmunityTag::Freezing => "freezing",
        };
        self.query(&format!("tag {n}"), n)
    }
    fn creative_source(&mut self) -> io::Result<bool> {
        self.query("creative", "creative")
    }
    fn fire_immune(&mut self) -> io::Result<bool> {
        self.query("fire-immune", "fire_immune")
    }
    fn fall_immune(&mut self) -> io::Result<bool> {
        self.query("fall-immune", "fall_immune")
    }
    fn enchantment_immunity(&mut self) -> io::Result<bool> {
        for s in self.slots {
            self.event(&format!("slot {}", s.as_u64().unwrap()))?;
        }
        Ok(false)
    }
    fn damage_rule(&mut self, r: DamageRule) -> io::Result<bool> {
        let n = match r {
            DamageRule::Drowning => "drowning",
            DamageRule::Fall => "fall",
            DamageRule::Fire => "fire",
            DamageRule::Freeze => "freeze",
        };
        self.event("rules")?;
        self.query(&format!("rule {n}"), &format!("rule_{n}"))
    }
    fn changing_dimension(&mut self) -> io::Result<bool> {
        self.query("changing", "changing")
    }
    fn ender_pearl(&mut self) -> io::Result<bool> {
        self.query("pearl", "pearl")
    }
    fn client_loaded(&mut self) -> io::Result<bool> {
        self.query("loaded", "loaded")
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let slots: Value = serde_json::from_str(&fs::read_to_string(dir.join("slots.json"))?)?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut e = Env {
            f: &f,
            out: &mut out,
            slots: slots.as_array().unwrap(),
            calls: 0,
            changed: false,
        };
        let result = match f["mode"].as_str().unwrap() {
            "entity" => damage_immunity::entity_invulnerable(&mut e),
            "base" => damage_immunity::base_immunity(&mut e),
            "living" => damage_immunity::living_immunity(&mut e),
            "player" => damage_immunity::player_immunity(&mut e),
            "server" => damage_immunity::server_player_immunity(&mut e),
            _ => panic!("mode"),
        };
        match result {
            Ok(v) => writeln!(e.out, "bool {v}\nresult ok")?,
            Err(_) => writeln!(e.out, "result error")?,
        };
        writeln!(e.out, "queries {} {}", e.calls, e.changed)?;
    }
    Ok(())
}
