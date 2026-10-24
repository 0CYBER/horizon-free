use crate::support::shapes::{box_values, emit};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::{
    entity_collision::{CollisionCandidate, EntityCollisionWorld, entity_collision_shapes},
    entity_obstruction::EntityIdentity,
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
fn bounds(v: &Value) -> Aabb {
    Aabb::new(
        Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])),
        Vec3::new(number(&v[3]), number(&v[4]), number(&v[5])),
    )
}
struct Entity {
    candidate: CollisionCandidate,
    root: EntityIdentity,
    collidable: bool,
    policy: u64,
}
struct World<'a, W> {
    out: &'a mut W,
    entities: Vec<Entity>,
    calls: i32,
    fail: i32,
}
impl<W: Write> World<'_, W> {
    fn query(&mut self, line: String) -> io::Result<()> {
        writeln!(self.out, "{line}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.fail {
            Err(io::Error::other("injected entity collision query failure"))
        } else {
            Ok(())
        }
    }
}
impl<W: Write> EntityCollisionWorld for World<'_, W> {
    type Error = io::Error;
    fn collision_candidates(
        &mut self,
        source: Option<EntityIdentity>,
        b: Aabb,
    ) -> io::Result<Vec<CollisionCandidate>> {
        let id = source.map_or(-1, |e| e.0 as i64);
        let mut output = Vec::new();
        box_values(&mut output, &format!("query {id}"), b)?;
        let line = String::from_utf8(output).unwrap();
        self.query(line.trim_end().to_owned())?;
        Ok(self.entities.iter().map(|e| e.candidate).collect())
    }
    fn can_collide(
        &mut self,
        source: Option<EntityIdentity>,
        candidate: EntityIdentity,
    ) -> io::Result<bool> {
        if let Some(source) = source {
            self.query(format!("collide {} {}", source.0, candidate.0))?;
            let policy = self.entities[source.0 as usize - 1].policy;
            if policy != 0 {
                return Ok(policy == 1);
            }
        }
        self.query(format!(
            "collidable {} {}",
            candidate.0,
            source.map_or(-1, |e| e.0 as i64)
        ))?;
        let entity = &self.entities[candidate.0 as usize - 1];
        Ok(entity.collidable
            && source.is_none_or(|id| self.entities[id.0 as usize - 1].root != entity.root))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut entities: Vec<Entity> = Vec::new();
        for v in f["entities"].as_array().unwrap() {
            let id = EntityIdentity(entities.len() as u64 + 1);
            let parent = v["parent"].as_i64().unwrap();
            let root = if parent < 0 {
                id
            } else {
                entities[parent as usize].root
            };
            entities.push(Entity {
                candidate: CollisionCandidate {
                    identity: id,
                    bounds: bounds(&v["box"]),
                    spectator: v["spectator"].as_bool().unwrap(),
                },
                root,
                collidable: v["collidable"].as_bool().unwrap(),
                policy: v["policy"].as_u64().unwrap(),
            });
        }
        let mut world = World {
            out: &mut out,
            entities,
            calls: 0,
            fail: f["fail"].as_i64().unwrap() as i32,
        };
        let source = f["source"].as_i64().unwrap();
        let source = (source >= 0).then_some(EntityIdentity(source.wrapping_add(1) as u64));
        match entity_collision_shapes(&mut world, source, bounds(&f["box"])) {
            Ok(shapes) => {
                writeln!(world.out, "selected {}", shapes.len())?;
                for shape in shapes {
                    emit(world.out, &shape)?;
                }
            }
            Err(_) => writeln!(world.out, "error")?,
        }
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
