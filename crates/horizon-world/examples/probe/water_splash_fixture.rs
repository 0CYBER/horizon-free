use crate::support::common::f64_bits;
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::water_splash::{self, SplashError, SplashParticle, WaterSplashWorld};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn vec(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn float_bits(v: f32) -> u32 {
    if v.is_nan() {
        f32::NAN.to_bits()
    } else {
        v.to_bits()
    }
}
fn vector(v: Vec3) -> String {
    format!(
        "{:016x} {:016x} {:016x}",
        f64_bits(v.x),
        f64_bits(v.y),
        f64_bits(v.z)
    )
}
struct World<'a, R> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    position: Vec3,
    width: f32,
    velocity: Vec3,
    other: Vec3,
    rng: R,
}
impl<R> World<'_, R> {
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected splash failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.position = vec(&self.f["after_position"]);
            self.width = single(&self.f["after_width"]);
            self.velocity = vec(&self.f["after_velocity"]);
            self.other = vec(&self.f["after_other_velocity"]);
        }
        Ok(())
    }
}
impl<R: RandomSource> WaterSplashWorld for World<'_, R> {
    type Error = io::Error;
    type Entity = u8;
    type Sound = bool;
    fn this_entity(&self) -> u8 {
        0
    }
    fn controlling_passenger(&mut self) -> io::Result<Option<u8>> {
        self.query("passenger".into())?;
        Ok(match self.f["passenger"].as_u64().unwrap() {
            0 => None,
            1 => Some(0),
            2 => Some(1),
            _ => panic!("fixture passenger"),
        })
    }
    fn velocity(&mut self, e: u8) -> io::Result<Vec3> {
        self.query(format!(
            "velocity {}",
            if e == 0 { "self" } else { "other" }
        ))?;
        Ok(if e == 0 { self.velocity } else { self.other })
    }
    fn splash_sound(&mut self, high: bool) -> io::Result<bool> {
        self.query(format!("sound {}", if high { "high" } else { "low" }))?;
        Ok(high)
    }
    fn random_float(&mut self) -> io::Result<f32> {
        self.query("float".into())?;
        Ok(self.rng.next_float())
    }
    fn random_double(&mut self) -> io::Result<f64> {
        self.query("double".into())?;
        Ok(self.rng.next_double())
    }
    fn play_sound(&mut self, s: bool, volume: f32, pitch: f32) -> io::Result<()> {
        self.query(format!(
            "play {} {:08x} {:08x}",
            if s { "high" } else { "low" },
            float_bits(volume),
            float_bits(pitch)
        ))
    }
    fn position(&self) -> Vec3 {
        self.position
    }
    fn width(&self) -> f32 {
        self.width
    }
    fn particle(&mut self, k: SplashParticle, p: Vec3, v: Vec3) -> io::Result<()> {
        self.query(format!(
            "particle {} {} {}",
            match k {
                SplashParticle::Bubble => "bubble",
                SplashParticle::Splash => "splash",
            },
            vector(p),
            vector(v)
        ))
    }
    fn splash_event(&mut self) -> io::Result<()> {
        self.query("event splash".into())
    }
}
fn world<'a, R>(f: &'a Value, out: &'a mut dyn Write, rng: R) -> World<'a, R> {
    World {
        f,
        out,
        calls: 0,
        position: vec(&f["position"]),
        width: single(&f["width"]),
        velocity: vec(&f["velocity"]),
        other: vec(&f["other_velocity"]),
        rng,
    }
}
fn run<R: RandomSource>(f: &Value, out: &mut dyn Write, rng: R) -> io::Result<()> {
    let mut w = world(f, out, rng);
    match water_splash::water_splash(&mut w, 10000) {
        Ok(()) => writeln!(w.out, "result ok")?,
        Err(SplashError::World(_)) => writeln!(w.out, "error")?,
        Err(e) => panic!("unexpected fixture limit: {e}"),
    };
    writeln!(
        w.out,
        "final {} {:08x}",
        vector(w.position),
        float_bits(w.width)
    )?;
    writeln!(w.out, "next {}", w.rng.next_long())?;
    writeln!(w.out, "queries {}", w.calls)
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    let input = fs::read_to_string(dir.join("inputs.txt"))?;
    let first: Value = serde_json::from_str(input.lines().next().unwrap())?;
    let mut sink = io::sink();
    let mut probe = world(&first, &mut sink, LegacyRandom::new(42));
    assert!(matches!(
        water_splash::water_splash(&mut probe, 0),
        Err(SplashError::BudgetExceeded)
    ));
    assert_eq!(probe.calls, 6);
    let mut expected = LegacyRandom::new(42);
    expected.next_float();
    expected.next_float();
    assert_eq!(probe.rng.next_long(), expected.next_long());
    for line in input.lines() {
        let f: Value = serde_json::from_str(line)?;
        let seed = f["seed"].as_i64().unwrap();
        if f["legacy"].as_bool().unwrap() {
            run(&f, &mut out, LegacyRandom::new(seed))?
        } else {
            run(&f, &mut out, XoroshiroRandom::new(seed))?
        }
    }
    Ok(())
}
