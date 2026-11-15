//! The `world_border_center_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0274

pub trait Hooks {
    /// The server world's border center changed.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn world_border_center_changed();
}
