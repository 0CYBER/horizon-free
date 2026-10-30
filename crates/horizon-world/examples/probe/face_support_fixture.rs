use crate::support::shapes::emit;
use horizon_core::geometry::{Aabb, Direction, ShapeOp, Vec3, VoxelShape};
use horizon_world::block_support::SupportType;
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn numbers(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap()))
        .collect()
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut shape = if f.get("coords").is_some() {
            let coords = std::array::from_fn(|i| numbers(&f["coords"][i]));
            let occupied = f["occupied"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_bool().unwrap())
                .collect::<Vec<_>>();
            if f["cube"].as_bool().unwrap() {
                VoxelShape::from_cube(coords.map(|v| v.len() - 1), &occupied)?
            } else {
                VoxelShape::from_grid(coords, &occupied)?
            }
        } else {
            let mut s = VoxelShape::empty();
            for v in f["boxes"].as_array().unwrap() {
                let v = numbers(v);
                s = s.join(
                    &VoxelShape::from_box(Aabb::new(
                        Vec3::new(v[0], v[1], v[2]),
                        Vec3::new(v[3], v[4], v[5]),
                    ))?,
                    ShapeOp::OR,
                )?;
            }
            s
        };
        if let Some(v) = f.get("move") {
            let v = numbers(v);
            shape = shape.moved(Vec3::new(v[0], v[1], v[2]));
        }
        for direction in Direction::ALL {
            let face = shape.face_shape(direction)?;
            writeln!(
                out,
                "face {} {} {}",
                direction as u8,
                face.same_identity(&shape),
                face.same_identity(&shape.face_shape(direction)?)
            )?;
            emit(&mut out, &face)?;
            writeln!(
                out,
                "support {} {} {} 3",
                SupportType::Full.is_supporting(&shape, direction)?,
                SupportType::Center.is_supporting(&shape, direction)?,
                SupportType::Rigid.is_supporting(&shape, direction)?
            )?;
            for nested in Direction::ALL {
                let second = face.face_shape(nested)?;
                writeln!(
                    out,
                    "nested {} {}",
                    nested as u8,
                    second.same_identity(&face)
                )?;
                emit(&mut out, &second)?;
            }
        }
    }
    Ok(())
}
