//! The `block_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0022

pub trait Hooks {
    /// Initial saved data for a declared block entity (`EntityBlock.newBlockEntity`),
    /// fired inside the write that creates it (`LevelChunk.setBlockState`), on
    /// a server and on a client of one. `state` is the actual placed state for a
    /// block declared through `stateful_block_entity`; its properties are available
    /// through the Host's state registry. The declaration owns the type id. No
    /// answer creates no entity for a state-aware factory; other declared types
    /// start with an empty compound. The level is read-only, as `newBlockEntity`
    /// gets no level.
    #[hook(
        side = "both",
        contract = "replace",
        out = "entity",
        scope = "block_entity",
        key = "block",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_entity",
        neoforge = "none (vanilla EntityBlock.newBlockEntity; NeoForge adds no event or extension for it)"
    )]
    fn block_entity();
}
