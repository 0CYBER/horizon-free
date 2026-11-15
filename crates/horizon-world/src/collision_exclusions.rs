//! Collision exclusions (arismoko/horizon#3717): boxes of an entity's own level in which that level's blocks do
//! not stop it, for a mod's portal in a wall. The part of a block's collision shape inside a box
//! is cut away, `Shapes.join(shape, box, ONLY_FIRST)`, as Immersive Portals'
//! `PortalCollisionHandler.processThisSideCollisionShape` does for the portals an entity collides
//! with. Entity colliders and the world border stay as they are.
//!
//! The set is the engine's, kept on the player and never saved; a client and a server each hold
//! their own, and nothing sends one to the other.
use crate::{
    block_collision_scan::{BlockCollisionWorld, CollisionBlock, CollisionScanError},
    collision_context::CollisionContext,
    entity_obstruction::EntityIdentity,
    world_movement::{self, CollisionCheckRequest, EntityMovementWorld, MovementRequest},
};
use horizon_core::{
    geometry::{Aabb, Axis, ShapeOp, Vec3, VoxelShape},
    position::{BlockPos, ChunkPos},
};

/// The most boxes a set holds.
pub const MAX_BOXES: usize = 64;

/// Why a set of boxes was refused. The set held before stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// More than [`MAX_BOXES`] boxes.
    TooMany,
    /// A coordinate that is NaN or infinite.
    NotFinite,
    /// A box whose minimum is above its maximum on an axis.
    Inverted,
}

/// An entity's collision exclusions: world boxes, none until a mod sets them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollisionExclusions(Vec<Aabb>);

impl CollisionExclusions {
    /// Replaces the set with `boxes` (`[min_x, min_y, min_z, max_x, max_y, max_z]` each); an
    /// empty slice clears it. Refused, leaving the set as it was: more than [`MAX_BOXES`] boxes,
    /// then a value that isn't finite, then a minimum above its maximum.
    pub fn set(&mut self, boxes: &[[f64; 6]]) -> Result<(), Refused> {
        if boxes.len() > MAX_BOXES {
            return Err(Refused::TooMany);
        }
        if boxes.iter().flatten().any(|v| !v.is_finite()) {
            return Err(Refused::NotFinite);
        }
        if boxes.iter().any(|b| (0..3).any(|i| b[i] > b[i + 3])) {
            return Err(Refused::Inverted);
        }
        self.0.clear();
        self.0.extend(
            boxes
                .iter()
                .map(|b| Aabb::new(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5]))),
        );
        Ok(())
    }
    /// No boxes any more (a change of level, a respawn).
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn boxes(&self) -> &[Aabb] {
        &self.0
    }
    /// Whether a box shares volume with the block at `p`: that block is no wall to suffocate in
    /// (`Entity.isInWall`, `LocalPlayer.suffocatesAt`).
    pub fn spares(&self, p: BlockPos) -> bool {
        self.0.iter().any(|b| b.intersects(Aabb::block(p)))
    }
}

/// `inner` contains all of `shape`, faces included (Immersive Portals' `Helper.boxContains`).
fn contains(inner: Aabb, shape: Aabb) -> bool {
    Axis::ALL
        .into_iter()
        .all(|a| inner.min(a) <= shape.min(a) && inner.max(a) >= shape.max(a))
}

/// The collision shape `shape` of the block at `p` (in the block's own coordinates) without its
/// parts inside `boxes`: a box holding the whole shape leaves nothing, one that shares no volume
/// with it leaves it as it is.
pub fn cut(mut shape: VoxelShape, p: BlockPos, boxes: &[Aabb]) -> VoxelShape {
    let corner = Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)).reverse();
    for b in boxes {
        let Some(bounds) = shape.bounds() else {
            break;
        };
        let local = b.move_by(corner);
        if !local.intersects(bounds) {
            continue;
        }
        if contains(local, bounds) {
            return VoxelShape::empty();
        }
        // Finite boxes always make a shape; a failed join keeps the block solid.
        if let Ok(rest) =
            VoxelShape::from_box(local).and_then(|part| shape.join(&part, ShapeOp::ONLY_FIRST))
        {
            shape = rest;
        }
    }
    shape
}

/// A collision world whose block shapes lose their parts inside `boxes`; entity colliders and the
/// border are `world`'s.
pub struct Excluding<'a, W> {
    pub world: &'a mut W,
    pub boxes: &'a [Aabb],
}

