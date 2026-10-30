use horizon_core::{
    geometry::{Axis, VoxelShape},
    position::BlockPos,
};
use horizon_world::{
    fluid::{FluidHeightWorld, FluidState},
    fluid_shape::FluidShapeCache,
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
struct World<'a> {
    out: &'a mut dyn Write,
    calls: i64,
    fail: i64,
    above: FluidState,
}
impl FluidHeightWorld for World<'_> {
    type Error = io::Error;
    fn fluid(&mut self, p: [i32; 3]) -> io::Result<FluidState> {
        writeln!(self.out, "fluid {} {} {}", p[0], p[1], p[2])?;
        let call = self.calls;
        self.calls += 1;
        if call == self.fail {
            Err(io::Error::other("injected fluid height failure"))
        } else {
            Ok(self.above)
        }
    }
    fn sturdy_down(&mut self, _: [i32; 3]) -> io::Result<bool> {
        panic!("ray shape does not use camera immersion support")
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut cache = FluidShapeCache::default();
        let mut identities = Vec::<VoxelShape>::new();
        for op in f["ops"].as_array().unwrap() {
            let state = FluidState::from_json(&op["state"].to_string())?;
            let above = FluidState::from_json(&op["above"].to_string())?;
            let p = &op["position"];
            let p = BlockPos::new(
                p[0].as_i64().unwrap() as i32,
                p[1].as_i64().unwrap() as i32,
                p[2].as_i64().unwrap() as i32,
            );
            let mut world = World {
                out: &mut out,
                calls: 0,
                fail: op["fail"].as_i64().unwrap(),
                above,
            };
            match cache.shape(&mut world, state, p) {
                Ok(s) => {
                    let id = identities
                        .iter()
                        .position(|old| old.same_identity(&s))
                        .unwrap_or_else(|| {
                            identities.push(s.clone());
                            identities.len() - 1
                        });
                    writeln!(world.out, "identity {id}")?;
                    writeln!(
                        world.out,
                        "shape {} {} {}",
                        s.same_identity(&VoxelShape::empty()),
                        s.same_identity(&VoxelShape::block()),
                        s.boxes().len()
                    )?;
                    for b in s.boxes() {
                        writeln!(
                            world.out,
                            "box {:016x} {:016x} {:016x} {:016x} {:016x} {:016x}",
                            b.min(Axis::X).to_bits(),
                            b.min(Axis::Y).to_bits(),
                            b.min(Axis::Z).to_bits(),
                            b.max(Axis::X).to_bits(),
                            b.max(Axis::Y).to_bits(),
                            b.max(Axis::Z).to_bits()
                        )?;
                    }
                }
                Err(_) => writeln!(world.out, "error")?,
            }
            writeln!(world.out, "cached {}", cache.cached_states())?;
            writeln!(world.out, "queries {}", world.calls)?;
        }
    }
    Ok(())
}
