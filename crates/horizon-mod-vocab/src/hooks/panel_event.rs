//! The `panel_event` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0021

pub trait Hooks {
    /// The player did `event` on mod `mod_id`'s panel (`native_panel_items`):
    /// act on it when it is yours.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn panel_event();
}
