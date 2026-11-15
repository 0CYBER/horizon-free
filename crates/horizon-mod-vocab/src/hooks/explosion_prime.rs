//! The `explosion_prime` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0214

pub trait Hooks {
    /// A server explosion is about to begin at `center`, with `radius` (Bukkit's
    /// `ExplosionPrimeEvent`). This is listen-only: the explosion proceeds as
    /// usual. Server-side, including the single-player server.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's ExplosionPrimeEvent)"
    )]
    fn explosion_prime();
}
