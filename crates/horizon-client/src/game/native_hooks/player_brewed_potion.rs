use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server player took `stack` from a brewing stand bottle slot
    /// (`BrewingStandMenu.PotionSlot.onTake`): `player` is their UUID and
    /// `player_entity_id` their network id.
    pub(in crate::game) fn native_player_brewed_potion(
        &mut self,
        player: u128,
        player_entity_id: i32,
        stack: &ItemStack,
    ) {
        self.fire_player_brewed_potion(player, player_entity_id, stack);
    }
}
