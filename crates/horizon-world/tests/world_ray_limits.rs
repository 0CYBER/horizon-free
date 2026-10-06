use horizon_core::{
    geometry::{TraversalError, Vec3, VoxelShape},
    position::BlockPos,
};
use horizon_world::world_ray::{RayBlockMode, RayError, RayFluidMode, RayWorld, clip_world};
#[derive(Default)]
struct World {
    reads: usize,
    fail_fluid: bool,
}
impl RayWorld for World {
    type Error = &'static str;
    type Block = ();
    type Fluid = ();
    fn block_state(&mut self, _: BlockPos) -> Result<(), Self::Error> {
        self.reads += 1;
        Ok(())
    }
    fn fluid_state(&mut self, _: BlockPos) -> Result<(), Self::Error> {
        self.reads += 1;
        if self.fail_fluid {
            Err("fluid unavailable")
        } else {
            Ok(())
        }
    }
    fn block_shape(
        &mut self,
        _: (),
        _: BlockPos,
        _: RayBlockMode,
    ) -> Result<VoxelShape, Self::Error> {
        Ok(VoxelShape::empty())
    }
    fn interaction_shape(&mut self, _: (), _: BlockPos) -> Result<VoxelShape, Self::Error> {
        panic!("empty block cannot have interaction hit")
    }
    fn fluid_is_source(&mut self, _: ()) -> Result<bool, Self::Error> {
        panic!("fluid mode is NONE")
    }
    fn fluid_is_empty(&mut self, _: ()) -> Result<bool, Self::Error> {
        panic!("fluid mode is NONE")
    }
    fn fluid_is_water(&mut self, _: ()) -> Result<bool, Self::Error> {
        panic!("fluid mode is NONE")
    }
    fn fluid_shape(&mut self, _: (), _: BlockPos) -> Result<VoxelShape, Self::Error> {
        panic!("fluid mode is NONE")
    }
}
#[test]
fn visit_limit_retains_queries_and_does_not_report_miss() {
    let mut world = World::default();
    let result = clip_world(
        &mut world,
        Vec3::new(0.5, 0.5, 0.5),
        Vec3::new(3.5, 0.5, 0.5),
        RayBlockMode::Collider,
        RayFluidMode::None,
        1,
    );
    assert!(matches!(
        result,
        Err(RayError::Traversal(TraversalError::VisitLimit))
    ));
    assert_eq!(world.reads, 2);
}
#[test]
fn nonfinite_ray_fails_before_world_reads() {
    let mut world = World::default();
    let result = clip_world(
        &mut world,
        Vec3::ZERO,
        Vec3::new(f64::INFINITY, 0., 0.),
        RayBlockMode::Collider,
        RayFluidMode::None,
        10,
    );
    assert!(matches!(
        result,
        Err(RayError::Traversal(TraversalError::NonFinite))
    ));
    assert_eq!(world.reads, 0);
}
#[test]
fn provider_failure_is_distinct_from_traversal_failure() {
    let mut world = World {
        fail_fluid: true,
        ..Default::default()
    };
    let result = clip_world(
        &mut world,
        Vec3::ZERO,
        Vec3::new(1., 0., 0.),
        RayBlockMode::Collider,
        RayFluidMode::None,
        10,
    );
    assert!(matches!(result, Err(RayError::World("fluid unavailable"))));
    assert_eq!(world.reads, 2);
}
