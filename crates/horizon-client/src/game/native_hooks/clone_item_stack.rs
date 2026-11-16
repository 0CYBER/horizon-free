use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `BlockBehaviour.getCloneItemStack`, asked by
    /// `ServerGamePacketListenerImpl.handlePickItemFromBlock`. `include_data`
    /// is true for a creative player picking with Ctrl. Leave `stack` unset to
    /// keep the game's item. Creative breaking, such as a beehive's
    /// `playerWillDestroy`, does not ask this query. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_clone_item_stack(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        block: &str,
        state: BlockStateId,
        include_data: bool,
        stack: &mut Option<ItemStack>,
    ) {
        self.fire_clone_item_stack(dimension, pos, block, state, include_data, stack);
    }
}
