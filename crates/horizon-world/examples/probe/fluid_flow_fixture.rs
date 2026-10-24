use horizon_core::{geometry::Direction, position::BlockPos};
use horizon_world::{
    fluid::FluidState,
    fluid_flow::{self, FluidFlowWorld},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn pos(p: BlockPos) -> String {
    format!("{} {} {}", p.x, p.y, p.z)
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    fluids: Vec<FluidState>,
}
impl World<'_> {
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected flow failure"))
        } else {
            Ok(())
        }
    }
    fn cell(&self, p: BlockPos) -> usize {
        self.f["cells"]
            .get(pos(p))
            .unwrap_or(&self.f["default"])
            .as_u64()
            .unwrap() as usize
    }
}
impl FluidFlowWorld for World<'_> {
    type Error = io::Error;
    type Block = usize;
    fn fluid(&mut self, p: BlockPos) -> io::Result<FluidState> {
        self.query(format!("fluid {}", pos(p)))?;
        Ok(self.fluids[self.cell(p)])
    }
    fn block(&mut self, p: BlockPos) -> io::Result<usize> {
        self.query(format!("block {}", pos(p)))?;
        Ok(self.cell(p))
    }
    fn blocks_fluid_flow(&self, s: usize) -> bool {
        self.f["states"][s]["blocks_flow"].as_bool().unwrap()
    }
    fn is_ice_class(&self, s: usize) -> bool {
        self.f["states"][s]["ice"].as_bool().unwrap()
    }
    fn is_face_sturdy(&mut self, s: usize, p: BlockPos, d: Direction) -> io::Result<bool> {
        let name = ["down", "up", "north", "south", "west", "east"][d as usize];
        self.query(format!("sturdy {} {name}", pos(p)))?;
        Ok(self.f["states"][s]["faces"].as_u64().unwrap() & (1 << (d as u8)) != 0)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let fluids = f["states"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| FluidState::from_json(&s["fluid"].to_string()))
            .collect::<io::Result<Vec<_>>>()?;
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            fluids,
        };
        let a = &f["position"];
        let p = BlockPos::new(
            a[0].as_i64().unwrap() as i32,
            a[1].as_i64().unwrap() as i32,
            a[2].as_i64().unwrap() as i32,
        );
        let root = FluidState::from_json(&f["root"].to_string())?;
        if f["mode"].as_str().unwrap() == "flow" {
            match fluid_flow::flow_at(&mut w, root, p) {
                Ok(v) => writeln!(
                    w.out,
                    "flow {:016x} {:016x} {:016x}",
                    v.x.to_bits(),
                    v.y.to_bits(),
                    v.z.to_bits()
                )?,
                Err(_) => writeln!(w.out, "error")?,
            }
        } else {
            match fluid_flow::solid_face(
                &mut w,
                root.kind(),
                p,
                Direction::ALL[f["direction"].as_u64().unwrap() as usize],
            ) {
                Ok(v) => writeln!(w.out, "face {v}")?,
                Err(_) => writeln!(w.out, "error")?,
            }
        }
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
