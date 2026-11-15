use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::{
    heightmap::HeightmapType,
    worldgen::{
        anchor::WorldGenerationContext,
        placement::{PlacementContext, PlacementError, PlacementModifier, PlacementQueryError},
    },
};
struct Context {
    queries: Vec<HeightmapType>,
}
impl PlacementContext for Context {
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(0, 16, 8)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        panic!("water depth does not query the world minimum")
    }
    fn height(&mut self, kind: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        self.queries.push(kind);
        if kind == HeightmapType::OceanFloor {
            Ok(7)
        } else {
            Err(PlacementQueryError::Failed("surface unavailable".into()))
        }
    }
}
#[test]
fn second_height_query_failure_preserves_order_without_emission_or_random_draws() {
    let mut context = Context { queries: vec![] };
    let mut random = LegacyRandom::new(17);
    let mut expected = random.clone();
    let result = PlacementModifier::SurfaceWaterDepth(7).modify_in(
        &mut context,
        &mut random,
        [1, 9, 2],
        |_, _| -> Result<(), ()> { panic!("failed query emitted a position") },
    );
    assert!(matches!(result, Err(PlacementError::World(_))));
    assert_eq!(
        context.queries,
        [HeightmapType::OceanFloor, HeightmapType::WorldSurface]
    );
    assert_eq!(random.next_long(), expected.next_long());
}
