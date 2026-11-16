//! The `client_pause_change` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ClientPauseChangeEvent.Pre`: cancel a proposed pause change.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        neoforge = "ClientPauseChangeEvent.Pre",
        cancellable = true
    )]
    fn client_pause_change_pre();
    /// NeoForge's `ClientPauseChangeEvent.Post`: the pause state changed.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ClientPauseChangeEvent.Post",
        cancellable = false
    )]
    fn client_pause_change_post();
}
