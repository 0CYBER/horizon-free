use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::fluid_box::{self, BoatFluidBoxWorld, FluidBoxWorld};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn bounds(v: &Value) -> Aabb {
    Aabb::new(
        Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])),
        Vec3::new(number(&v[3]), number(&v[4]), number(&v[5])),
    )
}
fn show(b: Option<Aabb>) -> String {
    let Some(b) = b else { return "none".into() };
    b.min_position()
        .components()
        .into_iter()
        .chain(b.max_position().components())
        .map(|v| {
            format!(
                "{:016x}",
                if v.is_nan() {
                    f64::NAN.to_bits()
                } else {
                    v.to_bits()
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    actor: Aabb,
    boat: Aabb,
    status: i64,
}
impl World<'_> {
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected fluid box failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.actor = bounds(&self.f["after_actor"]);
            self.boat = bounds(&self.f["after_boat"]);
            self.status = self.f["after_status"].as_i64().unwrap();
        }
        Ok(())
    }
}
impl FluidBoxWorld for World<'_> {
    type Error = io::Error;
    type Vehicle = u8;
    fn bounding_box(&self) -> Aabb {
        self.actor
    }
    fn vehicle(&mut self) -> io::Result<Option<u8>> {
        self.query("vehicle".into())?;
        let v = self.f["vehicle"].as_u64().unwrap() as u8;
        Ok((v != 0).then_some(v))
    }
    fn modify_passenger_box(&mut self, vehicle: u8, b: Aabb) -> io::Result<Option<Aabb>> {
        match vehicle {
            1 => Ok(Some(b)),
            2 => fluid_box::boat_passenger_box(self, b),
            3 => {
                self.query(format!("modifier {}", show(Some(b))))?;
                Ok((!self.f["modified"].is_null()).then(|| bounds(&self.f["modified"])))
            }
            _ => panic!("vehicle fixture"),
        }
    }
}
impl BoatFluidBoxWorld for World<'_> {
    type Error = io::Error;
    fn is_under_water(&mut self) -> io::Result<bool> {
        self.query("boat-under".into())?;
        Ok(self.status == 1 || self.status == 2)
    }
    fn boat_bounds(&self) -> Aabb {
        self.boat
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
            actor: bounds(&f["actor"]),
            boat: bounds(&f["boat"]),
            status: f["status"].as_i64().unwrap(),
        };
        let result = if f["direct"].as_bool().unwrap() {
            fluid_box::boat_passenger_box(&mut w, bounds(&f["input"]))
        } else {
            fluid_box::interaction_box(&mut w)
        };
        match result {
            Ok(b) => writeln!(w.out, "result {}", show(b))?,
            Err(_) => writeln!(w.out, "error")?,
        }
        writeln!(w.out, "actor {}", show(Some(w.actor)))?;
        writeln!(w.out, "boat {}", show(Some(w.boat)))?;
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
