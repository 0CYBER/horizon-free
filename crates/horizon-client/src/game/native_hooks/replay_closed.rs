use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The replay of `recording` closed.
    pub(in crate::game) fn native_replay_closed(&mut self, recording: &str) {
        self.fire_replay_closed(recording);
    }
}
