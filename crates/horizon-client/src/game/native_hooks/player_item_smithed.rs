use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `PlayerEvent.ItemSmithedEvent`: a server player took `result` from a
    /// smithing table (`SmithingMenu.onTake`, or `onQuickCraft` for a
    /// shift-click): `player` is their UUID and `player_entity_id` their
    /// network id, `inputs` the template, base and addition as they were
    /// before the take used them (`None` for an empty slot). It comes once the click is done, so the
    /// menu already holds the used inputs, but before the table sounds.
    /// Server-side.
    pub(in crate::game) fn native_player_item_smithed(
        &mut self,
        player: u128,
        player_entity_id: i32,
        inputs: &[Option<ItemStack>; 3],
        result: &ItemStack,
    ) {
        self.fire_player_item_smithed(player, player_entity_id, inputs, result);
    }
}
