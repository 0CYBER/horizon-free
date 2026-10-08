//! Registry-bound freezing and snow-placement eligibility using biome temperature.
use super::{
    DecodeError,
    biome_temperature::{BiomeTemperature, Precipitation},
    block_feature::FeatureBlockBindings,
    placement::{PlacementContext, PlacementQueryError},
};
use crate::{fluid::FluidKind, registry::BlockStateId};
use horizon_core::position::BlockPos;
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone, Debug)]
pub struct BiomeWeatherBindings {
    liquid_blocks: Arc<BTreeSet<BlockStateId>>,
    snow_states: Arc<BTreeSet<BlockStateId>>,
    snow: BlockStateId,
    air: [BlockStateId; 3],
}
impl BiomeWeatherBindings {
    pub fn new(bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let mut liquids = BTreeSet::new();
        // WATER and LAVA are the pinned vanilla registry's LiquidBlock instances.
        for name in ["minecraft:water", "minecraft:lava"] {
            liquids.extend(
                bindings
                    .blocks
                    .get(name)
                    .ok_or(DecodeError::Field("weather liquid blocks"))?
                    .iter()
                    .copied(),
            );
        }
        Ok(Self {
            liquid_blocks: Arc::new(liquids),
            snow_states: bindings
                .blocks
                .get("minecraft:snow")
                .cloned()
                .ok_or(DecodeError::Field("weather snow states"))?,
            snow: bindings
                .registry()
                .default_state("minecraft:snow")
                .ok_or(DecodeError::Field("weather snow state"))?,
            air: bindings.air,
        })
    }
    pub fn should_freeze(
        &self,
        world: &mut impl PlacementContext,
        temperature: &mut BiomeTemperature,
        p: [i32; 3],
        require_edge: bool,
    ) -> Result<bool, PlacementQueryError> {
        if temperature.warm_enough_to_rain(BlockPos::new(p[0], p[1], p[2]), world.sea_level()?) {
            return Ok(false);
        }
        if !world.inside_build_height(p)? || world.block_brightness(p)? >= 10 {
            return Ok(false);
        }
        let state = world.block_state(p)?;
        let fluid = world.fluid_state(p)?;
        if fluid.kind() != FluidKind::Water || !self.liquid_blocks.contains(&state) {
            return Ok(false);
        }
        if !require_edge {
            return Ok(true);
        }
        for d in [[-1, 0, 0], [1, 0, 0], [0, 0, -1], [0, 0, 1]] {
            if !world.water_at(std::array::from_fn(|i| p[i].wrapping_add(d[i])))? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn should_snow(
        &self,
        world: &mut impl PlacementContext,
        temperature: &mut BiomeTemperature,
        p: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        if temperature.precipitation_at(BlockPos::new(p[0], p[1], p[2]), world.sea_level()?)
            != Precipitation::Snow
        {
            return Ok(false);
        }
        if !world.inside_build_height(p)? || world.block_brightness(p)? >= 10 {
            return Ok(false);
        }
        let state = world.block_state(p)?;
        if !self.air.contains(&state) && !self.snow_states.contains(&state) {
            return Ok(false);
        }
        world.block_survives(self.snow, p)
    }
}
