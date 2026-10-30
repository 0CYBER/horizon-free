use horizon_world::worldgen::feature_sort::{
    FeatureIdentity, FeatureSortError, build_features_per_step,
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let biomes: Vec<Vec<Vec<u32>>> = serde_json::from_value(f["biomes"].clone())?;
        let biomes = biomes
            .into_iter()
            .map(|steps| {
                steps
                    .into_iter()
                    .map(|step| step.into_iter().map(FeatureIdentity).collect())
                    .collect()
            })
            .collect::<Vec<_>>();
        match build_features_per_step(&biomes) {
            Err(FeatureSortError::Cycle) => writeln!(out, "cycle")?,
            Err(error) => return Err(error.into()),
            Ok(steps) => {
                writeln!(out, "steps {}", steps.len())?;
                for (index, step) in steps.iter().enumerate() {
                    writeln!(out, "step {index} {}", step.features().len())?;
                    for feature in step.features() {
                        writeln!(out, "feature {}", feature.0)?;
                    }
                    for id in f["queries"].as_array().unwrap() {
                        let id = FeatureIdentity(id.as_u64().unwrap() as u32);
                        writeln!(
                            out,
                            "index {} {}",
                            id.0,
                            step.index(id).map_or(-1, |i| i as i32)
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}
