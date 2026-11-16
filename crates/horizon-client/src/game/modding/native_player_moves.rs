//! `player_move`: the game's half of a movement report the hub holds for the mods.
use super::Game;
use horizon_client::lan::hub::Hub;

impl Game<'_> {
    /// What the hub needs to know of the loaded mods each tick.
    pub(in crate::game) fn sync_hub_mod_flags(hub: &mut Hub) {
        hub.mods_loaded = !mod_api::__private::mods().is_empty();
        hub.moves_listened = mod_api::__private::listened(mod_api::HookId::PlayerMove);
    }

    /// The hub holds guest `conn`'s movement report: the mods hear it with the player's state
    /// swapped in and nothing of the hub borrowed, then the hub commits it or drops it and
    /// corrects the client. A report the hub no longer holds (the player left or was teleported
    /// first) is not heard.
    pub(in crate::game) fn lan_player_move(&mut self, conn: u32) {
        let Some(report) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.pending_move_report(conn))
        else {
            return;
        };
        let mut cancel = false;
        self.as_guest(conn, |game| {
            game.native_player_move(
                report.player,
                report.from,
                report.from_rotation,
                report.to,
                report.to_rotation,
                &mut cancel,
            );
        });
        if let Some(host) = self.lan.as_mut() {
            host.hub.resolve_move(conn, !cancel);
        }
    }
}
