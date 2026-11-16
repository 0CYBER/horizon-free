use super::*;

impl Game<'_> {
    /// A player removes the block `state` at `pos` with `tool`, the held stack (`None` for a
    /// bare hand)
    /// (`IBlockExtension.onDestroyedByPlayer`): the block's parts may do the
    /// removal themselves, and say whether the block was destroyed. `destroyed`
    /// left `None` keeps vanilla's `playerWillDestroy` and removal.
    pub(in crate::game) fn native_on_destroyed_by_player(
        &mut self,
        player: u128,
        pos: [i32; 3],
        state: BlockStateId,
        tool: Option<&ItemStack>,
        will_harvest: bool,
        destroyed: &mut Option<bool>,
    ) {
        self.fire_on_destroyed_by_player(player, pos, state, tool, will_harvest, destroyed);
    }
}
