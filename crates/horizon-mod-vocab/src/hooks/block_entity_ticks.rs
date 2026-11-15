//! The `block_entity_ticks` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0023

pub trait Hooks {
    /// Whether block `block`'s block entity ticks (`EntityBlock.getTicker`):
    /// set `ticks` when it is yours. Asked for each of the block's states
    /// when a world opens.
    /// Set `ticks` to true to make it tick, and never clear a value an earlier mod or mixin set.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block_entity",
        key = "block",
        host = "read",
        neoforge = "none (vanilla EntityBlock.getTicker; NeoForge adds no event or extension for it)"
    )]
    fn block_entity_ticks();
}
