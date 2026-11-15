//! A server mod's respawn of a dead player (`PlayerRef::respawn`): the Respawn button, pressed
//! for them.
use super::{Game, native_server::PlayerRead};
use mod_api::{PlayerRef, RespawnRefusal};

impl Game<'_> {
    /// The Respawn button for a dead player (`handleClientCommand`'s `PERFORM_RESPAWN`), outside
    /// every player's hook. Each refusal is a variant of [`RespawnRefusal`], checked in its order.
    pub(super) fn player_respawn(&mut self, player: PlayerRef) -> Result<(), RespawnRefusal> {
        if self.remote.is_some() {
            return Err(RespawnRefusal::Client);
        }
        if self.lan_acting.is_some() {
            return Err(RespawnRefusal::InPlayerHook);
        }
        let (conn, has_health) = match self.player_handle_read(player) {
            None => return Err(RespawnRefusal::Stale),
            // The world's own player has no connection: it respawns through its own screen.
            Some(PlayerRead::Acting { .. }) => return Err(RespawnRefusal::NoConnection),
            Some(PlayerRead::Guest(guest)) if guest.is_fake() => {
                return Err(RespawnRefusal::NoConnection);
            }
            Some(PlayerRead::Guest(guest)) => (guest.conn, guest.health > 0.0),
        };
        // `handleClientCommand`: a player who has won the game respawns on the button, alive or
        // not (the End poem's exit, taken on the next server tick, as the client's own button
        // is); otherwise a player with health above 0 is ignored.
        let won_game = self
            .lan
            .as_ref()
            .and_then(|host| host.play.get(&conn))
            .is_some_and(|play| play.won_game);
        if won_game {
            if let Some(host) = self.lan.as_mut() {
                host.play.entry(conn).or_default().respawn_requested = true;
            }
            return Ok(());
        }
        if has_health {
            return Err(RespawnRefusal::Alive);
        }
        self.guest_respawn(conn);
        Ok(())
    }
}
