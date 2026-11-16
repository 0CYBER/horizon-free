use super::*;

impl GameLevel<'_> {
    /// `IBlockExtension.isFertile` for the soil `state` at `pos` under a crop: `fertile` starts
    /// with vanilla's answer (`moisture > 0`) and the soil's parts may change it. The level is
    /// read-only.
    #[inline(always)]
    pub(in crate::game) fn native_is_fertile(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        pos: BlockPos,
        fertile: &mut bool,
    ) {
        self.fire_is_fertile(dimension, state, pos, fertile);
    }
}
