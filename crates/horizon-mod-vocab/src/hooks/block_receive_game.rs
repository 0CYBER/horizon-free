//! The `block_receive_game` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0180

pub trait Hooks {
    /// A sculk sensor or shrieker received a vibration for `event` at `pos`.
    /// Set `cancel` to suppress its response. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn block_receive_game();
}
