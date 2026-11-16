//! `player_disconnect`: a server mod sends a player away with a reason, as `/kick` does.

use super::Game;
use horizon_client::{lan::data::component_nbt, text::Text};
use mod_api::PlayerRef;

impl Game<'_> {
    /// Kicks a network or fake player with `reason` through the shared leave path.
    /// The calling hook finishes before the next poll removes the player, broadcasts
    /// their leave and fires `player_logged_out`; `player_kicked` hears the reason.
    /// False for a stale or unavailable player, the single-player owner or a player
    /// already leaving: the first reason stands, and nothing more is sent or heard.
    pub(super) fn player_disconnect(&mut self, player: PlayerRef, reason: Text) -> bool {
        let Some(conn) = self.disconnect_target(player) else {
            return false;
        };
        self.kick_conn(conn, component_nbt(&reason.component()))
    }

    /// The guest record of a network or fake player, including an acting or displaced one.
    fn disconnect_target(&self, player: PlayerRef) -> Option<u32> {
        (self.native_lifecycle.epoch()? == player.entity().epoch()).then_some(())?;
        let hub = &self.lan.as_ref()?.hub;
        let guest = hub.guests.get(
            hub.guests
                .identity_index(player.uuid(), player.entity().id())?,
        )?;
        (guest.placed
            && (guest.participates_online() || guest.is_fake())
            && hub.singleplayer_owner != Some(player.uuid()))
        .then_some(guest.conn)
    }
}
