use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Before client disconnect teardown (NeoForge's
    /// `ClientPlayerNetworkEvent.LoggingOut`). The player may be absent.
    /// Client-side.
    pub(in crate::game) fn native_client_player_logging_out(
        &mut self,
        player_id: Option<i32>,
        uuid: Option<u128>,
    ) {
        self.fire_client_player_logging_out(player_id, uuid);
    }
}
