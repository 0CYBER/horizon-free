//! The `block_entity_client_ticks` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether block `block`'s block entity ticks on the client
    /// (`EntityBlock.getTicker` asked with the client level): set `ticks` when
    /// it is yours, and `block_entity_client_tick` runs for it once per
    /// client tick. Asked once for each mod block, for all of its states,
    /// the first time a client level ticks with a part that has a client tick.
    /// Set `ticks` to true to make it tick, and never clear a value an earlier mod or mixin set.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "block",
        neoforge = "none (vanilla EntityBlock.getTicker with the client level; NeoForge adds no event or extension for it)"
    )]
    fn block_entity_client_ticks();
}
