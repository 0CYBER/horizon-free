use crate::support::common::f32_bits;
use horizon_world::blocking_rules::{
    BlockingRules, DamageReduction, ItemDamageFunction, TypeMatchWorld,
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
}
impl TypeMatchWorld for World<'_> {
    type Error = io::Error;
    type Set = u64;
    fn source_in_set(&mut self, set: u64) -> io::Result<bool> {
        writeln!(self.out, "type-holder")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected holder query failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
        }
        let id = self.f[if self.changed { "after_type" } else { "type" }]
            .as_u64()
            .unwrap();
        Ok(match set {
            1 => id == 0,
            2 => id == 1,
            3 => true,
            4 => false,
            _ => panic!("set"),
        })
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let item = &f["item"];
        let rules = BlockingRules {
            delay_seconds: single(&f["delay"]),
            disable_scale: single(&f["disable_scale"]),
            item_damage: ItemDamageFunction {
                threshold: single(&item["threshold"]),
                base: single(&item["base"]),
                factor: single(&item["factor"]),
            },
            reductions: f["reductions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    let set = r["set"].as_u64().unwrap();
                    DamageReduction {
                        horizontal_angle: single(&r["angle"]),
                        types: (set != 0).then_some(set),
                        base: single(&r["base"]),
                        factor: single(&r["factor"]),
                    }
                })
                .collect(),
        };
        let damage = single(&f["damage"]);
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            changed: false,
        };
        writeln!(w.out, "delay {}", rules.delay_ticks())?;
        writeln!(
            w.out,
            "disable {}",
            rules.disable_ticks(single(&f["disable_seconds"]))
        )?;
        writeln!(w.out, "item {}", rules.item_damage.apply(damage))?;
        match rules.resolve(&mut w, damage, number(&f["angle"])) {
            Ok(v) => writeln!(w.out, "blocked {:08x}\nresult ok", f32_bits(v))?,
            Err(_) => writeln!(w.out, "result error")?,
        };
        writeln!(w.out, "queries {} {}", w.calls, w.changed)?;
    }
    Ok(())
}
