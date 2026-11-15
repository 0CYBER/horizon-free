//! The tab list's entries as the client holds them, for reads that must not change them.

use super::Remote;
use horizon_client::tab_list::TabPlayer;

impl Remote {
    /// The listed players (`ClientPacketListener.getListedOnlinePlayers`), in no order:
    /// `tab_list::sorted_players` gives the overlay's.
    pub(in crate::game) fn listed_tab_players(&self) -> Vec<TabPlayer> {
        self.listed
            .iter()
            .filter_map(|uuid| self.tab_players.get(uuid).cloned())
            .collect()
    }
}
