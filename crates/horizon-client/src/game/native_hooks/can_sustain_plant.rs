use super::*;

impl GameLevel<'_> {
    /// On an authoritative server, `IBlockExtension.canSustainPlant` for the
    /// soil `ground` at `ground_pos` and the plant `state` above it, asked
    /// before the plant answers its own `mayPlaceOn`: `Some(true)` lets the
    /// plant stand there, `Some(false)` refuses it, and left `None` the
    /// plant's own rule answers. Answer only for your own soil. The level is
    /// read-only.
    #[inline(always)]
    pub(in crate::game) fn native_can_sustain_plant(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: BlockPos,
        sustains: &mut Option<bool>,
    ) {
        self.fire_can_sustain_plant(dimension, state, ground, ground_pos, sustains);
    }
}
