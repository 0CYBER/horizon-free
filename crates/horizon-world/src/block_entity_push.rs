//! Block.pushEntitiesUp when replacement collision geometry adds occupied space.
use crate::{entity_obstruction::EntityIdentity, registry::BlockStateId};
use horizon_core::{
    geometry::{Aabb, Axis, ShapeError, ShapeOp, Vec3, VoxelShape},
    position::BlockPos,
};

pub trait BlockPushWorld {
    type Error;
    /// Native no-context block collision query, first old state, then new state.
    fn collision_shape(
        &mut self,
        state: BlockStateId,
        position: BlockPos,
    ) -> Result<VoxelShape, Self::Error>;
    /// Return the native-selected nonspectator entities in query order. Selection
    /// excludes no entity and uses the added shape's bounds, not exact overlap.
    fn entities(&mut self, bounds: Aabb) -> Result<Vec<EntityIdentity>, Self::Error>;
    /// Read each current box just before its teleport: an earlier teleport can
    /// change later entities. Handles must remain valid throughout this call.
    fn entity_bounds(&self, entity: EntityIdentity) -> Aabb;
    fn teleport_relative(
        &mut self,
        entity: EntityIdentity,
        movement: Vec3,
    ) -> Result<(), Self::Error>;
}
#[derive(Debug)]
pub enum BlockPushError<E> {
    World(E),
    Shape(ShapeError),
}
impl<E: std::fmt::Display> std::fmt::Display for BlockPushError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Shape(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for BlockPushError<E> {}
pub fn push_entities_up<W: BlockPushWorld>(
    world: &mut W,
    old: BlockStateId,
    new: BlockStateId,
    position: BlockPos,
) -> Result<BlockStateId, BlockPushError<W::Error>> {
    let old_shape = world
        .collision_shape(old, position)
        .map_err(BlockPushError::World)?;
    let new_shape = world
        .collision_shape(new, position)
        .map_err(BlockPushError::World)?;
    let added = old_shape
        .join_unoptimized(&new_shape, ShapeOp::ONLY_SECOND)
        .map_err(BlockPushError::Shape)?
        .moved(Vec3::new(
            f64::from(position.x),
            f64::from(position.y),
            f64::from(position.z),
        ));
    if let Some(bounds) = added.bounds() {
        let entities = world.entities(bounds).map_err(BlockPushError::World)?;
        for entity in entities {
            let bounds = world
                .entity_bounds(entity)
                .move_by(Vec3::new(0.0, 1.0, 0.0));
            let dy = added.collide(Axis::Y, bounds, -1.0);
            world
                .teleport_relative(entity, Vec3::new(0.0, 1.0 + dy, 0.0))
                .map_err(BlockPushError::World)?;
        }
    }
    Ok(new)
}
