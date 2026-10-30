use horizon_world::{
    active_effects::ActiveEffects,
    effect_particles::{Catalog, EffectValue, Particle},
    effect_sync::{self as sync, EffectSyncWorld, Presence},
    java_hash_map::JavaKey,
    status_effect::{Details, EffectInstance},
    synced_values::{DataError, DataValue, Observer, SyncedValues},
};
use serde_json::Value;
use std::{
    cmp::Ordering,
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
type Result<T, E = &'static str> = std::result::Result<T, E>;
#[derive(Clone)]
struct Key {
    id: usize,
    hash: i32,
    identity: i32,
    name: String,
}
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Key {}
impl JavaKey for Key {
    fn java_hash(&self) -> i32 {
        self.hash
    }
    fn tie_break(&self, other: &Self) -> Ordering {
        if self.identity <= other.identity {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }
}
type Storage = ActiveEffects<Key>;
type Data = SyncedValues<EffectValue>;
struct World<'a> {
    keys: Vec<Key>,
    catalog: &'a Catalog,
    tables: [Data; 2],
    flags: u8,
    particles: u8,
    ambient: u8,
    data: usize,
    client: bool,
    tag: bool,
    calls: i64,
    op: Value,
    pool: Vec<EffectInstance<Key>>,
    particle_pool: Vec<Particle>,
    out: &'a mut dyn Write,
}
impl World<'_> {
    fn particle_ref(&mut self, particle: &Particle) -> String {
        let id = self
            .particle_pool
            .iter()
            .position(|p| p.same_reference(particle))
            .unwrap_or_else(|| {
                self.particle_pool.push(particle.clone());
                self.particle_pool.len() - 1
            });
        let p = particle.data();
        format!(
            "{id}:{}:{}",
            p.kind,
            p.color.map_or("none".into(), |v| v.to_string())
        )
    }
    fn value(&mut self, value: &EffectValue) -> String {
        match value {
            EffectValue::Flags(v) => format!("flags:{v}"),
            EffectValue::Ambient(v) => format!("ambient:{v}"),
            EffectValue::Particles(v) => format!(
                "particles:[{}]",
                v.iter()
                    .map(|p| self.particle_ref(p))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
    fn packed(&mut self, values: Option<Vec<DataValue<EffectValue>>>) -> String {
        match values {
            None => "none".into(),
            Some(v) if v.is_empty() => "empty".into(),
            Some(v) => v
                .iter()
                .map(|v| format!("{}={}", v.id, self.value(&v.value)))
                .collect::<Vec<_>>()
                .join("|"),
        }
    }
    fn controls(&mut self, s: &mut Storage, c: &Value) {
        if let Some(v) = c["data"].as_u64() {
            self.data = v as usize;
        }
        if let Some(v) = c["client"].as_bool() {
            self.client = v;
        }
        if let Some(v) = c["tag"].as_bool() {
            self.tag = v;
        }
        if let Some(v) = c["dirty"].as_bool() {
            s.dirty = v;
        }
    }
    fn event(&mut self, s: &mut Storage, name: impl AsRef<str>) -> Result<()> {
        let call = self.calls;
        self.calls += 1;
        writeln!(self.out, "event {call} {}", name.as_ref()).unwrap();
        if call == self.op["fail"].as_i64().unwrap() {
            return Err("callback");
        }
        if call == self.op["mutate"].as_i64().unwrap() {
            self.controls(s, &self.op["changes"].clone());
            for action in self.op["actions"].as_array().unwrap().clone() {
                self.raw(s, &action)?;
            }
        }
        Ok(())
    }
    fn raw(&mut self, s: &mut Storage, op: &Value) -> Result<()> {
        let target = op["target"].as_u64().unwrap() as usize;
        let key = op["key"].as_u64().unwrap() as usize;
        let slot = op["pool"].as_u64().unwrap() as usize;
        match op["mode"].as_str().unwrap() {
            "put" => {
                s.entries
                    .put(self.keys[key].clone(), self.pool[slot].clone());
            }
            "drop" => {
                s.entries.remove(&self.keys[key]);
            }
            "clear" => s.entries.clear(),
            "details" => {
                let mut e = self.pool[slot].write();
                e.details.visible = op["visible"].as_bool().unwrap();
                e.details.ambient = op["ambient"].as_bool().unwrap();
            }
            "controls" => self.controls(s, &op["changes"]),
            "flags" => self.store(
                s,
                target,
                self.flags,
                EffectValue::Flags(op["flags"].as_u64().unwrap() as u8),
            )?,
            "ambient" => self.store(
                s,
                target,
                self.ambient,
                EffectValue::Ambient(op["ambient"].as_bool().unwrap()),
            )?,
            "pack" => {
                let values = self.tables[target].take_dirty();
                let text = self.packed(values);
                writeln!(self.out, "packed {target} {text}").unwrap();
            }
            "defaults" => {
                let values = self.tables[target].non_default();
                let text = self.packed(values);
                writeln!(self.out, "defaults {target} {text}").unwrap();
            }
            mode => panic!("raw mode {mode}"),
        }
        Ok(())
    }
    fn store(&mut self, s: &mut Storage, data: usize, id: u8, value: EffectValue) -> Result<()> {
        // Fixture callbacks can inspect/pack/reenter the same table. Mirror the
        // actual item during the observer, then keep its captured identity.
        let mut table = self.tables[data].clone();
        let result = table.set(
            id,
            value,
            false,
            &mut Callback {
                world: self,
                storage: s,
                data,
            },
        );
        self.tables[data] = table;
        result.map_err(|e| match e {
            DataError::Observer(e) => e,
            _ => panic!("registered data IDs"),
        })
    }
    fn state(&mut self, s: &Storage) {
        writeln!(
            self.out,
            "state {} {} {} {} {}",
            self.data, self.client, self.tag, s.dirty, self.calls
        )
        .unwrap();
        let entries = s
            .entries
            .entries()
            .map(|(k, e)| {
                format!(
                    "{}={}",
                    k.id,
                    self.pool.iter().position(|v| v.same_reference(e)).unwrap()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        writeln!(self.out, "effects {entries}").unwrap();
        for (i, e) in self.pool.iter().enumerate() {
            let e = e.read();
            writeln!(
                self.out,
                "instance {i} {} {}",
                e.details.visible, e.details.ambient
            )
            .unwrap();
        }
        let mut ids = [self.flags, self.particles, self.ambient];
        ids.sort_unstable();
        for i in 0..2 {
            writeln!(self.out, "data {i} {}", self.tables[i].is_dirty()).unwrap();
            for id in ids {
                let value = self.tables[i].get(id).unwrap();
                let value = self.value(&value);
                writeln!(
                    self.out,
                    "field {id} {value} {} {}",
                    self.tables[i].item_dirty(id).unwrap(),
                    self.tables[i].is_default(id).unwrap()
                )
                .unwrap();
            }
        }
    }
}
struct Callback<'a, 'b> {
    world: &'a mut World<'b>,
    storage: &'a mut Storage,
    data: usize,
}
impl Observer<EffectValue> for Callback<'_, '_> {
    type Error = &'static str;
    fn updated(&mut self, data: &mut Data, id: u8) -> Result<()> {
        self.world.tables[self.data] = data.clone();
        let result = self.world.event(self.storage, format!("updated {id}"));
        *data = self.world.tables[self.data].clone();
        result
    }
}
impl EffectSyncWorld<Key> for World<'_> {
    type Error = &'static str;
    type Data = usize;
    type Level = ();
    fn data(&self) -> usize {
        self.data
    }
    fn store_particles(
        &mut self,
        s: &mut Storage,
        data: usize,
        particles: Arc<[Particle]>,
    ) -> Result<()> {
        self.store(s, data, self.particles, EffectValue::Particles(particles))
    }
    fn store_ambient(&mut self, s: &mut Storage, data: usize, ambient: bool) -> Result<()> {
        self.store(s, data, self.ambient, EffectValue::Ambient(ambient))
    }
    fn particle(&self, key: &Key, ambient: bool) -> Result<Particle> {
        Ok(self.catalog.create(&key.name, ambient).unwrap())
    }
    fn has_effect(&mut self, s: &mut Storage, kind: Presence) -> Result<bool> {
        let name = match kind {
            Presence::Invisibility => "minecraft:invisibility",
            Presence::Glowing => "minecraft:glowing",
        };
        self.event(s, format!("has {name}"))?;
        Ok(s.has(self.keys.iter().find(|k| k.name == name).unwrap()))
    }
    fn level(&mut self, s: &mut Storage) -> Result<()> {
        self.event(s, "level")
    }
    fn is_client(&mut self, s: &mut Storage, _: ()) -> Result<bool> {
        self.event(s, "client")?;
        Ok(self.client)
    }
    fn glowing_tag(&self) -> bool {
        self.tag
    }
    fn write_glowing_tag(&mut self, tag: bool) {
        self.tag = tag;
    }
    fn read_flags(&self, data: usize) -> Result<u8> {
        match self.tables[data].get(self.flags).unwrap() {
            EffectValue::Flags(v) => Ok(v),
            _ => panic!("flags type"),
        }
    }
    fn write_flags(&mut self, s: &mut Storage, data: usize, flags: u8) -> Result<()> {
        self.store(s, data, self.flags, EffectValue::Flags(flags))
    }
    fn get_flag(&mut self, s: &mut Storage, bit: i32) -> Result<bool> {
        self.event(s, format!("get-flag {bit}"))?;
        sync::shared_flag(self, bit)
    }
    fn set_flag(&mut self, s: &mut Storage, bit: i32, value: bool) -> Result<()> {
        self.event(s, format!("set-flag {bit} {value}"))?;
        sync::set_shared_flag(s, self, bit, value)
    }
    fn set_invisible(&mut self, s: &mut Storage, value: bool) -> Result<()> {
        self.event(s, format!("invisible {value}"))?;
        self.set_flag(s, 5, value)
    }
    fn currently_glowing(&mut self, s: &mut Storage) -> Result<bool> {
        self.event(s, "glowing")?;
        sync::currently_glowing(s, self)
    }
    fn remove_particles(&mut self, s: &mut Storage) -> Result<()> {
        self.event(s, "remove-particles")?;
        sync::remove_particles(s, self)
    }
    fn update_invisibility(&mut self, s: &mut Storage) -> Result<()> {
        self.event(s, "invisibility")?;
        sync::update_invisibility(s, self)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let dir = Path::new(&args[1]);
    let metadata: Value = serde_json::from_str(&fs::read_to_string(dir.join("catalog.json"))?)?;
    let catalog = Catalog::from_json(&metadata)?;
    let flags = metadata["schema"]["flags"].as_u64().unwrap() as u8;
    let ambient = metadata["schema"]["ambient"].as_u64().unwrap() as u8;
    let particles = metadata["schema"]["particles"].as_u64().unwrap() as u8;
    let mut out = io::BufWriter::new(fs::File::create(dir.join(format!("rust-{}.txt", args[2])))?);
    for line in fs::read_to_string(dir.join(format!("inputs-{}.txt", args[2])))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let keys: Vec<_> = metadata["effects"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(id, d)| Key {
                id,
                hash: f["hashes"][id].as_i64().unwrap() as i32,
                identity: f["identities"][id].as_i64().unwrap() as i32,
                name: d["id"].as_str().unwrap().into(),
            })
            .collect();
        let pool = f["pool"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                EffectInstance::new(
                    keys[e["effect"].as_u64().unwrap() as usize].clone(),
                    Details {
                        duration: 200,
                        amplifier: e["amplifier"].as_i64().unwrap() as i32,
                        ambient: e["ambient"].as_bool().unwrap(),
                        visible: e["visible"].as_bool().unwrap(),
                        show_icon: true,
                    },
                    None,
                )
            })
            .collect();
        let tables = std::array::from_fn(|i| {
            Data::new(&[
                DataValue {
                    id: flags,
                    value: EffectValue::Flags(f["flags"][i].as_u64().unwrap() as u8),
                },
                DataValue {
                    id: ambient,
                    value: EffectValue::Ambient(f["ambient"][i].as_bool().unwrap()),
                },
                DataValue {
                    id: particles,
                    value: EffectValue::Particles(Arc::from([])),
                },
            ])
            .unwrap()
        });
        let mut w = World {
            keys,
            catalog: &catalog,
            tables,
            flags,
            particles,
            ambient,
            data: 0,
            client: false,
            tag: false,
            calls: 0,
            op: Value::Null,
            pool,
            particle_pool: vec![],
            out: &mut out,
        };
        let mut storage = Storage::default();
        w.controls(&mut storage, &f["controls"]);
        for op in f["setup"].as_array().unwrap() {
            w.raw(&mut storage, op).unwrap();
        }
        for op in f["ops"].as_array().unwrap() {
            w.op = op.clone();
            w.calls = 0;
            let result = match op["mode"].as_str().unwrap() {
                "dirty-update" => sync::update_dirty(&mut storage, &mut w).map(|()| "ok".into()),
                "invisibility" => w.update_invisibility(&mut storage).map(|()| "ok".into()),
                "particles" => {
                    sync::synchronize_particles(&mut storage, &mut w).map(|()| "ok".into())
                }
                "glowing-update" => {
                    sync::update_glowing(&mut storage, &mut w).map(|()| "ok".into())
                }
                "remove-particles" => w.remove_particles(&mut storage).map(|()| "ok".into()),
                "glowing" => w.currently_glowing(&mut storage).map(|v| v.to_string()),
                "glow-tag" => {
                    sync::set_glowing_tag(&mut storage, &mut w, op["flag"].as_bool().unwrap())
                        .map(|()| "ok".into())
                }
                "invisible" => w
                    .set_invisible(&mut storage, op["flag"].as_bool().unwrap())
                    .map(|()| "ok".into()),
                "get-flag" => w
                    .get_flag(&mut storage, op["bit"].as_i64().unwrap() as i32)
                    .map(|v| v.to_string()),
                "set-flag" => w
                    .set_flag(
                        &mut storage,
                        op["bit"].as_i64().unwrap() as i32,
                        op["flag"].as_bool().unwrap(),
                    )
                    .map(|()| "ok".into()),
                "all-ambient" => Ok(sync::all_ambient(&storage).to_string()),
                _ => w.raw(&mut storage, op).map(|()| "ok".into()),
            };
            writeln!(
                w.out,
                "result {}",
                result.unwrap_or_else(|e| format!("error {e}"))
            )?;
            w.state(&storage);
        }
    }
    Ok(())
}
