use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `ClientPlayerChangeGameTypeEvent`: observe the local player info game type change.
    pub(in crate::game) fn native_client_player_change_game_type(
        &mut self,
        player_id: i32,
        player_uuid: u128,
        current_game_type: i32,
        new_game_type: i32,
    ) {
        self.fire_client_player_change_game_type(
            player_id,
            player_uuid,
            current_game_type,
            new_game_type,
        );
    }
}
