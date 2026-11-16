//! The `zoom` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0018

pub trait Hooks {
    /// The zoom a native mod asks for, applied while playing (the spyglass's
    /// own view wins): set `zoom` when it is none yet.
    #[hook(
        side = "client",
        contract = "replace",
        out = "zoom",
        neoforge = "none (Horizon-only; ViewportEvent.ComputeFov is the nearest way to change the view)"
    )]
    fn zoom();
}
