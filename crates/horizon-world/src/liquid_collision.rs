//! Contextual water/lava collision selection. Fluid simulation and entity-specific
//! standing shapes are supplied by the caller; no-context caches are not used.
use crate::{
    chunk::BlockState, collision_context::CollisionContext, entity_obstruction::EntityIdentity,
    fluid::FluidState,
};
use horizon_core::{geometry::VoxelShape, position::BlockPos};
use std::io;

pub trait LiquidCollisionWorld {
    type Error;
    fn entity_liquid_shape(&mut self, entity: EntityIdentity) -> Result<VoxelShape, Self::Error>;
    fn fluid(&mut self, position: BlockPos) -> Result<FluidState, Self::Error>;
    fn entity_can_stand(
        &mut self,
        entity: EntityIdentity,
        fluid: FluidState,
    ) -> Result<bool, Self::Error>;
}

#[derive(Clone, Copy, Debug)]
pub struct LiquidCollision {
    fluid: FluidState,
}
impl LiquidCollision {
    pub fn from_state(state: &BlockState) -> io::Result<Option<Self>> {
        if !matches!(state.name.as_str(), "minecraft:water" | "minecraft:lava") {
            return Ok(None);
        }
        Ok(Some(Self {
            fluid: FluidState::from_block(state)?,
        }))
    }

    pub fn shape<W: LiquidCollisionWorld>(
        &self,
        world: &mut W,
        position: BlockPos,
        context: &CollisionContext<'_>,
    ) -> Result<VoxelShape, W::Error> {
        if context.always_collide_with_fluid() {
            return Ok(VoxelShape::block());
        }
        if !self.fluid.is_source() {
            return Ok(VoxelShape::empty());
        }
        let Some(entity) = context.living_entity() else {
            return Ok(VoxelShape::empty());
        };
        let shape = world.entity_liquid_shape(entity)?;
        if context.is_above(&shape, position, true) {
            // The above-block query precedes the entity callback, even if the
            // candidate shape is empty or the two fluid types are equivalent.
            let above = world.fluid(position.offset(0, 1, 0))?;
            if context.can_stand_on_fluid(above, self.fluid, |entity, fluid| {
                world.entity_can_stand(entity, fluid)
            })? {
                return Ok(shape);
            }
        }
        Ok(VoxelShape::empty())
    }
}
