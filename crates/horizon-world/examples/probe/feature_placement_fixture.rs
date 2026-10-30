//! Transport for native modifier chains and a feature callback sharing the RNG.
use crate::support::placement_queries as queries;
use horizon_core::random::{LegacyRandom, RandomSource, XoroshiroRandom};
use horizon_world::worldgen::{
    anchor::WorldGenerationContext,
    placement::{FeaturePlacer, PlacementModifier},
};
use serde_json::Value;
use std::cell::RefCell;
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
    let out = RefCell::new(out);
    let mut context = queries::Queries::new(f, context, &out);
    let modifiers = f["modifiers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| PlacementModifier::from_json(&v.to_string()))
        .collect::<Result<Vec<_>, _>>();
    match modifiers {
        Err(_) => writeln!(out.borrow_mut(), "decode error")?,
        Ok(modifiers) => {
            writeln!(out.borrow_mut(), "decode ok")?;
            let mut placer = FeaturePlacer::new();
            for p in f["positions"].as_array().unwrap() {
                let mut count = 0;
                let fail = n(&f["fail"]);
                let draw = f["draw"].as_bool().unwrap();
                let mode = n(&f["result"]);
                let result = placer.place_in(
                    &modifiers,
                    &mut context,
                    &mut random,
                    [n(&p[0]), n(&p[1]), n(&p[2])],
                    |random, p| {
                        let previous = count;
                        count += 1;
                        if previous == fail {
                            return Err(io::Error::other("feature failure"));
                        }
                        writeln!(out.borrow_mut(), "feature {} {} {}", p[0], p[1], p[2])?;
                        let mut value = i64::from(p[0].wrapping_add(p[1]).wrapping_add(p[2]));
                        if draw {
                            value = random.next_long();
                            writeln!(out.borrow_mut(), "consumer {value}")?;
                        }
                        let result = mode == 1 || (mode == 2 && value & 1 == 0);
                        writeln!(out.borrow_mut(), "result {result}")?;
                        Ok(result)
                    },
                );
                match result {
                    Ok(result) => writeln!(out.borrow_mut(), "place {result}")?,
                    Err(_) => writeln!(out.borrow_mut(), "place error")?,
                }
                writeln!(out.borrow_mut(), "next {}", random.next_long())?;
            }
        }
    }
    writeln!(out.borrow_mut(), "tail {}", random.next_long())?;
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
