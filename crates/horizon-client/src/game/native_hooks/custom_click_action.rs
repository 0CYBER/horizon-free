use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `CustomClickActionEvent`: a server player's client sent a `custom`
    /// click action (a dialog button or a text component's click event): its id and
    /// optional payload. Vanilla only logs it; set `cancel` to skip that. Only play-phase
    /// clicks are reported: one sent during configuration (no player yet) is not. The
    /// player is not swapped in. Server-side.
    pub(in crate::game) fn native_custom_click_action(
        &mut self,
        player: u128,
        player_entity_id: i32,
        name: &str,
        id: &str,
        payload: Option<&Tag>,
        cancel: &mut bool,
    ) {
        self.fire_custom_click_action(player, player_entity_id, name, id, payload, cancel);
    }
}
