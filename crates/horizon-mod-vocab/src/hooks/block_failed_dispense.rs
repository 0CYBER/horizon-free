//! The `block_failed_dispense` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0176

pub trait Hooks {
    /// A dispenser at `pos` failed to dispense `item`. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_failed_dispense();
}
