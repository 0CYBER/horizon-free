use horizon_core::geometry::{Aabb, Axis, Direction, ShapeOp, Vec3, VoxelShape};
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
fn values(
    out: &mut impl Write,
    label: &str,
    values: impl IntoIterator<Item = f64>,
) -> io::Result<()> {
    write!(out, "{label}")?;
    for v in values {
        let bits = if v.is_nan() {
            0x7ff8_0000_0000_0000
        } else {
            v.to_bits()
        };
        write!(out, " {bits:016x}")?;
    }
    writeln!(out)
}
fn box_values(out: &mut impl Write, label: &str, b: Aabb) -> io::Result<()> {
    values(
        out,
        label,
        [
            b.min(Axis::X),
            b.min(Axis::Y),
            b.min(Axis::Z),
            b.max(Axis::X),
            b.max(Axis::Y),
            b.max(Axis::Z),
        ],
    )
}
fn emit(out: &mut impl Write, shape: &VoxelShape) -> io::Result<()> {
    writeln!(
        out,
        "shape {} {} {}",
        shape.is_empty(),
        shape.boxes().len(),
        shape.is_cube()
    )?;
    for (axis, name) in Axis::ALL.into_iter().zip(["x", "y", "z"]) {
        values(out, name, shape.coordinates(axis).iter().copied())?;
    }
    let axes = shape.cube_coordinate_axes();
    writeln!(
        out,
        "axes {}",
        u8::from(axes[0]) + 2 * u8::from(axes[1]) + 4 * u8::from(axes[2])
    )?;
    values(
        out,
        "extrema",
        Axis::ALL
            .map(|a| shape.min(a))
            .into_iter()
            .chain(Axis::ALL.map(|a| shape.max(a))),
    )?;
    match shape.bounds() {
        Some(b) => box_values(out, "bounds", b)?,
        None => writeln!(out, "bounds none")?,
    }
    for b in shape.boxes() {
        box_values(out, "box", *b)?;
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).expect("private fixture directory");
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
