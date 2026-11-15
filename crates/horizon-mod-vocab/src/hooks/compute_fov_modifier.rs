//! The `compute_fov_modifier` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Player FOV scale, once per client tick with a player, after vanilla
    /// effects and before zoom and easing.
    /// Invalid values or values outside 0.01..=10 revert to base.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ComputeFovModifierEvent",
        cancellable = false
    )]
    fn compute_fov_modifier();
}
