use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::{BlockPos, ChunkPos},
};
use horizon_world::{
    block_collision_scan::{
        BlockCollisionScan, BlockCollisionWorld, CollisionBlock, CollisionScanError,
    },
    collision_context::CollisionContext,
    registry::BlockStateId,
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

/// Records every cell the scan reads; every block passes the edge filters
/// and has an empty shape.
struct Recording {
    cells: Vec<BlockPos>,
}
impl BlockCollisionWorld for Recording {
    type Error = io::Error;
    type Chunk = ();
    fn collision_chunk(&mut self, _: ChunkPos) -> io::Result<Option<()>> {
        Ok(Some(()))
    }
    fn collision_block(&mut self, _: (), p: BlockPos) -> io::Result<CollisionBlock> {
        self.cells.push(p);
        Ok(CollisionBlock {
            state: BlockStateId(0),
            large_collision_shape: true,
            moving_piston: true,
            empty_shape: true,
        })
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

#[test]
fn cells_follow_the_cursor_order_without_corners() {
    let boxes = [
        Aabb::new(Vec3::new(0.2, 64.0, 0.3), Vec3::new(0.8, 65.95, 0.9)),
        Aabb::new(Vec3::new(-17.5, -3.25, 31.0), Vec3::new(-15.0, -3.25, 33.5)),
        Aabb::new(Vec3::new(5.0, 5.0, 5.0), Vec3::new(5.0, 5.0, 5.0)),
        Aabb::new(Vec3::new(-0.3, 0.0, -0.3), Vec3::new(0.3, 0.9, 0.3)),
    ];
    for bounds in boxes {
        let mut world = Recording { cells: Vec::new() };
        let hits = BlockCollisionScan::new(
            &mut world,
            CollisionContext::empty(false),
            bounds,
            false,
            1_000_000,
        )
        .unwrap()
        .count();
        assert_eq!(hits, 0);
        // Cursor3D: x fastest, then y, then z, skipping the 8 corners.
        let (min, max) = (bounds.min_position(), bounds.max_position());
        let start = BlockPos::containing(Vec3::new(min.x - 1.0e-7, min.y - 1.0e-7, min.z - 1.0e-7))
            .offset(-1, -1, -1);
        let end = BlockPos::containing(Vec3::new(max.x + 1.0e-7, max.y + 1.0e-7, max.z + 1.0e-7))
            .offset(1, 1, 1);
        let sizes = [
            end.x - start.x + 1,
            end.y - start.y + 1,
            end.z - start.z + 1,
        ];
        let mut expected = Vec::new();
        for i in 0..sizes[0] * sizes[1] * sizes[2] {
            let r = [
                i % sizes[0],
                i / sizes[0] % sizes[1],
                i / sizes[0] / sizes[1],
            ];
            let edges = (0..3)
                .filter(|&a| r[a] == 0 || r[a] == sizes[a] - 1)
                .count();
            if edges < 3 {
                expected.push(start.offset(r[0], r[1], r[2]));
            }
        }
        assert_eq!(world.cells, expected);
    }
}
