//! Spring cavity checks, fluid block placement and immediate scheduled ticks.
use super::{
    DecodeError, Object,
    block_feature::{BlockFeatureError, FeatureBlockBindings},
    block_predicate::PredicateBlockResolver,
    feature::FeatureWorld,
    integer, optional,
    placement::PlacementContext,
    required,
};
use crate::{
    fluid::{FluidBlockBindings, FluidKind, FluidState},
    registry::BlockStateId,
};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone, Debug)]
pub struct SpringFeature {
    state: BlockStateId,
    fluid: FluidKind,
    requires_below: bool,
    rocks: i32,
    holes: i32,
    valid: Arc<BTreeSet<BlockStateId>>,
    air: [BlockStateId; 3],
}
impl SpringFeature {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let fluid = FluidState::from_json(required(&object, "state")?.get())
            .map_err(|e| DecodeError::Registry(e.to_string()))?;
        let state = FluidBlockBindings::new(bindings.registry())
            .map_err(|e| DecodeError::Registry(e.to_string()))?
            .block(fluid);
        let requires_below = optional(&object, "requires_block_below")
            .map(|v| serde_json::from_str(v.get()))
            .transpose()?
            .unwrap_or(true);
        let rocks = optional(&object, "rock_count")
            .map(integer)
            .transpose()?
            .unwrap_or(4);
        let holes = optional(&object, "hole_count")
            .map(integer)
            .transpose()?
            .unwrap_or(1);
        Ok(Self {
            state,
            fluid: fluid.kind(),
            requires_below,
            rocks,
            holes,
            valid: bindings.predicate_block_set(required(&object, "valid_blocks")?.get())?,
            air: bindings.air,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        p: [i32; 3],
    ) -> Result<bool, BlockFeatureError<W::Error>> {
        let above = [p[0], p[1].wrapping_add(1), p[2]];
        let below = [p[0], p[1].wrapping_sub(1), p[2]];
        if !self
            .valid
            .contains(&world.block(above).map_err(BlockFeatureError::World)?)
        {
            return Ok(false);
        }
        if self.requires_below
            && !self
                .valid
                .contains(&world.block(below).map_err(BlockFeatureError::World)?)
        {
            return Ok(false);
        }
        let state = world.block(p).map_err(BlockFeatureError::World)?;
        if !self.air.contains(&state) && !self.valid.contains(&state) {
            return Ok(false);
        }
        let neighbors = [
            [p[0].wrapping_sub(1), p[1], p[2]],
            [p[0].wrapping_add(1), p[1], p[2]],
            [p[0], p[1], p[2].wrapping_sub(1)],
            [p[0], p[1], p[2].wrapping_add(1)],
            below,
        ];
        let mut rocks = 0;
        for position in neighbors {
            if self
                .valid
                .contains(&world.block(position).map_err(BlockFeatureError::World)?)
            {
                rocks += 1;
            }
        }
        let mut holes = 0;
        for position in neighbors {
            if self
                .air
                .contains(&world.block(position).map_err(BlockFeatureError::World)?)
            {
                holes += 1;
            }
        }
        if rocks != self.rocks || holes != self.holes {
            return Ok(false);
        }
        world
            .set_block(p, self.state, 2)
            .map_err(BlockFeatureError::World)?;
        world
            .schedule_fluid_tick(p, self.fluid, 0)
            .map_err(BlockFeatureError::Query)?;
        Ok(true)
    }
}
