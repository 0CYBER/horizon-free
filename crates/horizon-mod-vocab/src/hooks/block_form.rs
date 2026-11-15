//! The `block_form` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0177

pub trait Hooks {
    /// Precipitation is about to form ice or place or thicken snow at `pos`.
    /// `from` and `to` are block state IDs. Set `cancel` to keep the old state.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_form"
    )]
    fn block_form();
}
