use super::*;

impl GameLevel<'_> {
    /// `IBlockExtension.isSlimeBlock` for the piston structure resolver: `slime` starts with
    /// vanilla's answer and the block's parts may change it.
    #[inline(always)]
    pub(in crate::game) fn native_is_slime_block(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        slime: &mut bool,
    ) {
        self.fire_is_slime_block(dimension, state, slime);
    }

    /// `IBlockExtension.isStickyBlock` for the piston structure resolver: `sticky` starts with
    /// vanilla's answer and the block's parts may change it.
    #[inline(always)]
    pub(in crate::game) fn native_is_sticky_block(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        sticky: &mut bool,
    ) {
        self.fire_is_sticky_block(dimension, state, sticky);
    }

    /// `IBlockExtension.canStickTo` for the piston structure resolver: `sticks` starts with
    /// vanilla's answer for `state` beside `other`, and the parts of `state` may change it.
    #[inline(always)]
    pub(in crate::game) fn native_can_stick_to(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        other: BlockStateId,
        sticks: &mut bool,
    ) {
        self.fire_can_stick_to(dimension, state, other, sticks);
    }
}

impl GameLevel<'_> {
    /// `LevelAccess::adjust_is_slime_block`: the parts of `state` answer on an authoritative
    /// server, and `vanilla` stands elsewhere.
    pub(in crate::game) fn part_is_slime_block(&self, state: BlockStateId, vanilla: bool) -> bool {
        let mut slime = vanilla;
        if self.native_authoritative {
            self.native_is_slime_block(self.dimension, state, &mut slime);
        }
        slime
    }

    /// `LevelAccess::adjust_is_sticky_block`, as `part_is_slime_block`.
    pub(in crate::game) fn part_is_sticky_block(&self, state: BlockStateId, vanilla: bool) -> bool {
        let mut sticky = vanilla;
        if self.native_authoritative {
            self.native_is_sticky_block(self.dimension, state, &mut sticky);
        }
        sticky
    }

    /// `LevelAccess::adjust_can_stick_to`, as `part_is_slime_block`.
    pub(in crate::game) fn part_can_stick_to(
        &self,
        state: BlockStateId,
        other: BlockStateId,
        vanilla: bool,
    ) -> bool {
        let mut sticks = vanilla;
        if self.native_authoritative {
            self.native_can_stick_to(self.dimension, state, other, &mut sticks);
        }
        sticks
    }
}
