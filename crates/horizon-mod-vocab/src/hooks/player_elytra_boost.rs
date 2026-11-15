//! The `player_elytra_boost` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0287

pub trait Hooks {
    /// A server player used a firework rocket to boost while gliding, after the rocket was launched
    /// and consumed. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_elytra_boost();
}
