//! Powder snow collision over entity metadata and lazily queried walkability.
use crate::{collision_context::CollisionContext, entity_obstruction::EntityIdentity};
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::BlockPos,
};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug)]
pub struct PowderSnowEntity {
    pub fall_distance: f64,
    pub falling_block: bool,
}

pub trait PowderSnowWorld {
    type Error;
    /// Snapshot of the entity's fallDistance field and FallingBlockEntity type.
    fn powder_snow_entity(
        &mut self,
        entity: EntityIdentity,
    ) -> Result<PowderSnowEntity, Self::Error>;
    fn powder_snow_walkable_tag(&mut self, entity: EntityIdentity) -> Result<bool, Self::Error>;
    /// Invoke only for a living entity; item identity follows ItemStack.is,
    /// including its empty-stack normalization.
    fn feet_are_leather_boots(&mut self, entity: EntityIdentity) -> Result<bool, Self::Error>;
}

pub fn can_walk_on_powder_snow<W: PowderSnowWorld>(
    world: &mut W,
    entity: EntityIdentity,
    living: bool,
) -> Result<bool, W::Error> {
    if world.powder_snow_walkable_tag(entity)? {
        return Ok(true);
    }
    if living {
        return world.feet_are_leather_boots(entity);
    }
    Ok(false)
}

pub fn powder_snow_collision_shape<W: PowderSnowWorld>(
    world: &mut W,
    position: BlockPos,
    context: &CollisionContext<'_>,
) -> Result<VoxelShape, W::Error> {
    if context.is_placement() {
        return Ok(VoxelShape::empty());
    }
    let Some(entity) = context.entity() else {
        return Ok(VoxelShape::empty());
    };
    let snapshot = world.powder_snow_entity(entity)?;
    if snapshot.fall_distance > 2.5 {
        static FALLING: OnceLock<VoxelShape> = OnceLock::new();
        return Ok(FALLING
            .get_or_init(|| {
                VoxelShape::from_box(Aabb::new(
                    Vec3::ZERO,
                    Vec3::new(1.0, f64::from(0.9f32), 1.0),
                ))
                .expect("bounded falling powder snow shape")
            })
            .clone());
    }
    if snapshot.falling_block
        || (can_walk_on_powder_snow(world, entity, context.living_entity().is_some())?
            && context.is_above(&VoxelShape::block(), position, false)
            && !context.is_descending())
    {
        return Ok(VoxelShape::block());
    }
    Ok(VoxelShape::empty())
}
