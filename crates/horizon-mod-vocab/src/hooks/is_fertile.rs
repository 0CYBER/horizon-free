//! The `is_fertile` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether the soil this part is attached to, the block `state` at `pos` under a growing crop,
    /// counts as moist (`IBlockExtension.isFertile`): set `fertile` to decide. `fertile` starts
    /// with vanilla's answer, `moisture > 0`. `CropBlock.getGrowthSpeed` asks it for each of the
    /// 3×3 blocks under a crop, stem or pitcher crop on its random tick, but only for a block in
    /// `#minecraft:grows_crops`: a fertile one adds 3 to the speed, any other tagged block 1
    /// (a quarter of that off-centre), so a block outside the tag is never asked. Asked on an
    /// authoritative server; the level is read-only. With no part attached to the state, the
    /// cost is one table lookup. Diverges from upstream: new hook.
    #[hook(
        side = "server",
        contract = "collect",
        out = "fertile",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::is_fertile::GameLevel<'_>::native_is_fertile",
        neoforge = "IBlockExtension.isFertile"
    )]
    fn is_fertile();
}
