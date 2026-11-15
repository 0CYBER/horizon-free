pub trait Hooks {
    /// Whether this block receives block entity changes through an adjacent conductor. Starts false.
    #[hook(
        side = "server",
        contract = "collect",
        out = "receive",
        scope = "block",
        key = "state",
        neoforge = "IBlockExtension.getWeakChanges",
        target = "horizon_client::game::native_hooks::block_entity_neighbors::GameLevel<'_>::native_weak_changes"
    )]
    fn weak_changes();
}
