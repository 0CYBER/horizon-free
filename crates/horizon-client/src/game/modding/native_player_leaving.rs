//! `player_leaving`: the mods hear a placed network player before the game removes and saves them.
use super::Game;

impl Game<'_> {
    /// Tells the mods that the network player on connection `conn` (`uuid`, `name`) is about to
    /// leave with the plain-text `reason`, with that player's state swapped in: they are still
    /// listed and placed, before their menus close and their file is saved. Server-side.
    ///
    /// Not heard for a guest whose proxy transfer is released or still unresolved: the
    /// destination may already own their state, so what a mod did to it here would be lost or,
    /// worse, duplicated.
    pub(in crate::game) fn announce_player_leaving(
        &mut self,
        conn: u32,
        uuid: u128,
        name: &str,
        reason: &str,
    ) {
        // The flag is only set for a guest the hook is heard for.
        let skip = self
            .lan
            .as_mut()
            .and_then(|host| {
                host.hub.update_guest(conn, |g| {
                    let skip = g.released || !g.transfer_unresolved.is_empty();
                    g.leaving = !skip;
                    skip
                })
            })
            .unwrap_or(true);
        if skip {
            return;
        }
        self.as_guest(conn, |game| {
            game.native_player_leaving(uuid, name, reason);
        });
        if let Some(host) = self.lan.as_mut() {
            host.hub.update_guest(conn, |g| g.leaving = false);
        }
    }
}
