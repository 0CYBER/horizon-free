use crate::support::common::tag;
use horizon_world::{
    DATA_VERSION,
    fluid::{FluidBlockBindings, FluidHeightWorld, FluidKind, FluidState, FluidStateTable},
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
struct World<'a, W> {
    out: &'a mut W,
    above: FluidState,
    sturdy: bool,
    calls: i32,
    fail: i32,
}
impl<W: Write> World<'_, W> {
    fn query(&mut self, kind: &str, p: [i32; 3]) -> io::Result<()> {
        writeln!(self.out, "{kind} {} {} {}", p[0], p[1], p[2])?;
        let call = self.calls;
        self.calls += 1;
        if call == self.fail {
            Err(io::Error::other("injected fluid query failure"))
        } else {
            Ok(())
        }
    }
}
impl<W: Write> FluidHeightWorld for World<'_, W> {
    type Error = io::Error;
    fn fluid(&mut self, p: [i32; 3]) -> io::Result<FluidState> {
        self.query("fluid", p)?;
        Ok(self.above)
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> io::Result<bool> {
        self.query("block", p)?;
        Ok(self.sturdy)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let facts: Value = serde_json::from_str(&fs::read_to_string(dir.join("facts-server.json"))?)?;
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("states", tag(&facts["states"])),
        ("defaults", tag(&facts["defaults"])),
    ]))?;
    let fluids = FluidStateTable::new(&registry)?;
    let blocks = FluidBlockBindings::new(&registry)?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for first in FluidKind::ALL {
        for second in FluidKind::ALL {
            writeln!(
                out,
                "family {} {} {}",
                first.name(),
                second.name(),
                first.same_family(second)
            )?;
        }
    }
    for i in 0..registry.len() {
        let fluid = fluids.get(BlockStateId(i as u32)).unwrap();
        writeln!(
            out,
            "state {i} {} {} {} {} {} {} {}",
            fluid.kind().name(),
            fluid.amount(),
            fluid.falling(),
            fluid.is_full(),
            fluid.is_source(),
            fluid.own_height().to_bits(),
            blocks.block(fluid).0
        )?;
    }
    for line in fs::read_to_string(dir.join("codec-inputs.txt"))?.lines() {
        match FluidState::from_json(line) {
            Err(_) => writeln!(out, "codec error")?,
            Ok(state) => {
                writeln!(
                    out,
                    "codec {} {} {} {} {}",
                    state.kind().name(),
                    state.amount(),
                    state.falling(),
                    state.is_source(),
                    blocks.block(state).0
                )?;
                writeln!(out, "encoded {}", state.to_json()?)?;
            }
        }
    }
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let fluid = fluids
            .get(BlockStateId(f["state"].as_u64().unwrap() as u32))
            .unwrap();
        let above = fluids
            .get(BlockStateId(f["above"].as_u64().unwrap() as u32))
            .unwrap();
        let p = std::array::from_fn(|i| f["position"][i].as_i64().unwrap() as i32);
        let mut world = World {
            out: &mut out,
            above,
            sturdy: f["sturdy"].as_bool().unwrap(),
            calls: 0,
            fail: f["fail"].as_i64().unwrap() as i32,
        };
        for camera in [false, true] {
            let result = if camera {
                fluid.camera_height_at(&mut world, p)
            } else {
                fluid.height_at(&mut world, p)
            };
            match result {
                Ok(value) => writeln!(world.out, "height {}", value.to_bits())?,
                Err(_) => writeln!(world.out, "height error")?,
            }
        }
        writeln!(world.out, "calls {}", world.calls)?;
    }
    Ok(())
}
