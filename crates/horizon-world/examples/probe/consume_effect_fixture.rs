use horizon_core::{
    position::BlockPos,
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::{
    consume_effect::{ConsumeEffect, EffectWorld},
    status_effect::{BlendState, Details, EffectInstance},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
type Instance = EffectInstance<u8>;
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
fn desc(e: &Instance) -> String {
    let e = e.read();
    format!(
        "{} {} {} {} {} {} {} {} {}",
        e.effect,
        e.details.duration,
        e.details.amplifier,
        e.details.ambient,
        e.details.visible,
        e.details.show_icon,
        e.hidden.is_some(),
        bits(e.blend.factor),
        bits(e.blend.previous)
    )
}
struct World<'a, R> {
    f: Value,
    out: &'a mut dyn Write,
    random: [R; 2],
    inputs: Vec<Instance>,
    added: Vec<Instance>,
    changed: bool,
    calls: i32,
}
impl<R: RandomSource> World<'_, R> {
    fn config(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn reset(&mut self) {
        let c = self.config().clone();
        for (i, v) in c["instances"].as_array().unwrap().iter().enumerate() {
            let mut e = self.inputs[i].write();
            e.details = details(v);
            e.blend = BlendState {
                factor: fl(v, "factor"),
                previous: fl(v, "previous"),
            };
        }
        if c["reseed"].as_bool().unwrap() {
            for i in 0..2 {
                self.random[i].set_seed(c["seeds"][i].as_i64().unwrap());
            }
        }
    }
    fn all(&mut self) {
        writeln!(
            self.out,
            "state {} {} {}",
            self.calls,
            self.changed,
            n(self.config(), "random")
        )
        .unwrap();
        for (i, e) in self.inputs.iter().enumerate() {
            writeln!(self.out, "input {i} {}", desc(e)).unwrap();
        }
        for (i, e) in self.added.iter().enumerate() {
            writeln!(self.out, "added {i} {}", desc(e)).unwrap();
        }
    }
    fn event(&mut self, name: String) -> Result<bool, &'static str> {
        writeln!(self.out, "event {name}").unwrap();
        let i = self.calls;
        self.calls += 1;
        self.all();
        if i == n(&self.f, "fail") {
            return Err("callback");
        }
        if i == n(&self.f, "mutate") {
            self.changed = true;
            self.reset();
        }
        let r = self.config()["returns"].as_array().unwrap();
        Ok(r[i as usize % r.len()].as_bool().unwrap())
    }
}
impl<R: RandomSource> EffectWorld<u8, u8, Vec<u8>> for World<'_, R> {
    type Error = &'static str;
    type Level = i32;
    type Random = usize;
    type Source = R;
    type SoundCategory = i32;
    fn get_random(&mut self) -> Result<usize, Self::Error> {
        self.event("random".into())?;
        Ok(n(self.config(), "random") as usize)
    }
    fn random_source(&mut self, r: usize) -> &mut R {
        &mut self.random[r]
    }
    fn add_effect(&mut self, e: Instance) -> Result<bool, Self::Error> {
        let fresh = !self
            .inputs
            .iter()
            .chain(self.added.iter())
            .any(|v| v.same_reference(&e));
        let result = self.event(format!("add {fresh} {}", desc(&e)))?;
        self.added.push(e);
        Ok(result)
    }
    fn effect_set(&self, set: &Vec<u8>) -> Vec<u8> {
        set.clone()
    }
    fn remove_effect(&mut self, e: u8) -> Result<bool, Self::Error> {
        self.event(format!("remove {e}"))
    }
    fn remove_all_effects(&mut self) -> Result<bool, Self::Error> {
        self.event("clear".into())
    }
    fn block_position(&mut self) -> Result<BlockPos, Self::Error> {
        self.event("position".into())?;
        let p = &self.config()["position"];
        Ok(BlockPos::new(
            p[0].as_i64().unwrap() as i32,
            p[1].as_i64().unwrap() as i32,
            p[2].as_i64().unwrap() as i32,
        ))
    }
    fn sound_category(&mut self) -> Result<i32, Self::Error> {
        self.event("category".into())?;
        Ok(n(self.config(), "category"))
    }
    fn play_sound(&mut self, level: i32, p: BlockPos, s: u8, c: i32) -> Result<(), Self::Error> {
        self.event(format!("sound {level} {} {} {} {s} {c}", p.x, p.y, p.z))?;
        Ok(())
    }
}
fn run<R: RandomSource>(f: Value, out: &mut dyn Write, make: fn(i64) -> R) {
    let random = std::array::from_fn(|i| make(f["seeds"][i].as_i64().unwrap()));
    let inputs = (0..3)
        .map(|i| {
            let d = details(&f["initial"]["instances"][i]);
            Instance::new(i as u8, d, Some(Instance::new(i as u8, d, None)))
        })
        .collect();
    let mut w = World {
        f,
        out,
        random,
        inputs,
        added: Vec::new(),
        changed: false,
        calls: 0,
    };
    w.reset();
    w.all();
    let commands = w.f["ops"].as_array().unwrap().clone();
    for op in commands {
        let list = op["list"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect::<Vec<_>>();
        let effect: ConsumeEffect<u8, u8> = match n(&op, "kind") {
            0 => ConsumeEffect::ApplyStatus {
                effects: list.iter().map(|&i| w.inputs[i as usize].clone()).collect(),
                probability: fl(&op, "probability"),
            },
            1 => ConsumeEffect::RemoveStatus(list),
            2 => ConsumeEffect::ClearStatus,
            _ => ConsumeEffect::PlaySound(n(&op, "sound") as u8),
        };
        let result = effect.apply(&mut w, n(&op, "level"));
        writeln!(
            w.out,
            "result {}",
            result.map(|b| b.to_string()).unwrap_or_else(str::to_string)
        )
        .unwrap();
        w.all();
    }
    for (i, r) in w.random.iter_mut().enumerate() {
        writeln!(w.out, "random {i} {} {}", r.next_long(), r.next_long()).unwrap();
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        if f["xor"].as_bool().unwrap() {
            run(f, &mut out, XoroshiroRandom::new);
        } else {
            run(f, &mut out, LegacyRandom::new);
        }
    }
    Ok(())
}
