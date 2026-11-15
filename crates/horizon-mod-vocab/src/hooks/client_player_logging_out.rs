//! The `client_player_logging_out` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before client disconnect teardown (NeoForge's
    /// `ClientPlayerNetworkEvent.LoggingOut`). The player may be absent.
    /// Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ClientPlayerNetworkEvent.LoggingOut",
        cancellable = false
    )]
    fn client_player_logging_out();
}
