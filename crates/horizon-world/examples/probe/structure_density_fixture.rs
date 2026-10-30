//! Original JSON transport for structure-density comparison.
use horizon_core::{
    density::{
        Beardifier, DensitySampler, JigsawJunction, RigidPiece, SamplerContext, StructureBounds,
        TerrainAdjustment,
    },
    noise::DensityVolume,
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
};
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn bounds(v: &Value) -> Option<StructureBounds> {
    if v.is_null() {
        None
    } else {
        StructureBounds::new(
            std::array::from_fn(|i| n(&v[i])),
            std::array::from_fn(|i| n(&v[i + 3])),
        )
    }
}
fn bits(value: f32) -> u32 {
    if value.is_nan() {
        0x7fc00000
    } else {
        value.to_bits()
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let mut out = io::BufWriter::new(fs::File::create(&args[2])?);
    for line in fs::read_to_string(&args[1])?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let pieces = f["pieces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| RigidPiece {
                bounds: bounds(&a[0]).unwrap(),
                adjustment: match a[1].as_str().unwrap() {
                    "NONE" => TerrainAdjustment::None,
                    "BURY" => TerrainAdjustment::Bury,
                    "BEARD_THIN" => TerrainAdjustment::BeardThin,
                    "BEARD_BOX" => TerrainAdjustment::BeardBox,
                    "ENCAPSULATE" => TerrainAdjustment::Encapsulate,
                    _ => panic!("unknown adjustment"),
                },
                ground_level_delta: n(&a[2]),
            })
            .collect();
        let junctions = f["junctions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| JigsawJunction {
                source_x: n(&a[0]),
                source_ground_y: n(&a[1]),
                source_z: n(&a[2]),
            })
            .collect();
        let sampler = Beardifier::new(pieces, junctions, bounds(&f["affected"]));
        let mut context = SamplerContext::default();
        writeln!(
            out,
            "beardifier {} {}",
            f["pieces"].as_array().unwrap().len(),
            f["junctions"].as_array().unwrap().len()
        )?;
        for p in f["points"].as_array().unwrap() {
            writeln!(
                out,
                "value {:x}",
                bits(sampler.sample_value(&mut context, std::array::from_fn(|i| n(&p[i])))?)
            )?;
        }
        for a in f["volumes"].as_array().unwrap() {
            let volume = DensityVolume::new(
                std::array::from_fn(|i| n(&a[i])),
                std::array::from_fn(|i| n(&a[i + 3])),
                std::array::from_fn(|i| n(&a[i + 6])),
            )?;
            let mut buffer = vec![-123.5; volume.size() + n(&a[9]) as usize];
            sampler.sample_volume(&mut context, &mut buffer, volume)?;
            writeln!(out, "volume {}", buffer.len())?;
            for value in buffer {
                writeln!(out, "{:x}", bits(value))?;
            }
        }
    }
    out.flush()?;
    Ok(())
}
