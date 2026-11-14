//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `living` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait LivingHost {
    /// Side: both.
    /// The entity as a living entity (vanilla's `LivingEntity`: a mob, a
    /// player, an armor stand or the dragon), or None for another kind or a
    /// stale handle.
    ///
    /// On a client, only the local player is one: the client keeps the others'
    /// equipment, effects and health only as far as it draws them. API 382.
    #[access(WorldOnly)]
    fn living(&self, entity: EntityRef) -> Option<LivingRef>;
}
/// The handles' reads of the `living` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait LivingBackend {
    /// Side: both.
    /// The entity's instance of `effect` (`LivingEntity.getEffect`): its id
    /// namespaced as the registry has it, the ticks left (-1 infinite), the
    /// amplifier and the flags. The effect is a registration-slot key resolved at load.
    /// None when it has the effect not, before effect resolution, or for an armor
    /// stand, the dragon or a stale handle; on a client, for any entity but the
    /// local player, whose copy of what the server sent it is read.
    #[access(WorldOnly)]
    fn living_effect(&self, living: LivingRef, effect: EffectKey) -> Option<ActiveEffect>;
    /// Side: both.
    /// The entity's active effects (`LivingEntity.getActiveEffects`), in the
    /// order their store keeps them, in `Some` even when it has none. None
    /// for an armor stand, the dragon or a stale handle; on a client, for any
    /// entity but the local player (its copy of what the server sent it).
    #[access(WorldOnly)]
    fn living_effects(&self, living: LivingRef) -> Option<Vec<ActiveEffect>>;
    /// Side: both.
    /// Whether the entity has `effect` (`LivingEntity.hasEffect`), using its resolved key.
    /// False before effect resolution, for an armor stand, the dragon or a stale handle;
    /// on a client, for any entity but the local player, whose copy of what the server
    /// sent it is read.
    #[access(WorldOnly)]
    fn living_potion_affected(&self, living: LivingRef, effect: EffectKey) -> bool;
    /// Side: both.
    /// The ticks the entity has glided (`LivingEntity.getFallFlyingTicks`),
    /// counted from 0 again the tick after the glide ends. Players (including
    /// inactive guests) and mobs read their own stored body counter. None
    /// for an armor stand or the dragon.
    #[access(WorldOnly)]
    fn living_gliding_ticks(&self, living: LivingRef) -> Option<i32>;
    /// Side: both.
    /// Whether the entity glides on elytra (`LivingEntity.isFallFlying`;
    /// Paper's `LivingEntity#isGliding`). The player in place and the guests
    /// answer from their fall-flying flag, a mob from its own. None for an
    /// armor stand or the dragon: the game keeps no glide for them.
    #[access(WorldOnly)]
    fn living_is_gliding(&self, living: LivingRef) -> Option<bool>;
    /// Side: both.
    /// Whether the entity sleeps in a bed (`LivingEntity.isSleeping`): the
    /// player in place, a guest, or a villager or wandering trader asleep in
    /// its bed. Other mobs never sleep. None for an armor stand or the
    /// dragon.
    #[access(WorldOnly)]
    fn living_is_sleeping(&self, living: LivingRef) -> Option<bool>;
    /// Side: both.
    /// The head of the bed the entity sleeps in (`LivingEntity.getSleepingPos`),
    /// as the block position it lies at. A guest keeps the bed it last slept
    /// in while it sleeps, a villager or wandering trader its own. None for
    /// an awake entity, another mob, an armor stand or the dragon.
    #[access(WorldOnly)]
    fn living_sleeping_pos(&self, living: LivingRef) -> Option<[i32; 3]>;
    /// Side: both.
    /// Whether the entity jumps (`LivingEntity.isJumping`; Paper's
    /// `LivingEntity#isJumping`): the player in place answers from their
    /// own jump state. The guests' jump is their client's, and a mob has no
    /// jump controller in the game's own AI, so both read false. None for
    /// an armor stand or the dragon.
    #[access(WorldOnly)]
    fn living_is_jumping(&self, living: LivingRef) -> Option<bool>;
    /// Side: both.
    /// Whether the entity is on a climbable block this tick
    /// (`LivingEntity.onClimbable`; Paper's `LivingEntity#isClimbing`), by
    /// the block at its feet: a ladder, a vine, a scaffold or an open
    /// trapdoor above a ladder. A spectator never climbs; a gliding entity
    /// ignores blocks tagged to glide through, such as vines. None for an
    /// armor stand or the dragon.
    #[access(WorldOnly)]
    fn living_is_climbing(&self, living: LivingRef) -> Option<bool>;
    /// Side: both.
    /// Whether the entity is a baby (`LivingEntity.isBaby`): a mob answers
    /// from its age, negative until it grows up. A player, an armor stand
    /// and the dragon are never babies.
    #[access(WorldOnly)]
    fn living_baby(&self, living: LivingRef) -> Option<bool>;
    /// Side: both.
    /// The entity's health (`LivingEntity.getHealth`; Paper's
    /// `Damageable#getHealth`). During a player's turn only that player
    /// resolves: the hub is out for their action, and other players'
    /// handles read `None` until it is back.
    #[access(WorldOnly)]
    fn living_health(&self, living: LivingRef) -> Option<f32>;
    /// Side: both.
    /// The entity's absorption amount (`LivingEntity.getAbsorptionAmount`; Paper's
    /// `Damageable#getAbsorptionAmount`), the extra hearts that take a hit before
    /// health does. Resolves as `living_health` does: during a player's turn only
    /// that player. None for an armor stand and the dragon, which carry none.
    #[access(WorldOnly)]
    fn living_absorption(&self, living: LivingRef) -> Option<f32>;
    /// Side: both.
    /// The entity's maximum health (`LivingEntity.getMaxHealth`; Paper's
    /// `Damageable#getMaxHealth`). Players and guests read their attribute
    /// map; a mob, an armor stand and the dragon their own fields (the
    /// armor stand the fixed 20 the game seeds it with, the dragon its
    /// 200) until live attribute maps cover them.
    #[access(WorldOnly)]
    fn living_max_health(&self, living: LivingRef) -> Option<f32>;
    /// Side: server.
    /// Whether the entity is alive (`LivingEntity.isAlive`; Paper's
    /// `Entity#isDead` is its inverse). A dying mob, mid death
    /// animation, is not.
    #[access(WorldOnly)]
    fn living_alive(&self, living: LivingRef) -> Option<bool>;
    /// Side: server.
    /// `LivingEntity.getKillCredit`: the remembered player takes precedence,
    /// even if the player cannot be resolved; otherwise the last hurting mob.
    /// Unlike vanilla, a mob that changed dimension cannot be resolved.
    #[access(WorldOnly)]
    fn living_kill_credit(&self, living: LivingRef) -> Option<EntityRef>;
    /// Side: server.
    /// `LivingEntity.getLastHurtByMob`: the entity that last hurt this one,
    /// within a hundred ticks of the hurt (the window of
    /// `CombatTracker.getKiller`'s credit and a mob's hurt-by memory, as
    /// `armadillo.rs` reads it). None for a stale handle, a window that has
    /// passed, an attacker that is gone, an armor stand, the dragon or a
    /// client.
    #[access(WorldOnly)]
    fn living_last_attacker(&self, living: LivingRef) -> Option<EntityRef>;
    /// Side: server.
    /// `LivingEntity.hasLineOfSight(Entity)`: whether nothing that collides
    /// (`ClipContext.Block.COLLIDER`, fluids ignored) lies between the
    /// living entity's eyes and `target`'s eye height, within 128 blocks.
    /// False for a target in another level. None for a stale or unavailable
    /// living entity or target, one outside the level in place, or on a
    /// client.
    #[access(WorldOnly)]
    fn living_has_line_of_sight(&self, living: LivingRef, target: EntityRef) -> Option<bool>;
    /// Side: both.
    /// The effective value of an attribute on a living entity
    /// (`AttributeMap.getValue`). None for an unknown or absent attribute, an
    /// unavailable entity, a stale handle, or on a client for any entity but
    /// the local player, whose copy of the synced instances it reads. A mob,
    /// an armor stand and the dragon answer from their live attribute map: the
    /// type's defaults, the saved `attributes`, and the modifiers of what they
    /// wear or hold.
    #[access(WorldOnly)]
    fn living_attribute_value(&self, living: LivingRef, id: &str) -> Option<f64>;
    /// Side: both.
    /// The base value of an attribute, before modifiers
    /// (`AttributeMap.getBaseValue`), read as the effective value is.
    #[access(WorldOnly)]
    fn living_attribute_base(&self, living: LivingRef, id: &str) -> Option<f64>;
    /// Side: both.
    /// The ids of the attributes the entity's type has
    /// (`AttributeMap`'s holder's attribute set), in registry order: the same
    /// set the value and base reads answer for. None for a stale handle or an
    /// unavailable entity; on a client, for any entity but the local player
    /// (the client keeps no attribute map of another entity's, only the few
    /// fields it draws).
    #[access(WorldOnly)]
    fn living_attributes(&self, living: LivingRef) -> Option<Vec<Identifier>>;
    /// Side: both.
    /// The modifiers on the entity's attribute `id`
    /// (`AttributeInstance.getModifiers`), in the order they were added: a
    /// mod's transient and permanent ones, the saved list's, and what the
    /// worn and held items grant. An attribute no modifier is on answers an
    /// empty list; None for an attribute the entity type does not have, a
    /// stale handle, or on a client for any entity but the local player,
    /// whose copy of the synced instances it reads.
    #[access(WorldOnly)]
    fn living_attribute_modifiers(
        &self,
        living: LivingRef,
        id: &str,
    ) -> Option<Vec<AttributeModifier>>;
    /// Side: both.
    /// The stack in `slot` (`LivingEntity.getItemBySlot`). Players, guests,
    /// mobs, armor stands and the dragon store all eight slots. An armor
    /// stand's body and saddle are storage only: vanilla's `canUseSlot`
    /// refuses them for interaction. None means empty, unsupported or stale.
    /// The stack in equipment slot `slot` (`LivingEntity.getItemBySlot`):
    /// None when it is empty, the entity has no such slot (a player has no
    /// body or saddle slot), or the handle is stale. API 382.
    #[access(WorldOnly)]
    fn living_equipment(&self, living: LivingRef, slot: EquipmentSlot) -> Option<ItemStack>;
    /// Side: both.
    /// The stack in hand `hand` (`LivingEntity.getItemInHand`). API 382.
    #[access(WorldOnly)]
    fn living_hand_item(&self, living: LivingRef, hand: Hand) -> Option<ItemStack>;
    /// Side: server.
    /// DamageSource.getWeaponItem on a living attacker: the active auto-spin
    /// stack for a player, otherwise its live main hand. None if unavailable.
    #[access(WorldOnly)]
    fn living_weapon_item(&self, living: LivingRef) -> Option<ItemStack>;
    /// Side: server.
    /// `TamableAnimal.isTame`, `AbstractHorse.isTamed`: None for a kind that
    /// can't be tamed or a stale handle. API @API:living_refs@.
    #[access(WorldOnly)]
    fn living_is_tamed(&self, living: LivingRef) -> Option<bool>;
    /// Side: server.
    /// The owner's UUID (`OwnableEntity.getOwnerReference`): None for a wild
    /// mob, a kind that can't be tamed, or a stale handle. API @API:living_refs@.
    #[access(WorldOnly)]
    fn living_owner(&self, living: LivingRef) -> Option<u128>;
    /// Side: server.
    /// `LivingEntity.getUseItemRemainingTicks`: the ticks left of the item a
    /// player or a mob uses, 0 when it uses none (a piglin admiring gold and
    /// a drinking witch use none: their counter is their own timer). A mob's
    /// is the item's use duration less the ticks its use counter has run.
    /// None for armor stands and the dragon, and for a stale handle.
    #[access(WorldOnly)]
    fn living_item_use_remaining(&self, living: LivingRef) -> Option<i32>;
    /// Side: both.
    /// `LivingEntity.getUsedItemHand` while `isUsingItem`: the hand of the item
    /// a player or a mob uses, None when it uses none. On a client, the local
    /// player's only.
    #[access(WorldOnly)]
    fn living_used_item_hand(&self, living: LivingRef) -> Option<Hand>;
    /// Side: both.
    /// `LivingEntity.isBlocking`: whether a player or a mob holds up an item that blocks attacks
    /// (a shield, or any item with `minecraft:blocks_attacks`): it is using that item, from either
    /// hand, and has used it for the component's `block_delay_seconds` (5 ticks for a shield).
    /// False while it uses nothing or another item. On a client, the local player's only. None
    /// for armor stands and the dragon, and for a stale handle.
    #[access(WorldOnly)]
    fn living_is_blocking(&self, living: LivingRef) -> Option<bool>;
    /// Side: both.
    /// `LivingEntity.getDimensions(Pose)`: the box the living entity has in
    /// `pose` — its width, height and eye height, `minecraft:scale`
    /// included, the fixed 0.2 box of a sleeping pose included. The species'
    /// own state still answers where its `getDefaultDimensions` reads it: a
    /// digging sniffer or an emerging warden is low in every pose. None for
    /// a stale or unavailable handle.
    #[access(WorldOnly)]
    fn living_dimensions_for_pose(&self, living: LivingRef, pose: Pose) -> Option<[f64; 3]>;
    /// Side: server.
    /// `Entity.canUsePortal`: the engine's one gate for a player,
    /// `player_can_use_portal` (`Entity.canUsePortal(false)`: alive, nobody's
    /// passenger — and not asleep, vanilla `LivingEntity.canUsePortal`'s
    /// `isSleeping` half; a parity fix: the client's portal entry used to
    /// skip it), a guest's the same rule over their body and vehicle, a
    /// mob's walk gate (`mob_can_use_portal`, 's read: not a boss that
    /// never travels, not a creaking bound to its heart, not asleep in its
    /// bed, riding nothing, carrying no one this side holds back). None for
    /// a stale handle, an armor stand or the dragon: the portal walk
    /// carries neither as its own traveller.
    #[access(WorldOnly)]
    fn living_can_use_portal(&self, living: LivingRef) -> Option<bool>;
    /// Side: server.
    /// `Mob.isWithinMeleeAttackRange`: the melee goal's own reach check —
    /// the mob's box inflated by sqrt(2.04) − 0.6 over the target's box, or
    /// an active item's attack range for a mob (a spear's reach) — whether
    /// this living entity's next melee strike reaches `target`. The check is
    /// a mob's; a player's attack reach is another rule (the interaction
    /// range attribute), so a player answers None — as does every living
    /// entity that is no mob (a guest, an armor stand, the dragon), a
    /// stale `living` handle, a mob already removed, or a `target` with no
    /// box here (a stale or unavailable handle).
    #[access(WorldOnly)]
    fn living_in_melee_range(&self, living: LivingRef, target: EntityRef) -> Option<bool>;
    /// Side: server.
    /// `LivingEntity.canFreeze`: the decision the freeze tick itself runs —
    /// a mob's (`ai::can_freeze`: not `#freeze_immune_entity_types`, no
    /// `#freeze_immune_wearables` worn), a player's
    /// (`PlayerMotion::can_freeze`: not a spectator, no immune armor).
    /// None for a stale handle, an armor stand or the dragon.
    #[access(WorldOnly)]
    fn living_can_freeze(&self, living: LivingRef) -> Option<bool>;
    /// Side: server.
    /// `LivingEntity.canBreatheUnderwater`: the species' own breath —
    /// `is(#can_breathe_under_water)` in vanilla, `breathes_water` here (a
    /// squid's true, a dolphin's false); a player answers false (no player
    /// kind is in the tag). This is the tag read only, not the whole drown
    /// decision: the air tick itself also keeps the air up behind Water
    /// Breathing, the conduit's power and a player's invulnerability,
    /// which this read does not carry. None for a stale handle, an armor
    /// stand or the dragon.
    #[access(WorldOnly)]
    fn living_can_breathe_underwater(&self, living: LivingRef) -> Option<bool>;
    /// Side: server.
    /// The bed a sleeper lies in, as its block faces
    /// (`LivingEntity.getBedOrientation` over `BedBlock.getBedOrientation`):
    /// the block at the sleeping position, its `facing` where that block is
    /// one of `#minecraft:beds` — one path for every sleeper. None where
    /// the sleeper keeps no position, the block is not a bed, or the handle
    /// is stale.
    #[access(WorldOnly)]
    fn living_bed_orientation(&self, living: LivingRef) -> Option<Direction>;
}
/// The handles' client reads of the `living` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait LivingClientBackend {
    /// Side: client.
    /// `LivingEntity.getEffectBlendFactor(effect, partialTick)` on a client:
    /// how far the effect has faded in, from 0 to 1, between the last tick
    /// and the next at `partial`. Darkness fades in and out over 22 ticks and
    /// nausea in over 150 and out over 20 (`MobEffect.setBlendDuration`), the
    /// fades the fog, lightmap and screen spin draw. Other effects start at
    /// 0 after a blended add and interpolate to their post-countdown target
    /// during each client tick: 1 while active, 0 once expired. An unblended
    /// add starts at its target immediately; an update preserves the old blend.
    /// `partial` is not clamped. The registration-slot key is resolved at load.
    /// 0 when the entity lacks the effect. None before effect resolution, for a stale
    /// handle, or an entity other than the local player: the client keeps no
    /// other entity's effects.
    #[access(WorldOnly)]
    fn living_effect_blend_factor(
        &self,
        living: LivingRef,
        effect: EffectKey,
        partial: f32,
    ) -> Option<f32>;
}
/// The handles' writes of the `living` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait LivingBackendMut {
    /// Side: server.
    /// Apply one loaded consumable effect without consuming the item.
    /// Mod-defined kinds enter the normal consume_effect hook: Some(true) means
    /// dispatch was entered, not that a listener changed game state. Reapplying
    /// the same kind from its own handler on this call stack returns Some(false)
    /// without recursing; other kinds dispatch normally. None means an unavailable
    /// item/index/target or an unsupported vanilla effect for that target.
    #[access(WorldOnly)]
    fn living_apply_consume_effect(
        &mut self,
        living: LivingRef,
        stack: &ItemStack,
        index: usize,
    ) -> Result<Option<bool>, StackWriteError>;
    /// Side: server.
    /// `LivingEntity.addEffect(instance)`: gives the living entity the
    /// effect by the game's own path. A player's or a guest's goes through
    /// `add_player_effect` (so `MobEffectInstance.update`'s rules, the
    /// attribute modifiers, absorption and the added-effect hooks of a
    /// content mod's effect all apply); a mob's goes past `canBeAffected`
    /// (a spider shrugs off poison) into its effects with the same hooks.
    /// The amplifier is cut to 0..=255 and the duration is -1 (infinite) or
    /// ticks. False before effect resolution, for a duration below -1, an armor
    /// stand, the dragon, a stale handle or a client, or when the active
    /// instance did not change, as in vanilla's `addEffect`.
    #[access(WorldOnly)]
    fn living_add_effect(&mut self, living: LivingRef, effect: &ActiveEffect) -> bool;
    /// Side: server.
    /// `LivingEntity.removeEffect(effect)` the way its running out goes
    /// (`LivingEntity.tickEffects` drops it): what the effect gave the
    /// player goes with it — the attribute modifiers recompute
    /// (`apply_effect_status`) and absorption's hearts leave — and a mob's
    /// hidden and ambient lists drop it too, with absorption's hearts
    /// leaving the same way its expiry takes them. False (and nothing
    /// changes) when the entity has the effect not, before effect resolution, or for
    /// an armor stand, the dragon, a stale handle or a client.
    #[access(WorldOnly)]
    fn living_remove_effect(&mut self, living: LivingRef, effect: EffectKey) -> bool;
    /// Side: server.
    /// `LivingEntity.removeAllEffects()` the way their running out goes:
    /// every effect's attribute modifiers and absorption hearts leave with
    /// them. False when the entity has none, or for an armor stand, the
    /// dragon, a stale handle or a client.
    #[access(WorldOnly)]
    fn living_remove_all_effects(&mut self, living: LivingRef) -> bool;
    /// Side: server.
    /// `LivingEntity.stopFallFlying`: clears the glide flag, so the glide's
    /// wear and sound stop and the next fall is a fall. The players around
    /// learn it with the entity's next flag sync, as they do when a glide
    /// ends on its own. True for a player, a guest or a mob; false for an
    /// armor stand or the dragon.
    #[access(WorldOnly)]
    fn living_stop_gliding(&mut self, living: LivingRef) -> bool;
    /// Side: server.
    /// `LivingEntity.startSleeping`: the entity lies in the bed whose head
    /// `pos` names, with the effect the game's own sleep has. The player in
    /// place is moved onto the bed, put to sleep and occupies it; a
    /// villager or wandering trader takes a valid, unoccupied bed. Normal
    /// integrated-owner, LAN and dedicated guests use their server body,
    /// inside or outside their own turn, in the level in place. False for
    /// a guest in another level, a fake player, another
    /// mob, an armor stand or the dragon, or a bed without a sleep shape.
    #[access(WorldOnly)]
    fn living_start_sleeping(&mut self, living: LivingRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// `LivingEntity.stopSleeping`: the sleeper stands up beside the bed, as
    /// the game's own waking does, with its pose and packets. True when the
    /// entity slept; false for a fake player, another mob, an armor stand
    /// or the dragon. Normal server guests in the level in place use the
    /// same wake-up path; guests in another level are refused.
    #[access(WorldOnly)]
    fn living_stop_sleeping(&mut self, living: LivingRef) -> bool;
    /// Side: server.
    /// `LivingEntity.setSleepingPos`: the bed `pos` names becomes the one
    /// the player sleeps in, without moving them; the next entity sync
    /// tells the players around they sleep there, as it tells them the
    /// game's own sleep. The villagers' beds belong to the evening's bed
    /// path (`start_sleeping`). Normal server guests in the level in place
    /// are supported; guests in another level, fake players, mobs, armor
    /// stands and the dragon are refused. This only
    /// changes the sleeping position/pose, not bed occupancy.
    #[access(WorldOnly)]
    fn living_set_sleeping_pos(&mut self, living: LivingRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// `LivingEntity.clearSleepingPos`: the player no longer names a bed, so
    /// `is_sleeping` is false and the next entity sync stands them up.
    /// Normal server guests in the level in place are supported; guests in
    /// another level, fake players, mobs, armor stands and the dragon are
    /// refused. This does not run the wake-up animation
    /// or move the player; use `stop_sleeping` to stand beside the bed.
    #[access(WorldOnly)]
    fn living_clear_sleeping_pos(&mut self, living: LivingRef) -> bool;
    /// Side: server.
    /// `LivingEntity.setJumping`: whether the player in place's next step
    /// jumps, as their own key does. False for a guest outside its turn,
    /// whose client holds its own jump, a mob, an armor stand or the
    /// dragon.
    #[access(WorldOnly)]
    fn living_set_is_jumping(&mut self, living: LivingRef, jumping: bool) -> bool;
    /// Side: server.
    /// Sets the entity's health (`LivingEntity.setHealth`; Paper's
    /// `Damageable#setHealth`), cut to its maximum health. No heal or damage
    /// hook runs. False for a stale handle, a non-finite value, an armor stand
    /// or the dragon.
    #[access(WorldOnly)]
    fn living_set_health(&mut self, living: LivingRef, health: f32) -> bool;
    /// Side: server.
    /// Sets the entity's absorption amount (`LivingEntity.setAbsorptionAmount`;
    /// Paper's `Damageable#setAbsorptionAmount`), cut to `0..=max_absorption`
    /// (the `minecraft:max_absorption` attribute). A player's new amount reaches
    /// their client and the trackers through the synced data the same tick. No
    /// heal or damage hook runs. False for a stale handle, a non-finite amount, an
    /// armor stand or the dragon.
    #[access(WorldOnly)]
    fn living_set_absorption(&mut self, living: LivingRef, amount: f32) -> bool;
    /// Side: server.
    /// Heals the entity by `amount` (`LivingEntity.heal`), through the
    /// `living_heal` hook, which may change or cancel it. A dead or dying
    /// entity isn't healed, and health stops at the maximum. False for a stale
    /// handle, a non-finite amount, a zero-health entity, an armor stand or the dragon.
    #[access(WorldOnly)]
    fn living_heal(&mut self, living: LivingRef, amount: f32) -> bool;
    /// Side: server.
    /// `LivingEntity.randomTeleport(x, y, z, true, avoid)`: the point `at`
    /// is clamped into the world border and drops to the first block in
    /// `minecraft:entities_can_teleport_to` below it, losing a block of height
    /// per step down. The entity lands there, keeping its motion, unless that
    /// ground or a block in its box at the landing is in the `avoid` block
    /// tag, the box collides or holds a liquid, or the species refuses the
    /// spot (an enderman never lands on ground holding water). A landing
    /// stops a mob's navigation and broadcasts entity event 46 (the portal
    /// particles). Nothing else is played: a chorus fruit's sound and an
    /// enderman's own teleport effects belong to their callers.
    ///
    /// True when the entity moved. False when no spot was found, for a
    /// non-finite point, a fake player, a player in another level, an armor
    /// stand or the dragon, a stale handle, or on a client.
    #[access(WorldOnly)]
    fn living_random_teleport(
        &mut self,
        living: LivingRef,
        at: [f64; 3],
        avoid: Option<&str>,
    ) -> bool;
    /// Side: server.
    /// Adds an attribute modifier (`AttributeInstance.addTransientModifier`
    /// or `addPermanentModifier`). A permanent one is saved with the entity;
    /// a transient one lasts until it unloads. It goes where `/attribute`
    /// writes (`Game::write_attribute`), so gameplay and the entity's clients
    /// see it. False when the entity already has a modifier with that id on
    /// the attribute (vanilla throws), for an attribute the entity type does
    /// not have, a non-finite amount, the dragon (whose attributes nothing
    /// reads yet), a stale handle, or on a client.
    #[access(WorldOnly)]
    fn living_add_attribute_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        modifier: AttributeModifier,
        lifetime: ModifierLifetime,
    ) -> bool;
    /// Side: server.
    /// Adds an attribute modifier in place of the one with the same id, if
    /// any (`addOrUpdateTransientModifier`, `addOrReplacePermanentModifier`):
    /// the new one's lifetime wins. False as `add_attribute_modifier` is,
    /// except that a taken id is replaced.
    #[access(WorldOnly)]
    fn living_replace_attribute_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        modifier: AttributeModifier,
        lifetime: ModifierLifetime,
    ) -> bool;
    /// Side: server.
    /// Removes the modifier with id `id` from the attribute
    /// (`AttributeInstance.removeModifier`), whatever its lifetime. False when
    /// there was none, and in the cases `add_attribute_modifier` is.
    #[access(WorldOnly)]
    fn living_remove_attribute_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        id: &Identifier,
    ) -> bool;
    /// Side: server.
    /// Sets the attribute's base value (`AttributeInstance.setBaseValue`);
    /// the value stays within the attribute's range. False for a non-finite
    /// base, and in the cases `add_attribute_modifier` is.
    #[access(WorldOnly)]
    fn living_set_attribute_base(&mut self, living: LivingRef, attribute: &str, base: f64) -> bool;
    /// Side: server.
    /// Puts `stack` in `slot` (`LivingEntity.setItemSlot`); None, air or a
    /// zero count empties it. Players, guests, mobs, armor stands and the
    /// dragon store all eight slots. An armor stand stores body and saddle
    /// too: `ArmorStand.canUseSlot` only stops interaction, and
    /// `LivingEntity.setItemSlot` keeps whatever it is given. False means
    /// unsupported or stale.
    /// This is a direct write: it takes effect and is final immediately, even
    /// inside `host.transaction`. A mob's write makes any equip/unequip event
    /// and equip sound immediately; a stand's or the dragon's makes neither. API 382.
    #[access(WorldOnly)]
    fn living_set_equipment(
        &mut self,
        living: LivingRef,
        slot: EquipmentSlot,
        stack: Option<ItemStack>,
    ) -> Result<bool, StackWriteError>;
    /// Side: server.
    /// Puts `stack` in hand `hand` (`LivingEntity.setItemInHand`): the main
    /// hand's slot or the off hand's, written as `set_equipment`
    /// writes them, with its equip events and sound. None, air or a zero count
    /// empties the hand. False means unsupported or stale.
    #[access(WorldOnly)]
    fn living_set_hand_item(
        &mut self,
        living: LivingRef,
        hand: Hand,
        stack: Option<ItemStack>,
    ) -> Result<bool, StackWriteError>;
    /// Side: server.
    /// Damage an equipped stack without an equip/unequip notification.
    #[access(WorldOnly)]
    fn living_hurt_and_break(
        &mut self,
        living: LivingRef,
        slot: EquipmentSlot,
        amount: i32,
    ) -> Option<ItemStack>;
    /// Side: server.
    /// Throws `stack`, which the mod holds, from the eyes (`LivingEntity.drop`,
    /// `ServerPlayer.drop`), never through a slot: along the view with a
    /// main-hand swing, or with `throw_randomly` scattered with none, as a
    /// death's drops. `include_thrower_name` sets the thrower, and a player's
    /// throw along the view then counts in its stats and fires `item_toss`.
    /// None for an empty stack, a stand or the dragon, a stale handle, or an
    /// item a mod kept out. Inside a transaction its join waits for the root commit, so a mod
    /// keeping the item out is not known at the call. API @API:living_refs@.
    #[access(WorldOnly)]
    fn living_drop(
        &mut self,
        living: LivingRef,
        stack: ItemStack,
        throw_randomly: bool,
        include_thrower_name: bool,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// `LivingEntity.swing(hand, DEFAULT, sendToSwingingEntity)`: starts the
    /// arm swing (shortened by haste, lengthened by mining fatigue) unless
    /// one is under way and past its first half, and tells the players
    /// around at the end of the tick, and the swinger too when
    /// `send_to_self`. True when a swing started. Mobs and players; false
    /// for armor stands and the dragon, and for a player other than the
    /// acting one while another player acts.
    #[access(WorldOnly)]
    fn living_swing(&mut self, living: LivingRef, hand: Hand, send_to_self: bool) -> bool;
    /// Side: server.
    /// `LivingEntity.getArrowCount`: the arrows stuck in a player or a mob.
    /// None for armor stands and the dragon, which Horizon keeps no count
    /// for, and for a stale handle.
    #[access(WorldOnly)]
    fn living_arrow_count(&mut self, living: LivingRef) -> Option<i32>;
    /// Side: server.
    /// `LivingEntity.setArrowCount`: players around see the arrows on a
    /// player or a mob (vanilla does not limit the count). False for armor
    /// stands and the dragon, and for a stale handle.
    #[access(WorldOnly)]
    fn living_set_arrow_count(&mut self, living: LivingRef, count: i32) -> bool;
    /// Side: server.
    /// `LivingEntity.getStingerCount`: the bee stingers stuck in a player or
    /// a mob. None for armor stands and the dragon, and for a stale handle.
    #[access(WorldOnly)]
    fn living_stinger_count(&mut self, living: LivingRef) -> Option<i32>;
    /// Side: server.
    /// `LivingEntity.setStingerCount`: players around see the stingers on a
    /// player or a mob. False for armor stands and the dragon, and for a
    /// stale handle.
    #[access(WorldOnly)]
    fn living_set_stinger_count(&mut self, living: LivingRef, count: i32) -> bool;
    /// Side: server.
    /// `LivingEntity.removeArrowTime`: the ticks until the next arrow falls
    /// out of a player or a mob; at 0 or less it restarts at
    /// `20 * (30 - arrows)`. False for armor stands and the dragon, and for a
    /// stale handle.
    #[access(WorldOnly)]
    fn living_set_arrow_removal_ticks(&mut self, living: LivingRef, ticks: i32) -> bool;
    /// Side: server.
    /// `LivingEntity.removeStingerTime`, as `set_arrow_removal_ticks` for the
    /// stingers.
    #[access(WorldOnly)]
    fn living_set_stinger_removal_ticks(&mut self, living: LivingRef, ticks: i32) -> bool;
    /// Side: server.
    /// Sets `LivingEntity.useItemRemaining` of the item a player uses (the
    /// way a mod's tick moves the end of the use): at 1 the use completes on
    /// the next tick, at 0 or less it runs until the player lets go, as in
    /// vanilla. It moves the deadline the use-item Tick hook moves, so only
    /// the uses that count it (food, drink, mod items) follow it: shield, bow,
    /// trident, spear, instrument and bundle uses ignore it, and a call from
    /// inside the Tick hook itself is overwritten by that hook's result.
    ///
    /// A mob's use never runs out, so the write moves its use counter to
    /// where `ticks` are left: a spear's stabs, a bow's draw and the use
    /// reads follow it on the next ticks, and a time past the item's use
    /// duration is as long as it. False when the entity uses nothing (a
    /// piglin admiring gold and a drinking witch use none), for armor stands
    /// and the dragon, and for a stale handle.
    #[access(WorldOnly)]
    fn living_set_item_use_remaining(&mut self, living: LivingRef, ticks: i32) -> bool;
    /// Side: server.
    /// `LivingEntity.startUsingItem`: starts using the item in `hand`, for its
    /// `getUseDuration` as a mod's use-item Start hook left it, without the
    /// item's own `use` (no ammo check, no sound). True when the use started:
    /// false for an empty hand, a use under way, a refused Start, armor stands
    /// and the dragon, and a player other than the acting one while another
    /// player acts.
    ///
    /// A player's client lets go of a use whose button isn't down, as in
    /// vanilla (`Minecraft.handleKeybinds`), so that use lasts until its next
    /// tick, which releases it; an item with no use of its own ends then too.
    /// A mob's use shows and its goals see it, but it never runs out: only
    /// `release_using_item` or a goal ends it. False for a piglin with
    /// something in its off hand and a drinking witch, whose counter is their
    /// own timer.
    #[access(WorldOnly)]
    fn living_start_using_item(&mut self, living: LivingRef, hand: Hand) -> bool;
    /// Side: server.
    /// `LivingEntity.releaseUsingItem`: lets go of the item in use as the use
    /// button does: the use-item Stop hook, the item's own release (a player's
    /// bow or trident shoots by how long it was held) and a mod's
    /// `release_using`, then the use is over. True when a use was released:
    /// false when none runs, for armor stands and the dragon, and for a player
    /// other than the acting one while another player acts. A mob's bow or
    /// trident has no release of its own, as in vanilla.
    #[access(WorldOnly)]
    fn living_release_using_item(&mut self, living: LivingRef) -> bool;
    /// Side: server.
    /// Sets the style of the entity's locator bar waypoint
    /// (`Waypoint.Icon.style`, a `waypoint_style` asset id such as
    /// `minecraft:default`), as `/waypoint modify <entity> style` does: the
    /// waypoint is untracked, changed and tracked again, so the players it
    /// reaches get the new icon. Only a player's waypoint reaches other
    /// players (`waypoint_transmit_range` is 0 for every other entity); a
    /// mob's or an armor stand's style is kept for its waypoint. False for
    /// the dragon, a stale handle and a client.
    #[access(WorldOnly)]
    fn living_set_waypoint_style(&mut self, living: LivingRef, style: &Identifier) -> bool;
    /// Side: server.
    /// Sets the colour of the entity's locator bar waypoint
    /// (`Waypoint.Icon.color`) to `rgb` (`0xRRGGBB`; the bits above are
    /// dropped), or back to none, which shows a player in their team's
    /// colour. Reaches players as `set_waypoint_style` does. False for the
    /// dragon, a stale handle and a client.
    #[access(WorldOnly)]
    fn living_set_waypoint_color(&mut self, living: LivingRef, rgb: Option<u32>) -> bool;
    /// Side: server.
    /// `LivingEntity.startAutoSpinAttack(ticks, damage)`: the player spins
    /// like a Riptide launch for `ticks` ticks, hitting the first mob its box
    /// touches for `damage` (the damage Riptide uses is 8), and the players
    /// around see the spin. It moves the player not at all: a Riptide launch
    /// adds its own velocity. A `ticks` of 0 or less ends a spin under way
    /// and clears its damage. False for a mob, an armor stand and the dragon
    /// (Horizon runs no spin for them), a dead player, a non-finite `damage`,
    /// a stale handle and a client.
    #[access(WorldOnly)]
    fn living_set_riptiding(&mut self, living: LivingRef, ticks: i32, damage: f32) -> bool;
}
