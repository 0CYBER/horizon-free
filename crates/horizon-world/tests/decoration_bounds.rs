use horizon_core::position::ChunkPos;
use horizon_world::worldgen::{
    decoration::{DecorationBiome, DecorationError, DecorationWorld, FeatureDecorationPlan},
    feature_sort::{FeatureIdentity, FeatureSortError},
};
struct NoChunks;
impl DecorationWorld for NoChunks {
    type Error = ();
    fn min_section_y(&mut self) -> Result<i32, ()> {
        Ok(-4)
    }
    fn chunk_biomes(&mut self, _: ChunkPos) -> Result<Vec<DecorationBiome>, ()> {
        panic!("unbounded neighborhood must be rejected before reading chunks")
    }
    fn set_currently_generating(&mut self, _: Option<FeatureIdentity>) -> Result<(), ()> {
        panic!("invalid neighborhood must not start or finish decoration")
    }
}
#[test]
fn invalid_registry_identity_and_unbounded_neighborhood_are_rejected() {
    assert!(matches!(
        FeatureDecorationPlan::new(vec![
            (DecorationBiome(0), vec![]),
            (DecorationBiome(0), vec![])
        ]),
        Err(FeatureSortError::DuplicateBiome)
    ));
    let plan = FeatureDecorationPlan::new(vec![]).unwrap();
    for chunk in [ChunkPos::new(i32::MIN, 0), ChunkPos::new(0, i32::MAX)] {
        let result = plan.decorate_features(&mut NoChunks, chunk, 0, |_, _, _, _, _| {
            panic!("invalid neighborhood must not place features")
        });
        assert!(matches!(result, Err(DecorationError::WorkLimit)));
    }
}
