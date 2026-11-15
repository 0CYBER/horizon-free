//! The `weather_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0341

pub trait Hooks {
    /// The server changed its weather parameters. `raining` and `thundering`
    /// are the requested final states. Set `cancel` to keep the previous
    /// weather parameters. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn weather_change();
}
