//! The `minimap` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0019

pub trait Hooks {
    /// The minimap a native mod shows: set `minimap` when it is none yet.
    #[hook(
        side = "client",
        contract = "replace",
        out = "minimap",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn minimap();
}
