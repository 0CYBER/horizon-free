use crate::support::common::f32_bits;
use horizon_world::{
    attribute_health::{AttributeHealth, HealthAttributes, HealthError, HealthObserver},
    attributes::{
        Modifier, Operation,
        entity::{AttributeId, Catalog, EntityAttributes},
        live::{Key, LiveAttributes},
    },
    food::{Difficulty, FoodData, FoodWorld},
    living_health::SyncedFloat,
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn double_bits(v: f64) -> u64 {
    if v.is_nan() {
        f64::NAN.to_bits()
    } else {
        v.to_bits()
    }
}
struct Observer<'a> {
    out: &'a mut dyn Write,
    calls: i64,
    fail: i64,
}
impl Observer<'_> {
    fn effect(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.fail {
            Err(io::Error::other("injected effect failure"))
        } else {
            Ok(())
        }
    }
}
impl HealthObserver for Observer<'_> {
    type Error = io::Error;
    fn updated(&mut self, h: &mut SyncedFloat) -> io::Result<()> {
        self.effect(&format!("updated {:08x}", f32_bits(h.value())))
    }
}
// This fixture observes health/food values, not cross-attribute hash order.
// Live map/queue order is checked separately with actual JVM hash sequences.
trait TestAttributes: HealthAttributes<Error: std::fmt::Debug> + Sized {
    fn create(catalog: &Arc<Catalog>, entity: &str) -> Self;
    fn set_attribute(&mut self, key: Self::Key, value: f64);
    fn update_attribute(&mut self, key: Self::Key, modifier: Modifier);
    fn remove_attribute(&mut self, key: Self::Key, id: &str);
    fn reset_attribute(&mut self, key: Self::Key);
}
impl TestAttributes for EntityAttributes {
    fn create(catalog: &Arc<Catalog>, entity: &str) -> Self {
        catalog.create(entity).unwrap()
    }
    fn set_attribute(&mut self, key: AttributeId, value: f64) {
        self.edit(key, |a, o| a.set_base_value(value, o))
            .unwrap()
            .unwrap();
    }
    fn update_attribute(&mut self, key: AttributeId, modifier: Modifier) {
        self.edit(key, |a, o| a.add_or_update_transient(modifier, o))
            .unwrap()
            .unwrap();
    }
    fn remove_attribute(&mut self, key: AttributeId, id: &str) {
        self.edit(key, |a, o| a.remove(id, o)).unwrap().unwrap();
    }
    fn reset_attribute(&mut self, key: AttributeId) {
        self.reset_base(key);
    }
}
type Live = LiveAttributes<Box<dyn FnMut() -> i32>>;
impl TestAttributes for Live {
    fn create(catalog: &Arc<Catalog>, entity: &str) -> Self {
        let hashes: Vec<_> = (0..catalog.attributes().count())
            .map(|i| i as i32)
            .collect();
        let mut identity = 0;
        Self::new(
            catalog.clone(),
            entity,
            &hashes,
            Box::new(move || {
                identity += 1;
                identity
            }),
        )
        .unwrap()
    }
    fn set_attribute(&mut self, key: Key, value: f64) {
        let handle = self.get_instance(Some(key)).unwrap().unwrap();
        self.edit(&handle, |a, o| a.set_base_value(value, o))
            .unwrap();
    }
    fn update_attribute(&mut self, key: Key, modifier: Modifier) {
        let handle = self.get_instance(Some(key)).unwrap().unwrap();
        self.edit(&handle, |a, o| a.add_or_update_transient(modifier, o))
            .unwrap();
    }
    fn remove_attribute(&mut self, key: Key, id: &str) {
        let handle = self.get_instance(Some(key)).unwrap().unwrap();
        self.edit(&handle, |a, o| a.remove(id, o)).unwrap();
    }
    fn reset_attribute(&mut self, key: Key) {
        self.reset_base(Some(key)).unwrap();
    }
}
struct World<'a, A> {
    attributes: A,
    health: SyncedFloat,
    absorption: f32,
    dead: bool,
    regen: bool,
    difficulty: Difficulty,
    observer: Observer<'a>,
}
impl<'a, A: TestAttributes> World<'a, A> {
    fn binding(&mut self) -> AttributeHealth<'_, Observer<'a>, A> {
        AttributeHealth::new(
            &mut self.attributes,
            &mut self.health,
            &mut self.absorption,
            self.dead,
            &mut self.observer,
        )
        .unwrap()
    }
}
impl<A: TestAttributes> FoodWorld for World<'_, A> {
    type Error = HealthError<io::Error, A::Error>;
    type Level = ();
    fn level(&mut self, _: &mut FoodData) -> Result<(), Self::Error> {
        Ok(())
    }
    fn difficulty(&mut self, _: (), _: &mut FoodData) -> Result<Difficulty, Self::Error> {
        Ok(self.difficulty)
    }
    fn natural_regeneration(&mut self, _: (), _: &mut FoodData) -> Result<bool, Self::Error> {
        Ok(self.regen)
    }
    fn is_hurt(&mut self, _: &mut FoodData) -> Result<bool, Self::Error> {
        self.binding().is_hurt()
    }
    fn heal(&mut self, _: &mut FoodData, v: f32) -> Result<(), Self::Error> {
        self.binding().heal(v)
    }
    fn health(&mut self, _: &mut FoodData) -> Result<f32, Self::Error> {
        Ok(self.health.value())
    }
    fn hurt_starvation(&mut self, _: (), _: &mut FoodData, v: f32) -> Result<(), Self::Error> {
        self.observer
            .effect(&format!("starve {:08x}", f32_bits(v)))
            .map_err(HealthError::Observer)
    }
}
fn run<A: TestAttributes>(dir: &Path, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut out = io::BufWriter::new(fs::File::create(dir.join(output))?);
    let catalog = Arc::new(Catalog::from_json(&serde_json::from_str(
        &fs::read_to_string(dir.join("catalog.json"))?,
    )?)?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut w = World {
            attributes: A::create(&catalog, f["entity"].as_str().unwrap()),
            health: SyncedFloat::new(single(&f["health"])),
            absorption: single(&f["absorption"]),
            dead: f["dead"].as_bool().unwrap(),
            regen: f["regen"].as_bool().unwrap(),
            difficulty: [
                Difficulty::Peaceful,
                Difficulty::Easy,
                Difficulty::Normal,
                Difficulty::Hard,
            ][f["difficulty"].as_u64().unwrap() as usize],
            observer: Observer {
                out: &mut out,
                calls: 0,
                fail: f["fail"].as_i64().unwrap(),
            },
        };
        let keys = [
            w.attributes
                .health_attribute_key("minecraft:max_health")
                .unwrap(),
            w.attributes
                .health_attribute_key("minecraft:max_absorption")
                .unwrap(),
        ];
        let t = &f["food"];
        let mut food = FoodData {
            food: t["food"].as_i64().unwrap() as i32,
            saturation: single(&t["saturation"]),
            exhaustion: single(&t["exhaustion"]),
            timer: t["timer"].as_i64().unwrap() as i32,
        };
        for op in f["ops"].as_array().unwrap() {
            let v = single(&op["value"]);
            let key = keys[op["attribute"].as_u64().unwrap() as usize].clone();
            let result = match op["mode"].as_str().unwrap() {
                "base" => {
                    w.attributes.set_attribute(key, number(&op["amount"]));
                    Ok(())
                }
                "add" => {
                    let m = Modifier::new(
                        op["id"].as_str().unwrap(),
                        number(&op["amount"]),
                        Operation::ALL[op["operation"].as_u64().unwrap() as usize],
                    )?;
                    w.attributes.update_attribute(key, m);
                    Ok(())
                }
                "remove" => {
                    w.attributes
                        .remove_attribute(key, op["id"].as_str().unwrap());
                    Ok(())
                }
                "reset" => {
                    w.attributes.reset_attribute(key);
                    Ok(())
                }
                "set" => w.binding().set_health(v),
                "heal" => w.binding().heal(v),
                "absorption" => w.binding().set_absorption(v),
                "hurt" => w.binding().is_hurt().and_then(|v| {
                    writeln!(w.observer.out, "bool {v}").map_err(HealthError::Observer)
                }),
                "dead" => w.binding().is_dead_or_dying().and_then(|v| {
                    writeln!(w.observer.out, "bool {v}").map_err(HealthError::Observer)
                }),
                "tick" => food.tick(&mut w),
                "eat" => {
                    food.eat(op["nutrition"].as_i64().unwrap() as i32, v);
                    Ok(())
                }
                "exhaust" => {
                    food.add_exhaustion(v);
                    Ok(())
                }
                "pack" => writeln!(
                    w.observer.out,
                    "packed {}",
                    w.health
                        .take_dirty()
                        .map(|v| format!("{:08x}", f32_bits(v)))
                        .unwrap_or_else(|| "none".into())
                )
                .map_err(HealthError::Observer),
                _ => panic!("mode"),
            };
            writeln!(
                w.observer.out,
                "result {}",
                if result.is_ok() { "ok" } else { "error" }
            )?;
            writeln!(
                w.observer.out,
                "state {:08x} {} {} {:08x} {} {:08x} {:08x} {} {}",
                f32_bits(w.health.value()),
                w.health.is_dirty(),
                w.health.is_default(),
                f32_bits(w.absorption),
                food.food,
                f32_bits(food.saturation),
                f32_bits(food.exhaustion),
                food.timer,
                w.observer.calls
            )?;
            writeln!(
                w.observer.out,
                "limits {:016x} {:016x}",
                double_bits(
                    w.attributes
                        .health_attribute_value(keys[0].clone())
                        .unwrap()
                ),
                double_bits(
                    w.attributes
                        .health_attribute_value(keys[1].clone())
                        .unwrap()
                )
            )?;
        }
    }
    Ok(())
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    if crate::args().nth(2).as_deref() == Some("live") {
        run::<Live>(dir, "rust-live.txt")
    } else {
        run::<EntityAttributes>(dir, "rust.txt")
    }
}
