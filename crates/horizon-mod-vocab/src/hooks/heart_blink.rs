//! The `heart_blink` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0357

pub trait Hooks {
    /// The local player's vanilla heart blink for this HUD frame. Set `blink` to change only
    /// whether the blinking heart containers and remembered-health sprites are drawn.
    /// The health history, damage cooldown, and the other HUD rows are unchanged. Not called
    /// in creative or spectator mode, under F1, or when `health_hud` replaces the layout.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)",
        cancellable = false
    )]
    fn heart_blink();
}
