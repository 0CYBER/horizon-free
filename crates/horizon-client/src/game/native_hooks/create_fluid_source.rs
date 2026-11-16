use super::*;

impl GameLevel<'_> {
    /// A fluid source at `pos` (`state`) is about to count towards turning
    /// the flowing fluid beside it into a source (NeoForge's
    /// `CreateFluidSourceEvent`, `FlowingFluid.getNewLiquid`): two counted
    /// sources beside a block over a solid block or a source make it one.
    /// Asked for each horizontal neighbour source, whenever the game works
    /// out a block's new fluid, spread checks included: answer fast.
    /// `can_convert` starts as the fluid's rule (`water_source_conversion`
    /// or `lava_source_conversion`). Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_create_fluid_source(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        can_convert: &mut bool,
    ) {
        self.fire_create_fluid_source(dimension, pos, state, can_convert);
    }
}
