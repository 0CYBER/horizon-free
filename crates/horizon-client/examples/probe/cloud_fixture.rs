//! CloudRenderer face selection and mesh traversal, without a GPU.
use horizon_client::clouds;
use std::{fmt::Write, fs};
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let mut output = String::new();
    for (case, line) in fs::read_to_string(&args[1])?.lines().enumerate() {
        let values: Vec<i32> = line
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        let [kind, x, z, bottom, neighbors] = values.as_slice() else {
            panic!("invalid case")
        };
        write!(output, "{case}")?;
        let mut cell = |x, z, neighbors| {
            let empty = std::array::from_fn(|i| neighbors & (1 << i) != 0);
            for (direction, inside) in clouds::faces(clouds::face_mask(x, z, *bottom as f32, empty))
            {
                write!(output, " {x},{z},{direction},{}", u8::from(inside)).unwrap();
            }
        };
        let radius = if *kind == 2 {
            clouds::radius_cells(*x)
        } else {
            *x
        };
        if *kind == 0 {
            cell(*x, *z, *neighbors);
        } else {
            for (x, z) in clouds::cells(radius) {
                cell(x, z, 0);
            }
        }
        output.push('\n');
    }
    fs::write(&args[2], output)?;
    Ok(())
}
