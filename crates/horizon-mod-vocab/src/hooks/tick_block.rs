//! The `tick_block` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0015

pub trait Hooks {
    /// An authoritative server's `ServerLevel.tickBlock` is about to run a
    /// due scheduled tick of the block `state` at `pos` (`BlockState.tick`):
    /// the block is still the one the tick was scheduled for. Change the
    /// block with `native_set_block_property` and set `handled` to skip the
    /// block's own tick; a block changed without `handled` takes its own
    /// tick as it is now, if it is still the tick's block.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "handled",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_tick_block",
        neoforge = "none (vanilla BlockBehaviour.tick; NeoForge adds no event or extension for it)"
    )]
    fn tick_block();
}
