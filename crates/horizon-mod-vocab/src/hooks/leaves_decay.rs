//! The `leaves_decay` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0233

pub trait Hooks {
    /// A non-persistent leaf block at distance 7 is about to decay at `pos`.
    /// Set `cancel` to keep it. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        paper = "LeavesDecayEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_leaves_decay"
    )]
    fn leaves_decay();
}
