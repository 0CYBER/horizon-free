//! Entity.collide collection order and step resolution over a collision world.
//! Travel, velocity, grounded flags and inside-block effects remain caller work.
use crate::{
    block_collision_scan::{BlockCollisionScan, BlockCollisionWorld, CollisionScanError},
    collision_context::CollisionContext,
    entity_obstruction::EntityIdentity,
};
use horizon_core::geometry::{
    Aabb, Vec3, VoxelShape, entity_collision_bounds, try_resolve_step_movement,
};

pub trait EntityMovementWorld {
    type Error;
    /// Return native-selected entity shapes in query order. Selection occurs
    /// once, even for zero movement, and the list is reused during stepping.
    fn entity_collisions(
        &mut self,
        entity: EntityIdentity,
        bounds: Aabb,
    ) -> Result<Vec<VoxelShape>, Self::Error>;
    /// Obtain the border, check isInsideCloseToBorder and fetch its shape only
    /// when applicable. Called before block iteration for each collision query.
    fn movement_border(
        &mut self,
        entity: EntityIdentity,
        bounds: Aabb,
    ) -> Result<Option<VoxelShape>, Self::Error>;
}

pub struct MovementRequest<'a> {
    pub entity: EntityIdentity,
    /// Captured context for this entity. Experimental minecart exclusions need
    /// their separate context layer; ordinary captured contexts are supported.
    pub context: CollisionContext<'a>,
    pub bounds: Aabb,
    pub movement: Vec3,
    pub max_step: f32,
    pub on_ground: bool,
    /// Per block scan, including skipped cursor cells.
    pub candidate_budget: usize,
}

pub fn resolve_world_movement<W, E>(
    world: &mut W,
    request: MovementRequest<'_>,
) -> Result<Vec3, CollisionScanError<E>>
where
    W: BlockCollisionWorld<Error = E> + EntityMovementWorld<Error = E>,
{
    let entities = world
        .entity_collisions(
            request.entity,
            entity_collision_bounds(request.movement, request.bounds, request.max_step),
        )
        .map_err(CollisionScanError::World)?;
    try_resolve_step_movement(
        request.movement,
        request.bounds,
        request.max_step,
        request.on_ground,
        |bounds| {
            let mut shapes = entities.clone();
            if let Some(border) = world
                .movement_border(request.entity, bounds)
                .map_err(CollisionScanError::World)?
            {
                shapes.push(border);
            }
            for hit in BlockCollisionScan::new(
                world,
                request.context,
                bounds,
                false,
                request.candidate_budget,
            )? {
                shapes.push(hit?.shape);
            }
            Ok(shapes)
        },
    )
}

/// Entity-bearing CollisionGetter.noCollision, with block/context selection
/// supplied by the caller (including an always-fluid context when requested).
/// Native short-circuits blocks, then entity selection, then border geometry.
pub struct CollisionCheckRequest<'a> {
    pub entity: EntityIdentity,
    pub context: CollisionContext<'a>,
    pub bounds: Aabb,
    pub candidate_budget: usize,
}
pub fn no_world_collision<W, E>(
    world: &mut W,
    request: CollisionCheckRequest<'_>,
) -> Result<bool, CollisionScanError<E>>
where
    W: BlockCollisionWorld<Error = E> + EntityMovementWorld<Error = E>,
{
    for hit in BlockCollisionScan::new(
        world,
        request.context,
        request.bounds,
        false,
        request.candidate_budget,
    )? {
        if !hit?.shape.is_empty() {
            return Ok(false);
        }
    }
    if !world
        .entity_collisions(request.entity, request.bounds)
        .map_err(CollisionScanError::World)?
        .is_empty()
    {
        return Ok(false);
    }
    if let Some(border) = world
        .movement_border(request.entity, request.bounds)
        .map_err(CollisionScanError::World)?
    {
        let entity = VoxelShape::from_box(request.bounds).map_err(CollisionScanError::Shape)?;
        return border
            .join_is_not_empty(&entity, horizon_core::geometry::ShapeOp::AND)
            .map(|overlap| !overlap)
            .map_err(CollisionScanError::Shape);
    }
    Ok(true)
}
