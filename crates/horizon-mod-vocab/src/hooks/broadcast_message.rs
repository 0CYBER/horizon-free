//! The `broadcast_message` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0185

pub trait Hooks {
    /// A server broadcast line is about to be logged and sent. Change `line`
    /// or set `cancel` to drop it.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        neoforge = "none (NeoForge has no such event; Bukkit's BroadcastMessageEvent)",
        cancellable = true
    )]
    fn broadcast_message();
}
