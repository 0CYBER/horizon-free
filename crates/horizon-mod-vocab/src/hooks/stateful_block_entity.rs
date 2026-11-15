//! Actual-state block entity factory ownership.
trait Hooks {
    /// Opt a block with a registered block entity type into state-aware creation.
    /// Asked once per block when a level's immutable factory ownership table is built.
    /// Set true and never clear an earlier answer. For these blocks, `block_entity`
    /// receives the actual state and no answer creates no entity. Existing entities
    /// survive property changes within the same block.
    #[notes = r#"`stateful_block_entity` declares static factory ownership; pure `native_block_entity_from_state` receives the borrowed actual block/property map. An empty result creates no new entity and never falls through to API15. LevelChunk mutation creates inline after onPlace; existing same-block entities survive state property changes. Bundles need "hooks": @API:state_block_entity_factory@."#]
    #[hook(
        side = "both",
        contract = "listen",
        host = "read",
        scope = "block_entity",
        key = "block",
        neoforge = "none (static factory ownership for vanilla EntityBlock.newBlockEntity)"
    )]
    fn stateful_block_entity();
}
