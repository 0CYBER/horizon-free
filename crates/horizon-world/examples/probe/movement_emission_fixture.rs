use crate::support::common::{f64_bits, position};
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::movement_emission::*;
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
fn vector(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn pos(p: BlockPos) -> String {
    format!("{} {} {}", p.x, p.y, p.z)
}
fn fbits(v: f32) -> u32 {
    if v.is_nan() { 0x7fc00000 } else { v.to_bits() }
}
fn emission(v: &Value) -> MovementEmission {
    [
        MovementEmission::None,
        MovementEmission::Sounds,
        MovementEmission::Events,
        MovementEmission::All,
    ][v.as_u64().unwrap() as usize]
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    distances: MovementDistances,
}
impl World<'_> {
    fn flag(&self, n: &str) -> bool {
        self.f[n].as_bool().unwrap()
    }
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.f["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected emission failure"))
        } else {
            Ok(())
        }
    }
    fn mutate(&mut self) {
        if self.flag("mutate") {
            self.distances = MovementDistances {
                movement: single(&self.f["mutated_move"]),
                flight: single(&self.f["mutated_fly"]),
                next_step: single(&self.f["mutated_next"]),
            };
        }
    }
    fn predicate(&mut self, n: &str, key: &str, s: usize) -> io::Result<bool> {
        self.query(format!("{n} {s}"))?;
        Ok(self.f["states"][s][key].as_bool().unwrap())
    }
    fn boolean(&mut self, n: &str) -> io::Result<bool> {
        self.query(n.into())?;
        Ok(self.flag(n))
    }
}
impl MovementEmissionWorld for World<'_> {
    type Error = io::Error;
    type State = usize;
    fn distances(&self) -> MovementDistances {
        self.distances
    }
    fn set_movement_distance(&mut self, v: f32) {
        self.distances.movement = v;
    }
    fn set_flight_distance(&mut self, v: f32) {
        self.distances.flight = v;
    }
    fn set_next_step(&mut self, v: f32) {
        self.distances.next_step = v;
    }
    fn on_position(&mut self) -> io::Result<BlockPos> {
        self.query("on-pos".into())?;
        Ok(position(&self.f["on_pos"]))
    }
    fn block_state(&mut self, p: BlockPos) -> io::Result<usize> {
        self.query(format!("state {}", pos(p)))?;
        Ok(1)
    }
    fn is_air(&mut self, s: usize) -> io::Result<bool> {
        self.predicate("air", "air", s)
    }
    fn climbable_tag(&mut self, s: usize) -> io::Result<bool> {
        self.predicate("climbable", "climbable", s)
    }
    fn is_powder_snow(&mut self, s: usize) -> io::Result<bool> {
        self.predicate("powder", "powder", s)
    }
    fn on_ground(&mut self) -> io::Result<bool> {
        self.boolean("ground")
    }
    fn crouching(&mut self) -> io::Result<bool> {
        self.boolean("crouch")
    }
    fn on_rails(&mut self) -> io::Result<bool> {
        self.boolean("rails")
    }
    fn swimming(&mut self) -> io::Result<bool> {
        self.boolean("swimming")
    }
    fn in_water(&mut self) -> io::Result<bool> {
        self.boolean("water")
    }
    fn walking_step_sound(&mut self, p: BlockPos, s: usize) -> io::Result<()> {
        self.query(format!("walking {} {s}", pos(p)))?;
        self.mutate();
        self.query(format!("crystal {s}"))
    }
    fn step_event(&mut self, s: usize) -> io::Result<()> {
        let p = vector(&self.f["position"]);
        self.query(format!(
            "step {s} {:016x} {:016x} {:016x}",
            f64_bits(p.x),
            f64_bits(p.y),
            f64_bits(p.z)
        ))?;
        self.mutate();
        Ok(())
    }
    fn next_step(&mut self) -> io::Result<f32> {
        self.query("next-step".into())?;
        Ok(self.distances.next_step())
    }
    fn water_swim_sound(&mut self) -> io::Result<()> {
        self.query("water-sound".into())?;
        self.mutate();
        Ok(())
    }
    fn entity_event(&mut self, e: MovementEvent) -> io::Result<()> {
        self.query(format!(
            "event {}",
            match e {
                MovementEvent::Step => "step",
                MovementEvent::Swim => "swim",
                MovementEvent::Flap => "flap",
            }
        ))?;
        self.mutate();
        Ok(())
    }
    fn flapping(&mut self) -> io::Result<bool> {
        self.boolean("flapping")
    }
    fn on_flap(&mut self) -> io::Result<()> {
        self.query("on-flap".into())?;
        self.mutate();
        Ok(())
    }
    fn movement_emission(&mut self) -> io::Result<MovementEmission> {
        self.query("emission".into())?;
        Ok(emission(&self.f["flap_emission"]))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            distances: MovementDistances {
                movement: single(&f["move"]),
                flight: single(&f["fly"]),
                next_step: single(&f["next"]),
            },
        };
        let p = position(&f["legacy"]);
        let m = vector(&f["movement"]);
        if w.flag("block") {
            let sounds = w.flag("sounds");
            let events = w.flag("events");
            match block_movement_effects(&mut w, p, 0, sounds, events, m) {
                Ok(v) => writeln!(w.out, "result {v}")?,
                Err(_) => writeln!(w.out, "error")?,
            }
        } else {
            match apply_movement_emission(&mut w, emission(&f["emission"]), m, p, 0) {
                Ok(()) => writeln!(w.out, "ok")?,
                Err(_) => writeln!(w.out, "error")?,
            }
        }
        writeln!(
            w.out,
            "distances {:08x} {:08x} {:08x}",
            fbits(w.distances.movement),
            fbits(w.distances.flight),
            fbits(w.distances.next_step)
        )?;
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
