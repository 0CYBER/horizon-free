//! The `may_place_on` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0079

pub trait Hooks {
    /// `may_place_on` again, for the world generation of the server: while a feature places the
    /// plant `state` on `ground` at `ground_pos`, its parts answer `may` here, and the soil's
    /// parts answer `can_sustain_plant_v2` before this. `ground` is the block the plant stands
    /// on, so a rule that only picks its soil belongs in `can_sustain_plant_v2`. Answer only
    /// for your own blocks. Server-side, on the worldgen worker threads and on the server
    /// thread, with the server's worldgen `context`: no `Host`, no writes. A panicking part is
    /// contained and the plant's own rule kept. Diverges from upstream: `may_place_on` in world
    /// generation.
    #[hook(
        side = "server",
        contract = "replace",
        out = "may",
        context = "worldgen",
        scope = "block",
        key = "state",
        target = "mod_api::world::survival::native_may_place_on_v2",
        neoforge = "IBlockExtension.canSustainPlant"
    )]
    fn may_place_on_v2();
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
