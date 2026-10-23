use crate::support::common::f64_bits;
use crate::support::shapes::bounds;
use horizon_core::{
    geometry::{Aabb, ShapeOp, Vec3, VoxelShape},
    position::BlockPos,
};
use horizon_world::{
    block_entity_push::{BlockPushWorld, push_entities_up},
    entity_obstruction::EntityIdentity,
    registry::BlockStateId,
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
fn bbox(v: &Value) -> Aabb {
    Aabb::new(
        Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])),
        Vec3::new(number(&v[3]), number(&v[4]), number(&v[5])),
    )
}
fn shape(v: &Value) -> Result<VoxelShape, Box<dyn std::error::Error>> {
    let mut s = VoxelShape::empty();
    for b in v.as_array().unwrap() {
        s = s.join(&VoxelShape::from_box(bbox(b))?, ShapeOp::OR)?;
    }
    Ok(s)
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    shapes: [VoxelShape; 2],
    entities: Vec<Aabb>,
}
impl World<'_> {
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.f["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected push failure"))
        } else {
            Ok(())
        }
    }
}
impl BlockPushWorld for World<'_> {
    type Error = io::Error;
    fn collision_shape(&mut self, s: BlockStateId, p: BlockPos) -> io::Result<VoxelShape> {
        self.query(format!("shape {} {} {} {}", s.0, p.x, p.y, p.z))?;
        Ok(self.shapes[s.0 as usize].clone())
    }
    fn entities(&mut self, b: Aabb) -> io::Result<Vec<EntityIdentity>> {
        self.query(bounds("entities", b))?;
        Ok((0..self.entities.len())
            .map(|i| EntityIdentity(i as u64))
            .collect())
    }
    fn entity_bounds(&self, e: EntityIdentity) -> Aabb {
        self.entities[e.0 as usize]
    }
    fn teleport_relative(&mut self, e: EntityIdentity, v: Vec3) -> io::Result<()> {
        self.query(format!(
            "teleport {} {:016x} {:016x} {:016x}",
            e.0,
            f64_bits(v.x),
            f64_bits(v.y),
            f64_bits(v.z)
        ))?;
        let index = e.0 as usize;
        self.entities[index] = self.entities[index].move_by(v);
        if self.f["mutate"].as_bool().unwrap() && index + 1 < self.entities.len() {
            self.entities[index + 1] = self.entities[index + 1].move_by(Vec3::new(0.0, 0.125, 0.0));
        }
        Ok(())
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let p = &f["position"];
        let pos = BlockPos::new(
            p[0].as_i64().unwrap() as i32,
            p[1].as_i64().unwrap() as i32,
            p[2].as_i64().unwrap() as i32,
        );
        let mut world = World {
            f: &f,
            out: &mut out,
            calls: 0,
            shapes: [shape(&f["old"])?, shape(&f["new"])?],
            entities: f["entities"].as_array().unwrap().iter().map(bbox).collect(),
        };
        match push_entities_up(&mut world, BlockStateId(0), BlockStateId(1), pos) {
            Ok(state) => writeln!(world.out, "result {}", state == BlockStateId(1))?,
            Err(_) => writeln!(world.out, "error")?,
        };
        for (i, b) in world.entities.iter().enumerate() {
            writeln!(world.out, "{}", bounds(&format!("final {i}"), *b))?;
        }
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
