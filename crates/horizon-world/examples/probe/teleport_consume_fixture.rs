use crate::support::common::f64_bits;
use horizon_core::{
    geometry::Vec3,
    position::BlockPos,
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::teleport_consume::{TeleportConsume, TeleportConsumeWorld};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn n(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
struct World<'a, R> {
    f: Value,
    out: &'a mut dyn Write,
    random: [R; 2],
    pos: Vec3,
    fall: f64,
    grace: i32,
    impact: bool,
    riding: bool,
    changed: bool,
    calls: i32,
    attempts: i32,
}
impl<R: RandomSource> World<'_, R> {
    fn config(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn reset(&mut self) {
        let c = self.config().clone();
        let p = &c["position"];
        self.pos = Vec3::new(
            p[0].as_f64().unwrap(),
            p[1].as_f64().unwrap(),
            p[2].as_f64().unwrap(),
        );
        self.fall = c["fall"].as_f64().unwrap();
        self.grace = n(&c, "grace");
        self.impact = c["impact"].as_bool().unwrap();
        self.riding = c["riding"].as_bool().unwrap();
        if c["reseed"].as_bool().unwrap() {
            for i in 0..2 {
                self.random[i].set_seed(c["seeds"][i].as_i64().unwrap());
            }
        }
    }
    fn all(&mut self) {
        writeln!(
            self.out,
            "state {} {} {} {} {} {} {} {} {} {}",
            self.calls,
            self.attempts,
            self.changed,
            self.riding,
            f64_bits(self.pos.x),
            f64_bits(self.pos.y),
            f64_bits(self.pos.z),
            f64_bits(self.fall),
            self.grace,
            self.impact
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
impl<R: RandomSource> TeleportConsumeWorld for World<'_, R> {
    type Error = &'static str;
    type Level = i32;
    type Random = usize;
    type Source = R;
    fn position_field(&self) -> Vec3 {
        self.pos
    }
    fn get_random(&mut self) -> Result<usize, Self::Error> {
        self.event("random".into())?;
        Ok(n(self.config(), "random") as usize)
    }
    fn random_source(&mut self, r: usize) -> &mut R {
        &mut self.random[r]
    }
    fn min_y(&mut self, l: i32) -> Result<i32, Self::Error> {
        self.event(format!("min {l}"))?;
        Ok(n(self.config(), "min"))
    }
    fn logical_height(&mut self, l: i32) -> Result<i32, Self::Error> {
        self.event(format!("height {l}"))?;
        Ok(n(self.config(), "height"))
    }
    fn is_passenger(&mut self) -> Result<bool, Self::Error> {
        self.event("passenger".into())?;
        Ok(self.riding)
    }
    fn stop_riding(&mut self) -> Result<(), Self::Error> {
        self.event("dismount".into())?;
        self.riding = false;
        Ok(())
    }
    fn position(&mut self) -> Result<Vec3, Self::Error> {
        self.event("position".into())?;
        Ok(self.pos)
    }
    fn random_teleport(&mut self, p: Vec3) -> Result<bool, Self::Error> {
        let i = self.attempts;
        self.attempts += 1;
        self.event(format!(
            "teleport {} {} {}",
            f64_bits(p.x),
            f64_bits(p.y),
            f64_bits(p.z)
        ))?;
        let at = n(self.config(), "success_at");
        let success = at >= 0 && i >= at;
        if self.config()["move"].as_bool().unwrap() {
            self.pos = p;
        }
        Ok(success)
    }
    fn teleport_event(&mut self, l: i32, p: Vec3) -> Result<(), Self::Error> {
        self.event(format!(
            "game-event {l} {} {} {}",
            f64_bits(p.x),
            f64_bits(p.y),
            f64_bits(p.z)
        ))
    }
    fn is_fox(&self) -> bool {
        self.f["fox"].as_bool().unwrap()
    }
    fn teleport_sound(&mut self, l: i32, p: Vec3, fox: bool) -> Result<(), Self::Error> {
        self.event(format!(
            "sound {l} {fox} {} {} {}",
            f64_bits(p.x),
            f64_bits(p.y),
            f64_bits(p.z)
        ))
    }
    fn block_position(&mut self) -> Result<BlockPos, Self::Error> {
        self.event("block".into())?;
        Ok(BlockPos::new(
            self.pos.x.floor() as i32,
            self.pos.y.floor() as i32,
            self.pos.z.floor() as i32,
        ))
    }
    fn directional_particles(
        &mut self,
        l: i32,
        p: BlockPos,
        offset: i32,
    ) -> Result<(), Self::Error> {
        self.event(format!("level-event {l} {} {} {} {offset}", p.x, p.y, p.z))
    }
    fn reset_fall_distance(&mut self) -> Result<(), Self::Error> {
        self.event("fall".into())?;
        self.fall = 0.0;
        Ok(())
    }
    fn reset_impulse(&mut self) -> Result<(), Self::Error> {
        self.event("impulse".into())?;
        self.grace = 0;
        self.impact = false;
        Ok(())
    }
}
fn run<R: RandomSource>(f: Value, out: &mut dyn Write, make: fn(i64) -> R) {
    let random = std::array::from_fn(|i| make(f["seeds"][i].as_i64().unwrap()));
    let effect = TeleportConsume {
        diameter: f32::from_bits(f["diameter"].as_u64().unwrap() as u32),
        directional_particles: f["directional"].as_bool().unwrap(),
    };
    let l = n(&f, "level");
    let mut w = World {
        f,
        out,
        random,
        pos: Vec3::ZERO,
        fall: 0.0,
        grace: 0,
        impact: false,
        riding: false,
        changed: false,
        calls: 0,
        attempts: 0,
    };
    w.reset();
    w.all();
    let result = effect.apply(&mut w, l);
    writeln!(
        w.out,
        "result {}",
        result.map(|v| v.to_string()).unwrap_or_else(str::to_string)
    )
    .unwrap();
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
