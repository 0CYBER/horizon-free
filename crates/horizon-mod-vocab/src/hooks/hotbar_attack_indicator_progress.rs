//! Client hook for the hotbar's attack indicator.

pub trait Hooks {
    /// Change the hotbar attack indicator's progress for this frame. `progress`
    /// starts at the local player's attack strength at partial tick 0. The HUD
    /// uses values from 0 through 1, with 1 hiding the fully charged indicator.
    /// Other values keep the original progress.
    /// Runs only when the normal hotbar is drawn in HOTBAR indicator mode.
    /// Crosshair rendering and the player's attack strength are independent.
    #[hook(
        side = "client",
        contract = "listen",
        cancellable = false,
        neoforge = "none (vanilla Hud.extractItemHotbar reads LocalPlayer.getAttackStrengthScale)"
    )]
    fn hotbar_attack_indicator_progress();
}
