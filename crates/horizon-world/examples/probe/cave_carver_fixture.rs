//! Transport for decoded cave paths, ordered mask writes and RNG continuation.
use horizon_core::{
    carving::{CarverOutput, CarvingMask},
    position::ChunkPos,
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::worldgen::{anchor::WorldGenerationContext, carver::CaveCarver};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
struct Output<'a, W: Write> {
    mask: &'a mut CarvingMask,
    out: &'a mut W,
    calls: i32,
    fail: i32,
}
impl<W: Write> CarverOutput for Output<'_, W> {
    type Error = ();
    fn min_y(&self) -> i32 {
        self.mask.min_y()
    }
    fn max_y(&self) -> i32 {
        self.mask.max_y()
    }
    fn carve(&mut self, x: i32, y: i32, z: i32) -> Result<(), ()> {
        writeln!(self.out, "cell {x} {y} {z}").unwrap();
        let previous = self.calls;
        self.calls += 1;
        if previous == self.fail {
            return Err(());
        }
        self.mask.carve(x, y, z).unwrap();
        Ok(())
    }
}
fn fixture(
    f: &Value,
    mut random: impl RandomSource,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let c = &f["context"];
    let context = WorldGenerationContext::new(n(&c[0]), n(&c[1]), n(&c[2]));
    match CaveCarver::from_json(f["definition"].as_str().unwrap()) {
        Ok(carver) => {
            writeln!(out, "decode ok")?;
            let mut mask = CarvingMask::new(context.min_y + 1, context.min_y + context.height - 8)?;
            let target = ChunkPos::new(n(&f["target"][0]), n(&f["target"][1]));
            let origin = ChunkPos::new(n(&f["origin"][0]), n(&f["origin"][1]));
            for _ in 0..n(&f["repeats"]) {
                writeln!(out, "start {}", carver.is_start_chunk(&mut random))?;
                let mut output = Output {
                    mask: &mut mask,
                    out,
                    calls: 0,
                    fail: n(&f["fail"]),
                };
                let result = carver.carve(context, &mut random, target, origin, &mut output);
                let calls = output.calls;
                writeln!(
                    out,
                    "carve {} {calls}",
                    match result {
                        Ok(true) => "true",
                        Ok(false) => "false",
                        Err(_) => "error",
                    }
                )?;
                for c in mask.columns() {
                    writeln!(out, "column {} {} {} {}", c.x, c.z, c.min_y, c.max_y)?;
                }
                writeln!(out, "next {}", random.next_long())?;
            }
        }
        Err(_) => writeln!(out, "decode error")?,
    }
    writeln!(out, "tail {}", random.next_long())?;
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let seed = f["seed"].as_i64().unwrap();
        if f["legacy"].as_bool().unwrap() {
            fixture(&f, LegacyRandom::new(seed), &mut out)?;
        } else {
            fixture(&f, XoroshiroRandom::new(seed), &mut out)?;
        }
    }
    Ok(())
}
