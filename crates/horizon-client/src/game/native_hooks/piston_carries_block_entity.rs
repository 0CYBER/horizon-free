use super::*;

impl GameLevel<'_> {
    /// Whether a piston carries the block `state` at `pos` with its block
    /// entity (`piston_carries_block_entity`, a Horizon hook), asked at the
    /// last step of `PistonBaseBlock.isPushable` for a block that got past its
    /// push reaction: `Some(false)` keeps it in place, `Some(true)` moves it
    /// with its block entity, and left `None` vanilla's rule holds (a
    /// block with a block entity does not move). Both sides; only the block's
    /// own part is asked.
    #[inline(always)]
    pub(in crate::game) fn native_piston_carries_block_entity(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        movable: &mut Option<bool>,
    ) {
        self.fire_piston_carries_block_entity(dimension, pos, state, movable);
    }
}
