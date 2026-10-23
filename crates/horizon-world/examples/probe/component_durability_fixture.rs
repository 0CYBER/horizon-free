use horizon_world::{
    component_durability::{
        Change, ComponentDurability, DurabilityPatch, DurabilityPrototype, Key,
    },
    component_patch::MissingPrototype,
    item_durability::{self, DurabilityWorld},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn changes(v: &Value) -> [Change; 3] {
    [
        Change::MaxDamage(v["max"].as_i64().map(|v| v as i32)),
        Change::Damage(v["damage"].as_i64().map(|v| v as i32)),
        Change::Unbreakable(v["unbreakable"].as_bool().unwrap()),
    ]
}
fn reset(s: &mut ComponentDurability, v: &Value) -> io::Result<()> {
    s.count = v["count"].as_i64().unwrap() as i32;
    for change in changes(v) {
        match change {
            Change::MaxDamage(None) => {
                s.remove(Key::MaxDamage);
            }
            Change::Damage(None) => {
                s.remove(Key::Damage);
            }
            Change::Unbreakable(false) => {
                s.remove(Key::Unbreakable);
            }
            _ => {
                s.set(change).map_err(io::Error::other)?;
            }
        }
    }
    Ok(())
}
fn patch(v: &Value) -> DurabilityPatch {
    DurabilityPatch::new(
        v.as_array()
            .unwrap()
            .iter()
            .map(|v| match v[0].as_u64().unwrap() {
                0 => Change::MaxDamage(v[1].as_i64().map(|v| v as i32)),
                1 => Change::Damage(v[1].as_i64().map(|v| v as i32)),
                _ => Change::Unbreakable(!v[1].is_null()),
            }),
    )
}
fn patch_text(s: &ComponentDurability) -> String {
    s.as_patch()
        .changes()
        .map(|c| match c {
            Change::MaxDamage(v) => format!("0={}", optional(v)),
            Change::Damage(v) => format!("1={}", optional(v)),
            Change::Unbreakable(v) => format!("2={}", if v { "0" } else { "none" }),
        })
        .collect::<Vec<_>>()
        .join(",")
}
fn optional(v: Option<i32>) -> String {
    v.map_or("none".into(), |v| v.to_string())
}
fn snapshot(s: &ComponentDurability) -> String {
    format!(
        "{} {} {} {} {} {} {} {} {} {} {} {} | {}",
        s.count,
        s.count(),
        optional(s.raw_component(Key::MaxDamage)),
        optional(s.raw_component(Key::Damage)),
        s.raw_component(Key::Unbreakable).is_some(),
        s.max_damage(),
        s.damage(),
        s.is_empty(),
        s.is_damageable(),
        s.is_damaged(),
        s.is_broken(),
        s.next_damage_will_break(),
        patch_text(s)
    )
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    changed: bool,
}
impl World<'_> {
    fn event(&mut self, s: &mut ComponentDurability, name: &str) -> io::Result<()> {
        writeln!(self.out, "{name} | {}", snapshot(s))?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected callback failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
            reset(s, &self.f["after"])?;
        }
        Ok(())
    }
}
impl DurabilityWorld for World<'_> {
    type Error = io::Error;
    type Stack = ComponentDurability;
    fn storage_error(error: MissingPrototype) -> Self::Error {
        io::Error::other(error)
    }
    fn infinite_materials(&mut self, s: &mut ComponentDurability) -> io::Result<bool> {
        self.event(s, "infinite")?;
        Ok(self.f[if self.changed {
            "after_infinite"
        } else {
            "infinite"
        }]
        .as_bool()
        .unwrap())
    }
    fn process_enchantments(
        &mut self,
        _: &mut ComponentDurability,
        amount: i32,
    ) -> io::Result<i32> {
        Ok(item_durability::unenchanted_change(amount))
    }
    fn durability_changed(&mut self, s: &mut ComponentDurability, _: i32) -> io::Result<()> {
        self.event(s, "criterion")
    }
    fn broken(&mut self, s: &mut ComponentDurability, copy: ComponentDurability) -> io::Result<()> {
        writeln!(self.out, "copy {}", snapshot(&copy))?;
        self.event(s, "break")?;
        writeln!(self.out, "copy-after {}", snapshot(&copy))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let prototype = DurabilityPrototype::new(changes(&f["prototype"]));
        let mut s = ComponentDurability::new(&prototype, 1, f["air"].as_bool().unwrap());
        reset(&mut s, &f["initial"])?;
        if let Some(v) = f.get("restore") {
            s.restore_patch(&patch(v));
        }
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            changed: false,
        };
        let amount = f["amount"].as_i64().unwrap() as i32;
        let result = match f["mode"].as_str().unwrap() {
            "hurt" => item_durability::hurt_and_break(
                &mut s,
                &mut w,
                amount,
                f["player"].as_bool().unwrap(),
            ),
            "without" => w
                .event(&mut s, "level")
                .and_then(|()| item_durability::hurt_without_breaking(&mut s, &mut w, amount)),
            "set" => s.set_damage(amount).map_err(io::Error::other),
            "grow" => {
                s.grow(amount);
                Ok(())
            }
            "shrink" => {
                s.shrink(amount);
                Ok(())
            }
            "query" => Ok(()),
            _ => panic!("mode"),
        };
        writeln!(
            w.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        writeln!(w.out, "state {} {} {}", snapshot(&s), w.calls, w.changed)?;
    }
    Ok(())
}
