use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `player` (`name`) is about to leave with the plain-text `reason`: still
    /// listed, placed and with their state swapped in, before their menus
    /// close and their file is saved. Once per placed network player, before
    /// `native_player_logged_out`. Server-side.
    pub(in crate::game) fn native_player_leaving(
        &mut self,
        player: u128,
        name: &str,
        reason: &str,
    ) {
        self.fire_player_leaving(player, name, reason);
    }
}
