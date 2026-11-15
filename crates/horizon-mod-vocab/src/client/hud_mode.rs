//! `hud_mode()` reads the effective `HudMode`; `set_hud_mode(mode)` selects `AllVisible`, `NoHud` (HUD hidden, hands allowed) or `AllHidden`. Existing `input_key` runs after vanilla F1 handling and may set the mod's own three-state mode there. F1 still toggles all visible/all hidden without a mod. Other hand gates (spectator, camera, sleeping, death) and `render_hand` cancellation remain in effect. `gui_render_post` and `client_state.gui_hidden` report the effective HUD visibility. Bundles need "hooks": @API:hud_mode@.

/// HUD and first-person hand visibility. Other hand-render restrictions still apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum HudMode {
    /// Show both HUD and hands.
    #[default]
    AllVisible,
    /// Hide the HUD while allowing the first-person hands and held items.
    NoHud,
    /// Hide both HUD and hands.
    AllHidden,
}
