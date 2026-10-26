//! Chunk biome generation from bundled climate routers and imported preset tables.
use horizon_core::{
    biome::{BiomeZoom, obfuscate_seed},
    density::{SamplerContext, graph::DensityCompiler},
    position::{BlockPos, ChunkPos},
};
use horizon_world::worldgen::{
    biome_chunk::BiomeChunk,
    biome_source::{BiomeSet, BiomeSourceDefinition},
    density::DensityRegistry,
    router::{NoiseRouter, RouterField},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::Path,
};
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let bundle: Value = serde_json::from_str(&fs::read_to_string(dir.join("registries.json"))?)?;
    let noises: BTreeMap<String, String> = serde_json::from_value(bundle["noises"].clone())?;
    let densities: BTreeMap<String, String> = serde_json::from_value(bundle["densities"].clone())?;
    let presets: BTreeMap<String, String> = serde_json::from_value(bundle["presets"].clone())?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let settings: Value = serde_json::from_str(
            bundle["settings"][f["setting"].as_str().unwrap()]
                .as_str()
                .unwrap(),
        )?;
        let mut definitions = DensityRegistry::from_json(&noises, &densities)?;
        let router =
            NoiseRouter::from_json(&settings["noise_router"].to_string(), &mut definitions)?;
        let seed = f["seed"].as_i64().unwrap();
        let mut compiler = DensityCompiler::new_seeded(
            definitions.into_graph(),
            seed,
            f["legacy"].as_bool().unwrap(),
        );
        let climate = router.compile_climate(&mut compiler)?;
        let definition = match f["source"].as_str().unwrap_or("multi") {
            "fixed" => BiomeSourceDefinition::Fixed(
                f["fixed_biome"]
                    .as_str()
                    .unwrap_or("minecraft:plains")
                    .into(),
            ),
            "checkerboard" => BiomeSourceDefinition::Checkerboard {
                biomes: BiomeSet::Direct(vec![
                    "minecraft:plains".into(),
                    "minecraft:the_end".into(),
                    "minecraft:nether_wastes".into(),
                ]),
                scale: n(&f["scale"]),
            },
            "end" => BiomeSourceDefinition::End,
            "multi" => BiomeSourceDefinition::Preset {
                name: f["preset"].as_str().unwrap().into(),
                inline: false,
            },
            _ => panic!("unknown source"),
        };
        let source = definition.resolve(&presets)?;
        let erosion = compiler.get_sampler(router.function(RouterField::Erosion))?;
        let mut resolver = source.resolver(&climate, erosion.as_ref());
        let (cx, cz, min, height) = (n(&f["cx"]), n(&f["cz"]), n(&f["min"]), n(&f["height"]));
        let mut chunk = BiomeChunk::new(
            ChunkPos::new(cx, cz),
            min,
            height,
            "minecraft:plains".to_owned(),
        )?;
        let retrogen = f["retrogen"].as_bool().unwrap_or(false);
        if retrogen {
            chunk.fill(min, min + height - 1, |[x, y, z]| {
                Ok::<_, std::convert::Infallible>(format!(
                    "horizon:old_{}",
                    x.wrapping_mul(31)
                        .wrapping_add(y.wrapping_mul(7))
                        .wrapping_add(z.wrapping_mul(17))
                        .rem_euclid(3)
                ))
            })?;
        }
        for repeat in 0..n(&f["repeats"]) {
            let mut context = SamplerContext::cached();
            let volume = resolver.prepare_volume(
                &mut context,
                [cx.wrapping_shl(4) >> 2, min >> 2, cz.wrapping_shl(4) >> 2],
                [4, height >> 2, 4],
            )?;
            chunk.fill_with_retrogen(
                n(&f["write_min"]),
                n(&f["write_max"]),
                retrogen,
                |quart| {
                    resolver
                        .sample_prepared(&mut context, volume.as_ref(), quart)
                        .map(str::to_owned)
                },
                |name| Some(name),
            )?;
            writeln!(out, "fill {repeat}")?;
            for section in 0..height as usize / 16 {
                writeln!(out, "section {section}")?;
                for name in chunk.section(section).unwrap() {
                    writeln!(out, "biome {name}")?;
                }
            }
            let zoom = BiomeZoom::new(obfuscate_seed(seed));
            for q in f["queries"].as_array().unwrap() {
                let block = BlockPos::new(n(&q[0]), n(&q[1]), n(&q[2]));
                writeln!(
                    out,
                    "query {}",
                    chunk.noise_biome(zoom.quart_at_block(block))
                )?;
            }
        }
    }
    Ok(())
}
