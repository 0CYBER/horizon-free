//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `mobs` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait MobsHost {
    /// Side: both.
    /// The entity as a vanilla Mob: a mob or the ender dragon. Players, armor
    /// stands, stale handles and clients are rejected. Mobs taken out for
    /// ticking are unavailable.
    #[access(WorldOnly)]
    fn mob(&self, entity: EntityRef) -> Option<MobRef>;
}
/// The server host's methods of the `mobs` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait MobsServerHost {
    /// Side: server.
    /// Kills mob `id` as `/kill` does: it falls and drops its loot. False for
    /// no such living mob or another dimension.
    #[access(WorldOnly)]
    fn kill_mob(&mut self, dimension: &str, id: u32) -> bool;
    /// Side: server.
    /// The entity as the ender dragon, or None for another kind, a stale
    /// handle or a client.
    #[access(WorldOnly)]
    fn dragon(&self, entity: EntityRef) -> Option<DragonRef>;
}
/// The handles' reads of the `mobs` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait MobsBackend {
    /// Side: server.
    /// `EnderDragonPhaseManager.getCurrentPhase`.
    #[access(WorldOnly)]
    fn dragon_phase(&self, dragon: DragonRef) -> Option<DragonPhase>;
    /// Side: server.
    /// `EnderDragonFight.aliveCrystals`: the crystals the last scan found
    /// (every 100 ticks). None for a stale handle, a client, or a dragon
    /// outside the End or whose fight is not loaded.
    #[access(WorldOnly)]
    fn dragon_alive_crystals(&self, dragon: DragonRef) -> Option<i32>;
    /// Side: server.
    /// `EnderDragonFight.hasPreviouslyKilledDragon`. None as
    /// `alive_crystals` is.
    #[access(WorldOnly)]
    fn dragon_previously_killed(&self, dragon: DragonRef) -> Option<bool>;
    /// Side: server.
    /// `GoalSelector.getRunningGoals` read wider: every goal of the goal
    /// selector as a `MobGoalEntry` (the priority, the kind id and
    /// whether it runs), in registration order. None for a stale or
    /// unavailable mob, a client, and the dragon.
    #[access(WorldOnly)]
    fn mob_goals(&self, mob: MobRef) -> Option<Vec<MobGoalEntry>>;
    /// Side: server.
    /// Every goal of the target selector, as `goals` reads the goal
    /// selector's.
    #[access(WorldOnly)]
    fn mob_target_goals(&self, mob: MobRef) -> Option<Vec<MobGoalEntry>>;
    /// Side: server.
    /// `PathNavigation.isDone`; None for a stale or unavailable mob, or on a
    /// client, and for the dragon (no navigation).
    #[access(WorldOnly)]
    fn mob_is_done(&self, mob: MobRef) -> Option<bool>;
    /// Side: server.
    /// `PathNavigation.getSpeedModifier`: the multiplier the current path
    /// is followed at (1.0 is the mob's normal speed; 0 until a path sets
    /// one). This is not the mob's movement-speed attribute. None for a
    /// stale or unavailable mob, or on a client, and for the dragon.
    #[access(WorldOnly)]
    fn mob_navigation_speed(&self, mob: MobRef) -> Option<f64>;
    /// Side: server.
    /// `MoveControl.getWantedX/Y/Z`: the stored wish, even after consumption.
    /// None for a stale mob, a client, the dragon or a mob held out of the level.
    #[access(WorldOnly)]
    fn mob_wanted_position(&self, mob: MobRef) -> Option<[f64; 3]>;
    /// Side: server.
    /// `MoveControl.getSpeedModifier`: the stored move-control speed.
    /// None for a stale mob, a client, the dragon or a mob held out of the level.
    #[access(WorldOnly)]
    fn mob_speed_modifier(&self, mob: MobRef) -> Option<f64>;

    /// Side: server.
    /// `Mob.getPathfindingMalus`: what a node of `kind` costs this mob's
    /// pathfinder — its own override, else the kind's default (a negative
    /// malus closes the kind). None for a stale or unavailable mob, the
    /// dragon, or on a client.
    #[access(WorldOnly)]
    fn mob_pathfinding_malus(&self, mob: MobRef, kind: PathType) -> Option<f32>;
    /// Side: both.
    /// The existing portal decision for an available mob in the current level.
    #[access(WorldOnly)]
    fn mob_can_use_portal(&self, mob: MobRef) -> Option<bool>;
    /// Side: server.
    /// `Mob.getTarget`. Brain-backed mobs read their attack memory; piglins
    /// and hoglins use Horizon's `ai.target` stand-in. Invalid targets, an
    /// unavailable mob or player binding, or a target without a living handle
    /// read as None. The dragon reads what a mod set, while it stays a valid
    /// target (`EnderDragon.canAttack`); its flight never sets one.
    #[access(WorldOnly)]
    fn mob_target(&self, mob: MobRef) -> Option<LivingRef>;
    /// Side: server.
    /// `Mob.isAggressive`, the synced mob flag (the dragon's reaches its
    /// clients as `DATA_MOB_FLAGS_ID`).
    #[access(WorldOnly)]
    fn mob_is_aggressive(&self, mob: MobRef) -> Option<bool>;
    /// Side: server.
    /// `Mob.isPersistenceRequired`. The dragon never despawns
    /// (`EnderDragon.checkDespawn` is empty), but keeps and saves the flag
    /// (`PersistenceRequired`).
    #[access(WorldOnly)]
    fn mob_persistence_required(&self, mob: MobRef) -> Option<bool>;
    /// Side: server.
    /// `Mob.requiresCustomPersistence` and its species overrides: passengers,
    /// leashed mobs and species-specific reasons to stay, such as an enderman
    /// carrying a block. Independent of `PersistenceRequired` and `NoAI`.
    /// None for an unavailable mob or the separately modeled dragon.
    #[access(WorldOnly)]
    fn mob_requires_custom_persistence(&self, mob: MobRef) -> Option<bool>;
    /// Side: server.
    /// `Mob.isNoAi`: the mob stands where it is and its goals do not run,
    /// while what NoAI keeps running in vanilla (the daylight burn of the
    /// undead, a horse's tail) still does. Saved with it as `NoAI`. None
    /// for anything but a mob.
    #[access(WorldOnly)]
    fn mob_no_ai(&self, mob: MobRef) -> Option<bool>;
    /// Side: server.
    /// `Mob.getRestrictCenter`: the restriction's block position, kept
    /// after `clear_restriction` and after a load (`[0, 0, 0]` for a mob
    /// that never had one, vanilla's `ZERO`). None for a stale or
    /// unavailable mob, a client, and the dragon.
    #[access(WorldOnly)]
    fn mob_restrict_center(&self, mob: MobRef) -> Option<[i32; 3]>;
    /// Side: server.
    /// `Mob.getRestrictRadius`: `-1` while the mob has no restriction.
    /// None for a stale or unavailable mob, a client, and the dragon.
    #[access(WorldOnly)]
    fn mob_restrict_radius(&self, mob: MobRef) -> Option<i32>;
    /// Side: server.
    /// `Mob.hasRestriction`. None for a stale or unavailable mob, a client,
    /// and the dragon.
    #[access(WorldOnly)]
    fn mob_has_restriction(&self, mob: MobRef) -> Option<bool>;
    /// Side: server.
    /// `Mob.isWithinRestriction(pos)`: whether the block position is inside
    /// the home area; a mob without a restriction is within everywhere, as
    /// in vanilla. None for a stale or unavailable mob, a client, and the
    /// dragon.
    #[access(WorldOnly)]
    fn mob_is_within_restriction(&self, mob: MobRef, at: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// `Mob.isLeftHanded`: whether the mob holds things in its left hand
    /// (the mob flags in its entity data). None for anything but a mob.
    #[access(WorldOnly)]
    fn mob_left_handed(&self, mob: MobRef) -> Option<bool>;
    /// Side: server.
    /// `Mob.canPickUpLoot`, with the overrides of an allay (holding
    /// something) and a sulfur cube (with no block). None for anything but
    /// a mob.
    #[access(WorldOnly)]
    fn mob_can_pick_up_loot(&self, mob: MobRef) -> Option<bool>;
    /// Side: server.
    /// `LivingEntity.getNoActionTime`: the ticks since the mob last had a
    /// reason to stay (a player near, a hit). Despawning and a raid's
    /// recruiting read it. None for a stale or unavailable mob, the dragon,
    /// or on a client.
    #[access(WorldOnly)]
    fn mob_no_action_time(&self, mob: MobRef) -> Option<i32>;
    /// Side: server.
    /// `Raider.getCurrentRaid`: the raid a raider belongs to, even a stopped
    /// one. None for a mob that is no raider or in no raid.
    #[access(WorldOnly)]
    fn mob_raid(&self, mob: MobRef) -> Option<RaidRef>;
    /// Side: server.
    /// The unspawned mob's entity type (`minecraft:zombie`). `Expired` once the hook that lent it has returned.
    #[access(WorldOnly)]
    fn unspawned_mob_kind(&self, mob: UnspawnedMobRef) -> Result<Identifier, UnspawnedMobError>;
    /// Side: server.
    /// Where the unspawned mob will join the level. `Expired` once the hook that lent it has returned.
    #[access(WorldOnly)]
    fn unspawned_mob_position(&self, mob: UnspawnedMobRef) -> Result<[f64; 3], UnspawnedMobError>;
    /// Side: server.
    /// The stack in `slot` (`LivingEntity.getItemBySlot`) and its count; `None` for an empty slot. `Expired` once the hook that lent it has returned.
    #[access(WorldOnly)]
    fn unspawned_mob_equipment(
        &self,
        mob: UnspawnedMobRef,
        slot: EquipmentSlot,
    ) -> Result<Option<ItemStack>, UnspawnedMobError>;
    /// Side: server.
    /// An attribute's base value, before modifiers; `Unsupported` for one the mob's type does not have. `Expired` once the hook that lent it has returned.
    #[access(WorldOnly)]
    fn unspawned_mob_attribute_base(
        &self,
        mob: UnspawnedMobRef,
        attribute: &Identifier,
    ) -> Result<f64, UnspawnedMobError>;
    /// Side: server.
    /// An attribute's value with its modifiers, its equipment's included; `Unsupported` for one the mob's type does not have. `Expired` once the hook that lent it has returned.
    #[access(WorldOnly)]
    fn unspawned_mob_attribute_value(
        &self,
        mob: UnspawnedMobRef,
        attribute: &Identifier,
    ) -> Result<f64, UnspawnedMobError>;
    /// Side: server.
    /// The tags a mod gave the unspawned mob so far (`Entity.getTags`). `Expired` once the hook that lent it has returned.
    #[access(WorldOnly)]
    fn unspawned_mob_tags(&self, mob: UnspawnedMobRef) -> Result<Vec<String>, UnspawnedMobError>;
    /// Side: server.
    /// The unspawned mob's custom name, if it has one. `Expired` once the hook that lent it has returned.
    #[access(WorldOnly)]
    fn unspawned_mob_custom_name(
        &self,
        mob: UnspawnedMobRef,
    ) -> Result<Option<Text>, UnspawnedMobError>;
}
/// The handles' writes of the `mobs` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait MobsBackendMut {
    /// Side: server.
    /// `EnderDragonPhaseManager.setPhase`: the dragon ends its phase and
    /// begins the new one fresh (targets and counters cleared), and the
    /// clients get the synced `DATA_PHASE`. Setting the phase it holds does
    /// nothing, as in vanilla (a strafing dragon keeps its player). No
    /// random draw. False for a stale handle or a client. The new phase
    /// runs from the dragon's next tick.
    #[access(WorldOnly)]
    fn dragon_set_phase(&mut self, dragon: DragonRef, phase: DragonPhase) -> bool;
    /// Side: server.
    /// `GoalSelector.addGoal(priority, goal)` on a mob's goal selector.
    /// Refuses a stale or unavailable mob, a client, and the dragon (it
    /// keeps no selector); a goal the mob's vanilla class does not hold (a
    /// `PathfinderMob` goal for a bat, ghast or phantom, `BreedGoal` or
    /// `FollowParentGoal` for a mob that is no animal); and non-finite
    /// numbers, negative speeds and distances, a probability outside
    /// 0..=1, a negative interval, and an unknown species id (in `prey`,
    /// `what` or `only`) or empty `items`. The target selector's kinds
    /// (`nearest_target`, `hurt_by_target`) go in through
    /// `add_target_goal`; both selectors take them, as vanilla's two
    /// selectors do.
    #[access(WorldOnly)]
    fn mob_add_goal(&mut self, mob: MobRef, priority: i32, goal: &GoalSpec) -> bool;
    /// Side: server.
    /// `GoalSelector.addGoal` on the mob's target selector. The same rules
    /// as `add_goal`.
    #[access(WorldOnly)]
    fn mob_add_target_goal(&mut self, mob: MobRef, priority: i32, goal: &GoalSpec) -> bool;
    /// Side: server.
    /// `GoalSelector.removeGoal` on the goal selector, by kind: every goal
    /// whose id, as `goals` reads it, is `id` goes (`removeAllGoals`), a
    /// running one stopped first. How many went; None for a stale or
    /// unavailable mob, a client, and the dragon. Nothing in a mob's tick
    /// resolves, as its copy is held out there: add what a mod needs in
    /// `entity_join`, which fires for a loaded chunk's mobs too.
    #[access(WorldOnly)]
    fn mob_remove_goal(&mut self, mob: MobRef, id: &str) -> Option<usize>;
    /// Side: server.
    /// `GoalSelector.removeGoal` on the target selector, by kind, as
    /// `remove_goal` does on the goal selector.
    #[access(WorldOnly)]
    fn mob_remove_target_goal(&mut self, mob: MobRef, id: &str) -> Option<usize>;
    /// Side: server.
    /// `PathNavigation.moveTo(x, y, z, speed)`: true if a path was started.
    /// Refuses a stale or unavailable mob, a client, non-finite arguments,
    /// and coordinates too large for safe block arithmetic. As in vanilla,
    /// the mob's own goals may replace the path next tick. False for the
    /// dragon, which flies by its phase and keeps no navigation.
    #[access(WorldOnly)]
    fn mob_move_to(&mut self, mob: MobRef, at: [f64; 3], speed: f64) -> bool;
    /// Side: server.
    /// `PathNavigation.stop`. False for a stale or unavailable mob, or on a
    /// client, and for the dragon (no navigation).
    #[access(WorldOnly)]
    fn mob_stop(&mut self, mob: MobRef) -> bool;
    /// Side: server.
    /// `PathNavigation.setSpeedModifier`: sets the multiplier the current
    /// path is followed at, as `move_to`'s speed argument does. The mob's
    /// own AI — a goal's or a brain species' next `moveTo` — sets its own
    /// again, as in vanilla. False for a non-finite speed, a stale or
    /// unavailable mob, or on a client, and for the dragon.
    #[access(WorldOnly)]
    fn mob_set_navigation_speed(&mut self, mob: MobRef, speed: f64) -> bool;
    /// Side: server.
    /// `Mob.setPathfindingMalus`: what the mob's next paths cost through
    /// `kind` (negative closes it, as the species' own overrides do); not
    /// saved, as in vanilla, so a reload or conversion keeps the species'
    /// own. False for a malus that is not finite, a stale or unavailable mob,
    /// the dragon, or on a client.
    #[access(WorldOnly)]
    fn mob_set_pathfinding_malus(&mut self, mob: MobRef, kind: PathType, malus: f32) -> bool;
    /// Side: server.
    /// `Mob.setTarget`; None clears it. Only mobs and players whose binding
    /// can be stored in the current level are supported; armor stands are
    /// refused. The dragon stores a mob or a player, which its flight ignores
    /// and its save does not keep (vanilla keeps no target either). Brain-backed mobs refuse writes: vanilla's setter
    /// does not update their brain memory. A fox, an enderman or a
    /// zombified piglin stores the target with its own `setTarget` side
    /// effects (`Mob::set_target`).
    /// Invalid targets are refused (false, mob unchanged); vanilla instead
    /// stores `asValidTarget(target)`, clearing the target when it is invalid.
    /// False also means a stale handle, client, or an unavailable mob/player.
    #[access(WorldOnly)]
    fn mob_set_target(&mut self, mob: MobRef, target: Option<LivingRef>) -> bool;
    /// Side: server.
    /// `Mob.setAggressive`. False for an unavailable mob.
    #[access(WorldOnly)]
    fn mob_set_aggressive(&mut self, mob: MobRef, value: bool) -> bool;
    /// Side: server.
    /// `Mob.setPersistenceRequired`, which sets the flag and cannot clear it.
    /// False for an unavailable mob.
    #[access(WorldOnly)]
    fn mob_set_persistence_required(&mut self, mob: MobRef) -> bool;
    /// Side: server.
    /// `Mob.setNoAi`: whether the mob's own AI runs. With no AI the mob
    /// stands (what NoAI keeps in vanilla still runs), and the flag is
    /// saved with it as `NoAI`. False for anything but a mob.
    #[access(WorldOnly)]
    fn mob_set_no_ai(&mut self, mob: MobRef, no_ai: bool) -> bool;
    /// Side: server.
    /// `Mob.setBaby` through the species' setter: ageable mobs use their
    /// baby start age (`-48000` for sniffers, otherwise `-24000`) or `0`,
    /// including age-boundary attribute and AI setup changes. Non-ageable
    /// babies, such as zombies, keep their non-growing baby state instead.
    /// Leaves `NoAI` and `PersistenceRequired` unchanged. False, with no
    /// changes, for an unavailable mob or a species with no baby form.
    #[access(WorldOnly)]
    fn mob_set_baby(&mut self, mob: MobRef, baby: bool) -> bool;
    /// Side: server.
    /// `LookControl.setLookAt`, by the mob's `lookAt` path: the wanted
    /// position its look control rotates the head toward over the next
    /// ticks. As `move_to`'s path, the mob's own AI — a goal's look, or a
    /// brain species' `LookAtTargetSink`, which writes through this same
    /// control — may issue its own and replace this one; no species refuses.
    /// A creaking being looked at, an angry bee and an axolotl playing dead
    /// hold their look control, as in vanilla: the wanted position waits
    /// until it runs again. False for non-finite coordinates, a stale or
    /// unavailable mob, a client, and the dragon.
    #[access(WorldOnly)]
    fn mob_look_at(&mut self, mob: MobRef, at: [f64; 3]) -> bool;
    /// Side: server.
    /// `MoveControl.setWantedPosition`: a momentary wish consumed by the mob's
    /// own movement. Its AI may replace it next tick. Unloaded positions and
    /// finite negative speeds are accepted. Non-finite inputs return named
    /// errors without changing the wish. Ok(false) for a stale mob, a client,
    /// the dragon or a mob held out of the level.
    #[access(WorldOnly)]
    fn mob_set_wanted_position(
        &mut self,
        mob: MobRef,
        at: [f64; 3],
        speed: f64,
    ) -> Result<bool, WantedPositionError>;

    /// Side: server.
    /// `JumpControl.jump`: the mob jumps on its next own tick by its own
    /// jump path — no velocity is written, and the control consumes the
    /// wish on that tick (a rabbit's jump control reads it too). The mob's
    /// own wishes (a slime's hop, a swimming brain's) are the same flag; no
    /// species refuses. A creaking being looked at holds its jump control,
    /// as in vanilla: the wish waits until it runs again. False for a stale
    /// or unavailable mob, a client, and the dragon.
    #[access(WorldOnly)]
    fn mob_jump(&mut self, mob: MobRef) -> bool;
    /// Side: server.
    /// `Mob.restrictTo(pos, radius)`: the home area the mob keeps to — its
    /// own stroll and wander goals pick their positions inside it, as a
    /// lead's and a guardian's home do. Radius `-1` clears the restriction
    /// while keeping `pos` as the center, `clear_restriction`'s shape. The
    /// area is saved with the mob (`home_pos`, `home_radius`). False for a
    /// stale or unavailable mob, a client, and the dragon.
    #[access(WorldOnly)]
    fn mob_restrict_to(&mut self, mob: MobRef, at: [i32; 3], radius: i32) -> bool;
    /// Side: server.
    /// `Mob.clearRestriction`: the restriction goes, the center stays (its
    /// own goals read no restriction, as vanilla's `-1` radius). False for
    /// a stale or unavailable mob, a client, and the dragon.
    #[access(WorldOnly)]
    fn mob_clear_restriction(&mut self, mob: MobRef) -> bool;
    /// Side: server.
    /// `Mob.setLeftHanded`: sets the mob flag the players around get with
    /// the mob's next entity data and see in its swing and held items.
    /// Saved with the mob as `LeftHanded`. False for anything but a mob.
    #[access(WorldOnly)]
    fn mob_set_left_handed(&mut self, mob: MobRef, left_handed: bool) -> bool;
    /// Side: server.
    /// `Mob.setCanPickUpLoot`: whether the mob picks up the items it sees
    /// (saved with it as `CanPickUpLoot`; clients are not told, as in
    /// vanilla). An allay and a sulfur cube decide for themselves. False
    /// for anything but a mob.
    #[access(WorldOnly)]
    fn mob_set_can_pick_up_loot(&mut self, mob: MobRef, can_pick_up: bool) -> bool;
    /// Side: server.
    /// `LivingEntity.setNoActionTime`: the count the mob's own tick carries
    /// on from (a far-away mob's despawn chance starts past 600). Not saved,
    /// as in vanilla. False for a stale or unavailable mob, the dragon, or
    /// on a client.
    #[access(WorldOnly)]
    fn mob_set_no_action_time(&mut self, mob: MobRef, ticks: i32) -> bool;
    /// Side: server.
    /// `Brain.setMemory(ATTACK_TARGET, target)`, or `eraseMemory` for None:
    /// the target a brain-driven mob — an axolotl, a breeze, a creaking, a
    /// hoglin, a piglin or piglin brute, a warden or a zoglin — reads instead
    /// of `Mob.setTarget`'s. A plain, non-expiring memory write: the
    /// brain's own behaviours act on it from the next tick (a target the
    /// species may not attack is dropped there, as vanilla's
    /// `StopAttackingIfTargetInvalid` drops it), and none of a species' own
    /// setter effects run — a warden's `setAttackTarget` roar, reach and
    /// sonic-boom bookkeeping stays with its own sensing. A frog refuses: its
    /// target belongs to the running tongue attack, which starts from the
    /// nearest attackable, so a written one would not be pursued.
    /// False for a fake player, a player in another level, a mob without an
    /// attack-target brain (`MobRef::set_target` writes those), a stale or
    /// unavailable mob, the dragon, or on a client.
    #[access(WorldOnly)]
    fn mob_set_brain_target(&mut self, mob: MobRef, target: Option<LivingRef>) -> bool;
}
