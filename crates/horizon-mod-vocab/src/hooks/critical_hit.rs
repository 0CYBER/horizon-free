//! The `critical_hit` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0192

pub trait Hooks {
    /// NeoForge's `CriticalHitEvent`: a player's melee hit on a living
    /// `target_kind` (`minecraft:zombie`, `minecraft:armor_stand`) that deals
    /// damage, where `Player.attack` decides whether it is critical.
    /// `vanilla_critical` is vanilla's answer; `critical` starts as it and may
    /// be forced or denied (as in vanilla a critical hit prevents the sweep, so
    /// a forced crit loses it; a denied crit loses the crit sound and
    /// particles); `damage_multiplier` starts at 1.5 for vanilla's critical and
    /// 1.0 otherwise, and scales the damage only while `critical` ends true. Not fired for a
    /// target that is no living entity (a dragon part) or when the hit does
    /// no damage. Fires on the Game that runs the attack.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "CriticalHitEvent",
        cancellable = false
    )]
    fn critical_hit();
}
