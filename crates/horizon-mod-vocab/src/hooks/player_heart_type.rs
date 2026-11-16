//! The `player_heart_type` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Which hearts the local player's health bar draws this frame (NeoForge's
    /// `PlayerHeartTypeEvent`, from `Gui.HeartType.forPlayer`): `kind` comes in as vanilla's pick
    /// (poisoned, else withered, else frozen when fully frozen, else normal); set it to draw
    /// another kind. Asked once a frame while the HUD draws the hearts (not in creative or
    /// spectator, not under F1), before `health_hud`, whose `inputs.kind` carries the answer.
    /// Only the heart sprites change: health, regeneration and the other rows stay vanilla's.
    /// Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "PlayerHeartTypeEvent",
        cancellable = false
    )]
    fn player_heart_type();
}
