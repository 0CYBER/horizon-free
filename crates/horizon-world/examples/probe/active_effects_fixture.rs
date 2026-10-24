use horizon_world::{
    active_effects::{ActiveEffects, EffectHooks},
    java_hash_map::JavaKey,
    status_effect::{BlendState, Details, EffectInstance},
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
    f: Value,
    out: &'a mut dyn Write,
    keys: Vec<Key>,
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
    fn reset(&mut self) {
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
        writeln!(self.out, "state {} {}", self.calls, self.changed).unwrap();
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
            self.reset();
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
        let i = self.id(&e);
        self.event(s, format!("eligible {i}"))?;
        Ok(self.config()["allow"].as_bool().unwrap())
    }
    fn on_added(
        &mut self,
        s: &mut Storage,
        e: Effect,
        source: Option<i32>,
    ) -> Result<(), Self::Error> {
        let i = self.id(&e);
        self.event(s, format!("added {i} {}", source.unwrap_or(-1)))
    }
    fn on_updated(
        &mut self,
        s: &mut Storage,
        e: Effect,
        reapply: bool,
        source: Option<i32>,
    ) -> Result<(), Self::Error> {
        let i = self.id(&e);
        self.event(s, format!("updated {i} {reapply} {}", source.unwrap_or(-1)))
    }
    fn effect_added(&mut self, s: &mut Storage, e: Key, amp: i32) -> Result<(), Self::Error> {
        self.event(s, format!("effect-added {} {amp}", e.id))
    }
    fn effect_started(&mut self, s: &mut Storage, e: Key, amp: i32) -> Result<(), Self::Error> {
        self.event(s, format!("effect-started {} {amp}", e.id))
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
        let ids = es
            .iter()
            .map(|e| self.id(e).to_string())
            .collect::<Vec<_>>()
            .join(",");
        self.event(s, format!("removed {ids}"))
    }
    fn get_effect(&mut self, s: &mut Storage, key: Key) -> Result<Option<Effect>, Self::Error> {
        self.event(s, format!("get {}", key.id))?;
        Ok(s.get(&key))
    }
    fn actor_removed(&self) -> bool {
        self.config()["removed"].as_bool().unwrap()
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
    let mut w = World {
        f,
        out,
        keys,
        pool,
        ids: Vec::new(),
        calls: 0,
        changed: false,
    };
    let mut s = Storage::default();
    w.reset();
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
