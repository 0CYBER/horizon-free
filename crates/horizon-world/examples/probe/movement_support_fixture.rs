use crate::support::common::position;
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::movement_support::{self as support, MovementBlockWorld, MovementLocation};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
struct World<'a> {
    input: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    reads: usize,
}
impl World<'_> {
    fn query(&mut self, text: String) -> io::Result<()> {
        writeln!(self.out, "{text}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.input["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected query failure"))
        } else {
            Ok(())
        }
    }
    fn flag(&self, id: usize, name: &str) -> bool {
        self.input["states"][id][name].as_bool().unwrap()
    }
    fn predicate(&mut self, id: usize, name: &str) -> io::Result<bool> {
        self.query(format!("{name} {id}"))?;
        Ok(self.flag(id, name))
    }
    fn factor(&mut self, id: usize, name: &str) -> io::Result<f32> {
        self.query(format!("block {id}"))?;
        if self.flag(id, "gate") {
            return Ok(1.0);
        }
        self.query(format!("{name} {id}"))?;
        Ok(single(&self.input["states"][id][name]))
    }
}
impl MovementBlockWorld for World<'_> {
    type Error = io::Error;
    type State = usize;
    fn block_state(&mut self, p: BlockPos) -> io::Result<usize> {
        self.query(format!("state {} {} {}", p.x, p.y, p.z))?;
        let id = self.reads % self.input["states"].as_array().unwrap().len();
        self.reads += 1;
        Ok(id)
    }
    fn is_fence(&mut self, id: usize) -> io::Result<bool> {
        self.predicate(id, "fence")
    }
    fn is_wall(&mut self, id: usize) -> io::Result<bool> {
        self.predicate(id, "wall")
    }
    fn is_fence_gate(&mut self, id: usize) -> io::Result<bool> {
        self.query(format!("block {id}"))?;
        Ok(self.flag(id, "gate"))
    }
    fn is_water(&mut self, id: usize) -> io::Result<bool> {
        self.predicate(id, "water")
    }
    fn is_bubble_column(&mut self, id: usize) -> io::Result<bool> {
        self.predicate(id, "bubble")
    }
    fn jump_factor(&mut self, id: usize) -> io::Result<f32> {
        self.factor(id, "jump")
    }
    fn speed_factor(&mut self, id: usize) -> io::Result<f32> {
        self.factor(id, "speed")
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let p = &f["position"];
        let location = MovementLocation {
            position: Vec3::new(number(&p[0]), number(&p[1]), number(&p[2])),
            block_position: position(&f["cached"]),
            supporting_block: if f["support"].is_null() {
                None
            } else {
                Some(position(&f["support"]))
            },
        };
        let mut world = World {
            input: &f,
            out: &mut out,
            calls: 0,
            reads: 0,
        };
        let mode = f["mode"].as_u64().unwrap();
        if mode < 4 {
            let offset = match mode {
                0 => single(&f["offset"]),
                1 => support::LEGACY_SUPPORT_OFFSET,
                2 => support::ON_SUPPORT_OFFSET,
                3 => support::MOVEMENT_SUPPORT_OFFSET,
                _ => unreachable!(),
            };
            match support::support_position(
                &mut world,
                location.position,
                location.supporting_block,
                offset,
            ) {
                Ok(p) => writeln!(world.out, "position {} {} {}", p.x, p.y, p.z)?,
                Err(_) => writeln!(world.out, "error")?,
            }
        } else {
            let result = if mode == 4 {
                support::block_jump_factor(&mut world, location)
            } else {
                support::block_speed_factor(&mut world, location)
            };
            match result {
                Ok(factor) => writeln!(
                    world.out,
                    "factor {:08x}",
                    if factor.is_nan() {
                        0x7fc00000
                    } else {
                        factor.to_bits()
                    }
                )?,
                Err(_) => writeln!(world.out, "error")?,
            }
        }
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
