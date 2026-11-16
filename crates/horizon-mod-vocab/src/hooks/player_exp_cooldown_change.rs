//! The `player_exp_cooldown_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0288

pub trait Hooks {
    /// A server player's XP orb pickup cooldown changed to `ticks`. It is
    /// reported after the value changes. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_exp_cooldown_change();
}
