use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::{
    food::FoodData,
    food_properties::{ConsumptionSound, FoodConsumptionWorld, FoodProperties, SoundCategory},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn n(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
fn fl(v: &Value, k: &str) -> f32 {
    f32::from_bits(v[k].as_u64().unwrap() as u32)
}
fn bits(f: f32) -> u32 {
    if f.is_nan() { 0x7fc00000 } else { f.to_bits() }
}
struct World<'a, R> {
    f: Value,
    out: &'a mut dyn Write,
    foods: [FoodData; 2],
    random: [R; 2],
    position: Vec3,
    changed: bool,
    calls: i32,
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
        for i in 0..2 {
            let f = &c["foods"][i];
            self.foods[i] = FoodData {
                food: n(f, "food"),
                saturation: fl(f, "saturation"),
                exhaustion: fl(f, "exhaustion"),
                timer: n(f, "timer"),
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
            "state {} {} {} {}",
            self.calls,
            self.changed,
            n(self.config(), "random"),
            n(self.config(), "food")
        )
        .unwrap();
        for (i, f) in self.foods.iter().enumerate() {
            writeln!(
                self.out,
                "food {i} {} {} {} {}",
                f.food,
                bits(f.saturation),
                bits(f.exhaustion),
                f.timer
            )
            .unwrap();
        }
    }
    fn event(&mut self, name: String) -> Result<(), &'static str> {
        writeln!(self.out, "event {name}").unwrap();
        let index = self.calls;
        self.calls += 1;
        self.all();
        if index == n(&self.f, "fail") {
            return Err("callback");
        }
        if index == n(&self.f, "mutate") {
            self.changed = true;
            self.reset();
        }
        Ok(())
    }
}
impl<R: RandomSource> FoodConsumptionWorld for World<'_, R> {
    type Error = &'static str;
    type Level = ();
    type Sound = u8;
    type Random = usize;
    type Source = R;
    fn get_random(&mut self) -> Result<usize, Self::Error> {
        self.event("random".into())?;
        Ok(n(self.config(), "random") as usize)
    }
    fn random_source(&mut self, r: usize) -> &mut R {
        &mut self.random[r]
    }
    fn position(&self) -> Vec3 {
        self.position
    }
    fn is_player(&self) -> bool {
        self.f["player"].as_bool().unwrap()
    }
    fn food_data(&mut self) -> Result<&mut FoodData, Self::Error> {
        self.event("food-data".into())?;
        let i = n(self.config(), "food") as usize;
        Ok(&mut self.foods[i])
    }
    fn play_sound(&mut self, s: ConsumptionSound<(), u8>) -> Result<(), Self::Error> {
        let category = match s.category {
            SoundCategory::Neutral => 6,
            SoundCategory::Players => 7,
        };
        self.event(format!(
            "sound {} {} {} {} {} {} {}",
            s.sound,
            category,
            s.position.x.to_bits(),
            s.position.y.to_bits(),
            s.position.z.to_bits(),
            bits(s.volume),
            bits(s.pitch)
        ))
    }
}
fn run<R: RandomSource>(f: Value, out: &mut dyn Write, make: fn(i64) -> R) {
    let random = std::array::from_fn(|i| make(f["seeds"][i].as_i64().unwrap()));
    let properties = FoodProperties {
        nutrition: n(&f, "nutrition"),
        saturation: fl(&f, "saturation"),
        can_always_eat: f["always"].as_bool().unwrap(),
    };
    let mut w = World {
        f,
        out,
        foods: [FoodData::default(); 2],
        random,
        position: Vec3::ZERO,
        changed: false,
        calls: 0,
    };
    w.reset();
    w.all();
    let result = properties.on_consume(&mut w, (), 0, 1);
    writeln!(w.out, "result {}", result.err().unwrap_or("ok")).unwrap();
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
