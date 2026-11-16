//! The `time_skip` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0336

pub trait Hooks {
    /// The overworld clock is about to jump, from sleep or `/time`. Set
    /// `cancel` to keep `from`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn time_skip();
}
