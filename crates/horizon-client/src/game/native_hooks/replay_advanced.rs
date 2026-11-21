use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The replay's clock moved this frame; `position_ms` is where it is now.
    pub(in crate::game) fn native_replay_advanced(&mut self, position_ms: u32) {
        self.fire_replay_advanced(position_ms);
    }
}
