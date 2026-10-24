//! Transport for decoded scalar provider samples and subsequent random state.
use horizon_core::random::{LegacyRandom, RandomSource, XoroshiroRandom};
use horizon_world::worldgen::value_provider::{FloatProvider, IntProvider};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn bits(value: f32) -> u32 {
    if value.is_nan() {
        0x7fc00000
    } else {
        value.to_bits()
    }
}
fn fixture(
    f: &Value,
    mut random: impl RandomSource,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = f["definition"].as_str().unwrap();
    match f["kind"].as_str().unwrap() {
        "float" => match FloatProvider::from_json(json) {
            Ok(provider) => {
                let [min, max] = provider.bounds();
                writeln!(out, "float {:x} {:x}", bits(min), bits(max))?;
                for _ in 0..f["samples"].as_u64().unwrap() {
                    writeln!(out, "sample {:x}", bits(provider.sample(&mut random)))?;
                    writeln!(out, "next {}", random.next_long())?;
                }
            }
            Err(_) => writeln!(out, "decode error")?,
        },
        "int" => match IntProvider::from_json(json) {
            Ok(provider) => {
                let [min, max] = provider.bounds();
                writeln!(out, "int {min} {max}")?;
                for _ in 0..f["samples"].as_u64().unwrap() {
                    match provider.sample(&mut random) {
                        Ok(value) => writeln!(out, "sample {value}")?,
                        Err(_) => writeln!(out, "sample error")?,
                    }
                    writeln!(out, "next {}", random.next_long())?;
                }
            }
            Err(_) => writeln!(out, "decode error")?,
        },
        _ => unreachable!(),
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
