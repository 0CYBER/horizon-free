use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A grindstone at `pos` made its result after its inputs changed, for
    /// the server player `player` (UUID) and `player_entity_id`
    /// (`GrindstoneEvent.OnPlaceItem`, in `GrindstoneMenu.createResult`):
    /// `place` holds the inputs, the vanilla result and an initially empty
    /// override. Set `output` to replace the result and `xp` (0 or more)
    /// for what taking it gives; set `cancel` for no result.
    /// Server-side.
    pub(in crate::game) fn native_grindstone_place(
        &mut self,
        player: u128,
        player_entity_id: i32,
        pos: [i32; 3],
        place: &mut mod_api::item::menu::GrindstonePlace,
        cancel: &mut bool,
    ) {
        self.fire_grindstone_place(player, player_entity_id, pos, place, cancel);
    }
}
