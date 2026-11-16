//! The `visuals` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0001

pub trait Hooks {
    /// The world frame's local, world-space rim and colored-light sources.
    /// Push requests; Horizon validates and limits them before rendering.
    /// Called only while a world is visible.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn visuals();
}
