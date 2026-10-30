#![forbid(unsafe_code)]
//! Runs tools/test_entity_particles_parity.py's entity events through
//! `particles::entity_event_burst` and `particles::around_self`.
use horizon_client::{Result, particles};
use horizon_core::random::{LegacyRandom, RandomSource};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

fn d(v: f64) -> String {
    format!("{:016x}", v.to_bits())
}

pub fn main() -> Result<()> {
    let args: Vec<String> = crate::args().collect();
    let mut out = BufWriter::new(File::create(&args[2])?);
    for (n, line) in BufReader::new(File::open(&args[1])?).lines().enumerate() {
        let c: Value = serde_json::from_str(&line?)?;
        writeln!(out, "== {n}")?;
        let mut random = LegacyRandom::new(c["seed"].as_i64().ok_or("seed")?);
        let feet: [f64; 3] = std::array::from_fn(|i| c["feet"][i].as_f64().unwrap_or(0.0));
        // EntityDimensions hold floats.
        let width = f64::from(c["width"].as_f64().unwrap_or(0.0) as f32);
        let height = f64::from(c["height"].as_f64().unwrap_or(0.0) as f32);
        let entity = c["entity"].as_str().unwrap_or("");
        if let Some((kind, burst)) =
            particles::entity_event_burst(entity, c["event"].as_u64().unwrap_or(0) as u8)
        {
            let mut lines = Vec::new();
            particles::around_self(&mut random, feet, width, height, burst, |p, v| {
                lines.push(format!(
                    "+ {kind} {} {} {} {} {} {}",
                    d(p[0]),
                    d(p[1]),
                    d(p[2]),
                    d(v[0]),
                    d(v[1]),
                    d(v[2])
                ));
            });
            for l in lines {
                writeln!(out, "{l}")?;
            }
        }
        writeln!(out, "next {}", random.next_long())?;
    }
    out.flush()?;
    Ok(())
}
