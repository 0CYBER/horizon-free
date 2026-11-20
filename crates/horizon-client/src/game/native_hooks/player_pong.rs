use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A guest's client answered a ping whose number was `id`. The guest's state is
    /// swapped in. Server-side.
    pub(in crate::game) fn native_player_pong(&mut self, player: u128, name: &str, id: i32) {
        self.fire_player_pong(player, name, id);
    }
}
