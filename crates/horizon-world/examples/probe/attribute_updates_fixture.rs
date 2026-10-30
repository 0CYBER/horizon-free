use horizon_world::{
    active_effects::{ActiveEffects, EffectHooks},
    effect_notifications::{
        self as notify, EffectPacket, EligibilityWorld, ImmunityTag, NotificationWorld,
        RestrictedEffect,
    },
    java_hash_map::JavaKey,
    object_open_set::{CursorError, ObjectKey, ObjectOpenSet},
    status_effect::{BlendState, Details, EffectInstance},
};
use horizon_world::{
    attribute_updates::{self, UpdateWorld, UpdatedAttribute},
    living_health::{self, HealthWorld, SyncedFloat},
};
use serde_json::Value;
use std::{
    cmp::Ordering,
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone)]
struct Key {
    id: usize,
    hash: i32,
    identity: i32,
}
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Key {}
impl ObjectKey for Key {
    fn object_hash(&self) -> i32 {
        self.hash
    }
}
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
type Effect = EffectInstance<Key>;
type Storage = ActiveEffects<Key>;
fn n(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
fn fl(v: &Value, k: &str) -> f32 {
    f32::from_bits(v[k].as_u64().unwrap() as u32)
}
fn bits(v: f32) -> u32 {
    if v.is_nan() { 0x7fc00000 } else { v.to_bits() }
}
fn details(v: &Value) -> Details {
    Details {
        duration: n(v, "duration"),
        amplifier: n(v, "amplifier"),
        ambient: v["ambient"].as_bool().unwrap(),
        visible: v["visible"].as_bool().unwrap(),
        show_icon: v["icon"].as_bool().unwrap(),
    }
}
struct World<'a> {
    health: [SyncedFloat; 2],
    absorption: f32,
    f: Value,
    out: &'a mut dyn Write,
    keys: Vec<Key>,
    attribute_keys: Vec<Key>,
    updates: [ObjectOpenSet<Key>; 2],
    pool: Vec<Effect>,
    ids: Vec<Effect>,
    calls: i32,
    changed: bool,
}
impl World<'_> {
    fn config(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn id(&mut self, e: &Effect) -> usize {
        if let Some(i) = self.ids.iter().position(|v| v.same_reference(e)) {
            i
        } else {
            self.ids.push(e.clone());
            self.ids.len() - 1
        }
    }
    fn reset(&mut self, storage: &mut Storage) {
        if let Some(bits) = self.config()["absorption"].as_u64() {
            self.absorption = f32::from_bits(bits as u32);
        }
        if let Some(dirty) = self.config()["dirty"].as_bool() {
            storage.dirty = dirty;
        }
        let c = self.config().clone();
        for (i, v) in c["states"].as_array().unwrap().iter().enumerate() {
            let mut e = self.pool[i].write();
            e.details = details(v);
            e.blend = BlendState {
                factor: fl(v, "factor"),
                previous: fl(v, "previous"),
            };
        }
    }
    fn all(&mut self, s: &Storage) {
        writeln!(
            self.out,
            "state {} {} {}",
            self.calls, self.changed, s.dirty
        )
        .unwrap();
        for (i, h) in self.health.iter().enumerate() {
            writeln!(
                self.out,
                "health-data {i} {} {} {} {}",
                bits(h.value()),
                h.is_dirty(),
                h.item_dirty(),
                h.is_default()
            )
            .unwrap();
        }
        writeln!(self.out, "absorption-data {}", bits(self.absorption)).unwrap();
        for (i, updates) in self.updates.iter().enumerate() {
            writeln!(
                self.out,
                "updates {i} {}",
                updates
                    .values()
                    .map(|v| v.map_or("null".into(), |k| k.id.to_string()))
                    .collect::<Vec<_>>()
                    .join(",")
            )
            .unwrap();
        }
        let mut roots = Vec::new();
        let entries = s
            .entries
            .entries()
            .map(|(k, e)| {
                let id = self.id(e);
                roots.push(e.clone());
                format!("{}={id}", k.id)
            })
            .collect::<Vec<_>>()
            .join(",");
        writeln!(self.out, "map {entries}").unwrap();
        for (i, e) in self.pool.clone().iter().enumerate() {
            let id = self.id(e);
            writeln!(self.out, "pool {i} {id}").unwrap();
            roots.push(e.clone());
        }
        let mut seen = Vec::new();
        let mut i = 0;
        while i < roots.len() {
            let e = roots[i].clone();
            i += 1;
            if seen.iter().any(|v: &Effect| v.same_reference(&e)) {
                continue;
            }
            seen.push(e.clone());
            let id = self.id(&e);
            let d = e.read();
            let h = d
                .hidden
                .as_ref()
                .map(|h| {
                    roots.push(h.clone());
                    self.id(h).to_string()
                })
                .unwrap_or("none".into());
            writeln!(
                self.out,
                "node {id} {} {} {} {} {} {} {h} {} {}",
                d.effect.id,
                d.details.duration,
                d.details.amplifier,
                d.details.ambient,
                d.details.visible,
                d.details.show_icon,
                bits(d.blend.factor),
                bits(d.blend.previous)
            )
            .unwrap();
        }
    }
    fn raw(&mut self, s: &mut Storage, op: &Value) {
        let a = n(op, "a") as usize;
        let b = n(op, "b") as usize;
        match op["op"].as_str().unwrap() {
            "pending-add" => {
                let key = op["b"].as_i64().unwrap();
                self.updates[a].add((key >= 0).then(|| self.attribute_keys[key as usize].clone()));
            }
            "pending-remove" => {
                let key = op["b"].as_i64().unwrap();
                self.updates[a].remove((key >= 0).then(|| &self.attribute_keys[key as usize]));
            }
            "pending-clear" => self.updates[a].clear(),
            "put" => {
                s.entries.put(self.keys[a].clone(), self.pool[b].clone());
            }
            "drop" => {
                s.entries.remove(&self.keys[a]);
            }
            "clear" => s.entries.clear(),
            "copy" => self.pool[b] = self.pool[a].copy_instance(),
            "alias" => self.pool[b] = self.pool[a].clone(),
            _ => panic!("raw op"),
        }
    }
    fn event(&mut self, s: &mut Storage, name: String) -> Result<(), &'static str> {
        writeln!(self.out, "event {name}").unwrap();
        let i = self.calls;
        self.calls += 1;
        self.all(s);
        if i == n(&self.f, "fail") {
            return Err("callback");
        }
        if i == n(&self.f, "mutate") {
            self.changed = true;
            self.reset(s);
            for op in self.f["mutations"].as_array().unwrap().clone() {
                self.raw(s, &op);
            }
        }
        Ok(())
    }
}
impl EffectHooks<Key> for World<'_> {
    type Error = &'static str;
    type Source = i32;
    type Level = i32;
    fn can_be_affected(&mut self, s: &mut Storage, e: Effect) -> Result<bool, Self::Error> {
        notify::can_be_affected(s, &e, self)
    }
    fn on_added(&mut self, s: &mut Storage, e: Effect, _: Option<i32>) -> Result<(), Self::Error> {
        notify::on_added(s, &e, self)
    }
    fn on_updated(
        &mut self,
        s: &mut Storage,
        e: Effect,
        reapply: bool,
        _: Option<i32>,
    ) -> Result<(), Self::Error> {
        notify::on_updated(s, &e, reapply, self)
    }
    fn effect_added(&mut self, s: &mut Storage, e: Key, amp: i32) -> Result<(), Self::Error> {
        if e.id < 4 {
            Ok(())
        } else {
            self.event(s, format!("effect-added {} {amp}", e.id))
        }
    }
    fn effect_started(&mut self, s: &mut Storage, e: Key, amp: i32) -> Result<(), Self::Error> {
        if e.id < 4 {
            Ok(())
        } else {
            self.event(s, format!("effect-started {} {amp}", e.id))
        }
    }
    fn level(&mut self, s: &mut Storage) -> Result<i32, Self::Error> {
        self.event(s, "level".into())?;
        Ok(n(self.config(), "level"))
    }
    fn is_client(&mut self, s: &mut Storage, l: i32) -> Result<bool, Self::Error> {
        self.event(s, format!("client {l}"))?;
        Ok(self.config()["client"].as_bool().unwrap())
    }
    fn on_removed(&mut self, s: &mut Storage, es: Vec<Effect>) -> Result<(), Self::Error> {
        notify::on_removed(s, &es, self)
    }
    fn get_effect(&mut self, s: &mut Storage, key: Key) -> Result<Option<Effect>, Self::Error> {
        self.event(s, format!("get {}", key.id))?;
        Ok(s.get(&key))
    }
    fn actor_removed(&self) -> bool {
        self.config()["removed"].as_bool().unwrap()
    }
}
impl EligibilityWorld<Key> for World<'_> {
    type Error = &'static str;
    fn has_immunity_tag(&mut self, s: &mut Storage, tag: ImmunityTag) -> Result<bool, Self::Error> {
        let id = tag as usize;
        self.event(s, format!("tag {id}"))?;
        Ok(self.config()["tags"][id].as_bool().unwrap())
    }
    fn is_restricted_effect(&self, effect: &Key, kind: RestrictedEffect) -> bool {
        effect.id == kind as usize
    }
}
impl NotificationWorld<Key> for World<'_> {
    type Attributes = usize;
    type UpdateSet = usize;
    type AttributeInstance = Key;
    type Attribute = i32;
    type Passenger = usize;
    type Connection = i32;
    fn attributes(&mut self, s: &mut Storage) -> Result<usize, Self::Error> {
        self.event(s, "attributes".into())?;
        Ok(n(self.config(), "attributes") as usize)
    }
    fn add_modifiers(
        &mut self,
        s: &mut Storage,
        e: Key,
        a: usize,
        amp: i32,
    ) -> Result<(), Self::Error> {
        if e.id < 4 {
            return Ok(());
        }
        self.event(s, format!("add-modifiers {} {a} {amp}", e.id))
    }
    fn remove_modifiers(&mut self, s: &mut Storage, e: Key, a: usize) -> Result<(), Self::Error> {
        if e.id < 4 {
            return Ok(());
        }
        self.event(s, format!("remove-modifiers {} {a}", e.id))
    }
    fn pending_updates(&mut self, s: &mut Storage, a: usize) -> Result<usize, Self::Error> {
        self.event(s, format!("pending {a}"))?;
        Ok(self.config()["routes"][a].as_u64().unwrap() as usize)
    }
    fn update_set(&self, s: usize) -> &ObjectOpenSet<Key> {
        &self.updates[s]
    }
    fn update_set_mut(&mut self, s: usize) -> &mut ObjectOpenSet<Key> {
        &mut self.updates[s]
    }
    fn iterator_error(e: CursorError) -> Self::Error {
        match e {
            CursorError::MissingWrapped => "null",
            CursorError::IndexOutOfBounds => "bounds",
            CursorError::NoSuchElement => "empty",
            CursorError::NoCurrentEntry => "current",
        }
    }
    fn attribute_of(&mut self, s: &mut Storage, i: Option<Key>) -> Result<i32, Self::Error> {
        let i = i.ok_or("null")?;
        self.event(s, format!("attribute {}", i.id))?;
        Ok(self.config()["holders"][i.id].as_i64().unwrap() as i32)
    }
    fn attribute_updated(&mut self, s: &mut Storage, a: i32) -> Result<(), Self::Error> {
        attribute_updates::updated(&mut Update { w: self, s }, a)
    }
    fn passengers(&self) -> Vec<usize> {
        self.config()["passengers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_u64().unwrap() as usize)
            .collect()
    }
    fn is_server_player(&self, p: usize) -> bool {
        p < 2
    }
    fn connection(&self, p: usize) -> i32 {
        self.config()["connections"][p].as_i64().unwrap() as i32
    }
    fn entity_id(&mut self, s: &mut Storage) -> Result<i32, Self::Error> {
        self.event(s, "entity-id".into())?;
        Ok(n(self.config(), "entity"))
    }
    fn send_packet(
        &mut self,
        s: &mut Storage,
        c: i32,
        p: EffectPacket<Key>,
    ) -> Result<(), Self::Error> {
        if c < 0 {
            return Err("null");
        }
        let message = match p {
            EffectPacket::Update {
                entity,
                effect,
                amplifier,
                duration,
                flags,
            } => format!(
                "packet {c} update {entity} {} {amplifier} {duration} {flags}",
                effect.id
            ),
            EffectPacket::Remove { entity, effect } => {
                format!("packet {c} remove {entity} {}", effect.id)
            }
        };
        self.event(s, message)
    }
}
struct Update<'w, 'a> {
    w: &'w mut World<'a>,
    s: &'w mut Storage,
}
impl HealthWorld for Update<'_, '_> {
    type Error = &'static str;
    type Data = usize;
    fn health(&mut self) -> Result<f32, Self::Error> {
        self.w.event(self.s, "health".into())?;
        Ok(self.w.health[self.health_data()].value())
    }
    fn health_data(&self) -> usize {
        n(self.w.config(), "data") as usize
    }
    fn max_health_attribute(&mut self) -> Result<f64, Self::Error> {
        self.w.event(self.s, "max-health".into())?;
        Ok(f64::from_bits(
            self.w.config()["maximums"][0].as_u64().unwrap(),
        ))
    }
    fn max_absorption_attribute(&mut self) -> Result<f64, Self::Error> {
        self.w.event(self.s, "max-absorption".into())?;
        Ok(f64::from_bits(
            self.w.config()["maximums"][1].as_u64().unwrap(),
        ))
    }
    fn store_health(&mut self, data: usize, value: f32) -> Result<(), Self::Error> {
        let mut h = self.w.health[data];
        let result = h.set(value, false, |h| {
            self.w.health[data] = *h;
            self.w.event(self.s, "health-updated".into())
        });
        self.w.health[data] = h;
        result
    }
    fn store_absorption(&mut self, value: f32) -> Result<(), Self::Error> {
        self.w
            .event(self.s, format!("internal-absorption {}", bits(value)))?;
        self.w.absorption = value;
        Ok(())
    }
    fn dead_flag(&self) -> bool {
        false
    }
    fn dispatch_set_health(&mut self, value: f32) -> Result<(), Self::Error> {
        self.w
            .event(self.s, format!("set-health {}", bits(value)))?;
        living_health::set_health(self, value)
    }
}
impl UpdateWorld<i32> for Update<'_, '_> {
    type Level = i32;
    type Manager = usize;
    type Attributes = usize;
    fn attribute_kind(&self, h: &i32) -> UpdatedAttribute {
        match *h {
            0 => UpdatedAttribute::MaxHealth,
            1 => UpdatedAttribute::MaxAbsorption,
            2 => UpdatedAttribute::Scale,
            3 => UpdatedAttribute::WaypointRange,
            _ => UpdatedAttribute::Other,
        }
    }
    fn absorption(&mut self) -> Result<f32, Self::Error> {
        self.w.event(self.s, "absorption".into())?;
        Ok(self.w.absorption)
    }
    fn refresh_dimensions(&mut self) -> Result<(), Self::Error> {
        self.w.event(self.s, "dimensions".into())
    }
    fn level(&mut self) -> Result<i32, Self::Error> {
        EffectHooks::level(self.w, self.s)
    }
    fn is_server_level(&self, l: i32) -> bool {
        self.w.f["servers"][l as usize].as_bool().unwrap()
    }
    fn waypoint_manager(&mut self, l: i32) -> Result<usize, Self::Error> {
        self.w.event(self.s, format!("manager {l}"))?;
        Ok(self.w.config()["managers"][l as usize].as_u64().unwrap() as usize)
    }
    fn attributes_field(&self) -> usize {
        n(self.w.config(), "direct_attributes") as usize
    }
    fn attribute_value_in(&mut self, a: usize, h: i32) -> Result<f64, Self::Error> {
        self.w.event(self.s, format!("value-in {a} {h}"))?;
        Ok(f64::from_bits(
            self.w.config()["values"][a][h as usize].as_u64().unwrap(),
        ))
    }
    fn track_waypoint(&mut self, m: usize, track: bool) -> Result<(), Self::Error> {
        self.w.event(self.s, format!("waypoint {m} {track}"))
    }
}
fn run(f: Value, out: &mut dyn Write) {
    let keys = f["hashes"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(id, v)| Key {
            id,
            hash: v.as_i64().unwrap() as i32,
            identity: f["identities"][id].as_i64().unwrap() as i32,
        })
        .collect::<Vec<_>>();
    let mut pool: Vec<Effect> = Vec::new();
    for v in f["initial"]["states"].as_array().unwrap() {
        let h = n(v, "hidden");
        pool.push(Effect::new(
            keys[n(v, "key") as usize].clone(),
            details(v),
            (h >= 0).then(|| pool[h as usize].clone()),
        ));
    }
    let attribute_keys = f["attribute_hashes"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(id, h)| Key {
            id,
            hash: h.as_i64().unwrap() as i32,
            identity: 0,
        })
        .collect();
    let health = std::array::from_fn(|i| {
        SyncedFloat::new(f32::from_bits(f["health"][i].as_u64().unwrap() as u32))
    });
    let mut w = World {
        health,
        absorption: 0.0,
        attribute_keys,
        updates: std::array::from_fn(|_| ObjectOpenSet::default()),
        f,
        out,
        keys,
        pool,
        ids: Vec::new(),
        calls: 0,
        changed: false,
    };
    let mut s = Storage::default();
    w.reset(&mut s);
    for op in w.f["setup"].as_array().unwrap().clone() {
        w.raw(&mut s, &op);
    }
    w.all(&s);
    for op in w.f["ops"].as_array().unwrap().clone() {
        let a = n(&op, "a") as usize;
        let b = n(&op, "b") as usize;
        let source = n(&op, "source");
        let source = (source >= 0).then_some(source);
        let result: Result<String, &'static str> = match op["op"].as_str().unwrap() {
            "attribute-update" => attribute_updates::updated(
                &mut Update {
                    w: &mut w,
                    s: &mut s,
                },
                a as i32,
            )
            .map(|_| "ok".into()),
            "pack" => Ok(w.health[a]
                .take_dirty()
                .map(|v| bits(v).to_string())
                .unwrap_or("none".into())),
            "eligible" => {
                let e = w.pool[a].clone();
                notify::can_be_affected(&mut s, &e, &mut w).map(|v| v.to_string())
            }
            "notify-add" => {
                let e = w.pool[a].clone();
                notify::on_added(&mut s, &e, &mut w).map(|_| "ok".into())
            }
            "notify-update" => {
                let e = w.pool[a].clone();
                notify::on_updated(&mut s, &e, op["reapply"].as_bool().unwrap(), &mut w)
                    .map(|_| "ok".into())
            }
            "notify-remove" => {
                let es = op["list"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|i| w.pool[i.as_u64().unwrap() as usize].clone())
                    .collect::<Vec<_>>();
                notify::on_removed(&mut s, &es, &mut w).map(|_| "ok".into())
            }
            "send" => {
                let e = w.pool[a].clone();
                notify::send_to_passengers(&mut s, &e, &mut w).map(|_| "ok".into())
            }
            "refresh" => notify::refresh_attributes(&mut s, &mut w).map(|_| "ok".into()),
            "pending-add" | "pending-remove" | "pending-clear" => {
                w.raw(&mut s, &op);
                Ok("ok".into())
            }
            "add" => s
                .add(w.pool[a].clone(), source, &mut w)
                .map(|v| v.to_string()),
            "force" => s
                .force_add(w.pool[a].clone(), source, &mut w)
                .map(|_| "ok".into()),
            "remove" => s.remove(&w.keys[a].clone(), &mut w).map(|v| v.to_string()),
            "remove-raw" => Ok(s
                .remove_no_update(&w.keys[a])
                .map(|e| w.id(&e).to_string())
                .unwrap_or("none".into())),
            "all" => s.remove_all(&mut w).map(|v| v.to_string()),
            "has" => Ok(s.has(&w.keys[a]).to_string()),
            "get" => {
                let key = w.keys[a].clone();
                EffectHooks::get_effect(&mut w, &mut s, key)
                    .map(|e| e.map(|e| w.id(&e).to_string()).unwrap_or("none".into()))
            }
            "blend" => s
                .blend_factor(w.keys[a].clone(), fl(&op, "partial"), &mut w)
                .map(|v| bits(v).to_string()),
            "put" | "drop" | "clear" | "copy" | "alias" => {
                w.raw(&mut s, &op);
                Ok("ok".into())
            }
            _ => panic!("operation {b}"),
        };
        writeln!(w.out, "result {}", result.unwrap_or_else(str::to_string)).unwrap();
        w.all(&s);
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let side = crate::args().nth(2).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join(format!("rust-{side}.txt")))?);
    for line in fs::read_to_string(dir.join(format!("inputs-{side}.txt")))?.lines() {
        run(serde_json::from_str(line)?, &mut out);
    }
    Ok(())
}
