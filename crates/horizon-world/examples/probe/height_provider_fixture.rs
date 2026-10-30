//! Transport for anchored provider samples and subsequent random state.
use horizon_core::random::{LegacyRandom, RandomSource, XoroshiroRandom};
use horizon_world::worldgen::{anchor::WorldGenerationContext, height_provider::HeightProvider};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn fixture(
    f: &Value,
    mut random: impl RandomSource,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let c = &f["context"];
    let context = WorldGenerationContext::new(n(&c[0]), n(&c[1]), n(&c[2]));
    match HeightProvider::from_json(f["definition"].as_str().unwrap()) {
        Ok(provider) => {
            writeln!(out, "decode ok")?;
            for _ in 0..f["samples"].as_u64().unwrap() {
                match provider.sample(&mut random, context) {
                    Ok(value) => writeln!(out, "sample {value}")?,
                    Err(_) => writeln!(out, "sample error")?,
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
