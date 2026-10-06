//! Query interface for placement against a generated world.
use crate::worldgen::feature_sort::FeatureIdentity;
use crate::{heightmap::HeightmapType, worldgen::anchor::WorldGenerationContext};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlacementQueryError {
    Unavailable,
    WorkLimit,
    Failed(String),
}
impl std::fmt::Display for PlacementQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkLimit => f.write_str("predicate work limit exceeded"),
            Self::Unavailable => f.write_str("placement world query is unavailable"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for PlacementQueryError {}
/// Classification for layered placement: air states and exact WATER/LAVA blocks
/// are empty; exact BEDROCK is excluded from ground; every other block is ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerBlock {
    Empty,
    Bedrock,
    Ground,
}
pub trait PlacementContext {
    /// Look up the block entity at this position and set its exit only if it is
    /// an End gateway. Missing or other entities are successful no-ops. The
    /// owning world applies the gateway's dirty notification after the update.
    fn configure_end_gateway(
        &mut self,
        _position: [i32; 3],
        _exit: [i32; 3],
        _exact: bool,
    ) -> Result<(), PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }

    /// Use the previously resolved biome, retaining its temperature cache.
    fn biome_should_freeze(
        &mut self,
        _biome: crate::worldgen::biome::BiomeId,
        _position: [i32; 3],
        _require_edge: bool,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn biome_should_snow(
        &mut self,
        _biome: crate::worldgen::biome::BiomeId,
        _position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }

    /// Block light only, independently of sky light and sky darkening.
    fn block_brightness(&mut self, _position: [i32; 3]) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn fluid_state(
        &mut self,
        _position: [i32; 3],
    ) -> Result<crate::fluid::FluidState, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }

    /// Request destruction with no responsible entity. The world adapter owns
    /// replacement, drops, block entities and updates; false is a rejected request.
    fn destroy_block_without_entity(
        &mut self,
        _position: [i32; 3],
        _drop_resources: bool,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }

    /// World sea level, independently of the generator's placement context.
    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn schedule_fluid_tick(
        &mut self,
        _position: [i32; 3],
        _fluid: crate::fluid::FluidKind,
        _delay: i32,
    ) -> Result<(), PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Resolve the containing chunk and mark this position for postprocessing.
    fn mark_postprocessing(&mut self, _position: [i32; 3]) -> Result<(), PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Test the current fluid at this position against the world's water tag.
    fn water_at(&mut self, _position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Multiface attachment: neighbor support, then empty-context collision.
    fn attachment_at(
        &mut self,
        _position: [i32; 3],
        _direction: horizon_core::geometry::Direction,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Draw from the world's random stream, independently of the feature RNG.
    fn next_world_boolean(&mut self) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Schedule the block type identified by its registry default state.
    fn schedule_block_tick(
        &mut self,
        _position: [i32; 3],
        _block: crate::registry::BlockStateId,
        _delay: i32,
    ) -> Result<(), PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }

    /// Evaluate this candidate state's survival without placing it.
    fn block_survives(
        &mut self,
        _state: crate::registry::BlockStateId,
        _position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Evaluate FULL support using an already-read state, without reading it again.
    fn state_face_sturdy(
        &mut self,
        _state: crate::registry::BlockStateId,
        _position: [i32; 3],
        _direction: horizon_core::geometry::Direction,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Read the block state and evaluate FULL support on the requested face.
    fn face_sturdy(
        &mut self,
        _position: [i32; 3],
        _direction: horizon_core::geometry::Direction,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Test entity obstruction of a unit-block shape with no excluded entity.
    fn unobstructed_block(&mut self, _position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn biome_identity(
        &mut self,
        _position: [i32; 3],
    ) -> Result<crate::worldgen::biome::BiomeId, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn top_feature(&self) -> Option<FeatureIdentity> {
        None
    }
    /// Resolve the biome at this position and test its generation settings for
    /// the supplied placed feature. Registry equality/ownership belongs here.
    fn biome_allows_feature(
        &mut self,
        _position: [i32; 3],
        _feature: FeatureIdentity,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn block_state(
        &mut self,
        _position: [i32; 3],
    ) -> Result<crate::registry::BlockStateId, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Read the state at the requested position and test its block-tag identity.
    fn block_matches_tag(
        &mut self,
        _position: [i32; 3],
        _tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn inside_build_height(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Ok(position[1] >= self.min_y()? && position[1] <= self.max_y()?)
    }
    fn outside_build_height(&mut self, y: i32) -> Result<bool, PlacementQueryError> {
        Ok(y < self.min_y()? || y > self.max_y()?)
    }
    fn layer_block(&mut self, _position: [i32; 3]) -> Result<LayerBlock, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn generation(&self) -> WorldGenerationContext;
    /// Recreate the generation bounds for a block-predicate invocation. World
    /// adapters may perform fallible level/generator queries here; ordinary
    /// placement retains its already-created immutable generation context.
    fn predicate_generation(&mut self) -> Result<WorldGenerationContext, PlacementQueryError> {
        Ok(self.generation())
    }
    /// World minimum, which can differ from the generator's minimum.
    fn min_y(&mut self) -> Result<i32, PlacementQueryError>;
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError>;
}
impl PlacementContext for WorldGenerationContext {
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.min_y.wrapping_add(self.height).wrapping_sub(1))
    }
    fn generation(&self) -> WorldGenerationContext {
        *self
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.min_y)
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
}
