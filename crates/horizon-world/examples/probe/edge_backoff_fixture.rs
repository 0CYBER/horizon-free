use crate::support::common::f64_bits;
use horizon_core::geometry::{Aabb, Axis, Vec3};
use horizon_world::edge_backoff::{EdgeBackoffInput, EdgeBackoffWorld, back_off_from_edge};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn vector(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
struct World<'a> {
    input: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    probes: usize,
}
impl World<'_> {
    fn query(&mut self, text: String) -> io::Result<()> {
        writeln!(self.out, "{text}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.input["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected edge query failure"))
        } else {
            Ok(())
        }
    }
}
impl EdgeBackoffWorld for World<'_> {
    type Error = io::Error;
    fn max_up_step(&mut self) -> io::Result<f32> {
        self.query("step".into())?;
        Ok(f32::from_bits(
            u32::from_str_radix(self.input["step"].as_str().unwrap(), 16).unwrap(),
        ))
    }
    fn staying_on_ground_surface(&mut self) -> io::Result<bool> {
        self.query("staying".into())?;
        Ok(self.input["staying"].as_bool().unwrap())
    }
    fn on_ground(&mut self) -> io::Result<bool> {
        self.query("ground".into())?;
        Ok(self.input["ground"].as_bool().unwrap())
    }
    fn no_collision(&mut self, b: Aabb) -> io::Result<bool> {
        self.query(format!(
            "clear {:016x} {:016x} {:016x} {:016x} {:016x} {:016x}",
            f64_bits(b.min(Axis::X)),
            f64_bits(b.min(Axis::Y)),
            f64_bits(b.min(Axis::Z)),
            f64_bits(b.max(Axis::X)),
            f64_bits(b.max(Axis::Y)),
            f64_bits(b.max(Axis::Z))
        ))?;
        let p = self.probes;
        self.probes += 1;
        Ok(self.input["clear"]
            .as_array()
            .unwrap()
            .get(p)
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let b = &f["box"];
        let bounds = Aabb::new(
            Vec3::new(number(&b[0]), number(&b[1]), number(&b[2])),
            Vec3::new(number(&b[3]), number(&b[4]), number(&b[5])),
        );
        let mut world = World {
            input: &f,
            out: &mut out,
            calls: 0,
            probes: 0,
        };
        let result = back_off_from_edge(
            &mut world,
            EdgeBackoffInput {
                movement: vector(&f["movement"]),
                bounds,
                flying: f["flying"].as_bool().unwrap(),
                self_or_player: matches!(f["mover"].as_str().unwrap(), "SELF" | "PLAYER"),
                fall_distance: number(&f["fall"]),
                query_budget: 10000,
            },
        );
        match result {
            Ok(v) => writeln!(
                world.out,
                "movement {:016x} {:016x} {:016x}",
                f64_bits(v.x),
                f64_bits(v.y),
                f64_bits(v.z)
            )?,
            Err(_) => writeln!(world.out, "error")?,
        };
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
