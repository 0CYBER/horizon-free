//! The `living_damage_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingDamageEvent.Pre`: `LivingEntity.actuallyHurt` of entity `entity` of
    /// type `kind` (`minecraft:player`, `minecraft:zombie`) by `source` (named as
    /// `native_living_incoming_damage` names it) has taken armor and magic, and the absorption
    /// hearts are next: `container` holds the damage it started with, what it is now
    /// (`new_damage`, which may be 0) and each reduction so far (armor, enchantments, mob
    /// effects). Change its
    /// `new_damage` and the health takes that; set its post-attack invulnerability ticks and the
    /// entity keeps them instead of vanilla's. A value that is not a number is put back as it
    /// was. It fires for players, mobs and the ender dragon; a wolf whose armor takes the hit and
    /// armor stands do not fire it. The level is the hit's: its blocks, random and own
    /// attachments; the hurt entity is in the middle of its hit.
    #[notes = r#"## Combat hook ownership

The six combat hooks (`living_incoming_damage`, `damage_pre`, `damage_post`, `living_knockback`, `living_use_totem`, `living_shield_block`) are lent to each hit by the game that runs it, not installed once per process. A mod sees no difference: order, arguments and cancel rules are unchanged.

**Migration (`living_use_totem`, `living_shield_block`, now version 2):** both now take the hit's level host as their first argument, as `living_incoming_damage` does: its blocks, random and own attachments, with no writes beyond them. Add the parameter (`level: &mut dyn LevelRandomHost`); the event, its fields, its order and its cancel rules are unchanged. `living_shield_block` fires inside `hurtServer` before the armor and enchantment reductions and sees the hit's `damage` as it came in; `living_use_totem` fires after health reached 0 and before `die`. Tick-time effect: one extra pointer in the hit and the same `listened` early-outs. Diverges from upstream: the two hooks gain the level host that upstream's catalog rows did not have.

**Attacker and held stacks (`living_knockback`, the weapon of `living_incoming_damage`):** the attacker's id and the stacks it holds come from the hit that started the attack, not from process state, so a hit nested inside another (a thorns reply, a guest's attack during the host's) names its own attacker and nothing is left behind afterwards. No event field, order, random draw or cancel rule changes, so neither hook's version does. The game copies the held stacks only while a mod listens; without one the hit carries the attacker id and two empty slots, no allocation."#]
    /// The container is an API-owned scalar snapshot; the engine retains sequencing and normalization.
    #[hook(
        side = "server",
        contract = "listen",
        host = "level",
        neoforge = "LivingDamageEvent.Pre",
        target = "horizon_client::game::native_hooks::living_damage_pre::HitLevel<'_>::native_living_damage_pre"
    )]
    fn living_damage_pre();
}
