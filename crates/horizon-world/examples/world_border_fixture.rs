use horizon_core::geometry::{
    Aabb, Axis, Vec3, VoxelShape,
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
