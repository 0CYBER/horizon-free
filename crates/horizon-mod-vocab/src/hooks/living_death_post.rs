//! The `living_death_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mob or server player's death has finished (Fabric's
    /// `ServerLivingEntityEvents.AFTER_DEATH`, vanilla's `LivingEntity.die`
    /// at its `broadcastEntityEvent`): the loot, equipment and experience
    /// dropped and the death message went out. `entity` is its id;
    /// `damage_type` names the killing blow (`minecraft:player_attack`) and
    /// `attacker` is the mob that dealt it, 0 for none or a player. It only
    /// hears. The tail of `living_death`, a separate moment, and not fired
    /// when that hook cancelled the death. Server-side, once per death, for
    /// every ordinary living entity. The ender dragon uses a separate death
    /// sequence and fires neither living death hook. Armor stands, boats and other non-mob entities
    /// are not told.
    #[hook(
        side = "server",
        contract = "listen",
        fabric = "ServerLivingEntityEvents.AFTER_DEATH",
        neoforge = "none (LivingDeathEvent fires before the death)"
    )]
    fn living_death_post();
}
