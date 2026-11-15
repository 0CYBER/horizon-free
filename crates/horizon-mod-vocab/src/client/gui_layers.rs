//! HUD layers a native mod adds among vanilla's (NeoForge's `RegisterGuiLayersEvent`).

/// A HUD layer a native mod registers once at start: its id and where it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a mod builds it with a literal, an id and a placement
#[derive(horizon_mod_api_macros::Wire)]
pub struct GuiLayer {
    /// `<namespace>:<path>` in a namespace other than vanilla's, like `mymod:compass`.
    pub id: String,
    /// Where it goes in the layer order.
    pub placement: GuiLayerPlacement,
}

/// Where a mod's HUD layer goes, the four orderings of `RegisterGuiLayersEvent`.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: NeoForge's four orderings, matched without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum GuiLayerPlacement {
    /// Below every other layer, the crosshair included (`registerBelowAll`).
    BelowAll,
    /// Right below the layer with this id (`registerBelow`): a vanilla layer, like
    /// `minecraft:hotbar`, or one pushed before this one.
    Below(String),
    /// Right above the layer with this id (`registerAbove`), with the same rule.
    Above(String),
    /// Above every other layer, the tab list included (`registerAboveAll`).
    AboveAll,
}
