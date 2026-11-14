//! The receivers a mod keeps a public chat line from (`chat_recipient`), asked before the hub
//! broadcasts it.
use super::Game;

impl Game<'_> {
    /// The placed players a mod keeps `sender`'s (`name`) public line `message` from: each is
    /// asked once (`native_chat_recipient`), in the hub's order. Empty, asking nobody, when no
    /// mod listens.
    pub(in crate::game) fn chat_hidden_from(
        &mut self,
        sender: u128,
        name: &str,
        message: &str,
    ) -> Vec<u128> {
        if !mod_api::__private::listened(mod_api::HookId::ChatRecipient) {
            return Vec::new();
        }
        let Some(host) = self.lan.as_ref() else {
            return Vec::new();
        };
        let receivers: Vec<u128> = host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed)
            .map(|g| g.uuid)
            .collect();
        receivers
            .into_iter()
            .filter(|&recipient| {
                let mut hidden = false;
                self.native_chat_recipient(sender, name, message, recipient, &mut hidden);
                hidden
            })
            .collect()
    }
}
