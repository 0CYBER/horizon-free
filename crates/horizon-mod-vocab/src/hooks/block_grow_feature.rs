//! The `block_grow_feature` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A sapling, azalea, mushroom or fungus at `pos` is about to grow into the
    /// configured `feature`, `None` when vanilla has none to place (a lone dark
    /// oak sapling). Set `cancel`, or `feature` to `None`, to keep the block;
    /// set `feature` to another key to grow that one. Server-side.
    /// Mods can use `level.random()` to access the growth random source.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "BlockGrowFeatureEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::block_grow_feature::GameLevel<'_>::native_block_grow_feature"
    )]
    fn block_grow_feature();
}
