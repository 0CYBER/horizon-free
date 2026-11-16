//! The `target_hit` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0334

pub trait Hooks {
    /// A projectile hit the target block at `pos`, which now has `strength`
    /// output power. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn target_hit();
}
