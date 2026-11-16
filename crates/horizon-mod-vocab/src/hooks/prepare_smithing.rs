//! The `prepare_smithing` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0326

pub trait Hooks {
    /// A smithing result was prepared for a server player at `pos`. Change it before it is sent.
    #[hook(
        side = "server",
        contract = "replace",
        out = "result",
        neoforge = "none (NeoForge has no such event)",
        paper = "PrepareSmithingEvent"
    )]
    fn prepare_smithing();
}
