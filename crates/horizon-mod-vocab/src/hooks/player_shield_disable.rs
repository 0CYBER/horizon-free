//! The `player_shield_disable` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0309

pub trait Hooks {
    /// A server player's shield was disabled by a blocked melee hit. The shield cooldown is set
    /// immediately after this call. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_shield_disable();
}
