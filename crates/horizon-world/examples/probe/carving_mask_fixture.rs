//! Transport for native carving-mask traversal and callback mutation behavior.
use horizon_core::carving::CarvingMask;
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut mask = CarvingMask::new(n(&f["min"]), n(&f["max"]))?;
        writeln!(out, "mask {} {}", mask.min_y(), mask.max_y())?;
        for op in f["ops"].as_array().unwrap() {
            match op["kind"].as_str().unwrap() {
                "carve" => {
                    let q = &op["pos"];
                    let ok = mask.carve(n(&q[0]), n(&q[1]), n(&q[2])).is_ok();
                    writeln!(out, "carve {}", if ok { "ok" } else { "error" })?;
                }
                "fill" => {
                    let b = &op["box"];
                    let mut calls = 0;
                    for x in n(&b[0])..=n(&b[1]) {
                        for z in n(&b[4])..=n(&b[5]) {
                            for y in n(&b[2])..=n(&b[3]) {
                                mask.carve(x, y, z)?;
                                calls += 1;
                            }
                        }
                    }
                    writeln!(out, "fill {calls}")?;
                }
                "visit" => {
                    let mut calls = 0;
                    let result = mask.try_visit_mut(|mask, c| {
                        writeln!(out, "column {} {} {} {}", c.x, c.z, c.min_y, c.max_y).unwrap();
                        let previous = calls;
                        calls += 1;
                        if let Some(points) = op["add"].get(previous.to_string()) {
                            for q in points.as_array().unwrap() {
                                let ok = mask.carve(n(&q[0]), n(&q[1]), n(&q[2])).is_ok();
                                writeln!(out, "add {}", if ok { "ok" } else { "error" }).unwrap();
                            }
                        }
                        if previous == n(&op["fail"]) {
                            Err(())
                        } else {
                            Ok(())
                        }
                    });
                    writeln!(
                        out,
                        "visit {} {calls}",
                        if result.is_ok() { "ok" } else { "error" }
                    )?;
                }
                "read" => {
                    for c in mask.columns() {
                        writeln!(out, "column {} {} {} {}", c.x, c.z, c.min_y, c.max_y)?;
                    }
                    writeln!(out, "read")?;
                }
                _ => unreachable!(),
            }
            writeln!(out, "empty {}", mask.is_empty())?;
        }
    }
    Ok(())
}
