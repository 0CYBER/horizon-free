use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Before an actual connection is discarded, not on a dimension change.
    /// NeoForge's `ClientPlayerNetworkEvent.LoggingOut`, told here after
    /// `native_client_level_unload` and only when a connection ends.
    pub(in crate::game) fn native_client_disconnecting(&self) {
        self.fire_client_disconnecting();
    }
    /// The normal menu blur request, before finite/range validation. None disables it.
    pub(in crate::game) fn native_menu_background(&self, radius: &mut Option<f32>) {
        self.fire_menu_background(radius);
    }
}
