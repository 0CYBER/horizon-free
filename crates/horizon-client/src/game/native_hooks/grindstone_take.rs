use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The server player `player` (UUID) and `player_entity_id` took a
    /// grindstone's result at `pos` (`GrindstoneEvent.OnTakeItem`, in its
    /// result slot's `onTake`): `take` holds the inputs, the experience (-1
    /// for vanilla's, or what `grindstone_place` set) and what the inputs
    /// hold afterwards (empty). Set `xp` (0 or more) to give that much
    /// experience, `new_top` and `new_bottom` to leave items in the inputs.
    /// Set `cancel` to skip the rest of `onTake`: the result is still taken,
    /// but no experience, no sound, and the inputs stay. Server-side.
    pub(in crate::game) fn native_grindstone_take(
        &mut self,
        player: u128,
        player_entity_id: i32,
        pos: [i32; 3],
        take: &mut mod_api::item::menu::GrindstoneTake,
        cancel: &mut bool,
    ) {
        self.fire_grindstone_take(player, player_entity_id, pos, take, cancel);
    }
}
