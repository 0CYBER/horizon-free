//! The `can_sustain_plant` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `can_sustain_plant` again, for the world generation of the server: while a feature places
    /// the plant `state` on the soil `ground` at `ground_pos`, the soil's parts answer `sustains`
    /// here. Asked before the plant's `may_place_on_v2`, as the live game asks it before the
    /// plant's `may_place_on`: the soil's answer is the plant's, and the plant is not asked.
    /// `Some(true)` lets the plant stand there, `Some(false)` refuses it, and left `None` the
    /// plant answers. Answer only for your own soil. Server-side, on the worldgen worker threads
    /// and on the server thread, with the server's worldgen `context`: no `Host`, no writes. A
    /// panicking part is contained and the plant's own rule kept. Diverges from upstream:
    /// `can_sustain_plant` in world generation.
    #[hook(
        side = "server",
        contract = "replace",
        out = "sustains",
        context = "worldgen",
        scope = "block",
        key = "ground",
        target = "mod_api::world::survival::native_can_sustain_plant_v2",
        neoforge = "IBlockExtension.canSustainPlant"
    )]
    fn can_sustain_plant_v2();
    /// On an authoritative server, `IBlockExtension.canSustainPlant` for the soil `ground` at
    /// `ground_pos` and the plant `state` above it, asked before the plant answers its own
    /// `mayPlaceOn` (`may_place_on`) wherever vanilla's plant rule reads its ground: when the plant is
    /// placed, from its shape update and wherever else vanilla asks. `Some(true)` lets the plant
    /// stand there, `Some(false)` refuses it (a planted one pops off when its ground changes to this block),
    /// and left `None` the plant's own `may_place_on` answers, as vanilla's does. Answer only for
    /// your own soil. The level is read-only. Runs on every shape update of a plant over a part's
    /// block: keep it short.
    #[hook(
        side = "server",
        contract = "replace",
        out = "sustains",
        scope = "block",
        key = "ground",
        target = "horizon_client::game::native_hooks::can_sustain_plant::GameLevel<'_>::native_can_sustain_plant",
        neoforge = "IBlockExtension.canSustainPlant"
    )]
    fn can_sustain_plant();
}
