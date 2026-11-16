use super::*;

impl GameLevel<'_> {
    /// A sapling, azalea, mushroom or fungus at `pos` is about to grow into the
    /// configured `feature`, `None` when vanilla has none to place (a lone dark
    /// oak sapling). Set `cancel`, or `feature` to `None`, to keep the block;
    /// set `feature` to another key to grow that one. Server-side.
    /// Mods can use `level.random()` to access the growth random source.
    #[inline(always)]
    pub(in crate::game) fn native_block_grow_feature(
        &mut self,
        dimension: LevelDimension,
        pos: BlockPos,
        feature: &mut Option<String>,
        cancel: &mut bool,
    ) {
        self.fire_block_grow_feature(dimension, pos, feature, cancel);
    }
}
