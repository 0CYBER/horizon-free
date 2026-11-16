//! The `client_player_clone` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// After the client replaces its player on respawn or dimension change
    /// (NeoForge's `ClientPlayerNetworkEvent.Clone`). Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ClientPlayerNetworkEvent.Clone",
        cancellable = false
    )]
    fn client_player_clone();
}
