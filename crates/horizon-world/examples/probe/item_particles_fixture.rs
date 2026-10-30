use crate::support::common::f64_bits;
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::{
    component_patch::Prototype,
    item_particles::{self, ItemParticleWorld, ParticleError},
    item_stack_refs::{ItemStacks, StackRef},
    item_template::ItemTemplate,
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
type Stack = StackRef<u8, u8, i32>;
type Template = ItemTemplate<u8, u8, i32>;
fn n(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
fn fl(v: &Value, k: &str) -> f32 {
    f32::from_bits(v[k].as_u64().unwrap() as u32)
}
fn opt(v: Option<&i32>) -> String {
    v.map_or("none".into(), i32::to_string)
}
struct World<'a, R> {
    f: Value,
    out: &'a mut dyn Write,
    stack: Stack,
    random: [R; 2],
    position: Vec3,
    changed: bool,
    calls: i32,
    first: Option<Template>,
}
impl<R: RandomSource> World<'_, R> {
    fn config(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn reset(&mut self) {
        let c = self.config().clone();
        let p = &c["position"];
        self.position = Vec3::new(
            p[0].as_f64().unwrap(),
            p[1].as_f64().unwrap(),
            p[2].as_f64().unwrap(),
        );
        let mut s = self.stack.write();
        s.count = n(&c, "count");
        s.components
            .set(0, c["damage"].as_i64().map(|v| v as i32))
            .unwrap();
        if c["reseed"].as_bool().unwrap() {
            for i in 0..2 {
                self.random[i].set_seed(c["seeds"][i].as_i64().unwrap());
            }
        }
    }
    fn all(&mut self) {
        let s = self.stack.read();
        writeln!(
            self.out,
            "state {} {} {} {} {} {} {}",
            self.calls,
            self.changed,
            n(self.config(), "random"),
            n(self.config(), "level"),
            s.item(),
            s.count(),
            opt(s.component(&0))
        )
        .unwrap();
    }
    fn event(&mut self, name: String) -> Result<(), &'static str> {
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
        Ok(())
    }
}
impl<R: RandomSource> ItemParticleWorld<u8, u8, i32> for World<'_, R> {
    type Error = &'static str;
    type Level = i32;
    type Source = R;
    fn random(&mut self) -> &mut R {
        let i = n(self.config(), "random") as usize;
        &mut self.random[i]
    }
    fn pitch(&mut self) -> Result<f32, Self::Error> {
        self.event("pitch".into())?;
        Ok(fl(self.config(), "pitch"))
    }
    fn yaw(&mut self) -> Result<f32, Self::Error> {
        self.event("yaw".into())?;
        Ok(fl(self.config(), "yaw"))
    }
    fn position(&self) -> Vec3 {
        self.position
    }
    fn eye_y(&mut self) -> Result<f64, Self::Error> {
        self.event("eye".into())?;
        Ok(self.position.y + f64::from(fl(self.config(), "eye")))
    }
    fn level(&mut self) -> Result<i32, Self::Error> {
        self.event("level".into())?;
        Ok(n(self.config(), "level"))
    }
    fn add_particle(
        &mut self,
        level: i32,
        t: &Template,
        p: Vec3,
        v: Vec3,
    ) -> Result<(), Self::Error> {
        let same = self
            .first
            .as_ref()
            .is_none_or(|first| first.same_reference(t));
        if self.first.is_none() {
            self.first = Some(t.clone());
        }
        let damage = opt(t.component(&Prototype::new([]), &0));
        self.event(format!(
            "particle {level} {} {} {damage} {same} {} {} {} {} {} {}",
            t.item(),
            t.count(),
            f64_bits(p.x),
            f64_bits(p.y),
            f64_bits(p.z),
            f64_bits(v.x),
            f64_bits(v.y),
            f64_bits(v.z)
        ))
    }
}
fn run<R: RandomSource>(f: Value, out: &mut dyn Write, make: fn(i64) -> R) {
    let random = std::array::from_fn(|i| make(f["seeds"][i].as_i64().unwrap()));
    let stack = ItemStacks::new(0).create(n(&f, "item") as u8, 1, Prototype::new([]));
    let count = n(&f, "particles");
    let mut w = World {
        f,
        out,
        stack: stack.clone(),
        random,
        position: Vec3::ZERO,
        changed: false,
        calls: 0,
        first: None,
    };
    w.reset();
    w.all();
    let result = item_particles::spawn(&mut w, &stack, 0, count);
    let result = match result {
        Ok(()) => "ok",
        Err(ParticleError::World(e)) => e,
        Err(ParticleError::Template(_)) => "template",
    };
    writeln!(w.out, "result {result}").unwrap();
    w.all();
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
