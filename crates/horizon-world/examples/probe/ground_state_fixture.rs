use crate::support::common::f64_bits;
use horizon_core::{
    geometry::{Aabb, Axis, Vec3},
    position::BlockPos,
};
use horizon_world::ground_support::GroundState;
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
fn position(v: &Value) -> Option<BlockPos> {
    if v.is_null() {
        None
    } else {
        Some(BlockPos::new(
            v[0].as_i64().unwrap() as i32,
            v[1].as_i64().unwrap() as i32,
            v[2].as_i64().unwrap() as i32,
        ))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let initial = &f["initial"];
        let mut state = GroundState {
            on_ground: initial["grounded"].as_bool().unwrap(),
            horizontal_collision: initial["horizontal"].as_bool().unwrap(),
            on_ground_no_blocks: initial["empty"].as_bool().unwrap(),
            supporting_block: position(&initial["support"]),
        };
        for op in f["ops"].as_array().unwrap() {
            let v = &op["box"];
            let bounds = Aabb::new(
                Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])),
                Vec3::new(number(&v[3]), number(&v[4]), number(&v[5])),
            );
            let movement = if op["movement"].is_null() {
                None
            } else {
                Some(vector(&op["movement"]))
            };
            let mut calls = 0i32;
            let find = |b: Aabb| -> io::Result<Option<BlockPos>> {
                writeln!(
                    out,
                    "find {:016x} {:016x} {:016x} {:016x} {:016x} {:016x}",
                    f64_bits(b.min(Axis::X)),
                    f64_bits(b.min(Axis::Y)),
                    f64_bits(b.min(Axis::Z)),
                    f64_bits(b.max(Axis::X)),
                    f64_bits(b.max(Axis::Y)),
                    f64_bits(b.max(Axis::Z))
                )?;
                let call = calls;
                calls += 1;
                if call == op["fail"].as_i64().unwrap() as i32 {
                    return Err(io::Error::other("injected support query failure"));
                }
                Ok(position(&op["results"][call as usize]))
            };
            let ground = op["grounded"].as_bool().unwrap();
            let result = match op["mode"].as_u64().unwrap() {
                0 => state.check_support(ground, bounds, movement, find),
                1 => state.set_with_movement(
                    ground,
                    state.horizontal_collision,
                    bounds,
                    movement,
                    find,
                ),
                2 => state.set_with_movement(
                    ground,
                    op["horizontal"].as_bool().unwrap(),
                    bounds,
                    movement,
                    find,
                ),
                _ => panic!("unknown ground state mode"),
            };
            writeln!(
                out,
                "result {}",
                if result.is_ok() { "ok" } else { "error" }
            )?;
            writeln!(
                out,
                "flags {} {} {}",
                state.on_ground, state.horizontal_collision, state.on_ground_no_blocks
            )?;
            if let Some(p) = state.supporting_block {
                writeln!(out, "support {} {} {}", p.x, p.y, p.z)?;
            } else {
                writeln!(out, "support none")?;
            }
            writeln!(out, "queries {calls}")?;
        }
    }
    Ok(())
}
