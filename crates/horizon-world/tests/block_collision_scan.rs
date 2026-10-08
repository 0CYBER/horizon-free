use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::{BlockPos, ChunkPos},
};
use horizon_world::{
    block_collision_scan::{
        BlockCollisionScan, BlockCollisionWorld, CollisionBlock, CollisionScanError,
    },
    collision_context::CollisionContext,
};
use std::io;

struct FailedChunk {
    calls: usize,
}
impl BlockCollisionWorld for FailedChunk {
    type Error = io::Error;
    type Chunk = ();
    fn collision_chunk(&mut self, _: ChunkPos) -> io::Result<Option<()>> {
        self.calls += 1;
        Err(io::Error::other("missing test provider"))
    }
    fn collision_block(&mut self, _: (), _: BlockPos) -> io::Result<CollisionBlock> {
        unreachable!()
    }
    fn is_suffocating(&mut self, _: (), _: CollisionBlock, _: BlockPos) -> io::Result<bool> {
        unreachable!()
    }
    fn context_shape(
        &mut self,
        _: CollisionBlock,
        _: BlockPos,
        _: &CollisionContext<'_>,
    ) -> io::Result<VoxelShape> {
        unreachable!()
    }
}
fn bounds() -> Aabb {
    Aabb::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0))
}

#[test]
fn budgets_include_skipped_corners_and_errors_fuse_iteration() {
    for budget in [0, 1] {
        let mut world = FailedChunk { calls: 0 };
        let mut scan = BlockCollisionScan::new(
            &mut world,
            CollisionContext::empty(false),
            bounds(),
            false,
            budget,
        )
        .unwrap();
        assert!(matches!(
            scan.next(),
            Some(Err(CollisionScanError::VisitBudget))
        ));
        assert!(scan.next().is_none());
        drop(scan);
        assert_eq!(world.calls, 0);
    }
    let mut world = FailedChunk { calls: 0 };
    let mut scan = BlockCollisionScan::new(
        &mut world,
        CollisionContext::empty(false),
        bounds(),
        false,
        1000,
    )
    .unwrap();
    assert!(matches!(
        scan.next(),
        Some(Err(CollisionScanError::World(_)))
    ));
    assert!(scan.next().is_none());
    drop(scan);
    assert_eq!(world.calls, 1);
}

#[test]
fn invalid_or_excessive_domains_fail_before_world_queries() {
    let mut world = FailedChunk { calls: 0 };
    for coordinate in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let bounds = Aabb::new(Vec3::ZERO, Vec3::new(coordinate, 1.0, 1.0));
        assert!(matches!(
            BlockCollisionScan::new(
                &mut world,
                CollisionContext::empty(false),
                bounds,
                false,
                1000
            ),
            Err(CollisionScanError::InvalidBounds)
        ));
    }
    let bounds = Aabb::new(Vec3::ZERO, Vec3::new(100000.0, 100000.0, 100000.0));
    assert!(matches!(
        BlockCollisionScan::new(
            &mut world,
            CollisionContext::empty(false),
            bounds,
            false,
            1000
        ),
        Err(CollisionScanError::TooLarge)
    ));
    assert_eq!(world.calls, 0);
}
