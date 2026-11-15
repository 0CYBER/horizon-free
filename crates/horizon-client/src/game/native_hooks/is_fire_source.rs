use super::*;

impl GameLevel<'_> {
    /// On an authoritative server, whether the block `state` at `pos`, under a
    /// fire, keeps that fire burning (`FireBlock.tick`'s check of the
    /// dimension's `infiniburn` tag): `Some(true)` makes the fire endless,
    /// `Some(false)` refuses a block the tag lists, and left `None` the tag
    /// answers. Answer only for your own blocks. The level is read-only.
    #[inline(always)]
    pub(in crate::game) fn native_is_fire_source(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        pos: BlockPos,
        source: &mut Option<bool>,
    ) {
        self.fire_is_fire_source(dimension, state, pos, source);
    }
}
