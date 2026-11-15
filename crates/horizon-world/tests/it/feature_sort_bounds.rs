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
    assert!(matches!(
        build_features_per_step(&[vec![vec![FeatureIdentity(0); 65536]]]),
        Err(FeatureSortError::WorkLimit)
    ));
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
