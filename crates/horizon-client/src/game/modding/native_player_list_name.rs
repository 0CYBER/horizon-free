//! `player_set_list_name`: a player's name in everyone's Tab list.
use super::Game;
use horizon_client::text::Text;
use mod_api::PlayerRef;

impl Game<'_> {
    /// Shows `name` for that player in everyone's Tab list, or the game's name again for
    /// None. False when the server list is unavailable, the handle is stale or the player
    /// isn't online.
    pub(super) fn player_set_list_name(&mut self, player: PlayerRef, name: Option<Text>) -> bool {
        // A fake player acting on itself is no one's Tab entry.
        let online = (self.native_acting_player() == Some(player) && !self.lan_acting_fake)
            || self.online_player_record(player).is_some();
        if !online {
            return false;
        }
        let Some(host) = &mut self.lan else {
            return false;
        };
        host.hub.set_list_name(player.uuid(), name);
        true
    }
}
