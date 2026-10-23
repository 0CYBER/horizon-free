use crate::support::common::f64_bits;
use horizon_core::geometry::Vec3;
use horizon_world::piston_movement::PistonMovement;
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
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let d = vector(&f["deltas"]);
        let mut state = PistonMovement {
            game_time: f["time"].as_i64().unwrap(),
            deltas: [d.x, d.y, d.z],
        };
        for op in f["ops"].as_array().unwrap() {
            let mut calls = 0;
            let result = state.limit(vector(&op["movement"]), || -> io::Result<i64> {
                writeln!(out, "time")?;
                calls += 1;
                if op["fail"].as_bool().unwrap() {
                    Err(io::Error::other("injected time failure"))
                } else {
                    Ok(op["time"].as_i64().unwrap())
                }
            });
            match result {
                Ok(v) => writeln!(
                    out,
                    "movement {:016x} {:016x} {:016x}",
                    f64_bits(v.x),
                    f64_bits(v.y),
                    f64_bits(v.z)
                )?,
                Err(_) => writeln!(out, "error")?,
            }
            writeln!(
                out,
                "deltas {:016x} {:016x} {:016x}",
                f64_bits(state.deltas[0]),
                f64_bits(state.deltas[1]),
                f64_bits(state.deltas[2])
            )?;
            writeln!(out, "tick {}", state.game_time)?;
            writeln!(out, "queries {calls}")?;
        }
    }
    Ok(())
}
