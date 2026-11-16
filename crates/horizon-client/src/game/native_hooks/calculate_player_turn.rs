use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The mouse sensitivity and cinematic camera option a grabbed player's
    /// mouse turn reads this frame (`MouseHandler.turnPlayer`). Change either
    /// for this turn only; the options are unchanged. Client-side.
    pub(in crate::game) fn native_calculate_player_turn(
        &self,
        player_entity_id: i32,
        player_uuid: u128,
        mouse_sensitivity: &mut f64,
        cinematic_camera_enabled: &mut bool,
    ) {
        self.fire_calculate_player_turn(
            player_entity_id,
            player_uuid,
            mouse_sensitivity,
            cinematic_camera_enabled,
        );
    }
}
