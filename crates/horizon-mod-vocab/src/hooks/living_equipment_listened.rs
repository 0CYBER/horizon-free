//! The `living_equipment_listened` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0158

pub trait Hooks {
    /// Whether a mod listens to the mobs' and players'
    /// `living_equipment_change`: a mod that hooks it hooks this one too
    /// and sets `listened` (HEAD, `*listened = true`). Without it the game does
    /// not read every mob's and connected player's equipment each tick, and
    /// neither is told (armor stands still are). A mob that ticks, or a player
    /// who is in the level, once it is set counts from empty. Asked once a tick
    /// for the mobs, after their tick, and once for the players. Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "listened",
        cancellable = false,
        neoforge = "none (Horizon-only; NeoForge has no counterpart); an opt-in gate for a hook"
    )]
    fn living_equipment_listened();
}
