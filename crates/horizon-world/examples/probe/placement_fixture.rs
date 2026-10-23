//! Transport for placement decoding, emissions, failures and random state.
use crate::support::placement_queries as queries;
use horizon_core::random::{LegacyRandom, RandomSource, XoroshiroRandom};
use horizon_world::worldgen::{anchor::WorldGenerationContext, placement::PlacementModifier};
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
    match PlacementModifier::from_json(f["definition"].as_str().unwrap()) {
        Err(_) => writeln!(out.borrow_mut(), "decode error")?,
        Ok(modifier) => {
            writeln!(out.borrow_mut(), "decode ok")?;
            for p in f["positions"].as_array().unwrap() {
                if f["count_only"].as_bool().unwrap_or(false) {
                    match modifier.repetitions(&mut random, [n(&p[0]), n(&p[1]), n(&p[2])]) {
                        Ok(Some(count)) => writeln!(out.borrow_mut(), "count {count}")?,
                        _ => writeln!(out.borrow_mut(), "count error")?,
                    }
                    writeln!(out.borrow_mut(), "next {}", random.next_long())?;
                    continue;
                }
                let mut count = 0;
                let fail = n(&f["fail"]);
                let draw = f["draw"].as_bool().unwrap();
                let result = modifier.modify_in(
                    &mut context,
                    &mut random,
                    [n(&p[0]), n(&p[1]), n(&p[2])],
                    |random, p| {
                        let previous = count;
                        count += 1;
                        if previous == fail {
                            return Err(io::Error::other("consumer failure"));
                        }
                        writeln!(out.borrow_mut(), "position {} {} {}", p[0], p[1], p[2])?;
                        if draw {
                            writeln!(out.borrow_mut(), "consumer {}", random.next_long())?;
                        }
                        Ok(())
                    },
                );
                writeln!(
                    out.borrow_mut(),
                    "modify {}",
                    if result.is_ok() { "ok" } else { "error" }
                )?;
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
