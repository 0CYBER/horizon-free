//! The `client_player_change_game_type` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ClientPlayerChangeGameTypeEvent`: observe the local player info game type change.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ClientPlayerChangeGameTypeEvent",
        cancellable = false
    )]
    fn client_player_change_game_type();
}
