//! `player_pong`: the mods hear a network player's answer to a ping (`ServerboundPongPacket`).
use super::Game;

impl Game<'_> {
    /// Tells the mods that the network player on connection `conn` answered the ping numbered
    /// `id`, with that player's state swapped in. Nothing is asked when no mod listens, or when
    /// the connection has no guest any more. Server-side.
    pub(in crate::game) fn announce_player_pong(&mut self, conn: u32, id: i32) {
        if !mod_api::__private::listened(mod_api::HookId::PlayerPong) {
            return;
        }
        let Some((uuid, name)) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        self.as_guest(conn, |game| {
            game.native_player_pong(uuid, &name, id);
        });
    }
}
