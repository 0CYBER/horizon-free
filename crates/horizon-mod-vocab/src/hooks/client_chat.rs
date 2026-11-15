//! The `client_chat` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ClientChatEvent`: a client is about to send `message`.
    /// `original_message` is immutable; change `message` or set `cancel` to
    /// prevent sending.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        neoforge = "ClientChatEvent",
        cancellable = true
    )]
    fn client_chat();
}
