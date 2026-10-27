use horizon_client::{
    Result,
    cushion::{self, World},
};
use horizon_core::geometry::{Aabb, ShapeOp, Vec3, VoxelShape};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

struct Cell {
    outline: VoxelShape,
    collision: VoxelShape,
    suffocating: bool,
}
struct Fixture(HashMap<[i32; 3], Cell>);
impl World for Fixture {
    fn outline_bounds(&self, p: [i32; 3]) -> Option<Aabb> {
        self.0
            .get(&p)?
            .outline
            .bounds()
            .map(|b| b.move_by(Vec3::new(p[0] as f64, p[1] as f64, p[2] as f64)))
    }
    fn collision_shapes(&self, _: Aabb) -> Vec<VoxelShape> {
        self.0
            .iter()
            .map(|(p, c)| {
                c.collision
                    .moved(Vec3::new(p[0] as f64, p[1] as f64, p[2] as f64))
            })
            .collect()
    }
    fn suffocating(&self, p: [i32; 3]) -> bool {
        self.0.get(&p).is_some_and(|c| c.suffocating)
    }
}
fn shape(value: &Value) -> Result<VoxelShape> {
    let mut shape = VoxelShape::empty();
    for b in value.as_array().unwrap() {
        let f = |i: usize| b[i].as_f64().unwrap();
        shape = shape.join(
            &VoxelShape::from_box(Aabb::new(
                Vec3::new(f(0), f(1), f(2)),
                Vec3::new(f(3), f(4), f(5)),
            ))?,
            ShapeOp::OR,
        )?;
    }
    Ok(shape)
}
pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().collect();
    let mut out = BufWriter::new(File::create(&args[2])?);
    for line in BufReader::new(File::open(&args[1])?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let mut world = Fixture(HashMap::new());
        for c in row["cells"].as_array().unwrap() {
            let p = std::array::from_fn(|i| c[i].as_i64().unwrap() as i32);
            world.0.insert(
                p,
                Cell {
                    outline: shape(&c[3])?,
                    collision: shape(&c[4])?,
                    suffocating: c[5].as_bool().unwrap(),
                },
            );
        }
        let p = &row["pos"];
        let (x, y, z) = (
            p[0].as_f64().unwrap(),
            p[1].as_f64().unwrap(),
            p[2].as_f64().unwrap(),
        );
        let bounds = Aabb::new(
            Vec3::new(x - 0.5, y, z - 0.5),
            Vec3::new(x + 0.5, y + 0.25, z + 0.5),
        );
        writeln!(
            out,
            "{} {}",
            cushion::can_place(&world, bounds),
            cushion::survives(&world, bounds)
        )?;
    }
    Ok(())
}
