//! Original comparison transport for biome temperatures and ordered caches.
use horizon_core::position::BlockPos;
use horizon_world::worldgen::biome_temperature::{
    BiomeTemperature, BiomeTemperatureSettings, Precipitation, TemperatureModifier,
};
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
fn operations(worker: &mut BiomeTemperature, ops: &Value, out: &mut impl Write) -> io::Result<()> {
    for op in ops.as_array().unwrap() {
        match op[0].as_str().unwrap() {
            "query" => {
                let p = BlockPos::new(
                    op[1].as_i64().unwrap() as i32,
                    op[2].as_i64().unwrap() as i32,
                    op[3].as_i64().unwrap() as i32,
                );
                let sea = op[4].as_i64().unwrap() as i32;
                writeln!(
                    out,
                    "adjusted {:x}",
                    bits(worker.settings().height_adjusted(p, sea))
                )?;
                writeln!(out, "cached {:x}", bits(worker.temperature(p, sea)))?;
                let warm = worker.warm_enough_to_rain(p, sea);
                let cold = worker.cold_enough_to_snow(p, sea);
                let melt = worker.should_melt_frozen_ocean_iceberg_slightly(p, sea);
                let precipitation = match worker.precipitation_at(p, sea) {
                    Precipitation::None => "NONE",
                    Precipitation::Rain => "RAIN",
                    Precipitation::Snow => "SNOW",
                };
                writeln!(out, "weather {warm} {cold} {melt} {precipitation}")?;
            }
            "fill" => {
                for i in 0..op[1].as_i64().unwrap() as i32 {
                    let p = BlockPos::new(
                        (op[2].as_i64().unwrap() as i32).wrapping_add(i),
                        i & 255,
                        i.wrapping_mul(31),
                    );
                    writeln!(
                        out,
                        "fill {:x}",
                        bits(worker.temperature(p, op[3].as_i64().unwrap() as i32))
                    )?;
                }
            }
            "dump" => {
                writeln!(out, "cache {}", worker.cached_temperatures().len())?;
                for (key, value) in worker.cached_temperatures() {
                    writeln!(out, "{:x} {:x}", key as u64, bits(value))?;
                }
            }
            "thread" => operations(&mut BiomeTemperature::new(worker.settings()), &op[1], out)?,
            _ => panic!("unknown operation"),
        }
    }
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let settings = if let Some(json) = f.get("json") {
            match BiomeTemperatureSettings::from_json(json.as_str().unwrap()) {
                Ok(value) => value,
                Err(_) => {
                    writeln!(out, "decode error")?;
                    continue;
                }
            }
        } else {
            BiomeTemperatureSettings {
                base_temperature: f32::from_bits(u32::from_str_radix(
                    f["base"].as_str().unwrap(),
                    16,
                )?),
                downfall: f32::from_bits(u32::from_str_radix(
                    f.get("downfall").and_then(Value::as_str).unwrap_or("0"),
                    16,
                )?),
                modifier: if f["modifier"].as_str().unwrap() == "FROZEN" {
                    TemperatureModifier::Frozen
                } else {
                    TemperatureModifier::None
                },
                has_precipitation: f["precipitation"].as_bool().unwrap(),
            }
        };
        writeln!(
            out,
            "settings {} {:x} {} {:x}",
            settings.has_precipitation,
            bits(settings.base_temperature),
            if settings.modifier == TemperatureModifier::Frozen {
                "FROZEN"
            } else {
                "NONE"
            },
            bits(settings.downfall)
        )?;
        operations(&mut BiomeTemperature::new(settings), &f["ops"], &mut out)?;
    }
    out.flush()?;
    Ok(())
}
