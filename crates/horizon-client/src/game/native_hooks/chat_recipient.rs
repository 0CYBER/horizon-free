use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A public chat line from `sender` (`sender_name`), shown as `message`, is about to reach the
    /// placed player `recipient`. Set `hidden` and that receiver alone is not sent it.
    /// Server-side.
    pub(in crate::game) fn native_chat_recipient(
        &mut self,
        sender: u128,
        sender_name: &str,
        message: &str,
        recipient: u128,
        hidden: &mut bool,
    ) {
        self.fire_chat_recipient(sender, sender_name, message, recipient, hidden);
    }
}
