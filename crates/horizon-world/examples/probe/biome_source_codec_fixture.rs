//! Registry-aware codec transport for biome-source definitions.
use horizon_core::density::{ConstantSampler, SamplerContext};
use horizon_world::worldgen::{
    biome_source::BiomeSourceCatalog, climate::ClimateSampler, tags::TagDefinitions,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
fn normalized(value: Value) -> Value {
    match value {
        Value::Number(n) => {
            serde_json::json!({"float64_bits":format!("{:016x}",n.as_f64().unwrap().to_bits())})
        }
        Value::Array(values) => Value::Array(values.into_iter().map(normalized).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(k, v)| (k, normalized(v)))
                .collect(),
        ),
        value => value,
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let read = |name: &str| -> Result<BTreeMap<String, String>, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(&fs::read_to_string(
            dir.join(format!("{name}.json")),
        )?)?)
    };
    let biomes: BTreeSet<_> = read("biomes")?.into_keys().collect();
    let presets = read("presets")?;
    let mut tags = TagDefinitions::default();
    for (name, raw) in read("biome_tags")? {
        tags.apply_json(&name, "vanilla", &raw)?;
    }
    let tags = tags.build(|name, _| biomes.contains(name).then(|| name.to_owned()))?;
    assert!(tags.missing.is_empty());
    let catalog = BiomeSourceCatalog::new(&biomes, &tags.tags, &presets.keys().cloned().collect())?;
    let positions: Vec<[i32; 3]> =
        serde_json::from_str(&fs::read_to_string(dir.join("positions.json"))?)?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for json in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let definition = match catalog.decode(json) {
            Ok(v) => v,
            Err(_) => {
                writeln!(out, "codec error")?;
                continue;
            }
        };
        let encoded = catalog.encode(&definition)?;
        writeln!(out, "codec {}", normalized(serde_json::from_str(&encoded)?))?;
        let source = definition.resolve(&presets)?;
        writeln!(
            out,
            "possible {}",
            serde_json::to_string(&source.possible_biomes())?
        )?;
        let climate =
            ClimateSampler::new(std::array::from_fn(|_| Arc::new(ConstantSampler(0.)) as _));
        let erosion = ConstantSampler(0.);
        let mut resolver = source.resolver(&climate, &erosion);
        let mut context = SamplerContext::cached();
        for &pos in &positions {
            writeln!(
                out,
                "biome {}",
                resolver.sample(&mut context, pos).unwrap_or("error")
            )?;
        }
    }
    Ok(())
}
