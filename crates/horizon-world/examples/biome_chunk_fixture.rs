//! Original transport for native chunk biome fill order and failure state.
use horizon_core::position::ChunkPos;
use horizon_world::worldgen::biome_chunk::BiomeChunk;
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let height = n(&f["height"]);
        let mut chunk = BiomeChunk::new(
            ChunkPos::new(n(&f["cx"]), n(&f["cz"])),
            n(&f["min"]),
            height,
            n(&f["default"]),
        )?;
        for op in f["ops"].as_array().unwrap() {
            let mut calls = 0;
            let result = chunk.fill(n(&op[3]), n(&op[4]), |[x, y, z]| {
                writeln!(out, "resolve {x} {y} {z}").unwrap();
                let previous = calls;
                calls += 1;
                if previous == n(&op[2]) {
                    return Err(());
                }
                Ok(x.wrapping_mul(31)
                    .wrapping_add(y.wrapping_mul(7))
                    .wrapping_add(z.wrapping_mul(17))
                    .wrapping_add(n(&op[0]))
                    .rem_euclid(n(&op[1])))
            });
            writeln!(
                out,
                "fill {} {calls}",
                if result.is_ok() { "ok" } else { "error" }
            )?;
            for index in 0..height as usize / 16 {
                writeln!(out, "section {index}")?;
                for row in chunk.section(index).unwrap().chunks(4) {
                    writeln!(out, "biomes {} {} {} {}", row[0], row[1], row[2], row[3])?;
                }
            }
            for q in f["queries"].as_array().unwrap() {
                writeln!(
                    out,
                    "query {}",
                    chunk.noise_biome([n(&q[0]), n(&q[1]), n(&q[2])])
                )?;
            }
        }
    }
    Ok(())
}
