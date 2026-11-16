//! The `compute_fog_color` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0115

pub trait Hooks {
    /// Computed world fog RGB, after ordinary effects and before upload. Title panoramas skip it.
    /// Only finite RGB components in 0..=1 are accepted; alpha and distances stay unchanged.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ViewportEvent.ComputeFogColor"
    )]
    fn compute_fog_color();
}
