//! The `living_breathe` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0151

pub trait Hooks {
    /// An entity's breathing this tick (`LivingBreatheEvent`, posted from
    /// `LivingEntity.baseTick` every tick for a living entity with the generic
    /// air logic): `entity` is its ID and `kind` its type (`minecraft:player`,
    /// `minecraft:zombie`). `can_breathe`, `consume` and `refill` come in as
    /// Horizon does it (a mod-free game changes nothing); set them to
    /// change this tick. With `can_breathe` the air rises by `refill`, up to its
    /// maximum (300 for every entity except dolphins, which have 4800); a dolphin's
    /// vanilla `refill` is its maximum minus its current air. Without breath,
    /// air falls by `consume`, and at -20 it is set to 0
    /// and the entity takes 2 drowning damage. Negative amounts count as 0.
    /// Asked on servers for players and mobs. A water animal, axolotl or
    /// nautilus comes in with its own air logic's answer: where it breathes
    /// (water, or rain for an axolotl) `can_breathe` and a `refill` to its full
    /// air; elsewhere no breath and `consume` 1. Running out deals its own
    /// damage (drowning, or drying out). It is given the loop `context` of the server that ticks the entity (its registries, settings and mod list), because it fires inside the tick's own air update.
    #[hook(
        side = "server",
        contract = "listen",
        context = "loop",
        neoforge = "LivingBreatheEvent",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::breathing::native_living_breathe"
    )]
    fn living_breathe();
}
