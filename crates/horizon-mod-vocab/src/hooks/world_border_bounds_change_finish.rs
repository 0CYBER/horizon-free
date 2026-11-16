//! The `world_border_bounds_change_finish` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0343

pub trait Hooks {
    /// A timed world-border size change finished on the server, after its
    /// final size was applied. Server-side, including the single-player
    /// server.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's WorldBorderBoundsChangeFinish)",
        cancellable = false
    )]
    fn world_border_bounds_change_finish();
}
