//! The `neighbor_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0013

pub trait Hooks {
    /// An authoritative server is about to deliver `neighborChanged`.
    /// State and block IDs are valid in this level's registries for this call.
    /// The host is the level with its ticks, as vanilla's `neighborChanged` gets the
    /// level: a part may read its inputs (`signal`, `has_neighbor_signal`) and
    /// schedule its tick here, as a diode does (`DiodeBlock.checkTickOnNeighbor`).
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        version = 2,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_neighbor_changed",
        neoforge = "none (vanilla Block.neighborChanged; NeoForge adds no event or extension for it); BlockEvent.NeighborNotifyEvent is the neighbour broadcast, a different point"
    )]
    fn neighbor_changed();
}
