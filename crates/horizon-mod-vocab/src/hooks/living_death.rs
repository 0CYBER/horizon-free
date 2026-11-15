//! The `living_death` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mob or server player is about to die (NeoForge's `LivingDeathEvent`,
    /// vanilla's `LivingEntity.die`): its health has run out, and the loot,
    /// experience and death message have not run. `entity` is its id and
    /// `kind` its type (`minecraft:zombie`, `minecraft:player`). `source` and
    /// `weapon` are the complete killing blow, as `living_incoming_damage` saw
    /// it, including the causing player, direct projectile and weapon components.
    /// Cancellation skips death and loot without changing the listener's health:
    /// heal it to keep it alive; at health <= 0 its body is removed after 20 ticks.
    /// Armor stands, boats and other non-mob entities are not told.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "LivingDeathEvent",
        cancellable = true,
        version = 2
    )]
    fn living_death();
}
