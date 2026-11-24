//! The tab list's entries as the client holds them, for reads that must not change them.

use super::{OwnProfile, Remote};
use horizon_client::tab_list::TabPlayer;
use horizon_protocol::packets::play;

impl Remote {
    /// The listed players (`ClientPacketListener.getListedOnlinePlayers`), in no order:
    /// `tab_list::sorted_players` gives the overlay's.
    pub(in crate::game) fn listed_tab_players(&self) -> Vec<TabPlayer> {
        self.listed
            .iter()
            .filter_map(|uuid| self.tab_players.get(uuid).cloned())
            .collect()
    }

    /// The local player's tab list entry as a recording keeps it (their name, game mode and
    /// skin under the recorder id), for the snapshot a recording starts with. `None` while the
    /// session has no name for them.
    pub(in crate::game) fn recorder_tab_entry(&self) -> Option<play::Clientbound> {
        let name = self
            .tab_players
            .get(&self.profile)
            .map(|t| t.name.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.profile_name);
        if name.is_empty() {
            return None;
        }
        // The textures live rendering draws on them (`update_remote_skins`): the server's, with
        // the account's own filling the skin or cape the server names none of. The launcher's
        // offline skin and cape are images with no URL to keep.
        let account = match &self.own_profile {
            OwnProfile::Done(Some(t)) => Some(t),
            _ => None,
        };
        let textures = match (self.skin_textures.get(&self.profile), account) {
            (Some(server), Some(account)) => {
                let mut textures = server.clone();
                super::skins::fill_from_account(&mut textures, account);
                Some(textures)
            }
            (Some(textures), None) | (None, Some(textures)) => Some(textures.clone()),
            (None, None) => None,
        };
        Some(horizon_client::studio::recorder::recorder_entry(
            name,
            self.game_mode,
            textures.as_ref(),
        ))
    }
}
