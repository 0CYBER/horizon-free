use crate::support::common::f64_bits;
use horizon_core::{
    geometry::{Aabb, ShapeOp, Vec3, VoxelShape},
    position::BlockPos,
};
use horizon_world::{entity_obstruction::EntityIdentity, world_ray::*};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn vector(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn pos(p: BlockPos) -> String {
    format!("{} {} {}", p.x, p.y, p.z)
}
fn shape(v: &Value) -> Result<VoxelShape, Box<dyn std::error::Error>> {
    let mut result = VoxelShape::empty();
    for b in v.as_array().unwrap() {
        result = result.join(
            &VoxelShape::from_box(Aabb::new(
                Vec3::new(number(&b[0]), number(&b[1]), number(&b[2])),
                Vec3::new(number(&b[3]), number(&b[4]), number(&b[5])),
            ))?,
            ShapeOp::OR,
        )?;
    }
    Ok(result)
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    shapes: Vec<[VoxelShape; 5]>,
}
impl World<'_> {
    fn flag(&self, n: &str) -> bool {
        self.f[n].as_bool().unwrap()
    }
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.f["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected ray failure"))
        } else {
            Ok(())
        }
    }
    fn index(&self, p: BlockPos) -> usize {
        self.f["cells"][pos(p)].as_u64().unwrap_or(0) as usize
    }
    fn predicate(&mut self, n: &str, key: &str, s: usize) -> io::Result<bool> {
        self.query(format!("{n} {s}"))?;
        Ok(self.f["states"][s][key].as_bool().unwrap())
    }
}
impl FallResetWorld for World<'_> {
    type Error = io::Error;
    type Block = usize;
    fn fall_reset_tag(&mut self, s: usize) -> io::Result<bool> {
        self.predicate("reset-tag", "reset", s)
    }
    fn entity_is_player(&mut self, _: EntityIdentity) -> io::Result<bool> {
        self.query("player".into())?;
        Ok(self.flag("player"))
    }
    fn is_portal(&mut self, s: usize, p: ResetPortal) -> io::Result<bool> {
        let n = match p {
            ResetPortal::EndGateway => "gateway",
            ResetPortal::EndPortal => "end",
            ResetPortal::NetherPortal => "nether",
        };
        self.predicate(n, n, s)
    }
    fn is_server_level(&self) -> bool {
        self.flag("server")
    }
    fn players_nether_portal_delay(&mut self) -> io::Result<i32> {
        self.query("rules".into())?;
        self.query("portal-delay".into())?;
        Ok(self.f["delay"].as_i64().unwrap() as i32)
    }
}
impl RayWorld for World<'_> {
    type Error = io::Error;
    type Block = usize;
    type Fluid = usize;
    fn block_state(&mut self, p: BlockPos) -> io::Result<usize> {
        self.query(format!("block {}", pos(p)))?;
        Ok(self.index(p))
    }
    fn fluid_state(&mut self, p: BlockPos) -> io::Result<usize> {
        self.query(format!("fluid {}", pos(p)))?;
        Ok(self.index(p))
    }
    fn block_shape(&mut self, s: usize, p: BlockPos, m: RayBlockMode) -> io::Result<VoxelShape> {
        if m == RayBlockMode::FallDamageResetting {
            let entity = self.flag("entity").then_some(EntityIdentity(0));
            return fall_reset_shape(self, s, entity);
        }
        let (n, i) = match m {
            RayBlockMode::Collider => ("collider", 0),
            RayBlockMode::Outline => ("outline", 1),
            RayBlockMode::Visual => ("visual", 2),
            RayBlockMode::FallDamageResetting => unreachable!(),
        };
        self.query(format!("{n} {s} {}", pos(p)))?;
        Ok(self.shapes[s][i].clone())
    }
    fn interaction_shape(&mut self, s: usize, p: BlockPos) -> io::Result<VoxelShape> {
        self.query(format!("interaction {s} {}", pos(p)))?;
        Ok(self.shapes[s][3].clone())
    }
    fn fluid_is_source(&mut self, s: usize) -> io::Result<bool> {
        self.predicate("source", "source", s)
    }
    fn fluid_is_empty(&mut self, s: usize) -> io::Result<bool> {
        self.predicate("empty", "empty", s)
    }
    fn fluid_is_water(&mut self, s: usize) -> io::Result<bool> {
        self.predicate("water", "water", s)
    }
    fn fluid_shape(&mut self, s: usize, p: BlockPos) -> io::Result<VoxelShape> {
        self.query(format!("fluid-shape {s} {}", pos(p)))?;
        Ok(self.shapes[s][4].clone())
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut shapes = Vec::new();
        for s in f["states"].as_array().unwrap() {
            shapes.push([
                shape(&s["collider"])?,
                shape(&s["outline"])?,
                shape(&s["visual"])?,
                shape(&s["interaction"])?,
                shape(&s["fluid_shape"])?,
            ]);
        }
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            shapes,
        };
        let block_mode = [
            RayBlockMode::Collider,
            RayBlockMode::Outline,
            RayBlockMode::Visual,
            RayBlockMode::FallDamageResetting,
        ][f["block_mode"].as_u64().unwrap() as usize];
        let fluid_mode = [
            RayFluidMode::None,
            RayFluidMode::SourceOnly,
            RayFluidMode::Any,
            RayFluidMode::Water,
        ][f["fluid_mode"].as_u64().unwrap() as usize];
        match clip_world(
            &mut w,
            vector(&f["from"]),
            vector(&f["to"]),
            block_mode,
            fluid_mode,
            10000,
        ) {
            Ok(r) => {
                let h = r.hit;
                writeln!(
                    w.out,
                    "hit {} {:016x} {:016x} {:016x} {} {} {}",
                    r.miss,
                    f64_bits(h.location.x),
                    f64_bits(h.location.y),
                    f64_bits(h.location.z),
                    format!("{:?}", h.direction).to_lowercase(),
                    pos(h.block),
                    h.inside
                )?;
            }
            Err(_) => writeln!(w.out, "error")?,
        }
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
