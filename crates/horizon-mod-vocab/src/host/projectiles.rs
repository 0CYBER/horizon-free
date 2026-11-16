//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `projectiles` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait ProjectilesServerHost {
    /// Side: server. `Projectile.spawnProjectileFromRotation` for a thrown item
    /// projectile: snowball, egg, pearl, potion, experience bottle, wind charge,
    /// or a registered mod thrown type. Constructs at the owner's eyes minus
    /// 0.1F blocks (wind charges use eye height directly), sets ownership before
    /// shooting, inherits known movement
    /// (vertical only while airborne), adds to the level and fires projectile_launch.
    /// Spread draws x, y, z triangular noise from the new projectile's own random.
    /// Once it is in the level, `stack`'s enchantments run their
    /// `projectile_spawned` effects on it (`Projectile.applyOnProjectileSpawned`):
    /// in the stack's order, each effect's requirements tested with the level
    /// random just before it applies, its own draws from the projectile's random.
    /// Does not consume the stack or apply item-use cooldowns or statistics.
    /// None for an unavailable owner or one in another level, unsupported type,
    /// invalid stack or non-finite shot parameter. Also refuses a stack with a
    /// `projectile_spawned` effect the engine does not apply: `ignite`,
    /// `apply_impulse`, `play_sound` and `all_of` of those apply
    /// (`apply_mob_effect` and `apply_exhaustion` do nothing to a projectile,
    /// as in vanilla); the block, damage, explosion, particle, summon,
    /// function and item-damage effects are refused rather than skipped. No
    /// random is drawn and nothing spawns on those refusals. Unsupported
    /// requirement contexts (including entity flags) and invalid clamped
    /// level-value ranges also refuse before insertion. Type predicates,
    /// random chance and their boolean combinations are supported.
    /// The original stack
    /// is unchanged; carried count is one; the effects see the stack as given.
    #[access(WorldOnly)]
    fn spawn_projectile_from_rotation(
        &mut self,
        owner: LivingRef,
        entity_type: &Identifier,
        stack: &ItemStack,
        offset: f32,
        power: f32,
        inaccuracy: f32,
    ) -> Option<ProjectileRef>;
    /// Side: server.
    /// Narrows an entity to a projectile: one in the projectile list, a trident
    /// a player threw or a firework rocket. None for another kind, a stale
    /// handle or a client.
    #[access(WorldOnly)]
    fn projectile(&self, entity: EntityRef) -> Option<ProjectileRef>;
    /// Side: server.
    /// Narrows an entity to a cast fishing hook. None for another kind, a
    /// stale handle, a client, or a hook taken out for its tick.
    #[access(WorldOnly)]
    fn fishing_hook(&self, entity: EntityRef) -> Option<FishingHookRef>;
}
/// The handles' reads of the `projectiles` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait ProjectilesBackend {
    /// Side: server.
    /// `Projectile.getOwner`: the owner as a resolved entity. None for no
    /// owner, one that is gone or in another level, or a stale handle.
    #[access(WorldOnly)]
    fn projectile_owner(&self, projectile: ProjectileRef) -> Option<EntityRef>;
    /// Side: server.
    /// `AbstractArrow.pickup`, for an arrow or a mob's trident.
    #[access(WorldOnly)]
    fn projectile_pickup(&self, projectile: ProjectileRef) -> Option<ArrowPickup>;
    /// Side: server.
    /// `AbstractArrow.getPierceLevel`: the signed byte stored by vanilla.
    #[access(WorldOnly)]
    fn projectile_pierce_level(&self, projectile: ProjectileRef) -> Result<i8, PierceLevelError>;
    /// Side: server.
    /// `AbstractArrow.getBaseDamage`, for an arrow (a trident's is fixed).
    #[access(WorldOnly)]
    fn projectile_base_damage(&self, projectile: ProjectileRef) -> Option<f64>;
    /// Side: server.
    /// `AbstractArrow.isCritArrow`, for an arrow.
    #[access(WorldOnly)]
    fn projectile_critical(&self, projectile: ProjectileRef) -> Option<bool>;
    /// Side: server.
    /// `FishingHook.getPlayerOwner`: the player who cast it. None for a stale
    /// handle.
    #[access(WorldOnly)]
    fn fishing_hook_player_owner(&self, hook: FishingHookRef) -> Option<EntityRef>;
    /// Side: server.
    /// `FishingHook.getHookedIn`: what it holds. None when it holds nothing,
    /// what it holds is gone, or for a stale handle.
    #[access(WorldOnly)]
    fn fishing_hook_hooked_in(&self, hook: FishingHookRef) -> Option<EntityRef>;
    /// Side: server.
    /// `FishingHook.isOpenWaterFishing`, which the `fishing` loot's
    /// `in_open_water` predicate reads. None for a stale handle.
    #[access(WorldOnly)]
    fn fishing_hook_open_water(&self, hook: FishingHookRef) -> Option<bool>;
}
/// The handles' writes of the `projectiles` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait ProjectilesBackendMut {
    /// Side: server.
    /// `Projectile.setOwner`. False for a stale projectile or owner, or an
    /// owner that is a fishing hook (a trident a player threw takes a player
    /// only). Does not touch `leftOwner`.
    #[access(WorldOnly)]
    fn projectile_set_owner(&mut self, projectile: ProjectileRef, owner: Option<EntityRef>)
    -> bool;
    /// Side: server.
    /// `Projectile.deflect(deflection, by, newOwner, byAttack, power)`, with
    /// `power` per axis, through `Projectile::deflect` for the projectile list
    /// (the engine path gameplay uses). `REVERSE` draws one float from the
    /// projectile's own random; `AIM_DEFLECT` uses `by`'s look and
    /// `MOMENTUM_DEFLECT` its normalized known speed, and both change nothing
    /// without `by`. The new owner is set, then `onDeflection`: a hurting
    /// projectile's acceleration power is 0.1 by an attack, else halved. A
    /// wind charge refuses for its first five ticks, with no change and no
    /// draw. A trident a player threw and a rocket turn the same way (no
    /// acceleration power). False, and nothing changed, for a stale projectile,
    /// `by` or owner, a non-finite power or a refusal.
    #[access(WorldOnly)]
    fn projectile_deflect(
        &mut self,
        projectile: ProjectileRef,
        deflection: Deflection,
        by: Option<EntityRef>,
        new_owner: Option<EntityRef>,
        by_attack: bool,
        power: [f64; 3],
    ) -> bool;
    /// Side: server.
    /// Sets `AbstractArrow.pickup`. False for a stale handle or a projectile
    /// that is not an arrow or a mob's trident.
    #[access(WorldOnly)]
    fn projectile_set_pickup(&mut self, projectile: ProjectileRef, pickup: ArrowPickup) -> bool;
    /// Side: server.
    /// `AbstractArrow.setPierceLevel`: stores the signed byte without clamping.
    #[access(WorldOnly)]
    fn projectile_set_pierce_level(
        &mut self,
        projectile: ProjectileRef,
        level: i8,
    ) -> Result<(), PierceLevelError>;
    /// Side: server.
    /// `AbstractArrow.setBaseDamage`. False for a stale handle, a projectile
    /// that is not an arrow, or a value that is not finite.
    #[access(WorldOnly)]
    fn projectile_set_base_damage(&mut self, projectile: ProjectileRef, damage: f64) -> bool;
    /// Side: server.
    /// `AbstractArrow.setCritArrow`; clients see the flag at once through the
    /// entity data. False for a stale handle or a projectile that is not an arrow.
    #[access(WorldOnly)]
    fn projectile_set_critical(&mut self, projectile: ProjectileRef, critical: bool) -> bool;
}
