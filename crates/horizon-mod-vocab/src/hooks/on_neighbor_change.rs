pub trait Hooks {
    /// A neighbouring block entity changed, was created or removed. Vanilla comparator processing runs first.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        neoforge = "IBlockExtension.onNeighborChange",
        target = "horizon_client::game::native_hooks::block_entity_neighbors::GameLevel<'_>::native_on_neighbor_change"
    )]
    fn on_neighbor_change();
}
