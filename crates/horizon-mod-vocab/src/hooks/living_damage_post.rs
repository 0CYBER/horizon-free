//! The `living_damage_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingDamageEvent.Post`: entity `entity` of type `kind` has been hurt by the
    /// hit `container` describes, with its final values (health damage, the reductions, the
    /// post-attack invulnerability ticks). It fires once for every hit that passed
    /// `living_incoming_damage` and the invulnerability cooldown, at the very end of
    /// `hurtServer`: after a totem and the hurt sounds. The death hooks come after it, which is
    /// Horizon's order, not NeoForge's (there Post follows the death processing): a player's `die`
    /// is only a death request at that point, and a mob's death (`living_death`) is settled by the
    /// game once its hurt has returned. A hit that took no
    /// health (a shield, absorption, armor or a `living_damage_pre` listener took it all) fires
    /// it too, with `container.health_damage()` 0. `container.should_cause_side_effects()`
    /// carries `hurtServer`'s fresh-hit decision (`shouldCauseSideEffects`): false for a stronger
    /// hit inside the cooldown unless its source bypasses it, independent of the final damage
    /// or modded invulnerability ticks. It only listens. It can't write the
    /// entity or the world's blocks: the level host gives the hit's random and level attachments.
    /// The container is an API-owned scalar snapshot; the engine retains sequencing and normalization.
    #[hook(
        side = "server",
        contract = "listen",
        host = "level",
        neoforge = "LivingDamageEvent.Post",
        target = "horizon_client::game::native_hooks::living_damage_post::HitLevel<'_>::native_living_damage_post"
    )]
    fn living_damage_post();
}
