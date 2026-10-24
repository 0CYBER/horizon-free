use crate::support::common::f32_bits;
use horizon_core::geometry::Vec3;
use horizon_world::{
    blocking_rules::{BlockingRules, DamageReduction, ItemDamageFunction, TypeMatchWorld},
    item_blocking::{self, ItemBlockingWorld},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn vec(v: &Value) -> Option<Vec3> {
    (!v.is_null()).then(|| Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])))
}
struct Component {
    present: bool,
    bypass: Option<u64>,
    rules: Arc<BlockingRules<u64>>,
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    components: Vec<Component>,
    calls: i64,
    changed: bool,
}
impl World<'_> {
    fn state(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn event(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected blocking callback failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
        }
        Ok(())
    }
}
impl TypeMatchWorld for World<'_> {
    type Error = io::Error;
    type Set = u64;
    fn source_in_set(&mut self, set: u64) -> io::Result<bool> {
        self.event("type-holder")?;
        let id = self.state()["type"].as_u64().unwrap();
        Ok(match set {
            1 => id == 0,
            2 => id == 1,
            3 => true,
            4 => false,
            _ => panic!("set"),
        })
    }
}
impl ItemBlockingWorld for World<'_> {
    type Error = io::Error;
    type Item = usize;
    type Component = usize;
    type Entity = u64;
    type Level = u64;
    type Hand = u64;
    fn blocking_item(&mut self) -> io::Result<Option<usize>> {
        self.event("blocking-item")?;
        let id = self.state()["item"].as_i64().unwrap();
        Ok((id >= 0).then_some(id as usize))
    }
    fn component(&mut self, item: usize) -> Option<usize> {
        self.components[item].present.then_some(item)
    }
    fn bypasses_component(&mut self, c: usize) -> io::Result<bool> {
        match self.components[c].bypass {
            Some(set) => self.source_in_set(set),
            None => Ok(false),
        }
    }
    fn direct_entity(&mut self) -> io::Result<Option<u64>> {
        self.event("direct")?;
        let id = self.state()["direct"].as_u64().unwrap();
        Ok((id != 0).then_some(id))
    }
    fn is_arrow(&self, e: u64) -> bool {
        e == 2
    }
    fn pierce_level(&mut self, _: u64) -> io::Result<i8> {
        self.event("pierce")?;
        Ok(self.state()["pierce"].as_i64().unwrap() as i8)
    }
    fn source_position(&mut self) -> io::Result<Option<Vec3>> {
        self.event("source-position")?;
        Ok(vec(&self.state()["source"]))
    }
    fn head_yaw(&mut self) -> io::Result<f32> {
        self.event("yaw")?;
        Ok(single(&self.state()["yaw"]))
    }
    fn position(&mut self) -> io::Result<Vec3> {
        self.event("position")?;
        Ok(vec(&self.state()["position"]).unwrap())
    }
    fn resolve(&mut self, c: usize, damage: f32, angle: f64) -> io::Result<f32> {
        let rules = Arc::clone(&self.components[c].rules);
        rules.resolve(self, damage, angle)
    }
    fn level(&mut self) -> io::Result<u64> {
        self.event("level")?;
        Ok(self.state()["level"].as_u64().unwrap())
    }
    fn used_hand(&mut self) -> io::Result<u64> {
        self.event("hand")?;
        Ok(self.state()["hand"].as_u64().unwrap())
    }
    fn hurt_blocking_item(
        &mut self,
        c: usize,
        level: u64,
        item: usize,
        _: u64,
        blocked: f32,
    ) -> io::Result<()> {
        if self.f["player"].as_bool().unwrap() {
            self.event(&format!("client {level}"))?;
            if !self.state()["client"].as_bool().unwrap() {
                self.event(&format!("stat {item}"))?;
            }
            if self.components[c].rules.item_damage.apply(blocked) > 0 {
                self.event("level")?;
            }
        }
        Ok(())
    }
    fn source_is_projectile(&mut self) -> io::Result<bool> {
        self.event("projectile")?;
        Ok(self.state()["projectile"].as_bool().unwrap())
    }
    fn is_living(&self, e: u64) -> bool {
        e == 1
    }
    fn block_using_item(&mut self, _: u64, damage: f32, fully: bool) -> io::Result<()> {
        self.event(&format!("block-using 0 {:08x} {fully}", f32_bits(damage)))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let components = f["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                let item = &c["item_damage"];
                let bypass = c["bypass"].as_u64().unwrap();
                Component {
                    present: c["present"].as_bool().unwrap(),
                    bypass: (bypass != 0).then_some(bypass),
                    rules: Arc::new(BlockingRules {
                        delay_seconds: 0.0,
                        disable_scale: 1.0,
                        item_damage: ItemDamageFunction {
                            threshold: single(&item["threshold"]),
                            base: single(&item["base"]),
                            factor: single(&item["factor"]),
                        },
                        reductions: c["reductions"]
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
                    }),
                }
            })
            .collect();
        let mut w = World {
            f: &f,
            out: &mut out,
            components,
            calls: 0,
            changed: false,
        };
        match item_blocking::apply(&mut w, single(&f["damage"])) {
            Ok(v) => writeln!(w.out, "blocked {:08x}\nresult ok", f32_bits(v))?,
            Err(_) => writeln!(w.out, "result error")?,
        };
        writeln!(w.out, "queries {} {}", w.calls, w.changed)?;
        writeln!(w.out, "items 1 0 1 0")?;
    }
    Ok(())
}
