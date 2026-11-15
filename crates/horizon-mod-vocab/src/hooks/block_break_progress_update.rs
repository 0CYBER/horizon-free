//! The `block_break_progress_update` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0170

pub trait Hooks {
    /// A server guest's block-breaking animation changed to `progress` at
    /// `pos` (0 through 9).
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_break_progress_update();
}
