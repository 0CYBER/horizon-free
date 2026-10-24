use horizon_core::geometry::{Aabb, Axis, ShapeOp, Vec3, VoxelShape};
use horizon_world::entity_obstruction::{
    EntityIdentity, EntityObstructionWorld, ObstructionEntity, is_unobstructed,
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
struct World<'a, W> {
    entities: Vec<ObstructionEntity>,
    out: &'a mut W,
    fail: bool,
    calls: usize,
}
impl<W: Write> EntityObstructionWorld for World<'_, W> {
    type Error = io::Error;
    fn obstruction_candidates(
        &mut self,
        excluded: Option<EntityIdentity>,
        bounds: Aabb,
    ) -> io::Result<Vec<ObstructionEntity>> {
        let values = [
            bounds.min(Axis::X),
            bounds.min(Axis::Y),
            bounds.min(Axis::Z),
            bounds.max(Axis::X),
            bounds.max(Axis::Y),
            bounds.max(Axis::Z),
        ]
        .map(f64::to_bits);
        writeln!(
            self.out,
            "query {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x}",
            excluded.map(|id| id.0 as i64).unwrap_or(-1),
            values[0],
            values[1],
            values[2],
            values[3],
            values[4],
            values[5]
        )?;
        self.calls += 1;
        if self.fail {
            return Err(io::Error::other("injected entity query failure"));
        }
        let selected = self
            .entities
            .iter()
            .filter(|e| Some(e.identity) != excluded && e.bounds.intersects(bounds) && !e.spectator)
            .copied()
            .collect::<Vec<_>>();
        for entity in &selected {
            writeln!(self.out, "picked {}", entity.identity.0)?;
        }
        Ok(selected)
    }
}
fn bbox(v: &Value) -> Aabb {
    let values = v
        .as_array()
        .unwrap()
        .iter()
        .map(|v| f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap()))
        .collect::<Vec<_>>();
    Aabb::new(
        Vec3::new(values[0], values[1], values[2]),
        Vec3::new(values[3], values[4], values[5]),
    )
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut entities = Vec::<ObstructionEntity>::new();
        for value in f["entities"].as_array().unwrap() {
            let identity = EntityIdentity(entities.len() as u64 + 1);
            let parent = value["parent"].as_i64().unwrap();
            let root_vehicle = if parent < 0 {
                identity
            } else {
                entities[parent as usize].root_vehicle
            };
            entities.push(ObstructionEntity {
                identity,
                root_vehicle,
                bounds: bbox(&value["box"]),
                spectator: value["spectator"].as_bool().unwrap(),
                removed: value["removed"].as_bool().unwrap(),
                blocks_building: value["building"].as_bool().unwrap(),
            });
        }
        let mut shape = VoxelShape::empty();
        for value in f["shape"].as_array().unwrap() {
            shape = shape.join(&VoxelShape::from_box(bbox(value))?, ShapeOp::OR)?;
        }
        let excluded = f["exclude"].as_i64().unwrap();
        let excluded = (excluded >= 0).then(|| entities[excluded as usize]);
        let mut world = World {
            entities,
            out: &mut out,
            fail: f["fail"].as_bool().unwrap(),
            calls: 0,
        };
        match is_unobstructed(&mut world, excluded.as_ref(), &shape) {
            Ok(value) => writeln!(world.out, "clear {value}")?,
            Err(_) => writeln!(world.out, "clear error")?,
        }
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
