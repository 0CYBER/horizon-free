use crate::support::common::f32_bits;
use horizon_world::{
    blocking_rules::{BlockingRules, ItemDamageFunction},
    item_use::{self, ItemUseWorld},
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
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    changed: bool,
    item: usize,
    remaining: i32,
    delays: [Option<i32>; 2],
}
impl World<'_> {
    fn state(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn reset(&mut self) {
        self.item = self.state()["item"].as_u64().unwrap() as usize;
        self.remaining = self.state()["remaining"].as_i64().unwrap() as i32;
    }
    fn event(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected use callback failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
            self.reset();
        }
        Ok(())
    }
}
impl ItemUseWorld for World<'_> {
    type Error = io::Error;
    type Item = usize;
    type ItemType = usize;
    fn using_item(&mut self) -> io::Result<bool> {
        self.event("using")?;
        Ok(self.state()["using"].as_bool().unwrap())
    }
    fn use_item(&self) -> usize {
        self.item
    }
    fn blocking_delay(&self, item: usize) -> Option<i32> {
        self.delays.get(item).copied().flatten()
    }
    fn item_type(&self, item: usize) -> usize {
        if item == 1 && self.f["shared_type"].as_bool().unwrap() {
            0
        } else {
            item
        }
    }
    fn use_duration(&mut self, kind: usize, item: usize) -> io::Result<i32> {
        if kind == 2 {
            return Ok(0);
        }
        self.event(&format!("duration {kind} {item}"))?;
        Ok(self.state()["durations"][kind].as_i64().unwrap() as i32)
    }
    fn remaining_ticks(&self) -> i32 {
        self.remaining
    }
    fn remaining_ticks_virtual(&mut self) -> io::Result<i32> {
        self.event("remaining")?;
        Ok(self.state()["virtual_remaining"].as_i64().unwrap() as i32)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let delays = std::array::from_fn(|i| {
            let c = &f["components"][i];
            c["present"].as_bool().unwrap().then(|| {
                BlockingRules::<()> {
                    delay_seconds: single(&c["delay"]),
                    disable_scale: 1.0,
                    reductions: vec![],
                    item_damage: ItemDamageFunction {
                        threshold: 0.0,
                        base: 0.0,
                        factor: 0.0,
                    },
                }
                .delay_ticks()
            })
        });
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            changed: false,
            item: 0,
            remaining: 0,
            delays,
        };
        w.reset();
        let result = match f["mode"].as_str().unwrap() {
            "blocking" => item_use::blocking_item(&mut w)
                .and_then(|v| writeln!(w.out, "item {}", v.map_or(-1, |v| v as i32))),
            "is-blocking" => {
                item_use::is_blocking(&mut w).and_then(|v| writeln!(w.out, "bool {v}"))
            }
            "ticks" => {
                item_use::ticks_using_item(&mut w).and_then(|v| writeln!(w.out, "ticks {v}"))
            }
            "partial" => item_use::ticks_using_item_partial(&mut w, single(&f["partial"]))
                .and_then(|v| writeln!(w.out, "float {:08x}", f32_bits(v))),
            _ => panic!("mode"),
        };
        writeln!(
            w.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        writeln!(
            w.out,
            "state {} {} {} {}",
            w.item, w.remaining, w.calls, w.changed
        )?;
    }
    Ok(())
}
