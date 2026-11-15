//! The `thunder_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0335

pub trait Hooks {
    /// The server changed its thunder state through `/weather`. Set `cancel`
    /// to keep the previous thunder state and timer. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn thunder_change();
}
