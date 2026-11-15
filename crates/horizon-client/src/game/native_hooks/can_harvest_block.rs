use super::*;

impl Game<'_> {
    /// Whether the player's tool harvests the block `state` at `pos`
    /// (`IBlockExtension.canHarvestBlock`): `can_harvest` starts with
    /// vanilla's answer and the block's parts may change it.
    pub(in crate::game) fn native_can_harvest_block(
        &mut self,
        player: u32,
        pos: [i32; 3],
        state: BlockStateId,
        can_harvest: &mut bool,
    ) {
        self.fire_can_harvest_block(player, pos, state, can_harvest);
    }
}
