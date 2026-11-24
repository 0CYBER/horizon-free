//! The client chat history capacity query.

pub trait Hooks {
    /// Extend the three local chat histories: retained messages, wrapped display lines, and
    /// submitted messages and commands recallable with Up and Down. Each keeps up to the same
    /// number of entries. `vanilla` is 100; answers are clamped to 100–16,384. The separate
    /// command history saved to disk is unaffected. Asked once at client startup, before joining
    /// any world or server; changing a mod's answer requires restarting the client.
    #[hook(
        side = "client",
        contract = "replace",
        out = "capacity",
        target = "horizon_client::game::native_hooks::client_services::Game<'_>::native_chat_history_capacity"
    )]
    fn chat_history_capacity();
}
