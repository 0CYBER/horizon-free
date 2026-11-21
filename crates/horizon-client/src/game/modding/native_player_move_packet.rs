//! `player_move_packet`: the mods hear every validated movement packet of a network player.
use super::Game;

impl Game<'_> {
    /// Tells the mods that the network player on connection `conn` sent a movement packet that
    /// passed the server's checks, with that player's state swapped in. `flags` is the packet's
    /// `(on_ground, has_position, has_rotation)`. Nothing is asked when no mod listens, or when
    /// the connection has no guest any more. Server-side.
    pub(in crate::game) fn announce_player_move_packet(
        &mut self,
        conn: u32,
        to: [f64; 3],
        to_rotation: [f32; 2],
        flags: (bool, bool, bool),
    ) {
        if !mod_api::__private::listened(mod_api::HookId::PlayerMovePacket) {
            return;
        }
        let Some(uuid) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| g.uuid)
        else {
            return;
        };
        let (on_ground, has_position, has_rotation) = flags;
        self.as_guest(conn, |game| {
            game.native_player_move_packet(mod_api::__private::PlayerMovePacketCall {
                player: uuid,
                to,
                to_rotation,
                on_ground,
                has_position,
                has_rotation,
            });
        });
    }
}
