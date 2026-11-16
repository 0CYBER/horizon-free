use super::*;

impl Game<'_> {
    /// `IBlockExtension.getEnchantPowerBonus`: the power `state` at `pos` gives the enchanting
    /// table at `table_pos`. `power` is the block's own answer, which its parts may change.
    pub(in crate::game) fn native_enchant_power_bonus(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        table_pos: BlockPos,
        power: &mut f32,
    ) {
        self.fire_enchant_power_bonus(state, pos, table_pos, power);
    }
}
