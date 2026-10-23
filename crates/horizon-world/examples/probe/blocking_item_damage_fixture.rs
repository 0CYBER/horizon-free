use horizon_world::item_durability::{self, DurabilityStack, DurabilityWorld};
use horizon_world::{
    blocking_item_damage::{self, BlockingItemDamageWorld},
    blocking_rules::ItemDamageFunction,
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn stack(v: &Value, air: bool) -> DurabilityStack {
    DurabilityStack {
        count: v["count"].as_i64().unwrap() as i32,
        air,
        max_damage: v["max"].as_i64().map(|v| v as i32),
        damage: v["damage"].as_i64().map(|v| v as i32),
        unbreakable: v["unbreakable"].as_bool().unwrap(),
    }
}
fn optional(v: Option<i32>) -> String {
    v.map_or("none".into(), |v| v.to_string())
}
fn snapshot(s: &DurabilityStack) -> String {
    format!(
        "{} {} {} {} {} {} {} {} {} {} {} {}",
        s.count,
        s.count(),
        optional(s.max_damage),
        optional(s.damage),
        s.unbreakable,
        s.max_damage(),
        s.damage(),
        s.is_empty(),
        s.is_damageable(),
        s.is_damaged(),
        s.is_broken(),
        s.next_damage_will_break()
    )
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    changed: bool,
}
impl World<'_> {
    fn event(&mut self, s: &mut DurabilityStack, name: &str) -> io::Result<()> {
        writeln!(self.out, "{name} | {}", snapshot(s))?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected callback failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
            *s = stack(&self.f["after"], s.air);
        }
        Ok(())
    }
}
impl DurabilityWorld for World<'_> {
    type Error = io::Error;
    type Stack = DurabilityStack;
    fn storage_error(error: std::convert::Infallible) -> Self::Error {
        match error {}
    }
    fn infinite_materials(&mut self, s: &mut DurabilityStack) -> io::Result<bool> {
        self.event(s, "infinite")?;
        Ok(self.f[if self.changed {
            "after_infinite"
        } else {
            "infinite"
        }]
        .as_bool()
        .unwrap())
    }
    fn process_enchantments(&mut self, _: &mut DurabilityStack, amount: i32) -> io::Result<i32> {
        Ok(item_durability::unenchanted_change(amount))
    }
    fn durability_changed(&mut self, s: &mut DurabilityStack, _: i32) -> io::Result<()> {
        self.event(s, "criterion")
    }
    fn broken(&mut self, s: &mut DurabilityStack, copy: DurabilityStack) -> io::Result<()> {
        writeln!(self.out, "copy {}", snapshot(&copy))?;
        self.event(s, &format!("break {}", self.f["hand"].as_u64().unwrap()))
    }
}
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
impl World<'_> {
    fn flag(&self, name: &str) -> bool {
        self.f[format!("{}{name}", if self.changed { "after_" } else { "" })]
            .as_bool()
            .unwrap()
    }
}
impl BlockingItemDamageWorld for World<'_> {
    type Level = ();
    fn is_player(&self) -> bool {
        self.f["kind"].as_u64().unwrap() > 0
    }
    fn is_server_player(&self) -> bool {
        self.f["kind"].as_u64().unwrap() == 2
    }
    fn level_is_client(&mut self, s: &mut DurabilityStack, _: ()) -> io::Result<bool> {
        self.event(s, "client")?;
        Ok(self.flag("client"))
    }
    fn award_item_used(&mut self, s: &mut DurabilityStack) -> io::Result<()> {
        let kind = if s.is_empty() { 0 } else { 1 };
        self.event(s, &format!("stat {kind}"))
    }
    fn entity_level_is_server(&mut self, s: &mut DurabilityStack) -> io::Result<bool> {
        self.event(s, "level")?;
        Ok(self.flag("server"))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut s = stack(&f["initial"], f["air"].as_bool().unwrap());
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            changed: false,
        };
        let r = &f["rule"];
        let rule = ItemDamageFunction {
            threshold: single(&r["threshold"]),
            base: single(&r["base"]),
            factor: single(&r["factor"]),
        };
        let result = blocking_item_damage::hurt(&mut s, &mut w, (), single(&f["damage"]), rule);
        writeln!(
            w.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        writeln!(w.out, "state {} {} {}", snapshot(&s), w.calls, w.changed)?;
    }
    Ok(())
}
