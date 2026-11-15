use horizon_core::{
    density::{ConstantSampler, DensitySampler, GradientSampler, SamplerContext, TilingMode},
    geometry::Axis,
};
use horizon_world::worldgen::{
    biome::{BiomeError, MultiNoiseBiomeSource},
    climate::ClimateSampler,
};
use std::sync::Arc;

fn fixture() -> MultiNoiseBiomeSource {
    MultiNoiseBiomeSource::from_quantized_json(
        r#"[
          {"coordinates":[[-10,-10],[0,0],[0,0],[0,0],[0,0],[0,0]],"offset":0,"biome":"left"},
          {"coordinates":[[10,10],[0,0],[0,0],[0,0],[0,0],[0,0]],"offset":0,"biome":"right"}
        ]"#,
    )
    .unwrap()
}

#[test]
fn scalar_and_prepared_queries_share_ties_and_reject_outside_volume() {
    let fields: [Arc<dyn DensitySampler>; 6] =
        std::array::from_fn(|i| -> Arc<dyn DensitySampler> {
            if i == 0 {
                Arc::new(GradientSampler::new(
                    Axis::X,
                    TilingMode::ClampToEdge,
                    -4,
                    4,
                    -0.001,
                    0.001,
                ))
            } else {
                Arc::new(ConstantSampler(0.))
            }
        });
    let climate = ClimateSampler::new(fields);
    let source = fixture();
    let mut resolver = source.resolver(&climate);
    let mut context = SamplerContext::cached();
    assert_eq!(
        resolver.sample(&mut context, [1, 0, 0]).unwrap(),
        "minecraft:right"
    );
    let volume = resolver
        .prepare_volume(&mut context, [-1, 0, 0], [3, 2, 2])
        .unwrap();
    assert_eq!(
        resolver.sample_prepared(&volume, [0, 0, 0]).unwrap(),
        "minecraft:right"
    );
    assert_eq!(
        source
            .resolver(&climate)
            .sample_prepared(&volume, [0, 0, 0])
            .unwrap(),
        "minecraft:left"
    );
    assert!(matches!(
        resolver.sample_prepared(&volume, [-2, 1, 1]),
        Err(BiomeError::OutsideVolume)
    ));
    assert!(matches!(
        resolver.sample_prepared(&volume, [2, 0, 0]),
        Err(BiomeError::OutsideVolume)
    ));
    assert_eq!(
        resolver.sample_prepared(&volume, [-1, 1, 1]).unwrap(),
        "minecraft:left"
    );
    assert_eq!(
        resolver.sample(&mut context, [0, 0, 0]).unwrap(),
        "minecraft:left"
    );
}

#[test]
fn quantized_import_rejects_malformed_dimensions_numbers_and_identifiers() {
    for json in [
        "[]",
        r#"[{"coordinates":[[0,0]],"offset":0,"biome":"left"}]"#,
        r#"[{"coordinates":[[0,0],[0,0],[0,0],[0,0],[0,0],[0,0]],"offset":0.5,"biome":"left"}]"#,
        r#"[{"coordinates":[[0,0],[0,0],[0,0],[0,0],[0,0],[0,0]],"offset":0,"biome":"UPPER"}]"#,
    ] {
        assert!(
            MultiNoiseBiomeSource::from_quantized_json(json).is_err(),
            "{json}"
        );
    }
}
