//! The `compute_fov` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0112

pub trait Hooks {
    /// World field of view, after normal calculation and zoom and before projection/Nausea.
    /// Called for world rendering and near-plane overlays; hand/HUD FOV uses its own path.
    /// Only finite values in 1..=179 degrees replace the computed FOV.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ViewportEvent.ComputeFov"
    )]
    fn compute_fov();
}
