//! The `entity_leave_level` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0060

pub trait Hooks {
    /// NeoForge's `LivingIncomingDamageEvent`: `LivingEntity.hurtServer`, past its guards (an
    /// invulnerable or dead entity, fire on a fire-resistant one), is about to hurt entity
    /// `entity` of type `kind` (`minecraft:zombie`, `minecraft:player`) by `source`: its damage
    /// type (`minecraft:fall`), causing entity (`DamageSource.getEntity`: the shooter of an
    /// arrow), direct entity (`getDirectEntity`: the projectile's entity id when the
    /// damage type names one, on a player, mob or ender dragon; 0 for `magic()` and any
    /// other source with no direct entity, even during that projectile's impact), and
    /// where it comes from (`getSourcePosition`). `victim` is where the entity stands and
    /// looks, and `weapon` the direct entity's weapon (`getWeaponItem`) when it is known.
    /// `container` is an API-owned snapshot before any reduction; engine sequencing
    /// remains game-owned. Change its `new_damage`, the invulnerability ticks a fresh hit
    /// leaves (vanilla's 20), or add an ordered reduction modifier with
    /// `add_reduction_modifier`. Set `canceled` and nothing
    /// of `hurtServer` happens (no damage, hurt sound, knockback, cooldown, statistics or
    /// criteria); a canceled hit on the ender dragon still counts as landed (`EnderDragon.hurt`
    /// returns true whatever `reallyHurt` did). It fires on every damage route, for players,
    /// mobs and the ender dragon. The level is the hit's: its blocks, random and own
    /// attachments; the hurt entity is in the middle of its hit.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        host = "level",
        neoforge = "LivingIncomingDamageEvent",
        target = "horizon_client::game::native_hooks::base::HitLevel<'_>::native_living_incoming_damage"
    )]
    fn living_incoming_damage(
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        victim: &Victim,
        weapon: Option<&ItemStack>,
        container: &mut DamageContainer,
        canceled: &mut bool,
    );
    /// An entity leaves the server level `dimension` (NeoForge's
    /// `EntityLeaveLevelEvent`): vanilla's `onTrackingEnd`, when it is removed
    /// for any reason (picked up, merged, despawned, burnt, blown up, killed,
    /// taken by a hopper or a mob, gone through a portal) or its chunk unloads,
    /// a level closing with nobody in it included. Not for the entities still
    /// loaded when the server stops, as in vanilla. `entity` is its ID and
    /// `kind` its type (`minecraft:item`); it is no longer in the level's lists.
    /// It only hears. It tells every kind `entity_join_level` does
    /// (`kind` is the type), each once, when the game next notices it gone
    /// (the tick or action that removed
    /// it). Inside a transaction (`Host::transaction`) opened in the entity's
    /// own level an entity's removal takes effect, and this hook fires, at that
    /// transaction's root commit, not at the removal call (an entity in another
    /// level, as by a command run `in` it, leaves at once): from the call on its handle and selectors no longer find
    /// it. An aborted transaction removes nothing and tells nothing. An entity
    /// the transaction spawned and removed is never told to join, so it never
    /// leaves. Check `kind`. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "EntityLeaveLevelEvent",
        version = 2
    )]
    fn entity_leave_level();
}
