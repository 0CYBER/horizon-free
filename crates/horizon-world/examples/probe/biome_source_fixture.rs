//! Original transport for fixed, checkerboard and End biome-source queries.
use horizon_core::density::{DensitySampler, SampleError, SamplerContext};
use horizon_world::worldgen::biome_source::{
    CheckerboardBiomeSource, EndBiomeSource, FixedBiomeSource,
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
};
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
#[derive(Debug)]
struct Erosion {
    value: f32,
    fail: bool,
    trace: Arc<Mutex<Vec<[i32; 3]>>>,
}
impl DensitySampler for Erosion {
    fn sample_value(&self, _: &mut SamplerContext, pos: [i32; 3]) -> Result<f32, SampleError> {
        self.trace.lock().unwrap().push(pos);
        if self.fail {
            Err(SampleError::RecursiveCache)
        } else {
            Ok(self.value)
        }
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let kind = f["kind"].as_str().unwrap();
        let fixed = FixedBiomeSource { biome: 7 };
        let checker = CheckerboardBiomeSource::new(
            f["biomes"]
                .as_array()
                .map(|v| v.iter().map(n).collect())
                .unwrap_or_default(),
            f["scale"].as_i64().unwrap_or(0) as i32,
        )?;
        let end = EndBiomeSource {
            center: 0,
            highlands: 1,
            midlands: 2,
            islands: 3,
            barrens: 4,
        };
        let trace = Arc::new(Mutex::new(Vec::new()));
        let erosion = Erosion {
            value: f32::from_bits(u32::from_str_radix(f["bits"].as_str().unwrap_or("0"), 16)?),
            fail: f["fail"].as_bool().unwrap_or(false),
            trace: trace.clone(),
        };
        let mut context = SamplerContext::cached();
        for pos in f["positions"].as_array().unwrap() {
            let q = [n(&pos[0]), n(&pos[1]), n(&pos[2])];
            let value = match kind {
                "fixed" => Some(*fixed.noise_biome(q)),
                "checkerboard" => checker.noise_biome(q).ok().copied(),
                "end" => end.noise_biome(q, &erosion, &mut context).ok().copied(),
                _ => panic!("invalid source"),
            };
            for [x, y, z] in trace.lock().unwrap().drain(..) {
                writeln!(out, "sample {x} {y} {z}")?;
            }
            match value {
                Some(id) => writeln!(out, "biome {id}")?,
                None => writeln!(out, "error")?,
            }
        }
    }
    Ok(())
}
