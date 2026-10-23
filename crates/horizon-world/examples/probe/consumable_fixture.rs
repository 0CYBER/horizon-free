use crate::support::common::f64_bits;
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::consumable::{Consumable, EmissionWorld, UseAnimation};
use horizon_world::{
    component_patch::Prototype,
    item_particles::{self, ItemParticleWorld, ParticleError},
    item_stack_refs::{ItemStacks, StackRef},
    item_template::ItemTemplate,
};
use horizon_world::{
    consumable::{ConsumeResult, ConsumptionWorld},
    food::FoodData,
    food_properties::{ConsumptionSound, FoodConsumptionWorld, FoodProperties, SoundCategory},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone, Debug, PartialEq)]
enum Component {
    Damage(i32),
    Food(FoodProperties),
}
type Stack = StackRef<u8, u8, Component>;
type Template = ItemTemplate<u8, u8, Component>;
fn n(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
fn fl(v: &Value, k: &str) -> f32 {
    f32::from_bits(v[k].as_u64().unwrap() as u32)
}
fn opt(v: Option<&Component>) -> String {
    match v {
        None => "none".into(),
        Some(Component::Damage(v)) => v.to_string(),
        _ => panic!("not damage"),
    }
}
fn fbits(v: f32) -> u32 {
    if v.is_nan() { 0x7fc00000 } else { v.to_bits() }
}
fn food_component(c: &Value) -> Option<Component> {
    let f = &c["food_component"];
    if f.is_null() {
        None
    } else {
        Some(Component::Food(FoodProperties {
            nutrition: n(f, "nutrition"),
            saturation: fl(f, "saturation"),
            can_always_eat: f["always"].as_bool().unwrap(),
        }))
    }
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
    food: FoodData,
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
            .set(0, c["damage"].as_i64().map(|v| Component::Damage(v as i32)))
            .unwrap();
        s.components.set(1, food_component(&c)).unwrap();
        self.food = FoodData {
            food: n(&c, "food"),
            saturation: fl(&c, "saturation"),
            exhaustion: 0.0,
            timer: 0,
        };
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
    fn food_state(&mut self) {
        writeln!(
            self.out,
            "food {} {}",
            self.food.food,
            fbits(self.food.saturation)
        )
        .unwrap();
    }
    fn event(&mut self, name: String) -> Result<(), &'static str> {
        writeln!(self.out, "event {name}").unwrap();
        let i = self.calls;
        self.calls += 1;
        self.all();
        self.food_state();
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
impl<R: RandomSource> ItemParticleWorld<u8, u8, Component> for World<'_, R> {
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
impl<R: RandomSource> EmissionWorld for World<'_, R> {
    type Error = &'static str;
    type Stack = Stack;
    type Sound = u8;
    type Random = usize;
    type Source = R;
    fn random_source(&mut self, r: usize) -> &mut R {
        &mut self.random[r]
    }
    fn spawn_item_particles(&mut self, s: Stack, count: i32) -> Result<(), Self::Error> {
        assert!(s.same_reference(&self.stack));
        self.event(format!("spawn {count}"))?;
        item_particles::spawn(self, &s, 0, count).map_err(|e| match e {
            ParticleError::World(e) => e,
            ParticleError::Template(_) => "template",
        })
    }
    fn overrides_consume_sound(&self) -> bool {
        self.f["override"].as_bool().unwrap()
    }
    fn consume_sound(&mut self, s: Stack) -> Result<u8, Self::Error> {
        assert!(s.same_reference(&self.stack));
        self.event("override".into())?;
        Ok(n(self.config(), "sound") as u8)
    }
    fn play_sound(&mut self, s: u8, volume: f32, pitch: f32) -> Result<(), Self::Error> {
        self.event(format!(
            "sound {s} {} {}",
            volume.to_bits(),
            pitch.to_bits()
        ))
    }
}
impl<R: RandomSource> FoodConsumptionWorld for World<'_, R> {
    type Error = &'static str;
    type Level = i32;
    type Sound = u8;
    type Random = usize;
    type Source = R;
    fn get_random(&mut self) -> Result<usize, Self::Error> {
        self.event("random".into())?;
        Ok(n(self.config(), "virtual_random") as usize)
    }
    fn random_source(&mut self, r: usize) -> &mut R {
        &mut self.random[r]
    }
    fn position(&self) -> Vec3 {
        self.position
    }
    fn is_player(&self) -> bool {
        n(&self.f, "actor") != 0
    }
    fn food_data(&mut self) -> Result<&mut FoodData, Self::Error> {
        self.event("food-data".into())?;
        Ok(&mut self.food)
    }
    fn play_sound(&mut self, s: ConsumptionSound<i32, u8>) -> Result<(), Self::Error> {
        let category = match s.category {
            SoundCategory::Neutral => 6,
            SoundCategory::Players => 7,
        };
        self.event(format!(
            "food-sound {} {} {category} {} {} {} {} {}",
            s.level,
            s.sound,
            f64_bits(s.position.x),
            f64_bits(s.position.y),
            f64_bits(s.position.z),
            fbits(s.volume),
            fbits(s.pitch)
        ))
    }
}
impl<R: RandomSource> ConsumptionWorld<u8, u8, Component, u8> for World<'_, R> {
    type Level = i32;
    type Hand = i32;
    fn food(&self, s: &Stack) -> Option<FoodProperties> {
        match s.read().component(&1) {
            Some(Component::Food(f)) => Some(*f),
            _ => None,
        }
    }
    fn is_player(&self) -> bool {
        n(&self.f, "actor") != 0
    }
    fn can_eat(&mut self, always: bool) -> Result<bool, Self::Error> {
        self.event(format!("can-eat {always}"))?;
        Ok(self.config()["can_eat"].as_bool().unwrap())
    }
    fn start_using(&mut self, hand: i32) -> Result<(), Self::Error> {
        self.event(format!("start {hand}"))
    }
    fn level(&mut self) -> Result<i32, Self::Error> {
        ItemParticleWorld::level(self)
    }
    fn get_random(&mut self) -> Result<usize, Self::Error> {
        FoodConsumptionWorld::get_random(self)
    }
    fn is_server_player(&self) -> bool {
        n(&self.f, "actor") == 2
    }
    fn award_item_used(&mut self, item: u8) -> Result<(), Self::Error> {
        self.event(format!("stat {item}"))
    }
    fn consume_criterion(&mut self, s: Stack) -> Result<(), Self::Error> {
        assert!(s.same_reference(&self.stack));
        self.event("criterion".into())
    }
    fn consume_listeners(
        &mut self,
        level: i32,
        s: Stack,
        c: &Consumable<u8, u8>,
    ) -> Result<(), Self::Error> {
        if let Some(food) = ConsumptionWorld::food(self, &s) {
            food.on_consume(self, level, c.sound, 1)?;
        }
        Ok(())
    }
    fn is_client(&mut self, level: i32) -> Result<bool, Self::Error> {
        self.event(format!("client {level}"))?;
        Ok(self.config()["client"].as_bool().unwrap())
    }
    fn apply_effect(&mut self, e: &u8, level: i32, s: Stack) -> Result<bool, Self::Error> {
        assert!(s.same_reference(&self.stack));
        self.event(format!("effect {level} {e}"))?;
        Ok(self.config()["effect_result"].as_bool().unwrap())
    }
    fn consume_event(&mut self, drink: bool) -> Result<(), Self::Error> {
        self.event(format!("game-event {drink}"))
    }
    fn infinite_materials(&mut self) -> Result<bool, Self::Error> {
        self.event("infinite".into())?;
        Ok(self.config()["infinite"].as_bool().unwrap())
    }
}
fn run<R: RandomSource>(f: Value, out: &mut dyn Write, make: fn(i64) -> R) {
    let random = std::array::from_fn(|i| make(f["seeds"][i].as_i64().unwrap()));
    let stack = ItemStacks::new(0).create(n(&f, "item") as u8, 1, Prototype::new([]));
    let mut w = World {
        f,
        out,
        stack: stack.clone(),
        random,
        position: Vec3::ZERO,
        changed: false,
        calls: 0,
        first: None,
        food: FoodData::default(),
    };
    w.reset();
    w.all();
    let animation = [
        UseAnimation::None,
        UseAnimation::Eat,
        UseAnimation::Drink,
        UseAnimation::Block,
        UseAnimation::Bow,
        UseAnimation::Trident,
        UseAnimation::Crossbow,
        UseAnimation::Spyglass,
        UseAnimation::TootHorn,
        UseAnimation::Brush,
        UseAnimation::Bundle,
        UseAnimation::Spear,
    ][n(&w.f, "animation") as usize];
    let component = Consumable::<u8, u8> {
        seconds: fl(&w.f, "seconds"),
        animation,
        sound: n(&w.f, "sound") as u8,
        particles: w.f["has_particles"].as_bool().unwrap(),
        effects: (0..n(&w.f, "effects") as u8).collect(),
    };
    w.food_state();
    let op = n(&w.f, "op");
    let hand = n(&w.f, "hand");
    let level = n(&w.f, "passed_level");
    let result: Result<String, &'static str> = match op {
        0 => component
            .can_consume(&mut w, &stack)
            .map(|v| format!("can {v}")),
        1 => component
            .start(&mut w, stack.clone(), hand)
            .map(|v| match v {
                ConsumeResult::Fail => "fail".into(),
                ConsumeResult::Consume { transformed: None } => "consume none".into(),
                ConsumeResult::Consume {
                    transformed: Some(s),
                } => {
                    assert!(s.same_reference(&stack));
                    "consume input".into()
                }
            }),
        _ => component.on_consume(&mut w, level, stack.clone()).map(|s| {
            assert!(s.same_reference(&stack));
            "input".into()
        }),
    };
    writeln!(w.out, "result {}", result.unwrap_or_else(str::to_string)).unwrap();
    w.all();
    w.food_state();
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
