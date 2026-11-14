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
        let conn = match self.player_handle_read(player) {
            None => return Err(RespawnRefusal::Stale),
            // The world's own player has no connection: it respawns through its own screen.
            Some(PlayerRead::Acting { .. }) => return Err(RespawnRefusal::NoConnection),
            Some(PlayerRead::Guest(guest)) if guest.is_fake() => {
                return Err(RespawnRefusal::NoConnection);
            }
            Some(PlayerRead::Guest(guest)) if !guest.dead => return Err(RespawnRefusal::Alive),
            Some(PlayerRead::Guest(guest)) => guest.conn,
        };
        self.guest_respawn(conn);
        Ok(())
    }
}
