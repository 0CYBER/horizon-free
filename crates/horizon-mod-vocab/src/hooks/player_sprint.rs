//! The `player_sprint` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0261

pub trait Hooks {
    /// A server player's sprint started or stopped: their client sent
    /// `START_SPRINTING` or `STOP_SPRINTING` (the sprint key, a double tap
    /// forward, or letting go) and the server's player is now `sprinting`
    /// (vanilla's `setSprinting` in `handlePlayerCommand`). Heard only when it
    /// changes. It cannot be stopped, and it hears what the client asks, not
    /// what the server does: a sprint that ends by itself (a hit's knockback,
    /// hunger, a wall) is not reported. The player's state is swapped in, so
    /// the helpers and `PlayerRef::tell` work. Once for each change, on a server.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no sprint event)"
    )]
    fn player_sprint();
}
