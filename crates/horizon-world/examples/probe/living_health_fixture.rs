use crate::support::common::f32_bits;
use horizon_world::living_health::{self, HealthWorld, SyncedFloat};
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
struct Context<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    changed: bool,
    active: usize,
}
impl Context<'_> {
    fn query(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected health callback failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
            self.active = 1;
        }
        Ok(())
    }
    fn world(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "world" }]
    }
}
struct World<'a> {
    ctx: Context<'a>,
    stores: [SyncedFloat; 2],
    absorption: f32,
}
impl HealthWorld for World<'_> {
    type Error = io::Error;
    type Data = usize;
    fn health(&mut self) -> io::Result<f32> {
        self.ctx.query("health")?;
        Ok(self.stores[self.ctx.active].value())
    }
    fn health_data(&self) -> usize {
        self.ctx.active
    }
    fn max_health_attribute(&mut self) -> io::Result<f64> {
        self.ctx.query("max-health")?;
        Ok(number(&self.ctx.world()["max-health"]))
    }
    fn max_absorption_attribute(&mut self) -> io::Result<f64> {
        self.ctx.query("max-absorption")?;
        Ok(number(&self.ctx.world()["max-absorption"]))
    }
    fn store_health(&mut self, data: usize, v: f32) -> io::Result<()> {
        self.stores[data].set(v, false, |_| self.ctx.query("updated"))
    }
    fn store_absorption(&mut self, v: f32) -> io::Result<()> {
        self.ctx.query(&format!("absorption {:08x}", f32_bits(v)))?;
        self.absorption = v;
        Ok(())
    }
    fn dead_flag(&self) -> bool {
        self.ctx.world()["dead"].as_bool().unwrap()
    }
    fn dispatch_set_health(&mut self, v: f32) -> io::Result<()> {
        self.ctx.query(&format!("set {:08x}", f32_bits(v)))?;
        living_health::set_health(self, v)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut w = World {
            ctx: Context {
                f: &f,
                out: &mut out,
                calls: 0,
                changed: false,
                active: 0,
            },
            stores: [
                SyncedFloat::new(single(&f["health"][0])),
                SyncedFloat::new(single(&f["health"][1])),
            ],
            absorption: single(&f["absorption"]),
        };
        for op in f["ops"].as_array().unwrap() {
            let v = single(&op["value"]);
            let result = match op["mode"].as_str().unwrap() {
                "heal" => living_health::heal(&mut w, v),
                "set" => w.dispatch_set_health(v),
                "base" => living_health::set_health(&mut w, v),
                "absorption" => living_health::set_absorption(&mut w, v),
                "hurt" => {
                    living_health::is_hurt(&mut w).and_then(|v| writeln!(w.ctx.out, "bool {v}"))
                }
                "dead" => living_health::is_dead_or_dying(&mut w)
                    .and_then(|v| writeln!(w.ctx.out, "bool {v}")),
                "sync" => w.stores[w.ctx.active].set(v, op["force"].as_bool().unwrap(), |_| {
                    w.ctx.query("updated")
                }),
                "pack" => {
                    writeln!(
                        w.ctx.out,
                        "packed {}",
                        w.stores[w.ctx.active]
                            .take_dirty()
                            .map(|v| format!("{:08x}", f32_bits(v)))
                            .unwrap_or_else(|| "none".into())
                    )
                }
                _ => panic!("mode"),
            };
            writeln!(
                w.ctx.out,
                "result {}",
                if result.is_ok() { "ok" } else { "error" }
            )?;
            for (i, s) in w.stores.iter().enumerate() {
                writeln!(
                    w.ctx.out,
                    "data {i} {:08x} {} {} {}",
                    f32_bits(s.value()),
                    s.is_dirty(),
                    s.item_dirty(),
                    s.is_default()
                )?;
            }
            let dead = w.dead_flag();
            writeln!(
                w.ctx.out,
                "state {} {:08x} {dead} {}",
                w.ctx.active,
                f32_bits(w.absorption),
                w.ctx.calls
            )?;
        }
    }
    Ok(())
}
