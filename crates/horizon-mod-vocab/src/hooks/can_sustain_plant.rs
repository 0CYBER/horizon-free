//! The `can_sustain_plant` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
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
