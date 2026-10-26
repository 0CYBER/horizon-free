use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::{
    chunk::BlockSection,
    fluid_occupancy::*,
    nbt::{self, Limits},
    registry::{BlockRegistry, BlockStateId},
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
struct World<'a, 'r> {
    out: &'a mut dyn Write,
    calls: i64,
    fail: i64,
    min: i32,
    height: i32,
    sections: Vec<BlockSection<'r>>,
}
impl World<'_, '_> {
    fn query(&mut self, s: impl AsRef<str>) -> io::Result<()> {
        writeln!(self.out, "{}", s.as_ref())?;
        let n = self.calls;
        self.calls += 1;
        if n == self.fail {
            Err(io::Error::other("injected occupancy failure"))
        } else {
            Ok(())
        }
    }
}
impl FluidOccupancyWorld for World<'_, '_> {
    type Error = io::Error;
    type Chunk = ();
    fn minimum_section_y(&mut self) -> io::Result<i32> {
        self.query("min-section")?;
        Ok(self.min >> 4)
    }
    fn maximum_section_y(&mut self) -> io::Result<i32> {
        self.query("max-section")?;
        Ok((self.min + self.height - 1) >> 4)
    }
    fn chunk(&mut self, x: i32, z: i32) -> io::Result<()> {
        self.query(format!("chunk {x} {z}"))
    }
    fn section(&mut self, _: (), y: i32) -> io::Result<&BlockSection<'_>> {
        self.query(format!("section {y}"))?;
        Ok(&self.sections[(y - (self.min >> 4)) as usize])
    }
    fn minimum_y(&mut self) -> io::Result<i32> {
        self.query("min-y")?;
        Ok(self.min)
    }
    fn maximum_y(&mut self) -> io::Result<i32> {
        self.query("max-y")?;
        Ok(self.min + self.height - 1)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let registry = BlockRegistry::from_nbt(
        &nbt::read_named(
            &fs::read(dir.join("block-registry.nbt"))?,
            Limits::default(),
        )?
        .0
        .tag,
    )?;
    let fluid = FluidPresence::new(&registry)?;
    assert!(fluid.has_fluid(BlockStateId(u32::MAX)).is_err());
    assert!(std::ptr::eq(fluid.registry(), &registry));
    {
        let air = registry.default_state("minecraft:air").unwrap();
        let mut sink = io::sink();
        let mut w = World {
            out: &mut sink,
            calls: 0,
            fail: -1,
            min: 0,
            height: 16,
            sections: vec![BlockSection::uniform(air, &registry)?],
        };
        let bad = Aabb::new(Vec3::new(f64::NAN, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        assert!(matches!(
            contains_any_liquid(&mut w, &fluid, bad, 100),
            Err(FluidOccupancyError::NonFiniteBounds)
        ));
        assert_eq!(w.calls, 0);
        let box_ = Aabb::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0));
        assert!(matches!(
            contains_any_liquid(&mut w, &fluid, box_, 0),
            Err(FluidOccupancyError::BudgetExceeded)
        ));
        assert_eq!(w.calls, 2);
        w.calls = 0;
        assert!(matches!(
            contains_any_liquid(&mut w, &fluid, box_, 1),
            Err(FluidOccupancyError::BudgetExceeded)
        ));
        assert_eq!(w.calls, 3);
        assert!(!contains_any_liquid(&mut w, &fluid, box_, 2)?);
        let foreign = BlockRegistry::from_nbt(
            &nbt::read_named(
                &fs::read(dir.join("block-registry.nbt"))?,
                Limits::default(),
            )?
            .0
            .tag,
        )?;
        let foreign_presence = FluidPresence::new(&foreign)?;
        assert!(matches!(
            contains_any_liquid(&mut w, &foreign_presence, box_, 100),
            Err(FluidOccupancyError::Data(_))
        ));
    }
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for i in 0..registry.len() {
        writeln!(
            out,
            "state {i} {}",
            fluid.has_fluid(BlockStateId(i as u32))?
        )?;
    }
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let min = f["min"].as_i64().unwrap() as i32;
        let height = f["height"].as_i64().unwrap() as i32;
        let base = BlockStateId(f["base"].as_u64().unwrap() as u32);
        let mut w = World {
            out: &mut out,
            calls: 0,
            fail: -1,
            min,
            height,
            sections: (0..height / 16)
                .map(|_| BlockSection::uniform(base, &registry))
                .collect::<io::Result<Vec<_>>>()?,
        };
        for op in f["ops"].as_array().unwrap() {
            for v in op["writes"].as_array().unwrap() {
                let x = v[0].as_i64().unwrap() as i32;
                let y = v[1].as_i64().unwrap() as i32;
                let z = v[2].as_i64().unwrap() as i32;
                w.sections[((y - min) >> 4) as usize].set(
                    ((y & 15) * 256 + (z & 15) * 16 + (x & 15)) as usize,
                    BlockStateId(v[3].as_u64().unwrap() as u32),
                )?;
            }
            if op["compact"].as_bool().unwrap() {
                for section in &mut w.sections {
                    section.compact()?;
                }
            }
            w.calls = 0;
            w.fail = op["fail"].as_i64().unwrap();
            match contains_any_liquid(&mut w, &fluid, bounds(&op["bounds"]), 1000000) {
                Ok(b) => writeln!(w.out, "result {b}")?,
                Err(FluidOccupancyError::World(_)) => writeln!(w.out, "error")?,
                Err(e) => return Err(e.into()),
            };
            writeln!(w.out, "queries {}", w.calls)?;
        }
    }
    Ok(())
}
