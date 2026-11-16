use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The process is about to exit after its game has stopped.
    pub(in crate::game) fn native_game_shutting_down(&mut self) {
        self.fire_game_shutting_down();
    }
}
