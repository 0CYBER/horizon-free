use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `MobEffectEvent.Applicable`, vanilla's
    /// `LivingEntity.canBeAffected`: asked wherever the game checks whether
    /// the effect `effect` can land on the living entity `entity` (a
    /// `LivingRef`, the player's own included), before `addEffect`
    /// accepts it. `applicable` starts unanswered: leave it and the vanilla
    /// tag check decides; `Allow` or `Deny` overrides it. Server-side.
    pub(in crate::game) fn native_mob_effect_applicable(
        &mut self,
        entity: LivingRef,
        effect: EffectKey,
        applicable: &mut Option<mod_api::EffectApplicable>,
    ) {
        self.fire_mob_effect_applicable(entity, effect, applicable);
    }
    /// `LivingEntity.hurtServer`, past its guards (invulnerable, dead, fire
    /// with fire resistance), is about to hurt entity `entity` of type `kind`
    /// (`minecraft:zombie`, `minecraft:player`) by `amount` of `damage_type`
    /// (`minecraft:fall`), from the source's `causing` entity
    /// (`DamageSource.getEntity`: the shooter of an arrow) and `direct` one
    /// (`getDirectEntity`: the arrow), 0 for none or not known yet
    /// (docs/native-mods.md, API 50). Change `amount`, or set `canceled`,
    /// and nothing of `hurtServer` itself happens (no damage, hurt sound,
    /// knockback, invulnerable time, statistics or criteria); the caller acts
    /// as on a refused hit, except that a canceled hit on the ender dragon
    /// still counts as landed (`EnderDragon.hurt` returns true whatever
    /// `reallyHurt` did: the arrow is spent, the melee effects run). It runs
    /// on the thread that ticks the world (the server thread): state shared with render thread hooks needs synchronising.
    /// A player's weapon hit on a mob, before `Mob::damage_with` borrows the
    /// victim: melee, sweep and spear hits, including LAN guests. Unlike v1,
    /// this has a host and snapshots of the weapon, victim position/view and
    /// DamageSource position. The acting player's network id is `causing`
    /// (also the direct entity). Weapon enchantments include their levels.
    /// Runs after the victim's invulnerability check (an invulnerable or dead victim fires nothing) and before v1; cancellation refuses
    /// `LivingEntity.knockback` is about to push entity `entity` of type `kind`
    /// (`minecraft:zombie`, `minecraft:player`) `strength` away from the direction (`x`, `z`),
    /// on the raw strength, before the entity's knockback resistance takes its share and its motion
    /// changes
    /// (docs/native-mods.md, API 125). Vanilla's direction points at the
    /// source of a hit (the attacker's side), and the entity is pushed the other way. Change
    /// `strength`, `x` or `z`, or set `canceled`, and the push follows them: canceled, the
    /// entity's motion is left alone. A `strength` of 0 or less after the hook is no push, and a
    /// direction too short to tell draws a random one after it, as in vanilla. A value a mod
    /// leaves that is not a finite number is put back as it was. `damage_type`
    /// (`minecraft:player_attack`), `causing` (`DamageSource.getEntity`) and `direct`
    /// (`getDirectEntity`) are the push's `DamageSource`, named as `native_living_incoming_damage`
    /// names a hit's, with the same entity ids left at 0. It fires for the players and mobs the
    /// game knocks back: a hit's own push, a sprinting or Knockback hit, a sweep, a spear, a
    /// shield's push on its holder and a goat's ram. It does not fire for a sulfur cube with a
    /// block that a hit with a causing entity knocks back (`SulfurCube.knockback` does not call
    /// `super` there), for the pushes vanilla doesn't make with `LivingEntity.knockback` (the
    /// ender dragon's wings, Punch arrows, explosions, wind charges, a mace smash's area, a
    /// hoglin's throw), nor yet for the ones Horizon makes another way: a mob's melee push from
    /// its attack knockback or a Knockback weapon, an armor stand's, the ender dragon's. It runs
    /// on the thread that ticks the world (the server thread): state
    /// shared with render thread hooks needs synchronising. No `self`: the game is in the middle
    /// NeoForge's `MobEffectEvent.Expired`, vanilla `LivingEntity.tickEffects`:
    /// the effect `effect` (`minecraft:speed`, amplifier `amplifier`) on
    /// the living entity `entity` counted down to nothing and is about to
    /// leave. Cancel holds it there, asked again next tick; unanswered, the
    /// expiry runs. It runs inside the entity's tick, so the game is
    /// read-only there. Server-side.
    pub(in crate::game) fn native_mob_effect_expired(
        &self,
        entity: LivingRef,
        effect: EffectKey,
        amplifier: i32,
        cancel: &mut bool,
    ) {
        self.fire_mob_effect_expired(entity, effect, amplifier, cancel);
    }
    /// NeoForge's `MobEffectEvent.Remove`, vanilla `LivingEntity.removeEffect`:
    /// the effect `effect` on the living entity `entity` is about to be
    /// removed by any path that walks the vanilla funnel: a command or milk
    /// for the player, a cure for a zombie villager, a mod's removal
    /// helper. Cancel and the effect stays. Server-side.
    pub(in crate::game) fn native_mob_effect_remove(
        &mut self,
        entity: LivingRef,
        effect: EffectKey,
        amplifier: i32,
        cancel: &mut bool,
    ) {
        self.fire_mob_effect_remove(entity, effect, amplifier, cancel);
    }
}
