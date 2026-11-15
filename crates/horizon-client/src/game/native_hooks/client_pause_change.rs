use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `ClientPauseChangeEvent.Pre`: cancel a proposed pause change.
    pub(in crate::game) fn native_client_pause_change_pre(
        &mut self,
        paused: bool,
        cancel: &mut bool,
    ) {
        self.fire_client_pause_change_pre(paused, cancel);
    }
    /// NeoForge's `ClientPauseChangeEvent.Post`: the pause state changed.
    pub(in crate::game) fn native_client_pause_change_post(&mut self, paused: bool) {
        self.fire_client_pause_change_post(paused);
    }
}
