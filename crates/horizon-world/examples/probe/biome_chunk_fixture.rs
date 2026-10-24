//! Original transport for native chunk biome fill order and failure state.
use horizon_core::position::ChunkPos;
use horizon_world::{retrogen::RetrogenBiomeResolver, worldgen::biome_chunk::BiomeChunk};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn biome_key(id: &i32) -> Option<&str> {
    match *id {
        0 => Some("minecraft:lush_caves"),
        1 => Some("minecraft:dripstone_caves"),
        2 => Some("minecraft:deep_dark"),
        3 => Some("horizon:lush_caves"),
        511 => None,
        _ => Some("horizon:other"),
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
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
            let resolve = |[x, y, z]: [i32; 3]| {
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
            };
            let result = if let Some(before) = op.get(5) {
                // Native wrapper activation is captured before the later metadata
                // mutation at op[6]; filling still uses the supplied bounds.
                chunk.fill_with_retrogen(
                    n(&op[3]),
                    n(&op[4]),
                    before.as_bool().unwrap(),
                    resolve,
                    biome_key,
                )
            } else {
                chunk.fill(n(&op[3]), n(&op[4]), resolve)
            };
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
        if let Some(queries) = f["resolver_queries"].as_array() {
            for query in queries {
                let p = [n(&query[1]), n(&query[2]), n(&query[3])];
                let mut resolver =
                    RetrogenBiomeResolver::new(query[4].as_bool().unwrap(), |[x, y, z]| {
                        writeln!(out, "resolve_query {x} {y} {z}").unwrap();
                        if n(&query[0]) < 0 {
                            Err(())
                        } else {
                            Ok(n(&query[0]))
                        }
                    });
                let result = resolver.sample(p, biome_key, |old| Ok(*chunk.noise_biome(old)));
                match result {
                    Ok(value) => writeln!(out, "resolver_query {value}")?,
                    Err(()) => writeln!(out, "resolver_query error")?,
                }
            }
        }
    }
    Ok(())
}
