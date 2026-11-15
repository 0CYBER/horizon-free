use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `ServerChatEvent`: a player's chat message, through their
    /// chain and the chat filter, is about to be broadcast.
    /// `player_id` is the server's entity id for the player, `player` their
    /// UUID, `username` their name and `raw_text` what they typed. `message`
    /// starts as that text; replace it and players see the new message
    /// (`withUnsignedContent`: a signed message keeps its signature), and
    /// the log shows it. Set `cancel` and nobody sees it, and it doesn't
    /// count toward the chat spam limit. Lines pushed to `replies` go to the
    /// sender alone as system chat, whether or not the message is canceled
    /// (no other player sees them). Commands (`/say`, `/msg`...) are not
    /// chat here. Server-side, with no player's state swapped in.
    pub(in crate::game) fn native_server_chat(
        &mut self,
        call: mod_api::__private::ServerChatCall<'_>,
    ) {
        self.fire_server_chat(call);
    }
}
