//! Private custom-book connection state and client network services.
use super::Game;
use mod_api::{SendPacketError, ServerboundPlayPacket};
use std::collections::HashMap;
#[derive(Default)]
pub(super) struct NativeNet {
    pub(super) placement: super::native_book_placement::State,
    pub(super) book_connections:
        HashMap<u32, horizon_client::recipe_book::settings_connection::Connection>,
    pub(super) book_view: horizon_client::recipe_book::settings_sync::ClientView,
    pub(super) book_request: u64,
    pub(super) book_ready: bool,
    pub(super) book_retry_at: Option<std::time::Instant>,
    /// A mod's `disconnect` (its reason, if any), carried out at the end of the frame.
    pub(super) leave: Option<super::native_disconnect::LeaveRequest>,
}

impl Game<'_> {
    /// A mod's chat line or command (`/` first), sent as `send_chat` sends
    /// it (ClientPacketListener.sendChat / sendCommand), to any server: no
    /// chat history, and no client chat event (vanilla's `ClientChatEvent`
    /// comes only from ChatScreen). An error when the player isn't in a
    /// server's play state or the Chat option blocks the message. API
    /// 404.
    pub(super) fn send_chat(&mut self, message: &str) -> Result<(), String> {
        let Some(remote) = self.remote.as_ref() else {
            return Err("not connected to a server".into());
        };
        // A server that reconfigures the player has them out of the play state.
        if !remote.joined || remote.reconfiguring.is_some() {
            return Err("the player has not joined yet".into());
        }
        let message = message.trim();
        if message.is_empty() {
            return Err("the message is empty".into());
        }
        if !self.chat_abilities().allows(message) {
            return Err("the Chat option blocks this message".into());
        }
        self.send_signed_chat_input(message);
        Ok(())
    }

    /// Opens the chat screen with `text` typed in and the cursor at its end,
    /// as T and / open it (`ChatScreen` with its initial text): a command
    /// macro can fill in `/tp ` for the player to finish. Nothing is sent; the
    /// player edits and sends it, or presses Escape. The box keeps its limit
    /// (256 characters) and its character filter. An error with no player in a
    /// world, or while another screen is open. API @API:open_chat@.
    pub(super) fn open_chat(&mut self, text: &str) -> Result<(), String> {
        if self.player.is_none() {
            return Err("no player in a world".into());
        }
        if self.screen != super::Screen::Playing {
            return Err("another screen is open".into());
        }
        self.begin_chat_screen(text, false);
        Ok(())
    }

    /// Validate and reserve a vanilla packet for root-commit publication on this
    /// exact physical protocol incarnation. Registered and engine-reserved
    /// channels require their payload/service API; unrelated plugin payloads
    /// retain vanilla's discard-codec limit. No local effects are applied. API 409.
    pub(super) fn send_packet(
        &mut self,
        packet: ServerboundPlayPacket,
    ) -> Result<(), SendPacketError> {
        let remote = self.remote.as_ref().ok_or(SendPacketError::NotConnected)?;
        if !remote.joined || remote.reconfiguring.is_some() {
            return Err(SendPacketError::NotInPlay);
        }
        let prepared = remote.session.prepare_mod_packet(packet)?;
        self.after_commit(super::transactions::After::Packet(prepared));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
    use super::{SendPacketError, ServerboundPlayPacket};
    use horizon_protocol::{buf::Writer, connection::Pipe};
    use mod_api::Identifier;

    /// A joined flag cannot manufacture a physical transport. Packet codec,
    /// reservation, and framing cases live in net::mod_packet_tests.
    #[test]
    #[ignore = "needs private prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_packets_require_joined_player_and_physical_transport() {
        if run_fresh_cases(
            "game::native_net::tests::mod_packets_require_joined_player_and_physical_transport",
            "HORIZON_PACKET_GUARD_CHILD",
            &["guards"],
        ) {
            return;
        }
        with_client_game(|game| {
            let packet = || {
                let mut body = Writer::new();
                body.string("voicechat:request_secret");
                body.bytes(&[1, 2, 3, 4]);
                ServerboundPlayPacket::new(
                    Identifier::parse("minecraft:custom_payload").unwrap(),
                    body.data,
                )
            };
            assert_eq!(
                game.send_packet(packet()),
                Err(SendPacketError::NotConnected)
            );
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "PacketTest").unwrap();
            assert_eq!(game.send_packet(packet()), Err(SendPacketError::NotInPlay));
            // Deliberately inconsistent actor state: no handshake has installed
            // a writer. Acceptance must still require the physical provider.
            game.remote.as_mut().unwrap().joined = true;
            assert_eq!(
                game.send_packet(packet()),
                Err(SendPacketError::NotConnected)
            );
        });
    }
}
