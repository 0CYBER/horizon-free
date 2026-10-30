use crate::support::common::f32_bits;
use horizon_world::{
    food::FoodData,
    player_food::{self, PlayerFoodWorld, StoredPlayerFood},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    changed: bool,
    foods: [FoodData; 2],
}
impl World<'_> {
    fn state(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn reset(&mut self) {
        for i in 0..2 {
            let v = &self.state()["foods"][i];
            self.foods[i] = FoodData {
                food: v["food"].as_i64().unwrap() as i32,
                exhaustion: single(&v["exhaustion"]),
                ..FoodData::default()
            };
        }
    }
    fn index(&self, k: &str) -> usize {
        self.state()[k].as_u64().unwrap() as usize
    }
    fn query(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected food guard failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
            self.reset();
        }
        Ok(())
    }
}
impl PlayerFoodWorld for World<'_> {
    type Error = io::Error;
    fn invulnerable_direct(&self) -> bool {
        self.state()["invulnerable"].as_bool().unwrap()
    }
    fn level_is_client(&mut self) -> io::Result<bool> {
        self.query("level")?;
        self.query("client")?;
        Ok(self.state()["client"].as_bool().unwrap())
    }
    fn add_exhaustion_direct(&mut self, v: f32) -> io::Result<()> {
        let i = self.index("direct");
        self.query(&format!("exhaust {i} {:08x}", f32_bits(v)))?;
        self.foods[i].add_exhaustion(v);
        Ok(())
    }
    fn needs_food_direct(&mut self) -> io::Result<bool> {
        let i = self.index("direct");
        self.query(&format!("needs {i}"))?;
        Ok(self.foods[i].needs_food())
    }
    fn enough_food_virtual(&mut self) -> io::Result<bool> {
        self.query("food")?;
        let i = self.index("virtual");
        self.query(&format!("enough {i}"))?;
        Ok(self.foods[i].has_enough_food())
    }
    fn may_fly_virtual(&mut self) -> io::Result<bool> {
        self.query("abilities")?;
        Ok(self.state()["mayfly"].as_bool().unwrap())
    }
}
fn run<W: PlayerFoodWorld>(w: &mut W, f: &Value) -> Result<Option<bool>, W::Error> {
    match f["mode"].as_str().unwrap() {
        "exhaust" => player_food::cause_exhaustion(w, single(&f["amount"])).map(|()| None),
        "eat" => player_food::can_eat(w, f["always"].as_bool().unwrap()).map(Some),
        "enough" => player_food::enough_food_for_manoeuvres(w).map(Some),
        _ => panic!("mode"),
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            changed: false,
            foods: [FoodData::default(); 2],
        };
        w.reset();
        let result = if f["stored"].as_bool().unwrap() {
            let s = &f["initial"];
            let mut stored = StoredPlayerFood {
                food: &mut w.foods[0],
                invulnerable: s["invulnerable"].as_bool().unwrap(),
                client_side: s["client"].as_bool().unwrap(),
                may_fly: s["mayfly"].as_bool().unwrap(),
            };
            Ok(run(&mut stored, &f).unwrap())
        } else {
            run(&mut w, &f)
        };
        match result {
            Ok(v) => {
                if let Some(v) = v {
                    writeln!(w.out, "bool {v}")?;
                }
                writeln!(w.out, "result ok")?;
            }
            Err(_) => writeln!(w.out, "result error")?,
        }
        for (i, food) in w.foods.iter().enumerate() {
            writeln!(
                w.out,
                "food {i} {} {:08x}",
                food.food,
                f32_bits(food.exhaustion)
            )?;
        }
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
