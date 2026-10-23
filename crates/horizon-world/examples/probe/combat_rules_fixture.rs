use crate::support::common::f32_bits;
use horizon_world::combat_rules::{self, ArmorWorld};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    changed: bool,
}
impl World<'_> {
    fn query(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected armor query failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
        }
        Ok(())
    }
    fn flag(&self, name: &str) -> bool {
        self.f[format!("{}{name}", if self.changed { "after_" } else { "" })]
            .as_bool()
            .unwrap()
    }
}
impl ArmorWorld for World<'_> {
    type Error = io::Error;
    type Weapon = ();
    type ServerLevel = ();
    fn weapon(&mut self) -> io::Result<Option<()>> {
        self.query("weapon")?;
        Ok(self.flag("weapon").then_some(()))
    }
    fn server_level(&mut self) -> io::Result<Option<()>> {
        self.query("level")?;
        Ok(self.flag("server").then_some(()))
    }
    fn modify_armor_effectiveness(&mut self, _: (), _: (), v: f32) -> io::Result<f32> {
        Ok(v)
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
        };
        let damage = single(&f["damage"]);
        let armor = single(&f["armor"]);
        let result = if f["mode"] == "magic" {
            Ok(combat_rules::after_magic(damage, armor))
        } else {
            combat_rules::after_armor(&mut w, damage, armor, single(&f["toughness"]))
        };
        match result {
            Ok(v) => {
                writeln!(w.out, "value {:08x}\nresult ok", f32_bits(v))?;
            }
            Err(_) => writeln!(w.out, "result error")?,
        }
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
