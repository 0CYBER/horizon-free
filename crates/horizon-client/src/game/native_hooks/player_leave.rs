//! The leave notification shared by closed connections and kicked fake players.

use super::Game;
use horizon_client::{lan::guest::Guest, text::Text};

impl Game<'_> {
    /// Report a normal leave after the hub removed the guest and broadcast its leave line.
    pub(in crate::game) fn native_guest_left(&mut self, guest: &Guest) {
        // A fake silently displaced by a real login was never announced as online.
        if guest.is_fake() && !guest.disconnect_pending {
            return;
        }
        // A held player was recorded as joined at its login: it leaves too.
        if self.is_dedicated_server() && (guest.placed || guest.held) {
            crate::game::dedicated::events::record(
                "leave",
                &guest.name,
                guest.uuid,
                serde_json::json!({ "reason": guest.left_reason }),
            );
            if guest.held && !guest.placed {
                // Never announced, so no "left the game" either.
                crate::game::dedicated::log(
                    "INFO",
                    &horizon_client::lan::arrivals::dropped_line(&guest.name, &guest.left_reason),
                );
            } else {
                crate::game::dedicated::log(
                    "INFO",
                    &format!("{} lost connection: {}", guest.name, guest.left_reason),
                );
            }
            // A player moved to another server without `announce` was not
            // announced as leaving, so the line is not logged either; a held
            // player was never announced at all.
            if guest.placed && !guest.quiet_leave {
                crate::game::dedicated::log(
                    "INFO",
                    &format!("System chat: {} left the game", guest.name),
                );
            }
        } else if self.player.is_some() && guest.placed && !guest.quiet_leave {
            // ServerGamePacketListenerImpl.removePlayerFromWorld.
            let who = self.player_text(&guest.name, guest.uuid);
            self.system_message(
                Text::translate("multiplayer.player.left", &[who])
                    .with_color(horizon_client::text::YELLOW),
            );
        }
        if guest.placed
            && self
                .lan
                .as_ref()
                .is_some_and(|host| !host.players_told_left)
        {
            self.native_player_logged_out(guest.uuid, &guest.name);
        }
    }
}
