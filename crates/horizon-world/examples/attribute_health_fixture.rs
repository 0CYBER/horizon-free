use horizon_world::{
    attribute_health::{AttributeHealth, HealthError, HealthObserver},
    attributes::{
        Modifier, Operation,
        entity::{Catalog, EntityAttributes},
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
fn bits(v: f32) -> u32 {
    if v.is_nan() {
        f32::NAN.to_bits()
    } else {
        v.to_bits()
    }
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
        self.effect(&format!("updated {:08x}", bits(h.value())))
    }
}
struct World<'a> {
    attributes: EntityAttributes,
    health: SyncedFloat,
    absorption: f32,
    dead: bool,
    regen: bool,
    difficulty: Difficulty,
    observer: Observer<'a>,
}
impl<'a> World<'a> {
    fn binding(&mut self) -> AttributeHealth<'_, Observer<'a>> {
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
impl FoodWorld for World<'_> {
    type Error = HealthError<io::Error>;
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
            .effect(&format!("starve {:08x}", bits(v)))
            .map_err(HealthError::Observer)
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    let catalog = Arc::new(Catalog::from_json(&serde_json::from_str(
        &fs::read_to_string(dir.join("catalog.json"))?,
    )?)?);
    let keys = [
        catalog.id("minecraft:max_health").unwrap(),
        catalog.id("minecraft:max_absorption").unwrap(),
    ];
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut w = World {
            attributes: catalog.create(f["entity"].as_str().unwrap()).unwrap(),
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
        let t = &f["food"];
        let mut food = FoodData {
            food: t["food"].as_i64().unwrap() as i32,
            saturation: single(&t["saturation"]),
            exhaustion: single(&t["exhaustion"]),
            timer: t["timer"].as_i64().unwrap() as i32,
        };
        for op in f["ops"].as_array().unwrap() {
            let v = single(&op["value"]);
            let key = keys[op["attribute"].as_u64().unwrap() as usize];
            let result = match op["mode"].as_str().unwrap() {
                "base" => {
                    w.attributes
                        .edit(key, |a, o| a.set_base_value(number(&op["amount"]), o))
                        .unwrap()
                        .unwrap();
                    Ok(())
                }
                "add" => {
                    let m = Modifier::new(
                        op["id"].as_str().unwrap(),
                        number(&op["amount"]),
                        Operation::ALL[op["operation"].as_u64().unwrap() as usize],
                    )?;
                    w.attributes
                        .edit(key, |a, o| a.add_or_update_transient(m, o))
                        .unwrap()
                        .unwrap();
                    Ok(())
                }
                "remove" => {
                    w.attributes
                        .edit(key, |a, o| a.remove(op["id"].as_str().unwrap(), o))
                        .unwrap()
                        .unwrap();
                    Ok(())
                }
                "reset" => {
                    w.attributes.reset_base(key);
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
                        .map(|v| format!("{:08x}", bits(v)))
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
                bits(w.health.value()),
                w.health.is_dirty(),
                w.health.is_default(),
                bits(w.absorption),
                food.food,
                bits(food.saturation),
                bits(food.exhaustion),
                food.timer,
                w.observer.calls
            )?;
            writeln!(
                w.observer.out,
                "limits {:016x} {:016x}",
                double_bits(w.attributes.value(keys[0]).unwrap()),
                double_bits(w.attributes.value(keys[1]).unwrap())
            )?;
        }
    }
    Ok(())
}
