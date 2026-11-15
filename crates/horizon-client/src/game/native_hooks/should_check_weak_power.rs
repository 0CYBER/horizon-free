use super::*;

impl GameLevel<'_> {
    /// `IBlockExtension.shouldCheckWeakPower`: set `check` to decide whether
    /// indirect power is read through this block. `check` starts with vanilla's
    /// conductor answer. `direction` is the input side of the block being powered,
    /// opposite the queried block's output side. Authoritative servers only.
    #[inline(always)]
    pub(in crate::game) fn native_should_check_weak_power(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        check: &mut bool,
    ) {
        self.fire_should_check_weak_power(dimension, pos, state, direction, check);
    }
}
