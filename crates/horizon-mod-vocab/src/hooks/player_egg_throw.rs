//! The `player_egg_throw` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0286

pub trait Hooks {
    /// A server player threw an egg, after its projectile was launched and the use was applied.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_egg_throw();
}
