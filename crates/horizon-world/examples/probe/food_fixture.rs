use crate::support::common::f32_bits;
use horizon_world::food::{Difficulty, FoodData, FoodWorld};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn state(v: &Value) -> FoodData {
    FoodData {
        food: v["food"].as_i64().unwrap() as i32,
        saturation: single(&v["saturation"]),
        exhaustion: single(&v["exhaustion"]),
        timer: v["timer"].as_i64().unwrap() as i32,
    }
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    changed: bool,
}
impl World<'_> {
    fn query(&mut self, food: &mut FoodData, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected food callback failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            *food = state(&self.f["after"]);
            self.changed = true;
        }
        Ok(())
    }
    fn world(&self) -> &Value {
        &self.f[if self.changed { "after_world" } else { "world" }]
    }
}
impl FoodWorld for World<'_> {
    type Error = io::Error;
    type Level = ();
    fn level(&mut self, f: &mut FoodData) -> io::Result<()> {
        self.query(f, "level")
    }
    fn difficulty(&mut self, _: (), f: &mut FoodData) -> io::Result<Difficulty> {
        self.query(f, "difficulty")?;
        Ok([
            Difficulty::Peaceful,
            Difficulty::Easy,
            Difficulty::Normal,
            Difficulty::Hard,
        ][self.world()["difficulty"].as_u64().unwrap() as usize])
    }
    fn natural_regeneration(&mut self, _: (), f: &mut FoodData) -> io::Result<bool> {
        self.query(f, "rules")?;
        self.query(f, "regeneration")?;
        Ok(self.world()["regen"].as_bool().unwrap())
    }
    fn is_hurt(&mut self, f: &mut FoodData) -> io::Result<bool> {
        self.query(f, "is-hurt")?;
        Ok(self.world()["hurt"].as_bool().unwrap())
    }
    fn heal(&mut self, f: &mut FoodData, v: f32) -> io::Result<()> {
        self.query(f, &format!("heal {:08x}", f32_bits(v)))
    }
    fn health(&mut self, f: &mut FoodData) -> io::Result<f32> {
        self.query(f, "health")?;
        Ok(single(&self.world()["health"]))
    }
    fn hurt_starvation(&mut self, _: (), f: &mut FoodData, v: f32) -> io::Result<()> {
        self.query(f, "damage-sources")?;
        self.query(f, "starve-source")?;
        self.query(f, &format!("hurt {:08x}", f32_bits(v)))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut food = state(&f["initial"]);
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            changed: false,
        };
        let mut error = false;
        for op in f["ops"].as_array().unwrap() {
            match op["mode"].as_str().unwrap() {
                "eat" => food.eat(
                    op["nutrition"].as_i64().unwrap() as i32,
                    single(&op["amount"]),
                ),
                "properties" => food.eat_properties(
                    op["nutrition"].as_i64().unwrap() as i32,
                    single(&op["amount"]),
                ),
                "exhaust" => food.add_exhaustion(single(&op["amount"])),
                "food" => food.food = op["nutrition"].as_i64().unwrap() as i32,
                "saturation" => food.saturation = single(&op["amount"]),
                "tick" => {
                    if food.tick(&mut w).is_err() {
                        error = true;
                        break;
                    }
                }
                _ => panic!("mode"),
            }
            writeln!(
                w.out,
                "state {} {:08x} {:08x} {} {} {}",
                food.food,
                f32_bits(food.saturation),
                f32_bits(food.exhaustion),
                food.timer,
                food.has_enough_food(),
                food.needs_food()
            )?;
        }
        writeln!(w.out, "result {}", if error { "error" } else { "ok" })?;
        writeln!(
            w.out,
            "final {} {:08x} {:08x} {}",
            food.food,
            f32_bits(food.saturation),
            f32_bits(food.exhaustion),
            food.timer
        )?;
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
