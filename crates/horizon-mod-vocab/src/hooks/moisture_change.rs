//! The `moisture_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0236

pub trait Hooks {
    /// Farmland at `pos` is about to change moisture from `from` to `to`.
    /// Set `cancel` to keep its old moisture. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        paper = "MoistureChangeEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_moisture_change"
    )]
    fn moisture_change();
}
