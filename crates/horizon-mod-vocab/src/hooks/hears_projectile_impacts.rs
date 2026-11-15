//! The `hears_projectile_impacts` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0142

pub trait Hooks {
    /// Whether a mod hears projectiles' impacts (`native_projectile_impact`):
    /// set `hears`. Asked once a server tick before the projectiles fly; only
    /// then does the tick keep the copy of each flying projectile that a
    /// canceled impact flies on from, so a game without such a mod pays
    /// nothing. Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "hears",
        host = "read",
        neoforge = "none (Horizon-only; NeoForge has no counterpart); an opt-in gate for a hook"
    )]
    fn hears_projectile_impacts();
}
