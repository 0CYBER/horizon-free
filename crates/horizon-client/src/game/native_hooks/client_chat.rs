use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `ClientChatEvent`: a client is about to send `message`.
    /// `original_message` is immutable; change `message` or set `cancel` to
    /// prevent sending.
    pub(in crate::game) fn native_client_chat(
        &mut self,
        player_id: i32,
        player_uuid: u128,
        original_message: &str,
        message: &mut String,
        cancel: &mut bool,
    ) {
        self.fire_client_chat(player_id, player_uuid, original_message, message, cancel);
    }
}
