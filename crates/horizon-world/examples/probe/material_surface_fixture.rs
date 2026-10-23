//! Original comparison transport for terracotta bands and surface queries.
use horizon_core::{
    noise::{NanRange, Noise},
    random::{LegacyRandom, PositionalRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::{
    registry::BlockStateId,
    worldgen::{
        decode_noise,
        material_surface::{ClayBandBlocks, ClayBands, MaterialSurface},
    },
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
#[derive(Debug)]
struct ConstantNoise(f32);
impl Noise for ConstantNoise {
    fn range(&self) -> Result<[f32; 2], NanRange> {
        Ok([f32::NEG_INFINITY, f32::INFINITY])
    }
    fn get2(&self, _: f64, _: f64) -> f32 {
        panic!("unexpected 2d noise query")
    }
    fn get3(&self, _: f64, y: f64, _: f64) -> f32 {
        assert_eq!(y, 0.);
        self.0
    }
}
fn blocks() -> ClayBandBlocks {
    ClayBandBlocks {
        terracotta: BlockStateId(0),
        orange: BlockStateId(1),
        yellow: BlockStateId(2),
        brown: BlockStateId(3),
        red: BlockStateId(4),
        white: BlockStateId(5),
        light_gray: BlockStateId(6),
    }
}
fn fixture<R: RandomSource>(
    f: &Value,
    mut direct: R,
    root: R::Positional,
    noises: &BTreeMap<String, String>,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    if f["child"].as_bool().unwrap() {
        direct = direct
            .fork_positional()
            .from_hash_of("minecraft:clay_bands");
    }
    direct.consume_count(f["consume"].as_i64().unwrap() as i32);
    let bands = ClayBands::generate(&mut direct, blocks());
    writeln!(out, "bands {}", bands.states().len())?;
    for state in bands.states() {
        writeln!(out, "{}", state.0)?;
    }
    writeln!(out, "next {} {}", direct.next_long(), direct.next_int())?;
    let mut samplers: Vec<Arc<dyn Noise>> = Vec::new();
    for (i, name) in ["clay_bands_offset", "surface", "surface_secondary"]
        .iter()
        .enumerate()
    {
        let sampler: Arc<dyn Noise> = if let Some(bits) = f["overrides"][i].as_str() {
            Arc::new(ConstantNoise(f32::from_bits(u32::from_str_radix(
                bits, 16,
            )?)))
        } else {
            let id = format!("minecraft:{name}");
            Arc::new(decode_noise(&noises[&id])?.create(&mut root.from_hash_of(&id))?)
        };
        samplers.push(sampler);
    }
    let surface = MaterialSurface::new(
        root,
        blocks(),
        samplers[0].clone(),
        samplers[1].clone(),
        samplers[2].clone(),
    );
    writeln!(out, "system bands")?;
    for state in surface.bands().states() {
        writeln!(out, "{}", state.0)?;
    }
    for p in f["points"].as_array().unwrap() {
        let [x, y, z] = std::array::from_fn(|i| p[i].as_i64().unwrap() as i32);
        match surface.band([x, y, z]) {
            Some(state) => writeln!(out, "band {}", state.0)?,
            None => writeln!(out, "band error")?,
        }
        writeln!(out, "depth {}", surface.surface_depth(x, z))?;
        let value = surface.surface_secondary(x, z);
        let bits = if value.is_nan() {
            0x7ff8000000000000
        } else {
            value.to_bits()
        };
        writeln!(out, "secondary {bits:x}")?;
    }
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private directory");
    let dir = Path::new(&arg);
    let noises: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("noises.json"))?)?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let seed = f["seed"].as_i64().unwrap();
        if f["legacy"].as_bool().unwrap() {
            fixture(
                &f,
                LegacyRandom::new(seed),
                LegacyRandom::new(seed).fork_positional(),
                &noises,
                &mut out,
            )?;
        } else {
            fixture(
                &f,
                XoroshiroRandom::new(seed),
                XoroshiroRandom::new(seed).fork_positional(),
                &noises,
                &mut out,
            )?;
        }
    }
    out.flush()?;
    Ok(())
}
