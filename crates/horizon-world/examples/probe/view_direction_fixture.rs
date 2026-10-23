use crate::support::common::f64_bits;
use horizon_core::geometry::Vec3;
use horizon_world::view_direction;
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn vec(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        match f["mode"].as_str().unwrap() {
            "acos" => writeln!(
                out,
                "angle {:016x}",
                f64_bits(view_direction::acos(number(&f["value"])))
            )?,
            "view" => {
                let v = view_direction::view_vector(single(&f["pitch"]), single(&f["yaw"]));
                writeln!(
                    out,
                    "view {:016x} {:016x} {:016x}",
                    f64_bits(v.x),
                    f64_bits(v.y),
                    f64_bits(v.z)
                )?;
            }
            "blocking" => writeln!(
                out,
                "angle {:016x}",
                f64_bits(view_direction::blocking_angle(
                    vec(&f["source"]),
                    vec(&f["position"]),
                    single(&f["yaw"])
                ))
            )?,
            "fallback" => writeln!(
                out,
                "angle {:016x}",
                f64_bits(view_direction::BLOCKING_FALLBACK_ANGLE)
            )?,
            _ => panic!("mode"),
        }
    }
    Ok(())
}
