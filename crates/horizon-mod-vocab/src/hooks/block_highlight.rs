//! The `block_highlight` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0097

pub trait Hooks {
    /// Before an eligible targeted block's selection outline is built.
    /// Cancel skips only the default block highlight, after normal HUD and game-mode gates.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        cancellable = true,
        neoforge = "ExtractBlockOutlineRenderStateEvent"
    )]
    fn block_highlight();
}
