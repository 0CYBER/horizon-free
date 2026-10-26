#![forbid(unsafe_code)]
//! Runs tools/test_emitter_particles_parity.py's emitters through
//! `particles::on_block_faces`, `particles::along_axis` and
//! `particles::item_use_particles`.
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
        let mut lines = Vec::new();
        let record = |kind: &str| {
            let kind = kind.to_owned();
            move |p: [f64; 3], v: [f64; 3]| {
                format!(
                    "+ {kind} {} {} {} {} {} {}",
                    d(p[0]),
                    d(p[1]),
                    d(p[2]),
                    d(v[0]),
                    d(v[1]),
                    d(v[2])
                )
            }
        };
        if c["kind"] == "item_use" {
            let p: [f64; 3] = std::array::from_fn(|i| c["pos"][i].as_f64().unwrap_or(0.0));
            let f = |k: &str| c[k].as_f64().unwrap_or(0.0) as f32;
            let line = record("item");
            particles::item_use_particles(
                &mut random,
                f("pitch"),
                f("yaw"),
                p,
                p[1] + f64::from(f("eye")),
                c["count"].as_u64().unwrap_or(0) as u32,
                |p, v| lines.push(line(p, v)),
            );
        } else {
            let p: [i32; 3] = std::array::from_fn(|i| c["pos"][i].as_i64().unwrap_or(0) as i32);
            match c["axis"].as_i64() {
                Some(axis @ 0..=2) => {
                    let line = record("electric_spark");
                    particles::along_axis(&mut random, axis as usize, p, (10, 19), |p, v| {
                        lines.push(line(p, v))
                    });
                }
                _ => {
                    let line = record("wax_on");
                    particles::on_block_faces(&mut random, p, |p, v| lines.push(line(p, v)));
                }
            }
        }
        for l in lines {
            writeln!(out, "{l}")?;
        }
        writeln!(out, "next {}", random.next_long())?;
    }
    out.flush()?;
    Ok(())
}
