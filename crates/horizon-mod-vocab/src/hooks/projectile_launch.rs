//! The `projectile_launch` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0327

pub trait Hooks {
    /// A server projectile was added to the level. `player` is set only for
    /// a player launch; other sources have an empty `name`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn projectile_launch();
}
