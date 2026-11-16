//! The `emissive` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0002

pub trait Hooks {
    /// At most 32 pairs of already-loaded base and transparent overlay sprites.
    /// Per-pixel alpha in the overlay marks full-bright material, without
    /// illuminating the surrounding block or changing gameplay light.
    #[hook(
        side = "client",
        contract = "collect",
        out = "layers",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn emissive();
}
