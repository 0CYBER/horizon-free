//! Shapes and boxes printed the same way by the collision probes.
use super::common::f64_bits;
use horizon_core::geometry::{Aabb, Axis, VoxelShape};
use std::io::{self, Write};

pub fn emit(out: &mut impl Write, shape: &VoxelShape) -> io::Result<()> {
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

/// `label` and each value's bits in hex, NaN as Java's canonical one.
pub fn values(
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

pub fn box_values(out: &mut impl Write, label: &str, b: Aabb) -> io::Result<()> {
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

/// `label` and the box's six bounds' bits in hex.
pub fn bounds(label: &str, b: Aabb) -> String {
    format!(
        "{label} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x}",
        f64_bits(b.min(Axis::X)),
        f64_bits(b.min(Axis::Y)),
        f64_bits(b.min(Axis::Z)),
        f64_bits(b.max(Axis::X)),
        f64_bits(b.max(Axis::Y)),
        f64_bits(b.max(Axis::Z))
    )
}
