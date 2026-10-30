//! Transport for shared cave/canyon geometry and ordered output failures.
use crate::support::common::f64_bits;
use horizon_core::{
    carving::{CarverOutput, CarvingMask, can_reach, carve_ellipsoid},
    position::ChunkPos,
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn d(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
struct Output<'a> {
    mask: &'a mut CarvingMask,
    trace: &'a std::cell::RefCell<Vec<String>>,
    calls: i32,
    fail: i32,
}
impl CarverOutput for Output<'_> {
    type Error = ();
    fn min_y(&self) -> i32 {
        self.mask.min_y()
    }
    fn max_y(&self) -> i32 {
        self.mask.max_y()
    }
    fn carve(&mut self, x: i32, y: i32, z: i32) -> Result<(), ()> {
        self.trace.borrow_mut().push(format!("cell {x} {y} {z}"));
        let previous = self.calls;
        self.calls += 1;
        if previous == self.fail {
            return Err(());
        }
        self.mask.carve(x, y, z).unwrap();
        Ok(())
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let chunk = ChunkPos::new(n(&f["chunk"][0]), n(&f["chunk"][1]));
        let mut mask = CarvingMask::new(n(&f["min"]), n(&f["max"]))?;
        for e in f["ellipsoids"].as_array().unwrap() {
            let trace = std::cell::RefCell::new(Vec::new());
            let mut output = Output {
                mask: &mut mask,
                trace: &trace,
                calls: 0,
                fail: n(&e["fail"]),
            };
            let result = carve_ellipsoid(
                chunk,
                [d(&e["center"][0]), d(&e["center"][1]), d(&e["center"][2])],
                d(&e["horizontal"]),
                d(&e["vertical"]),
                &mut output,
                |[x, y, z], by| {
                    let skip = match n(&e["skip"]) {
                        0 => x * x + y * y + z * z >= 1.,
                        1 => y <= -0.7 || x * x + y * y + z * z >= 1.,
                        2 => by.rem_euclid(3) == 0,
                        _ => false,
                    };
                    trace.borrow_mut().push(format!(
                        "skip {:x} {:x} {:x} {by} {skip}",
                        f64_bits(x),
                        f64_bits(y),
                        f64_bits(z)
                    ));
                    skip
                },
            );
            let calls = output.calls;
            for event in trace.into_inner() {
                writeln!(out, "{event}")?;
            }
            writeln!(
                out,
                "ellipsoid {} {calls}",
                if result.is_ok() { "ok" } else { "error" }
            )?;
            for c in mask.columns() {
                writeln!(out, "column {} {} {} {}", c.x, c.z, c.min_y, c.max_y)?;
            }
        }
    }
    for line in fs::read_to_string(dir.join("reach.txt"))?.lines() {
        let q: Value = serde_json::from_str(line)?;
        let thickness = f32::from_bits(u32::from_str_radix(q[6].as_str().unwrap(), 16)?);
        writeln!(
            out,
            "reach {}",
            can_reach(
                ChunkPos::new(n(&q[0]), n(&q[1])),
                d(&q[2]),
                d(&q[3]),
                n(&q[4]),
                n(&q[5]),
                thickness
            )
        )?;
    }
    Ok(())
}
