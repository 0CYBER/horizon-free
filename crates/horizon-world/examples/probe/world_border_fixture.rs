use crate::support::shapes::{emit, values};
use horizon_core::geometry::{
    Aabb, Vec3,
    border::{BorderGeometry, BorderStatus},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut border = BorderGeometry::new([0.0, 0.0], 59999968.0, 29999984)?;
        let mut limit = 29999984;
        for op in f["ops"].as_array().unwrap() {
            match op[0].as_str().unwrap() {
                "center" => border.set_center([number(&op[1]), number(&op[2])])?,
                "size" => border.set_size(number(&op[1]))?,
                "limit" => {
                    limit = op[1].as_i64().unwrap() as i32;
                    border.set_absolute_max(limit)?;
                }
                "lerp" => border.lerp_size_between(
                    number(&op[1]),
                    number(&op[2]),
                    op[3].as_i64().unwrap(),
                )?,
                "tick" => {
                    for _ in 0..op[1].as_u64().unwrap() {
                        border.tick()?;
                    }
                }
                _ => panic!("unknown border operation"),
            }
            let status = match border.status() {
                BorderStatus::Stationary => "STATIONARY",
                BorderStatus::Growing => "GROWING",
                BorderStatus::Shrinking => "SHRINKING",
            };
            writeln!(out, "status {status} {} {limit}", border.lerp_time())?;
            let [x, z] = border.center();
            values(
                &mut out,
                "state",
                [
                    x,
                    z,
                    border.size(),
                    border.lerp_target(),
                    border.lerp_speed(),
                ],
            )?;
            for partial in f["partials"].as_array().unwrap() {
                values(
                    &mut out,
                    "bounds",
                    border.bounds(f32::from_bits(u32::from_str_radix(
                        partial.as_str().unwrap(),
                        16,
                    )?)),
                )?;
            }
            emit(&mut out, &border.collision_shape()?)?;
            for q in f["queries"].as_array().unwrap() {
                let x = number(&q[0]);
                let z = number(&q[1]);
                let margin = number(&q[2]);
                let b = Aabb::new(
                    Vec3::new(number(&q[3]), number(&q[4]), number(&q[5])),
                    Vec3::new(number(&q[6]), number(&q[7]), number(&q[8])),
                );
                values(&mut out, "distance", [border.distance_to_border(x, z)])?;
                writeln!(
                    out,
                    "inside {} {}",
                    border.is_within_bounds(x, z, margin),
                    border.is_inside_close_to_border(x, z, b)
                )?;
            }
        }
    }
    Ok(())
}
