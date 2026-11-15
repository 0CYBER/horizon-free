use super::*;

impl GameLevel<'_> {
    /// `IBlockExtension.canBeHydrated` for the farmland `state` at `pos` and the non-empty `fluid`
    /// at `fluid_pos`: `Some(true)` or `Some(false)` decides, left `None` vanilla's `#water`
    /// rule answers. The level is read-only.
    #[inline(always)]
    pub(in crate::game) fn native_can_be_hydrated(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        pos: BlockPos,
        fluid: mod_api::FluidState,
        fluid_pos: BlockPos,
        hydrates: &mut Option<bool>,
    ) {
        self.fire_can_be_hydrated(dimension, state, pos, fluid, fluid_pos, hydrates);
    }
}
