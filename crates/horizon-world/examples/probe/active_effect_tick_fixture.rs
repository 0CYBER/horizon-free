use horizon_core::{
    position::BlockPos,
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::{
    active_effect_tick::{TickError, TickHooks},
    active_effects::{ActiveEffects, EffectHooks},
    consume_effect::ConsumeEffect,
    java_hash_map::CursorError,
    java_hash_map::JavaKey,
    status_effect::{BlendState, Details, EffectInstance},
    stored_consume::{ConsumeHooks, StoredConsume},
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
struct World<'a, R> {
    random: [R; 2],
    f: Value,
    out: &'a mut dyn Write,
    keys: Vec<Key>,
    pool: Vec<Effect>,
    ids: Vec<Effect>,
    calls: i32,
    changed: bool,
}
impl<R: RandomSource> World<'_, R> {
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
        if c["reseed"].as_bool().unwrap() {
            for i in 0..2 {
                self.random[i].set_seed(c["seeds"][i].as_i64().unwrap());
            }
        }
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
        if i == n(&self.f, "cme") {
            return Err("concurrent");
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
impl<R: RandomSource> EffectHooks<Key> for World<'_, R> {
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
impl<R: RandomSource> ConsumeHooks<Key, u8, Vec<Key>> for World<'_, R> {
    type Random = usize;
    type Rng = R;
    type SoundCategory = i32;
    fn get_random(&mut self, s: &mut Storage) -> Result<usize, Self::Error> {
        self.event(s, "random".into())?;
        Ok(n(self.config(), "random") as usize)
    }
    fn random_source(&mut self, r: usize) -> &mut R {
        &mut self.random[r]
    }
    fn effect_set(&self, set: &Vec<Key>) -> Vec<Key> {
        set.clone()
    }
    fn block_position(&mut self, s: &mut Storage) -> Result<BlockPos, Self::Error> {
        self.event(s, "position".into())?;
        let p = &self.config()["position"];
        Ok(BlockPos::new(
            p[0].as_i64().unwrap() as i32,
            p[1].as_i64().unwrap() as i32,
            p[2].as_i64().unwrap() as i32,
        ))
    }
    fn sound_category(&mut self, s: &mut Storage) -> Result<i32, Self::Error> {
        self.event(s, "category".into())?;
        Ok(n(self.config(), "category"))
    }
    fn play_sound(
        &mut self,
        storage: &mut Storage,
        level: i32,
        p: BlockPos,
        sound: u8,
        category: i32,
    ) -> Result<(), Self::Error> {
        self.event(
            storage,
            format!("sound {level} {} {} {} {sound} {category}", p.x, p.y, p.z),
        )
    }
}
impl<R: RandomSource> TickHooks<Key> for World<'_, R> {
    type Rng = R;
    type Particle = i32;
    fn is_server_level(&self, level: i32) -> bool {
        self.f["servers"][level as usize].as_bool().unwrap()
    }
    fn is_concurrent_modification(&self, e: &Self::Error) -> bool {
        *e == "concurrent"
    }
    fn actor_tick_count(&self) -> i32 {
        n(self.config(), "tick")
    }
    fn should_apply(
        &mut self,
        s: &mut Storage,
        effect: Key,
        time: i32,
        amp: i32,
    ) -> Result<bool, Self::Error> {
        self.event(s, format!("should {} {time} {amp}", effect.id))?;
        Ok(self.config()["should"].as_bool().unwrap())
    }
    fn apply_tick(
        &mut self,
        s: &mut Storage,
        effect: Key,
        level: i32,
        amp: i32,
    ) -> Result<bool, Self::Error> {
        self.event(s, format!("apply {} {level} {amp}", effect.id))?;
        Ok(self.config()["apply"].as_bool().unwrap())
    }
    fn advance_ticks(&mut self, s: &mut Storage, effect: Key) -> Result<i32, Self::Error> {
        self.event(s, format!("advance {}", effect.id))?;
        Ok(n(self.config(), "advance"))
    }
    fn duration_ticks(
        &mut self,
        s: &mut Storage,
        effect: Key,
        blending_in: bool,
    ) -> Result<i32, Self::Error> {
        self.event(s, format!("blend-duration {} {blending_in}", effect.id))?;
        Ok(n(
            self.config(),
            if blending_in { "blend_in" } else { "blend_out" },
        ))
    }
    fn particles(&self) -> Vec<i32> {
        self.config()["particles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as i32)
            .collect()
    }
    fn ambient(&self) -> bool {
        self.config()["ambience"].as_bool().unwrap()
    }
    fn invisible(&mut self, s: &mut Storage) -> Result<bool, Self::Error> {
        self.event(s, "invisible".into())?;
        Ok(self.config()["invisible"].as_bool().unwrap())
    }
    fn random_source(&mut self) -> &mut R {
        let r = n(self.config(), "random") as usize;
        &mut self.random[r]
    }
    fn random_x(&mut self, s: &mut Storage, scale: f64) -> Result<f64, Self::Error> {
        assert_eq!(scale, 0.5);
        self.event(s, "x".into())?;
        Ok(f64::from_bits(
            self.config()["coordinates"][0].as_u64().unwrap(),
        ))
    }
    fn random_y(&mut self, s: &mut Storage) -> Result<f64, Self::Error> {
        self.event(s, "y".into())?;
        Ok(f64::from_bits(
            self.config()["coordinates"][1].as_u64().unwrap(),
        ))
    }
    fn random_z(&mut self, s: &mut Storage, scale: f64) -> Result<f64, Self::Error> {
        assert_eq!(scale, 0.5);
        self.event(s, "z".into())?;
        Ok(f64::from_bits(
            self.config()["coordinates"][2].as_u64().unwrap(),
        ))
    }
    fn add_particle(
        &mut self,
        s: &mut Storage,
        level: i32,
        particle: i32,
        p: [f64; 3],
    ) -> Result<(), Self::Error> {
        let bits = |v: f64| {
            if v.is_nan() {
                0x7ff8000000000000
            } else {
                v.to_bits()
            }
        };
        self.event(
            s,
            format!(
                "particle {level} {particle} {} {} {}",
                bits(p[0]),
                bits(p[1]),
                bits(p[2])
            ),
        )
    }
}
fn run<R: RandomSource>(f: Value, out: &mut dyn Write, make: fn(i64) -> R) {
    let random = std::array::from_fn(|i| make(f["seeds"][i].as_i64().unwrap()));
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
        random,
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
            "tick" => s.tick(&mut w).map(|_| "ok".into()).map_err(|e| match e {
                TickError::Callback(e) => e,
                TickError::Iterator(CursorError::ConcurrentModification) => "concurrent",
                other => panic!("{other:?}"),
            }),
            "apply-status" | "remove-status" | "clear-status" | "sound" => {
                let list = op["list"].as_array().unwrap();
                let effect: ConsumeEffect<Key, u8> = match op["op"].as_str().unwrap() {
                    "apply-status" => ConsumeEffect::ApplyStatus {
                        effects: list
                            .iter()
                            .map(|i| w.pool[i.as_u64().unwrap() as usize].clone())
                            .collect(),
                        probability: fl(&op, "probability"),
                    },
                    "remove-status" => ConsumeEffect::RemoveStatus(
                        list.iter()
                            .map(|i| w.keys[i.as_u64().unwrap() as usize].clone())
                            .collect(),
                    ),
                    "clear-status" => ConsumeEffect::ClearStatus,
                    _ => ConsumeEffect::PlaySound(n(&op, "sound") as u8),
                };
                effect
                    .apply(
                        &mut StoredConsume {
                            effects: &mut s,
                            world: &mut w,
                        },
                        n(&op, "level"),
                    )
                    .map(|v| v.to_string())
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
    for (i, r) in w.random.iter_mut().enumerate() {
        writeln!(w.out, "random {i} {} {}", r.next_long(), r.next_long()).unwrap();
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let side = crate::args().nth(2).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join(format!("rust-{side}.txt")))?);
    for line in fs::read_to_string(dir.join(format!("inputs-{side}.txt")))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        if f["xor"].as_bool().unwrap() {
            run(f, &mut out, XoroshiroRandom::new);
        } else {
            run(f, &mut out, LegacyRandom::new);
        }
    }
    Ok(())
}
