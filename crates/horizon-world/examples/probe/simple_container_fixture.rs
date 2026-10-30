use horizon_world::{
    component_patch::{Patch, Prototype},
    item_stack_refs::{ItemStacks, StackRef},
    simple_container::{ContainerHooks, SimpleContainer},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
type Stack = StackRef<u8, usize, i32>;
type Container = SimpleContainer<u8, usize, i32>;
fn integer(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
fn opt(v: Option<i32>) -> String {
    v.map_or("none".into(), |v| v.to_string())
}
fn max(s: &Stack) -> i32 {
    s.read().component(&0).copied().unwrap_or(1)
}
struct World {
    out: io::BufWriter<fs::File>,
    refs: Vec<Stack>,
    ids: Vec<Stack>,
    empty: Stack,
    op: Value,
    calls: i32,
}
impl World {
    fn snapshot(&mut self, s: &Stack) -> String {
        let id = self
            .ids
            .iter()
            .position(|v| v.same_reference(s))
            .unwrap_or_else(|| {
                self.ids.push(s.clone());
                self.ids.len() - 1
            });
        let s = s.read();
        let patch = s.components.as_patch();
        format!(
            "{} {} {} {} {} {} {} | {} | {}",
            id,
            s.item(),
            s.count,
            s.count(),
            s.pop_time,
            s.is_empty(),
            s.component(&0).copied().unwrap_or(1),
            (0..3)
                .map(|k| opt(s.component(&k).copied()))
                .collect::<Vec<_>>()
                .join(","),
            patch
                .entries()
                .iter()
                .map(|(k, v)| format!("{k}={}", opt(*v)))
                .collect::<Vec<_>>()
                .join(",")
        )
    }
    fn all(&mut self, c: &Container) {
        for i in 0..self.refs.len() {
            let s = self.snapshot(&self.refs[i].clone());
            writeln!(self.out, "ref {i} {s}").unwrap();
        }
        for (i, s) in c.items().iter().enumerate() {
            let s = self.snapshot(s);
            writeln!(self.out, "slot {i} {s}").unwrap();
        }
        let s = self.snapshot(&self.empty.clone());
        writeln!(self.out, "empty {s}").unwrap();
        writeln!(
            self.out,
            "container {} {} {} {} {}",
            c.size(),
            c.is_empty(),
            c.count_item(0),
            c.count_item(1),
            c.count_item(2)
        )
        .unwrap();
    }
    fn event(&mut self, c: &mut Container, name: &str) -> Result<(), ()> {
        self.calls += 1;
        writeln!(self.out, "event {name} {}", self.calls).unwrap();
        self.all(c);
        if self.calls == integer(&self.op, "fail_at") {
            return Err(());
        }
        if self.calls == integer(&self.op, "mutate_at") {
            let i = integer(&self.op, "mutate_ref") as usize;
            let mut s = self.refs[i].write();
            s.count = integer(&self.op, "after_count");
            s.components
                .set(0, Some(integer(&self.op, "after_max")))
                .map_err(|_| ())?;
            drop(s);
            let slot = integer(&self.op, "mutate_slot");
            if slot >= 0 && (slot as usize) < c.size() {
                c.items_mut()[slot as usize] =
                    self.refs[integer(&self.op, "replace_ref") as usize].clone();
            }
        }
        Ok(())
    }
}
impl ContainerHooks<u8, usize, i32> for World {
    type Error = ();
    fn max_stack_size(&mut self, c: &mut Container) -> Result<i32, ()> {
        self.event(c, "max")?;
        Ok(integer(&self.op, "max"))
    }
    fn item_max_stack_size(&self, s: &Stack) -> i32 {
        max(s)
    }
    fn changed(&mut self, c: &mut Container) -> Result<(), ()> {
        self.event(c, "changed")
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let prototypes: Vec<_> = f["prototypes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                Prototype::new(p.as_array().unwrap().iter().map(|v| {
                    (
                        v[0].as_u64().unwrap() as usize,
                        v[1].as_i64().unwrap() as i32,
                    )
                }))
            })
            .collect();
        let stacks = ItemStacks::new(0);
        let empty = stacks.empty();
        let refs: Vec<_> = f["stacks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let item = integer(v, "item");
                if item < 0 {
                    empty.clone()
                } else {
                    let s = stacks.create(
                        item as u8,
                        integer(v, "count"),
                        prototypes[item as usize].clone(),
                    );
                    s.write().pop_time = integer(v, "pop");
                    s.write().components.restore_patch(&Patch::new(
                        v["patch"].as_array().unwrap().iter().map(|v| {
                            (
                                v[0].as_u64().unwrap() as usize,
                                v[1].as_i64().map(|n| n as i32),
                            )
                        }),
                    ));
                    s
                }
            })
            .collect();
        let mut c = Container::from_items(
            f["slots"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| refs[i.as_u64().unwrap() as usize].clone())
                .collect(),
            stacks.clone(),
        );
        let mut w = World {
            out,
            refs,
            ids: vec![empty.clone()],
            empty,
            op: Value::Null,
            calls: 0,
        };
        w.all(&c);
        for op in f["ops"].as_array().unwrap() {
            w.op = op.clone();
            w.calls = 0;
            let a = integer(op, "a") as usize;
            let b = integer(op, "b") as usize;
            let slot = integer(op, "slot");
            let amount = integer(op, "amount");
            let result = (|| -> Result<(), ()> {
                match op["kind"].as_str().unwrap() {
                    "set" => c.set(slot, w.refs[a].clone(), &mut w).map_err(|_| ())?,
                    "add" => {
                        let input = w.refs[a].clone();
                        w.refs[b] = c.add(&input, &mut w).map_err(|_| ())?;
                    }
                    "remove" => w.refs[b] = c.remove(slot, amount, &mut w).map_err(|_| ())?,
                    "remove-type" => {
                        let i = integer(op, "item") as usize;
                        w.refs[b] = c
                            .remove_type(i as u8, prototypes[i].clone(), amount, &mut w)
                            .map_err(|_| ())?;
                    }
                    "no-update" => w.refs[b] = c.remove_no_update::<()>(slot).map_err(|_| ())?,
                    "clear" => c.clear(&mut w).map_err(|_| ())?,
                    "all" => {
                        let result = c.remove_all(&mut w).map_err(|_| ())?;
                        writeln!(w.out, "all-size {}", result.len()).unwrap();
                        for s in result {
                            let s = w.snapshot(&s);
                            writeln!(w.out, "removed {s}").unwrap();
                        }
                    }
                    "can-add" => {
                        let v = c.can_add(&w.refs[a], &w);
                        writeln!(w.out, "can-add {v}").unwrap();
                    }
                    "get" => w.refs[b] = c.get(slot),
                    "alias" => w.refs[b] = w.refs[a].clone(),
                    "copy" => w.refs[b] = stacks.copy(&w.refs[a]),
                    "count" => w.refs[a].write().count = amount,
                    "component" => {
                        w.refs[a]
                            .write()
                            .components
                            .set(
                                integer(op, "key") as usize,
                                op["value"].as_i64().map(|n| n as i32),
                            )
                            .map_err(|_| ())?;
                    }
                    _ => panic!("operation"),
                }
                Ok(())
            })();
            writeln!(
                w.out,
                "result {} {}",
                if result.is_ok() { "ok" } else { "error" },
                w.calls
            )?;
            w.all(&c);
        }
        out = w.out;
    }
    Ok(())
}
