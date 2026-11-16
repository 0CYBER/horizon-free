use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The client shutdown has completed.
    pub(in crate::game) fn native_client_stopped(&mut self) {
        self.fire_client_stopped();
    }
}
