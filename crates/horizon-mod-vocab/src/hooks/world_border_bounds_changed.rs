//! The `world_border_bounds_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0273

pub trait Hooks {
    /// The server world's border bounds changed, either immediately or when
    /// its timed size change was requested.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn world_border_bounds_changed();
}
