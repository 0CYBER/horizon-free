use super::*;

impl GameLevel<'_> {
    /// On an authoritative server, whether lava sets fire beside the block
    /// `state` at `pos` (`LavaFluid.isFlammable`), asked from its side `face`,
    /// the one that faces the air the fire would take: `Some(true)` lets lava
    /// light it, `Some(false)` refuses it, and left `None` the block's own
    /// flag answers. Answer only for your own blocks. The level is read-only.
    #[inline(always)]
    pub(in crate::game) fn native_ignited_by_lava(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
        ignites: &mut Option<bool>,
    ) {
        self.fire_ignited_by_lava(dimension, state, pos, face, ignites);
    }
}