impl<W: BlockCollisionWorld> BlockCollisionWorld for Excluding<'_, W> {
    type Error = W::Error;
    type Chunk = W::Chunk;
    fn collision_chunk(&mut self, p: ChunkPos) -> Result<Option<Self::Chunk>, Self::Error> {
        self.world.collision_chunk(p)
    }
    fn collision_block(
        &mut self,
        chunk: Self::Chunk,
        p: BlockPos,
    ) -> Result<CollisionBlock, Self::Error> {
        self.world.collision_block(chunk, p)
    }
    fn is_suffocating(
        &mut self,
        chunk: Self::Chunk,
        block: CollisionBlock,
        p: BlockPos,
    ) -> Result<bool, Self::Error> {
        self.world.is_suffocating(chunk, block, p)
    }
    fn context_shape(
        &mut self,
        block: CollisionBlock,
        p: BlockPos,
        context: &CollisionContext<'_>,
    ) -> Result<VoxelShape, Self::Error> {
        let shape = self.world.context_shape(block, p, context)?;
        Ok(cut(shape, p, self.boxes))
    }
    fn may_reach_out(&mut self, chunk: Self::Chunk, section_y: i32) -> Result<bool, Self::Error> {
        self.world.may_reach_out(chunk, section_y)
    }
    fn section_is_empty(
        &mut self,
        chunk: Self::Chunk,
        section_y: i32,
    ) -> Result<bool, Self::Error> {
        self.world.section_is_empty(chunk, section_y)
    }
    fn missing_chunks_stay_missing(&self) -> bool {
        self.world.missing_chunks_stay_missing()
    }
}

impl<W: EntityMovementWorld> EntityMovementWorld for Excluding<'_, W> {
    type Error = W::Error;
    fn entity_collisions(
        &mut self,
        entity: EntityIdentity,
        bounds: Aabb,
    ) -> Result<Vec<VoxelShape>, Self::Error> {
        self.world.entity_collisions(entity, bounds)
    }
    fn movement_border(
        &mut self,
        entity: EntityIdentity,
        bounds: Aabb,
    ) -> Result<Option<VoxelShape>, Self::Error> {
        self.world.movement_border(entity, bounds)
    }
}

/// `Entity.collide` ([`world_movement::resolve_world_movement`]) through `exclusions`. With none,
/// the move costs one empty-slice check more than without them.
pub fn resolve_world_movement<W, E>(
    world: &mut W,
    exclusions: &CollisionExclusions,
    request: MovementRequest<'_>,
) -> Result<Vec3, CollisionScanError<E>>
where
    W: BlockCollisionWorld<Error = E> + EntityMovementWorld<Error = E>,
{
    if exclusions.is_empty() {
        return world_movement::resolve_world_movement(world, request);
    }
    let boxes = exclusions.boxes();
    world_movement::resolve_world_movement(&mut Excluding { world, boxes }, request)
}

