//! The `experience_orb_can_merge` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0129

pub trait Hooks {
    /// NeoForge's `LivingKnockBackEvent`: `LivingEntity.knockback` is about to push entity
    /// `entity` of type `kind` (`minecraft:zombie`, `minecraft:player`) `strength` away from the
    /// direction (`x`, `z`), on the raw strength, before the entity's knockback resistance takes
    /// its share and its motion changes. Vanilla's direction points at the source of a hit (the
    /// attacker's side), and the entity is pushed the other way. `source` is the push's
    /// `DamageSource`, named as `native_living_incoming_damage` names a hit's. `attacker` is the
    /// attacking entity's network id, falling back to the victim's last hurt-by mob, or 0 when
    /// unknown; `attacker_main` and `attacker_off` are what it holds (a player's selected and
    /// off-hand stacks, a mob's main hand), none when unknown. `entry` is `Short` when the push
    /// came in through `knockback(power, x, z)`, which only delegates to the full entry, and `Full`
    /// when the full entry was called directly. The hook fires once per call of the full entry
    /// either way, so a listener that wants its change applied twice for a short-entry push, as
    /// a mixin on both entries would, applies it twice when `entry` is `Short`. Change
    /// `strength`, `x` or `z`, or
    /// set `canceled`, and the push follows them: canceled, the entity's motion is left alone. A
    /// `strength` of 0 or less after the hook is no push, and a direction too short to tell
    /// draws a random one after it, as in vanilla. A value a mod leaves that is not a finite
    /// number is put back as it was. It fires for the players, mobs and armor stands the game
    /// knocks back: a hit's own push, a sprinting or Knockback hit, a sweep, a spear, a
    /// shield's push on its holder, a goat's ram and a mob's melee push. It does not fire for a
    /// sulfur cube with a block that a hit with a causing entity knocks back
    /// (`SulfurCube.knockback` does not call `super` there), for the pushes vanilla doesn't make
    /// with `LivingEntity.knockback` (the ender dragon's wings, Punch arrows, explosions, wind
    /// charges, a mace smash's area, a hoglin's throw), nor for the ones Horizon makes another
    /// way (the ender dragon's). The level is the hit's: its blocks, random and own attachments.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        host = "level",
        neoforge = "LivingKnockBackEvent",
        target = "horizon_client::game::native_hooks::base::HitLevel<'_>::native_living_knockback"
    )]
    fn living_knockback(
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        attacker: i32,
        attacker_main: Option<&ItemStack>,
        attacker_off: Option<&ItemStack>,
        entry: KnockbackEntry,
        strength: &mut f64,
        x: &mut f64,
        z: &mut f64,
        canceled: &mut bool,
    );
    /// Whether the experience orb `into` takes the orb `from` in (vanilla's
    /// `ExperienceOrb.canMerge`): the same value, and ids a multiple of 40 apart. Asked
    /// for each pair whose boxes meet, when an orb spawns (`tryMergeToExisting`: `from` is
    /// 0, the new orb isn't an entity yet, and only `from_value` is known) and every 20
    /// ticks as an orb scans for neighbours (`scanForMerges`). `can_merge` holds vanilla's
    /// answer; change it either way. A merged orb counts as `count` orbs of `into`'s
    /// value, so forcing orbs of different values together drops points unless the mod
    /// raises `into`'s value with `native_set_experience_orb` in `experience_orb_merge`.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla ExperienceOrb.canMerge; NeoForge adds no event or extension for it)"
    )]
    fn experience_orb_can_merge();
}
