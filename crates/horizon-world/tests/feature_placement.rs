use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::worldgen::{
    anchor::WorldGenerationContext,
    placement::{FeaturePlacementError, FeaturePlacer, PlacementModifier},
    value_provider::IntProvider,
};

#[test]
fn oversized_chains_fail_before_random_draws_or_feature_calls() {
    let mut random = LegacyRandom::new(17);
    let mut expected = random.clone();
    let mut placer = FeaturePlacer::new();
    let result = placer.place(
        &vec![PlacementModifier::InSquare; 1025],
        WorldGenerationContext::new(-64, 384, 63),
        &mut random,
        [0, 0, 0],
        |_, _| -> Result<bool, ()> { panic!("oversized chain reached feature") },
    );
    assert!(matches!(result, Err(FeaturePlacementError::WorkLimit)));
    assert_eq!(random.next_long(), expected.next_long());
}

#[test]
fn multiplicative_counts_stop_at_the_shared_work_bound() {
    let mut random = LegacyRandom::new(17);
    let mut placer = FeaturePlacer::new();
    let mut calls = 0;
    let result = placer.place(
        &vec![PlacementModifier::Count(IntProvider::Constant(4096)); 2],
        WorldGenerationContext::new(-64, 384, 63),
        &mut random,
        [0, 0, 0],
        |_, _| {
            calls += 1;
            Ok::<_, ()>(true)
        },
    );
    assert!(matches!(result, Err(FeaturePlacementError::WorkLimit)));
    assert!(calls > 0 && calls < 1 << 20);
}
