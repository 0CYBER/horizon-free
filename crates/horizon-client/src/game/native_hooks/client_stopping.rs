use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The client is closing: once per process, at the start of the game's shutdown,
    /// after its world is left, while settings and resources are still held.
    pub(in crate::game) fn native_client_stopping(&mut self) {
        self.fire_client_stopping();
    }
}
