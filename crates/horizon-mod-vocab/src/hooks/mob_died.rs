//! The `mob_died` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0355

pub trait Hooks {
    /// A mob or server player died: `living_death` let the death through, so
    /// its health has run out and the loot, experience and death message come
    /// next. `entity` and `kind` as `living_death`. `killer` is the network id
    /// of the entity that caused the killing blow (the attacking mob or
    /// player, or a projectile's owner), 0 when there is none or it is not
    /// known. `damage_type` names the blow (`minecraft:player_attack`,
    /// `minecraft:arrow`, `minecraft:fall`), and `weapon` is the killing
    /// item's id (`minecraft:diamond_sword`), "" when there is no weapon. It
    /// only hears, once per death, for any cause: a fall, fire or a mob's
    /// bite included, where `mob_killed` covers melee kills by a player only.
    /// Armor stands, boats and other non-mob entities are not told.
    /// Server-side. Paper's `EntityDeathEvent` answers `getKiller` and the
    /// damage cause for every death.
    #[hook(side = "server", contract = "listen", neoforge = "LivingDeathEvent")]
    fn mob_died(entity: u32, kind: &str, killer: u32, damage_type: &str, weapon: &str);
}
