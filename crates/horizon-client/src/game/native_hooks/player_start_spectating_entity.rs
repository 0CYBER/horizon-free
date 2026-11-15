use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server player started spectating `entity_id`, after their camera moved to it
    /// (`ServerPlayer.setCamera`) and their client was told. Server-side.
    pub(in crate::game) fn native_player_start_spectating_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: i32,
    ) {
        self.fire_player_start_spectating_entity(player, name, entity_id);
    }
}
