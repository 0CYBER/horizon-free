//! The `block_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0012

pub trait Hooks {
    /// An authoritative server committed a changed state through `Level.setBlock`.
    /// The world contains `new`; block side effects and neighbour updates follow.
    #[hook(
        side = "server",
        contract = "listen",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_changed",
        neoforge = "none (NeoForge has no set-block event; IBlockExtension.onBlockStateChange is the block part hook on_block_state_change, at the end of the write)"
    )]
    fn block_changed();
}
