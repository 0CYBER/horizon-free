use crate::support::common::f64_hex;
use horizon_world::{
    attributes::{AttributeError, Definition, Instance, Modifier, Observer, Operation},
    effect_attribute_modifiers::{self as effects, Definitions, Error, ModifierWorld, Template},
    object_open_set::{CursorError, ObjectKey},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone, PartialEq, Eq)]
struct Key {
    id: usize,
    hash: i32,
}
impl ObjectKey for Key {
    fn object_hash(&self) -> i32 {
        self.hash
    }
}
#[derive(Debug)]
enum Stop {
    Callback,
    Duplicate,
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn desc(m: &Modifier) -> String {
    format!("{}:{}:{}", m.id(), f64_hex(m.amount()), m.operation() as u8)
}
fn key_desc(k: Option<&Key>) -> String {
    k.map_or("null".into(), |k| k.id.to_string())
}
struct Shared {
    keys: Vec<Key>,
    templates: Vec<Template>,
    routes: Vec<i32>,
    pool: Vec<Modifier>,
    calls: i64,
    changed: bool,
    op: Value,
    logs: Vec<String>,
}
impl Shared {
    fn key(&self, op: &Value) -> Option<Key> {
        let k = op["key"].as_i64().unwrap();
        (k >= 0).then(|| self.keys[k as usize].clone())
    }
    fn definitions(&mut self, defs: &Definitions<Key>) {
        self.logs.push(format!(
            "definitions {} | {}",
            defs.len(),
            defs.entries()
                .iter()
                .map(|(k, v)| {
                    let v = v.as_ref().map_or("null".into(), |t| {
                        format!("{}:{}:{}", t.id(), f64_hex(t.amount()), t.operation() as u8)
                    });
                    format!("{}={v}", key_desc(k.as_ref()))
                })
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    fn raw(&mut self, defs: &mut Definitions<Key>, op: &Value) {
        match op["op"].as_str().unwrap() {
            "define" => effects::add_definition(
                defs,
                self.key(op),
                self.templates[op["template"].as_u64().unwrap() as usize].clone(),
            ),
            "null" => {
                defs.put(self.key(op), None);
            }
            "drop" => {
                defs.remove(self.key(op).as_ref());
            }
            "clear" => defs.clear(),
            "route" => {
                let k = self.key(op).map_or(self.keys.len(), |k| k.id);
                self.routes[k] = op["target"].as_i64().unwrap() as i32;
            }
            _ => panic!("raw operation"),
        }
    }
    fn event(&mut self, defs: &mut Definitions<Key>, name: String) -> Result<(), Stop> {
        let call = self.calls;
        self.calls += 1;
        self.logs.push(format!("event {call} {name}"));
        self.definitions(defs);
        if call == self.op["fail"].as_i64().unwrap() {
            return Err(Stop::Callback);
        }
        if call == self.op["mutate"].as_i64().unwrap() {
            self.changed = true;
            for action in self.op["mutations"].as_array().unwrap().clone() {
                self.raw(defs, &action);
            }
        }
        Ok(())
    }
    fn refs(&self, mods: &[Modifier]) -> String {
        mods.iter()
            .map(|m| {
                self.pool
                    .iter()
                    .position(|p| p.same_identity(m))
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(",")
    }
    fn state(&mut self, id: usize, a: &Instance) {
        self.logs.push(format!(
            "instance {id} {} {} {} | {} | {}",
            f64_hex(a.base_value()),
            a.is_dirty(),
            f64_hex(a.cached_value()),
            self.refs(a.modifiers()),
            self.refs(a.permanent_modifiers())
        ));
    }
    fn modifier(&mut self, m: Modifier) -> usize {
        let id = self.pool.len();
        self.pool.push(m);
        id
    }
}
struct Dirty<'a> {
    shared: &'a mut Shared,
    definitions: &'a mut Definitions<Key>,
    id: usize,
}
impl Observer for Dirty<'_> {
    type Error = Stop;
    fn dirty(&mut self, a: &mut Instance) -> Result<(), Stop> {
        self.shared.state(self.id, a);
        self.shared
            .event(self.definitions, format!("dirty {}", self.id))?;
        if self.shared.op["read"].as_bool().unwrap() {
            self.shared
                .logs
                .push(format!("observed {}", f64_hex(a.value())));
        }
        Ok(())
    }
}
fn attribute_error(e: AttributeError<Stop>) -> Stop {
    match e {
        AttributeError::DuplicateModifier(_) => Stop::Duplicate,
        AttributeError::Observer(e) => e,
    }
}
struct World {
    shared: Shared,
    instances: Vec<Instance>,
}
impl ModifierWorld<Key> for World {
    type Instance = usize;
    type Error = Stop;
    fn get_instance(
        &mut self,
        defs: &mut Definitions<Key>,
        key: Option<Key>,
    ) -> Result<Option<usize>, Stop> {
        self.shared
            .event(defs, format!("get {}", key_desc(key.as_ref())))?;
        let i = key.as_ref().map_or(self.shared.keys.len(), |k| k.id);
        let id = self.shared.routes[i];
        Ok((id >= 0).then_some(id as usize))
    }
    fn remove_modifier(
        &mut self,
        defs: &mut Definitions<Key>,
        id: &usize,
        modifier: &str,
    ) -> Result<(), Stop> {
        self.shared.event(defs, format!("remove {id} {modifier}"))?;
        let mut cb = Dirty {
            shared: &mut self.shared,
            definitions: defs,
            id: *id,
        };
        self.instances[*id]
            .remove(modifier, &mut cb)
            .map_err(attribute_error)?;
        Ok(())
    }
    fn add_permanent(
        &mut self,
        defs: &mut Definitions<Key>,
        id: &usize,
        modifier: Modifier,
    ) -> Result<(), Stop> {
        let m = self.shared.modifier(modifier.clone());
        self.shared
            .event(defs, format!("permanent {id} {m} {}", desc(&modifier)))?;
        let mut cb = Dirty {
            shared: &mut self.shared,
            definitions: defs,
            id: *id,
        };
        self.instances[*id]
            .add_permanent(modifier, &mut cb)
            .map_err(attribute_error)
    }
}
fn error(e: Error<Stop>) -> &'static str {
    match e {
        Error::MissingTemplate | Error::Cursor(CursorError::MissingWrapped) => "null",
        Error::Cursor(CursorError::IndexOutOfBounds) => "bounds",
        Error::Cursor(CursorError::NoSuchElement) => "empty",
        Error::Cursor(CursorError::NoCurrentEntry) => "current",
        Error::World(Stop::Callback) => "callback",
        Error::World(Stop::Duplicate) => "duplicate",
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let keys = f["hashes"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(id, h)| Key {
                id,
                hash: h.as_i64().unwrap() as i32,
            })
            .collect();
        let templates = f["templates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                Template::new(
                    v["id"].as_str().unwrap(),
                    number(&v["amount"]),
                    Operation::ALL[v["operation"].as_u64().unwrap() as usize],
                )
            })
            .collect::<io::Result<Vec<_>>>()?;
        let routes = f["routes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i.as_i64().unwrap() as i32)
            .collect();
        let instances = f["defaults"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Instance::new(Definition::basic(number(v))))
            .collect();
        let shared = Shared {
            keys,
            templates,
            routes,
            pool: Vec::new(),
            calls: 0,
            changed: false,
            op: Value::Null,
            logs: Vec::new(),
        };
        let mut world = World { shared, instances };
        let mut defs = Definitions::default();
        for op in f["setup"].as_array().unwrap() {
            world.shared.raw(&mut defs, op);
        }
        for op in f["ops"].as_array().unwrap() {
            world.shared.op = op.clone();
            world.shared.calls = 0;
            world.shared.changed = false;
            let amp = op["amplifier"].as_i64().unwrap() as i32;
            let result = match op["op"].as_str().unwrap() {
                "add" => effects::add_modifiers(&mut defs, &mut world, amp),
                "remove" => effects::remove_modifiers(&mut defs, &mut world),
                "create" => effects::create_modifiers(&mut defs, amp, |defs, key, modifier| {
                    let id = world.shared.modifier(modifier.clone());
                    world.shared.event(
                        defs,
                        format!(
                            "consume {} {id} {}",
                            key_desc(key.as_ref()),
                            desc(&modifier)
                        ),
                    )
                }),
                "seed" => {
                    let template =
                        &world.shared.templates[op["template"].as_u64().unwrap() as usize];
                    let modifier = template.create(amp);
                    world
                        .add_permanent(
                            &mut defs,
                            &(op["target"].as_u64().unwrap() as usize),
                            modifier,
                        )
                        .map_err(Error::World)
                }
                "value" => {
                    let id = op["target"].as_u64().unwrap() as usize;
                    world.shared.logs.push(format!(
                        "value {id} {}",
                        f64_hex(world.instances[id].value())
                    ));
                    Ok(())
                }
                _ => {
                    world.shared.raw(&mut defs, op);
                    Ok(())
                }
            };
            world
                .shared
                .logs
                .push(format!("result {}", result.map_or_else(error, |()| "ok")));
            world.shared.logs.push(format!(
                "calls {} {}",
                world.shared.calls, world.shared.changed
            ));
            world.shared.definitions(&defs);
            for (id, a) in world.instances.iter().enumerate() {
                world.shared.state(id, a);
            }
            for log in world.shared.logs.drain(..) {
                writeln!(out, "{log}")?;
            }
        }
    }
    Ok(())
}
