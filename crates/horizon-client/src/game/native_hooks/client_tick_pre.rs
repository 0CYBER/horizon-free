use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Once a client tick, before the window's tick runs (`ClientTickEvent.Pre`).
    pub(in crate::game) fn native_client_tick_pre(&mut self) {
        self.fire_client_tick_pre();
    }
}
