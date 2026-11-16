/// Which of `ScreenEffectRenderer`'s overlays a block puts over the first-person view
/// (NeoForge's `RenderBlockScreenEffectEvent.OverlayType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum BlockScreenEffect {
    /// The player is on fire: the fire sprite over the lower view.
    Fire,
    /// The camera is inside a view-blocking block: its particle texture.
    Block,
    /// The eye is under water: the underwater texture.
    Water,
}
