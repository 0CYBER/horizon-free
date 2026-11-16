use super::*;

impl GameLevel<'_> {
    /// Liquid at `liquid_pos` is about to place `state` at `pos`. Set
    /// `cancel` to prevent placement or change `new_state` to replace it.
    /// Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_fluid_place_block(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        liquid_pos: BlockPos,
        state: BlockStateId,
        new_state: &mut BlockStateId,
        cancel: &mut bool,
    ) {
        self.fire_fluid_place_block(dimension, pos, liquid_pos, state, new_state, cancel);
    }
}
