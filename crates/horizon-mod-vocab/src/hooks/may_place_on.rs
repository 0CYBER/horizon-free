//! The `may_place_on` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0079

pub trait Hooks {
    /// On an authoritative server, `VegetationBlock.mayPlaceOn` for the
    /// plant `state` on `ground` at `ground_pos`, before the plant answers
    /// it: the blocks a mod plant (one whose template is a vanilla plant,
    /// such as a crop) can stand on. Its template's `canSurvive` asks it, so
    /// the plant is refused there and pops off when its ground changes to
    /// one it can't stand on. Answer only for your own blocks. The level is
    /// read-only.
    #[hook(
        side = "server",
        contract = "replace",
        out = "may",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_may_place_on",
        neoforge = "IBlockExtension.canSustainPlant"
    )]
    fn may_place_on();
}