/// `CollisionGetter.noCollision` ([`world_movement::no_world_collision`]) through `exclusions`,
/// at the same cost.
pub fn no_world_collision<W, E>(
    world: &mut W,
    exclusions: &CollisionExclusions,
    request: CollisionCheckRequest<'_>,
) -> Result<bool, CollisionScanError<E>>
where
    W: BlockCollisionWorld<Error = E> + EntityMovementWorld<Error = E>,
{
    if exclusions.is_empty() {
        return world_movement::no_world_collision(world, request);
    }
    let boxes = exclusions.boxes();
    world_movement::no_world_collision(&mut Excluding { world, boxes }, request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::BlockStateId;
    use std::convert::Infallible;

    /// Air, but a wall of stone one block thick at x = 2 from y = 64 up.
    struct Wall;
    impl BlockCollisionWorld for Wall {
        type Error = Infallible;
        type Chunk = ();
        fn collision_chunk(&mut self, _: ChunkPos) -> Result<Option<()>, Infallible> {
            Ok(Some(()))
        }
        fn collision_block(&mut self, _: (), p: BlockPos) -> Result<CollisionBlock, Infallible> {
            Ok(CollisionBlock {
                state: BlockStateId(1),
                large_collision_shape: false,
                moving_piston: false,
                empty_shape: p.x != 2 || p.y < 64,
            })
        }
        fn is_suffocating(
            &mut self,
            _: (),
            _: CollisionBlock,
            _: BlockPos,
        ) -> Result<bool, Infallible> {
            Ok(true)
        }
        fn context_shape(
            &mut self,
            _: CollisionBlock,
            _: BlockPos,
            _: &CollisionContext<'_>,
        ) -> Result<VoxelShape, Infallible> {
            Ok(VoxelShape::block())
        }
    }
    impl EntityMovementWorld for Wall {
        type Error = Infallible;
        fn entity_collisions(
            &mut self,
            _: EntityIdentity,
            _: Aabb,
        ) -> Result<Vec<VoxelShape>, Infallible> {
            Ok(Vec::new())
        }
        fn movement_border(
            &mut self,
            _: EntityIdentity,
            _: Aabb,
        ) -> Result<Option<VoxelShape>, Infallible> {
            Ok(None)
        }
    }

    fn set(boxes: &[[f64; 6]]) -> CollisionExclusions {
        let mut set = CollisionExclusions::default();
        set.set(boxes).unwrap();
        set
    }

    /// How far a player's box (0.6 wide, 1.8 tall, its east face at x = 1) gets when it walks
    /// 2 blocks east at the wall.
    fn walk_east(exclusions: &CollisionExclusions) -> f64 {
        let bounds = Aabb::new(Vec3::new(0.4, 64.0, 0.2), Vec3::new(1.0, 65.8, 0.8));
        resolve_world_movement(
            &mut Wall,
            exclusions,
            MovementRequest {
                entity: EntityIdentity(0),
                context: CollisionContext::position(64.0),
                bounds,
                movement: Vec3::new(2.0, 0.0, 0.0),
                max_step: 0.6,
                on_ground: true,
                candidate_budget: 1_000_000,
            },
        )
        .unwrap()
        .x
    }

    #[test]
    fn an_excluded_box_lets_the_entity_through_the_wall() {
        // A portal's opening, 1 wide and 2 tall, through the wall.
        assert_eq!(walk_east(&set(&[[2.0, 64.0, 0.0, 3.0, 66.0, 1.0]])), 2.0);
        // Half the wall's depth: the block's back half still stops it.
        assert_eq!(walk_east(&set(&[[2.0, 64.0, 0.0, 2.5, 66.0, 1.0]])), 1.5);
    }

    #[test]
    fn outside_its_boxes_the_entity_collides() {
        assert_eq!(walk_east(&CollisionExclusions::default()), 1.0);
        // An opening beside the body's column, and one only as high as its legs.
        assert_eq!(walk_east(&set(&[[2.0, 64.0, 3.0, 3.0, 66.0, 4.0]])), 1.0);
        assert_eq!(walk_east(&set(&[[2.0, 64.0, 0.0, 3.0, 65.0, 1.0]])), 1.0);
    }

    #[test]
    fn a_refused_set_keeps_the_one_before() {
        let opening = [[2.0, 64.0, 0.0, 3.0, 66.0, 1.0]];
        let mut exclusions = set(&opening);
        let too_many = [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]; 65];
        for (boxes, why) in [
            (&too_many[..], Refused::TooMany),
            (
                &[[0.0, 0.0, 0.0, f64::NAN, 1.0, 1.0]][..],
                Refused::NotFinite,
            ),
            (
                &[[0.0, f64::INFINITY, 0.0, 1.0, 1.0, 1.0]][..],
                Refused::NotFinite,
            ),
            (&[[0.0, 2.0, 0.0, 1.0, 1.0, 1.0]][..], Refused::Inverted),
        ] {
            assert_eq!(exclusions.set(boxes), Err(why));
            assert_eq!(exclusions, set(&opening));
        }
        assert_eq!(exclusions.set(&too_many[..64]), Ok(()));
        assert_eq!(exclusions.boxes().len(), 64);
        assert_eq!(exclusions.set(&[]), Ok(()));
        assert_eq!(walk_east(&exclusions), 1.0);
    }

    #[test]
    fn no_collision_reads_the_exclusions_too() {
        let inside = Aabb::new(Vec3::new(2.2, 64.0, 0.2), Vec3::new(2.8, 65.8, 0.8));
        let check = |exclusions: &CollisionExclusions| {
            no_world_collision(
                &mut Wall,
                exclusions,
                CollisionCheckRequest {
                    entity: EntityIdentity(0),
                    context: CollisionContext::position(64.0),
                    bounds: inside,
                    candidate_budget: 1_000_000,
                },
            )
            .unwrap()
        };
        assert!(!check(&CollisionExclusions::default()));
        assert!(check(&set(&[[2.0, 64.0, 0.0, 3.0, 66.0, 1.0]])));
        let opening = set(&[[2.0, 64.0, 0.0, 3.0, 66.0, 1.0]]);
        assert!(opening.spares(BlockPos::new(2, 65, 0)));
        assert!(!opening.spares(BlockPos::new(2, 66, 0)));
    }
}
