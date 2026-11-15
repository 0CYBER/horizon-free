//! The `sweep_attack` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `SweepAttackEvent`: a player's damaging melee hit on a
    /// `target` (`minecraft:zombie`, `minecraft:oak_boat`) is about to sweep or
    /// not, after `critical_hit`. `vanilla_sweep` is `Player.isSweepAttack`
    /// (a charged, grounded, not critical sword hit below the speed limit);
    /// `sweeping` starts as it and may force or stop the sweep; `canceled` skips
    /// the later mods and keeps `sweeping` as it is (a mod doing its own sweep
    /// cancels with `sweeping` false). A critical hit that sweeps has both
    /// sounds. Fires on the Game that runs the attack, and on the attacking
    /// client, which only asks.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "SweepAttackEvent",
        cancellable = true
    )]
    fn sweep_attack();
}
