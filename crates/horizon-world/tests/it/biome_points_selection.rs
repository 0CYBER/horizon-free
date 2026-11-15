//! A multi-noise biome list with several points, written as the game writes a mod dimension's
//! `BiomeSourceKind::Points` (inline `minecraft:multi_noise` entries with value intervals),
//! picks each biome where the climate falls in its interval and the nearest point elsewhere.
use horizon_world::worldgen::{
    biome::MultiNoiseBiomeSource,
    biome_source::{BiomeSourceCatalog, BiomeSourceDefinition},
    climate::TargetPoint,
};
use std::collections::{BTreeMap, BTreeSet};

fn entry(biome: &str, temperature: [f64; 2], offset: f64) -> serde_json::Value {
    serde_json::json!({
        "parameters": {
            "temperature": temperature,
            "humidity": [-2.0, 2.0],
            "continentalness": [-2.0, 2.0],
            "erosion": [-2.0, 2.0],
            "depth": [-2.0, 2.0],
            "weirdness": [-2.0, 2.0],
            "offset": offset,
        },
        "biome": biome,
    })
}

fn source(entries: Vec<serde_json::Value>) -> MultiNoiseBiomeSource {
    let catalog = BiomeSourceCatalog::new(
        &BTreeSet::from(["test:cold".into(), "test:warm".into(), "test:hot".into()]),
        &BTreeMap::new(),
        &BTreeSet::new(),
    )
    .unwrap();
    let json = serde_json::json!({"type": "multi_noise", "biomes": entries}).to_string();
    let BiomeSourceDefinition::MultiNoise(decoded) = catalog.decode(&json).unwrap() else {
        panic!("wrong source")
    };
    MultiNoiseBiomeSource::new(decoded).unwrap()
}

fn pick<'a>(
    source: &'a MultiNoiseBiomeSource,
    search: &mut horizon_world::worldgen::climate::ClimateSearch<'a, String>,
    temperature: f32,
) -> &'a str {
    let target = TargetPoint::from_values([temperature, 0.0, 0.0, 0.0, 0.0, 0.0]);
    let expected = source.parameters().brute_force(target);
    let found = search.find(target).unwrap();
    assert_eq!(found, expected);
    assert_eq!(search.find(target).unwrap(), expected);
    found
}

#[test]
fn each_point_wins_inside_its_interval_and_the_nearest_wins_between() {
    let source = source(vec![
        entry("test:cold", [-1.0, -0.4], 0.0),
        entry("test:warm", [-0.2, 0.3], 0.0),
        entry("test:hot", [0.6, 1.0], 0.0),
    ]);
    let mut search = source.parameters().searcher();
    // Inside an interval: that point's biome.
    assert_eq!(pick(&source, &mut search, -0.7), "test:cold");
    assert_eq!(pick(&source, &mut search, 0.0), "test:warm");
    assert_eq!(pick(&source, &mut search, 0.8), "test:hot");
    // Between two intervals: the nearer edge.
    assert_eq!(pick(&source, &mut search, -0.35), "test:cold");
    assert_eq!(pick(&source, &mut search, -0.25), "test:warm");
    assert_eq!(pick(&source, &mut search, -0.45), "test:cold");
    assert_eq!(pick(&source, &mut search, 0.4), "test:warm");
    assert_eq!(pick(&source, &mut search, 0.5), "test:hot");
    // Past the outermost points: the closest end.
    assert_eq!(pick(&source, &mut search, -1.9), "test:cold");
    assert_eq!(pick(&source, &mut search, 1.9), "test:hot");
}

#[test]
fn a_higher_offset_loses_to_an_overlapping_point() {
    let source = source(vec![
        entry("test:cold", [-1.0, 1.0], 0.5),
        entry("test:warm", [-0.2, 0.3], 0.0),
    ]);
    let mut search = source.parameters().searcher();
    // Both cover 0.0: the point with no offset is the closer one.
    assert_eq!(pick(&source, &mut search, 0.0), "test:warm");
    // Only the wide point covers -0.8.
    assert_eq!(pick(&source, &mut search, -0.8), "test:cold");
}
