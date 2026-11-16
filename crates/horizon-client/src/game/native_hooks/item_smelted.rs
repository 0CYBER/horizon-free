use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `FurnaceResultSlot.checkTakeAchievements` ran for a server player at a
    /// furnace, blast furnace or smoker's result slot: `player` is their UUID
    /// and `player_entity_id` their network id; `stack` and `amount_removed`
    /// are what vanilla passes there (`getSmelting`, `getAmountRemoved`). A
    /// click or a throw passes the taken stack and its count; a shift-click
    /// passes the slot's stack as it was with the count moved, then what
    /// stayed in the slot (`None` once it is empty) with 0; a hotbar swap
    /// passes the taken stack with 0. Fires before `furnace_extract`.
    /// Server-side.
    pub(in crate::game) fn native_item_smelted(
        &mut self,
        player: u128,
        player_entity_id: i32,
        stack: Option<&ItemStack>,
        amount_removed: u32,
    ) {
        self.fire_item_smelted(player, player_entity_id, stack, amount_removed);
    }
}
