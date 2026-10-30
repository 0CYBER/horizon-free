use crate::support::common::f64_hex;
use horizon_world::{
    attribute_map::AttributeMap,
    attributes::{AttributeError, Definition, Instance, Modifier, Observer, Operation},
    effect_attribute_modifiers::{self as effects, Definitions, ModifierWorld, Template},
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
#[derive(Clone, PartialEq, Eq)]
struct Handle {
    id: usize,
    hash: i32,
}
impl ObjectKey for Handle {
    fn object_hash(&self) -> i32 {
        self.hash
    }
}
type Map = AttributeMap<Key, Handle>;
#[derive(Debug)]
enum Stop {
    Callback,
    Duplicate,
    Null,
    Bounds,
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn desc(k: Option<&Key>) -> String {
    k.map_or("null".into(), |k| k.id.to_string())
}
fn reference(h: Option<&Handle>) -> String {
    h.map_or("null".into(), |h| h.id.to_string())
}
fn attribute_error(e: AttributeError<Stop>) -> Stop {
    match e {
        AttributeError::DuplicateModifier(_) => Stop::Duplicate,
        AttributeError::Observer(e) => e,
    }
}
struct Silent;
impl Observer for Silent {
    type Error = Stop;
    fn dirty(&mut self, _: &mut Instance) -> Result<(), Stop> {
        Ok(())
    }
}
struct Node {
    key: Key,
    handle: Handle,
    value: Option<Instance>,
}
struct World {
    keys: Vec<Key>,
    defaults: Vec<f64>,
    syncable: Vec<bool>,
    suppliers: Vec<Option<Instance>>,
    templates: Vec<Template>,
    instances: Vec<Node>,
    hashes: Vec<i32>,
    pool: Vec<Modifier>,
    calls: i64,
    changed: bool,
    op: Value,
    logs: Vec<String>,
}
impl World {
    fn key(&self, op: &Value) -> Option<Key> {
        let id = op["key"].as_i64().unwrap();
        (id >= 0).then(|| self.keys[id as usize].clone())
    }
    fn handle(&self, id: i64) -> Result<Option<Handle>, Stop> {
        if id < 0 {
            Ok(None)
        } else {
            self.instances
                .get(id as usize)
                .map(|n| Some(n.handle.clone()))
                .ok_or(Stop::Bounds)
        }
    }
    fn refs(&mut self, mods: &[Modifier]) -> String {
        mods.iter()
            .map(|m| {
                let id = self
                    .pool
                    .iter()
                    .position(|p| p.same_identity(m))
                    .unwrap_or_else(|| {
                        self.pool.push(m.clone());
                        self.pool.len() - 1
                    });
                format!(
                    "{id}:{}:{}:{}",
                    m.id(),
                    f64_hex(m.amount()),
                    m.operation() as u8
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    }
    fn state(&mut self, map: &Map, current: Option<(usize, &Instance)>) {
        self.logs
            .push(format!("state {} {}", self.calls, self.changed));
        self.logs.push(format!(
            "map {}",
            map.instances
                .entries()
                .iter()
                .map(|(k, h)| format!("{}={}", desc(k.as_ref()), reference(h.as_ref())))
                .collect::<Vec<_>>()
                .join(",")
        ));
        self.logs.push(format!(
            "updates {}",
            map.updates
                .values()
                .map(reference)
                .collect::<Vec<_>>()
                .join(",")
        ));
        self.logs.push(format!(
            "sync {}",
            map.sync
                .values()
                .map(reference)
                .collect::<Vec<_>>()
                .join(",")
        ));
        for id in 0..self.instances.len() {
            let n = &self.instances[id];
            let a = match current {
                Some((i, a)) if i == id => a,
                _ => n.value.as_ref().unwrap(),
            };
            let prefix = format!(
                "instance {id} {} {} {} {}",
                n.key.id,
                f64_hex(a.base_value()),
                a.is_dirty(),
                f64_hex(a.cached_value())
            );
            let all = a.modifiers().to_vec();
            let permanent = a.permanent_modifiers().to_vec();
            let all = self.refs(&all);
            let permanent = self.refs(&permanent);
            self.logs.push(format!("{prefix} | {all} | {permanent}"));
        }
    }
    fn raw(&mut self, map: &mut Map, op: &Value) -> Result<(), Stop> {
        match op["op"].as_str().unwrap() {
            "put" => {
                map.instances
                    .put(self.key(op), self.handle(op["instance"].as_i64().unwrap())?);
            }
            "drop" => {
                map.instances.remove(self.key(op).as_ref());
            }
            "clear" => map.instances.clear(),
            "queue-add" | "queue-drop" | "queue-clear" => {
                let handle = self.handle(op["instance"].as_i64().unwrap())?;
                let set = if op["target"].as_u64().unwrap() == 0 {
                    &mut map.updates
                } else {
                    &mut map.sync
                };
                match op["op"].as_str().unwrap() {
                    "queue-add" => {
                        set.add(handle);
                    }
                    "queue-drop" => {
                        set.remove(handle.as_ref());
                    }
                    _ => set.clear(),
                }
            }
            "syncable" => {
                self.syncable[op["key"].as_u64().unwrap() as usize] = op["flag"].as_bool().unwrap()
            }
            "default" => self.defaults[op["key"].as_u64().unwrap() as usize] = number(&op["value"]),
            _ => panic!("raw operation"),
        }
        Ok(())
    }
    fn event(
        &mut self,
        map: &mut Map,
        current: Option<(usize, &Instance)>,
        name: String,
    ) -> Result<(), Stop> {
        let call = self.calls;
        self.calls += 1;
        self.logs.push(format!("event {call} {name}"));
        self.state(map, current);
        if call == self.op["fail"].as_i64().unwrap() {
            return Err(Stop::Callback);
        }
        if call == self.op["mutate"].as_i64().unwrap() {
            self.changed = true;
            for action in self.op["mutations"].as_array().unwrap().clone() {
                self.raw(map, &action)?;
            }
        }
        Ok(())
    }
    fn modified(
        &mut self,
        map: &mut Map,
        handle: Option<Handle>,
        current: Option<(usize, &Instance)>,
    ) -> Result<(), Stop> {
        map.modified(handle, |map, handle| {
            let handle = handle.ok_or(Stop::Null)?;
            let key = self.instances[handle.id].key.id;
            self.event(map, current, format!("syncable {key}"))?;
            Ok(self.syncable[key])
        })
    }
    fn get(&mut self, map: &mut Map, key: Option<Key>) -> Result<Option<Handle>, Stop> {
        self.event(map, None, format!("get {}", desc(key.as_ref())))?;
        map.get_instance(key, |map, key| self.create(map, key))
    }
    fn create(&mut self, map: &mut Map, key: Option<Key>) -> Result<Option<Handle>, Stop> {
        let Some(key) = key else {
            return Ok(None);
        };
        let Some(source) = self.suppliers[key.id].clone() else {
            return Ok(None);
        };
        self.event(map, None, format!("default {}", key.id))?;
        let mut a = Instance::new(Definition::basic(self.defaults[key.id]));
        let id = self.instances.len();
        let handle = Handle {
            id,
            hash: self.hashes[id],
        };
        self.instances.push(Node {
            key,
            handle: handle.clone(),
            value: None,
        });
        let mut cb = Dirty {
            world: self,
            map,
            handle: handle.clone(),
        };
        let result = a.replace_from(&source, &mut cb).map_err(attribute_error);
        self.instances[id].value = Some(a);
        result?;
        Ok(Some(handle))
    }
    fn edit<R>(
        &mut self,
        map: &mut Map,
        handle: &Handle,
        edit: impl FnOnce(&mut Instance, &mut Dirty<'_>) -> Result<R, AttributeError<Stop>>,
    ) -> Result<R, Stop> {
        let mut a = self.instances[handle.id].value.take().unwrap();
        let mut cb = Dirty {
            world: self,
            map,
            handle: handle.clone(),
        };
        let result = edit(&mut a, &mut cb).map_err(attribute_error);
        self.instances[handle.id].value = Some(a);
        result
    }
}
struct Dirty<'a> {
    world: &'a mut World,
    map: &'a mut Map,
    handle: Handle,
}
impl Observer for Dirty<'_> {
    type Error = Stop;
    fn dirty(&mut self, a: &mut Instance) -> Result<(), Stop> {
        self.world.modified(
            self.map,
            Some(self.handle.clone()),
            Some((self.handle.id, a)),
        )
    }
}
struct Bound<'a> {
    world: &'a mut World,
    map: &'a mut Map,
}
impl ModifierWorld<Key> for Bound<'_> {
    type Instance = Handle;
    type Error = Stop;
    fn get_instance(
        &mut self,
        _: &mut Definitions<Key>,
        key: Option<Key>,
    ) -> Result<Option<Handle>, Stop> {
        self.world.get(self.map, key)
    }
    fn remove_modifier(
        &mut self,
        _: &mut Definitions<Key>,
        h: &Handle,
        id: &str,
    ) -> Result<(), Stop> {
        self.world
            .edit(self.map, h, |a, cb| a.remove(id, cb))
            .map(|_| ())
    }
    fn add_permanent(
        &mut self,
        _: &mut Definitions<Key>,
        h: &Handle,
        modifier: Modifier,
    ) -> Result<(), Stop> {
        self.world
            .edit(self.map, h, |a, cb| a.add_permanent(modifier, cb))
    }
}
fn effect_error(e: effects::Error<Stop>) -> Stop {
    match e {
        effects::Error::World(e) => e,
        effects::Error::Cursor(CursorError::IndexOutOfBounds) => Stop::Bounds,
        effects::Error::MissingTemplate | effects::Error::Cursor(_) => Stop::Null,
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let dir = Path::new(&args[1]);
    let mut out = io::BufWriter::new(fs::File::create(dir.join(format!("rust-{}.txt", args[2])))?);
    for line in fs::read_to_string(dir.join(format!("inputs-{}.txt", args[2])))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let keys: Vec<_> = f["hashes"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(id, h)| Key {
                id,
                hash: h.as_i64().unwrap() as i32,
            })
            .collect();
        let defaults: Vec<_> = f["defaults"]
            .as_array()
            .unwrap()
            .iter()
            .map(number)
            .collect();
        let suppliers = f["bases"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(id, v)| {
                if v.is_null() {
                    None
                } else {
                    let mut a = Instance::new(Definition::basic(defaults[id]));
                    a.set_base_value(number(v), &mut Silent).unwrap();
                    Some(a)
                }
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
        let syncable = f["syncable"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_bool().unwrap())
            .collect();
        let hashes = f["instance_hashes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as i32)
            .collect();
        let mut world = World {
            keys,
            defaults,
            suppliers,
            templates,
            syncable,
            hashes,
            instances: Vec::new(),
            pool: Vec::new(),
            calls: 0,
            changed: false,
            op: Value::Null,
            logs: Vec::new(),
        };
        let mut map = Map::default();
        let mut defs = Definitions::default();
        for op in f["definitions"].as_array().unwrap() {
            effects::add_definition(
                &mut defs,
                world.key(op),
                world.templates[op["template"].as_u64().unwrap() as usize].clone(),
            );
        }
        for op in f["ops"].as_array().unwrap() {
            world.op = op.clone();
            world.calls = 0;
            world.changed = false;
            let result = match op["op"].as_str().unwrap() {
                "get" => world
                    .get(&mut map, world.key(op))
                    .map(|h| reference(h.as_ref())),
                "lookup" => Ok(reference(
                    map.instances.get(world.key(op).as_ref()).as_ref(),
                )),
                "add" => effects::add_modifiers(
                    &mut defs,
                    &mut Bound {
                        world: &mut world,
                        map: &mut map,
                    },
                    op["amplifier"].as_i64().unwrap() as i32,
                )
                .map_err(effect_error)
                .map(|()| "ok".into()),
                "remove" => effects::remove_modifiers(
                    &mut defs,
                    &mut Bound {
                        world: &mut world,
                        map: &mut map,
                    },
                )
                .map_err(effect_error)
                .map(|()| "ok".into()),
                "modified" => world
                    .handle(op["instance"].as_i64().unwrap())
                    .and_then(|h| world.modified(&mut map, h, None))
                    .map(|()| "ok".into()),
                "value" => world
                    .handle(op["instance"].as_i64().unwrap())
                    .and_then(|h| h.ok_or(Stop::Null))
                    .map(|h| f64_hex(world.instances[h.id].value.as_mut().unwrap().value())),
                _ => world.raw(&mut map, op).map(|()| "ok".into()),
            };
            world.logs.push(format!(
                "result {}",
                result.unwrap_or_else(|e| match e {
                    Stop::Callback => "callback",
                    Stop::Duplicate => "duplicate",
                    Stop::Null => "null",
                    Stop::Bounds => "bounds",
                }
                .into())
            ));
            world.state(&map, None);
            for line in world.logs.drain(..) {
                writeln!(out, "{line}")?;
            }
        }
    }
    Ok(())
}
