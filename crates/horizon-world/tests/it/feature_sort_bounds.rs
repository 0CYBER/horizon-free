use horizon_world::worldgen::feature_sort::{
    FeatureIdentity, FeatureSortError, build_features_per_step,
};

#[test]
fn long_dependency_chains_use_bounded_iterative_traversal() {
    let features = (0..50000).map(FeatureIdentity).collect::<Vec<_>>();
    let result = build_features_per_step(&[vec![features.clone()]]).unwrap();
    assert_eq!(result[0].features(), features);
    assert_eq!(result[0].index(FeatureIdentity(49999)), Some(49999));
    assert_eq!(result[0].index(FeatureIdentity(50000)), None);
    // One step and 1 048 576 features: one past the limit.
    assert!(matches!(
        build_features_per_step(&[vec![vec![FeatureIdentity(0); 1 << 20]]]),
        Err(FeatureSortError::WorkLimit)
    ));
}

/// A world pack's thousand biomes, 99 000 steps and features: vanilla orders any number.
#[test]
fn a_thousand_biomes_are_ordered() {
    // Three kinds of biome, each with its own eight features in each of eleven steps.
    let biomes: Vec<Vec<Vec<FeatureIdentity>>> = (0..1000u32)
        .map(|biome| {
            (0..11u32)
                .map(|step| {
                    (0..8u32)
                        .map(|feature| FeatureIdentity(step * 100 + biome % 3 * 8 + feature))
                        .collect()
                })
                .collect()
        })
        .collect();
    let steps = build_features_per_step(&biomes).unwrap();
    assert_eq!(steps.len(), 11);
    for (step, data) in steps.iter().enumerate() {
        assert_eq!(data.features().len(), 24);
        let first = FeatureIdentity(step as u32 * 100);
        let last = FeatureIdentity(step as u32 * 100 + 7);
        assert!(data.index(first) < data.index(last));
    }
}

#[test]
fn opposite_biome_orders_and_repeated_identity_form_cycles() {
    let a = FeatureIdentity(91);
    let b = FeatureIdentity(7);
    assert!(matches!(
        build_features_per_step(&[vec![vec![a, b]], vec![vec![b, a]]]),
        Err(FeatureSortError::Cycle)
    ));
    assert!(matches!(
        build_features_per_step(&[vec![vec![a, a]]]),
        Err(FeatureSortError::Cycle)
    ));
    let result = build_features_per_step(&[vec![vec![a], vec![], vec![a, b]]]).unwrap();
    assert_eq!(result[0].features(), [a]);
    assert!(result[1].features().is_empty());
    assert_eq!(result[2].features(), [a, b]);
}
