//! Mob handles for native server mods.
use super::{Game, commands::Target};
use horizon_client::mobs::{self, Mob, Species, Who, ai::brain_target};
use mod_api::{EntityRef, LivingRef, MobRef};

/// What decides whether a player can be a mob's target.
pub(super) struct PlayerFacts {
    pub mode: i32,
    pub alive: bool,
    pub invulnerable: bool,
    pub uuid: [i32; 4],
}

impl Game<'_> {
    /// Restore a saved anger target after insertion, so the target-change hook
    /// receives a live actor and a currently resolvable living target. A
    /// target that is absent or invalid resolves to none, as EntityReference
    /// does while vanilla loads a neutral mob. Returns false only when the
    /// actor cannot yet be represented by a native handle.
    pub(in crate::game) fn native_restore_loaded_target(
        &mut self,
        id: u32,
        uuid: Option<[i32; 4]>,
    ) -> bool {
        let Some(mob) = self.entity(id).and_then(|entity| self.mob(entity)) else {
            return false;
        };
        let target = uuid.and_then(|words| {
            let uuid = horizon_client::lan::playerdata::ints_uuid(words);
            let entity = self
                .player(uuid)
                .map(|player| player.entity())
                .or_else(|| {
                    let target = self
                        .persist
                        .uuids
                        .iter()
                        .find_map(|(&id, found)| (*found == words).then_some(id))?;
                    self.entity(target)
                })?;
            self.living(entity)
        });
        let target = target.filter(|target| {
            let Some(kind) = self.native_entity_target(target.entity()) else {
                return false;
            };
            self.mobs
                .iter()
                .find(|state| state.id == id)
                .is_some_and(|state| self.native_mob_valid_target(state, kind))
        });
        // Cancellation leaves the newly loaded mob's target unchanged.
        let _ = self.mob_set_target(mob, target);
        true
    }

    /// A player target's game mode, liveness, invulnerability (the ability,
    /// the `Invulnerable` flag or its window) and uuid.
    pub(super) fn native_player_facts(&self, target: Target) -> Option<PlayerFacts> {
        match target {
            Target::Guest(conn) => {
                let g = self.command_guest(conn)?;
                if g.is_fake() {
                    return None;
                }
                Some(PlayerFacts {
                    mode: g.game_mode,
                    alive: !g.dead && g.health > 0.0,
                    invulnerable: g.motion.as_ref().is_some_and(|p| {
                        p.invulnerable
                            || p.health.permanently_invulnerable
                            || p.health.invulnerable_time > 0
                    }),
                    uuid: horizon_client::lan::playerdata::uuid_ints(g.uuid),
                })
            }
            Target::Player => {
                if self.lan_acting_fake {
                    return None;
                }
                let player = self.player.as_ref()?;
                Some(PlayerFacts {
                    mode: self.mode_id(),
                    alive: !player.health.dead_or_dying(),
                    invulnerable: player.invulnerable
                        || player.health.permanently_invulnerable
                        || player.health.invulnerable_time > 0,
                    uuid: self
                        .lan_acting_uuid
                        .map(horizon_client::lan::playerdata::uuid_ints)
                        .or(self.level.player_uuid)
                        .unwrap_or_default(),
                })
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum MobFlag {
    Aggressive,
    Persistent,
}

impl Game<'_> {
    /// `Mob.asValidTarget` and the modeled `canAttack` overrides: the game's
    /// own state as the target's facts, through the one rule set
    /// [`Mob::valid_target`] (arismoko/horizon#1175) a goal tick's view shares.
    pub(super) fn native_mob_valid_target(&self, mob: &Mob, target: Target) -> bool {
        if mob.species() == Species::Warden {
            return self.native_warden_valid_target(mob, target);
        }
        match target {
            Target::Mob(id) => {
                let Some(other) = self.mobs.iter().find(|m| m.id == id && !m.removed) else {
                    return false;
                };
                let seen = [super::creatures::seen(
                    other,
                    self.level.player_uuid,
                    self.items(),
                )];
                mob.valid_target(
                    &self.mob_world_base(&seen),
                    &mobs::goals::TargetFacts::Mob {
                        id,
                        species: other.species(),
                        alive: other.alive(),
                        invulnerable: other.invulnerable_flag,
                        baby: other.is_baby(),
                    },
                )
            }
            Target::Player | Target::Guest(_) => {
                let Some(PlayerFacts {
                    mode,
                    alive,
                    invulnerable,
                    uuid,
                }) = self.native_player_facts(target)
                else {
                    return false;
                };
                mob.valid_target(
                    &self.mob_world_base(&[]),
                    &mobs::goals::TargetFacts::Player {
                        alive,
                        invulnerable,
                        // The game modes a target cannot be attacked in:
                        // creative (1) and spectator (3).
                        creative: mode == 1,
                        spectator: mode == 3,
                        uuid,
                    },
                )
            }
            _ => false,
        }
    }
    /// `Warden.canTargetEntity`, which overrides the usual `Mob.canAttack`.
    fn native_warden_valid_target(&self, mob: &Mob, target: Target) -> bool {
        use horizon_core::geometry::Axis;
        let (bounds, team) = match target {
            Target::Mob(id) => {
                let Some(other) = self.mobs.iter().find(|m| m.id == id && !m.removed) else {
                    return false;
                };
                if !other.alive() || other.invulnerable_flag || other.species() == Species::Warden {
                    return false;
                }
                (other.bounds(), self.mob_team(id))
            }
            Target::Guest(conn) => {
                let Some(g) = self.command_guest(conn) else {
                    return false;
                };
                if g.dimension != self.dim.id()
                    || g.dead
                    || g.health <= 0.0
                    || matches!(g.game_mode, 1 | 3)
                    || g.motion.as_ref().is_some_and(|p| p.invulnerable)
                {
                    return false;
                }
                (g.bounds(), self.score.board.players_team(&g.name))
            }
            Target::Player => {
                let Some(player) = &self.player else {
                    return false;
                };
                if matches!(self.mode_id(), 1 | 3)
                    || player.health.dead_or_dying()
                    || player.invulnerable
                {
                    return false;
                }
                (player.bounds(), self.player_team())
            }
            _ => return false,
        };
        let border = self.border.geometry();
        !horizon_client::scoreboard::teams_allied(self.mob_team(mob.id), team)
            && border.is_within_bounds(bounds.min(Axis::X), bounds.min(Axis::Z), 0.0)
            && border.is_within_bounds(
                bounds.max(Axis::X) - f64::from(1.0e-5_f32),
                bounds.max(Axis::Z) - f64::from(1.0e-5_f32),
                0.0,
            )
    }

    /// The entity as a vanilla Mob: a mob or the ender dragon. Players, armor
    /// stands, stale handles and clients are rejected. Mobs taken out for
    /// ticking are unavailable (a arismoko/horizon#29 follow-up).
    pub(super) fn mob(&self, entity: EntityRef) -> Option<MobRef> {
        match self.native_entity_target(entity)? {
            Target::Mob(_) | Target::Dragon(_) => Some(MobRef::new(entity)),
            _ => None,
        }
    }

    /// The existing portal decision for an available mob in the current level.
    pub(super) fn mob_can_use_portal(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        let mob = self.mobs.iter().find(|m| m.id == id)?;
        Some(self.mob_portal_eligible(mob))
    }

    /// `Mob.getTarget`. Brain-backed mobs read their attack memory; piglins
    /// and hoglins use Horizon's `ai.target` stand-in. Invalid targets, an
    /// unavailable mob or player binding, or a target without a living handle
    /// read as None. The dragon reads what a mod set, while it stays a valid
    /// target (`EnderDragon.canAttack`); its flight never sets one.
    pub(super) fn mob_target(&self, mob: MobRef) -> Option<LivingRef> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return self.native_dragon_target(mob.entity());
        };
        let mob = self.mobs.iter().find(|m| m.id == id)?;
        let target = brain_target(mob).unwrap_or(mob.ai.target)?;
        let target_id = match target {
            Who::Mob(id) => id,
            Who::Player => {
                let host = self.lan.as_ref()?;
                let conn = host.mob_players.get(&id)?;
                u32::try_from(host.hub.guest(*conn)?.entity_id).ok()?
            }
        };
        let entity = self.entity(target_id)?;
        self.native_mob_valid_target(mob, self.native_entity_target(entity)?)
            .then(|| self.living(entity))?
    }

    /// `Mob.setTarget`; None clears it. Only mobs and players whose binding
    /// can be stored in the current level are supported; armor stands are
    /// refused. The dragon stores a mob or a player, which its flight ignores
    /// and its save does not keep (vanilla keeps no target either). Brain-backed mobs refuse writes: vanilla's setter
    /// does not update their brain memory (arismoko/horizon#934). A fox, an enderman or a
    /// zombified piglin stores the target with its own `setTarget` side
    /// effects (`Mob::set_target`).
    /// Invalid targets are refused (false, mob unchanged); vanilla instead
    /// stores `asValidTarget(target)`, clearing the target when it is invalid.
    /// False also means a stale handle, client, or an unavailable mob/player.
    pub(super) fn mob_set_target(&mut self, mob: MobRef, target: Option<LivingRef>) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return self.native_dragon_set_target(mob.entity(), target);
        };
        let Some(mob_state) = self.mobs.iter().find(|m| m.id == id) else {
            return false;
        };
        if mob_state.refuses_set_target() {
            return false;
        }
        if let Some(target) = target {
            let Some(target_kind) = self.native_entity_target(target.entity()) else {
                return false;
            };
            if !self.native_mob_valid_target(mob_state, target_kind) {
                return false;
            }
        }
        let Some(target) = self.native_mob_target_event(mob, target, false) else {
            return false;
        };
        if let Some(target) = target {
            let Some(kind) = self.native_entity_target(target.entity()) else {
                return false;
            };
            let Some(state) = self.mobs.iter().find(|m| m.id == id) else {
                return false;
            };
            if !self.native_mob_valid_target(state, kind) {
                return false;
            }
        }
        let Some(who) = self.native_mob_bind_target(id, target) else {
            return false;
        };
        let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        mob.set_target(who);
        true
    }

    /// Dispatch a mod-initiated change while the mob remains in the game.
    pub(super) fn native_mob_target_event(
        &self,
        mob: MobRef,
        original_target: Option<LivingRef>,
        is_brain: bool,
    ) -> Option<Option<LivingRef>> {
        if !mod_api::__private::listened(mod_api::HookId::LivingChangeTarget) {
            return Some(original_target);
        }
        let mut new_target = original_target;
        let mut cancel = false;
        super::native_hooks::NativeHookHost { host: self }.native_living_change_target(
            mod_api::__private::LivingChangeTargetCall {
                mob,
                original_target,
                new_target: &mut new_target,
                is_brain,
                cancel: &mut cancel,
            },
        );
        (!cancel).then_some(new_target)
    }

    /// The `Who` mob `id` stores for `target`, for either target setter: a
    /// mob, or a player in the level in place. A player target binds to the
    /// mob (`mob_players`), so the mob's tick sees that player. None for a
    /// fake player (the root rule for every mob target setter), a player in
    /// another level, or anything else (an armor stand, the dragon, a stale
    /// handle).
    pub(super) fn native_mob_bind_target(
        &mut self,
        id: u32,
        target: Option<LivingRef>,
    ) -> Option<Option<Who>> {
        let (who, player) = match target {
            None => (None, None),
            Some(target) => match self.native_entity_target(target.entity())? {
                Target::Mob(id) => (Some(Who::Mob(id)), None),
                t @ Target::Guest(conn) if self.lan_acting_uuid.is_none() => {
                    self.native_player_facts(t)?;
                    let guest = self.lan.as_ref()?.hub.guest(conn)?;
                    if guest.dimension != self.dim.id() {
                        return None;
                    }
                    (Some(Who::Player), Some(conn))
                }
                t @ Target::Player => {
                    self.native_player_facts(t)?;
                    match self.lan_acting {
                        Some(conn) if self.lan.is_some() => (Some(Who::Player), Some(conn)),
                        _ => return None,
                    }
                }
                _ => return None,
            },
        };
        if let Some(conn) = player {
            self.lan.as_mut()?.mob_players.insert(id, conn);
        }
        Some(who)
    }

    /// `Brain.setMemory(ATTACK_TARGET, target)`, or `eraseMemory` for None:
    /// the target a brain-driven mob — an axolotl, a breeze, a creaking, a
    /// hoglin, a piglin or piglin brute, a warden or a zoglin — reads instead
    /// of `Mob.setTarget`'s (arismoko/horizon#934). A plain, non-expiring memory write: the
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
    pub(super) fn mob_set_brain_target(&mut self, mob: MobRef, target: Option<LivingRef>) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(m) = self.mobs.iter().find(|m| m.id == id && !m.removed) else {
            return false;
        };
        if !horizon_client::mobs::ai::writes_brain_target(m) {
            return false;
        }
        let Some(target) = self.native_mob_target_event(mob, target, true) else {
            return false;
        };
        let Some(who) = self.native_mob_bind_target(id, target) else {
            return false;
        };
        self.mobs
            .iter_mut()
            .find(|m| m.id == id)
            .is_some_and(|m| horizon_client::mobs::ai::set_brain_target(m, who))
    }

    /// The mob flag a handle names: a mob's, or the dragon's own.
    fn native_mob_flag(&self, mob: MobRef, flag: MobFlag) -> Option<bool> {
        match self.native_entity_target(mob.entity())? {
            Target::Mob(id) => {
                let mob = self.mobs.iter().find(|m| m.id == id)?;
                Some(match flag {
                    MobFlag::Aggressive => mob.ai.aggressive,
                    MobFlag::Persistent => mob.persistent,
                })
            }
            Target::Dragon(_) => {
                let dragon = self.native_dragon(mob.entity())?;
                Some(match flag {
                    MobFlag::Aggressive => dragon.aggressive,
                    MobFlag::Persistent => dragon.persistent,
                })
            }
            _ => None,
        }
    }
    fn native_mob_set_flag(&mut self, mob: MobRef, flag: MobFlag, value: bool) -> bool {
        let slot = match self.native_entity_target(mob.entity()) {
            Some(Target::Mob(id)) => {
                self.mobs
                    .iter_mut()
                    .find(|m| m.id == id)
                    .map(|mob| match flag {
                        MobFlag::Aggressive => &mut mob.ai.aggressive,
                        MobFlag::Persistent => &mut mob.persistent,
                    })
            }
            Some(Target::Dragon(_)) => self.native_dragon_mut(mob.entity()).map(|d| match flag {
                MobFlag::Aggressive => &mut d.aggressive,
                MobFlag::Persistent => &mut d.persistent,
            }),
            _ => None,
        };
        slot.map(|slot| *slot = value).is_some()
    }

    /// `Mob.isAggressive`, the synced mob flag (the dragon's reaches its
    /// clients as `DATA_MOB_FLAGS_ID`).
    pub(super) fn mob_is_aggressive(&self, mob: MobRef) -> Option<bool> {
        self.native_mob_flag(mob, MobFlag::Aggressive)
    }

    /// `Mob.setAggressive`. False for an unavailable mob.
    pub(super) fn mob_set_aggressive(&mut self, mob: MobRef, value: bool) -> bool {
        self.native_mob_set_flag(mob, MobFlag::Aggressive, value)
    }

    /// `Mob.isPersistenceRequired`. The dragon never despawns
    /// (`EnderDragon.checkDespawn` is empty), but keeps and saves the flag
    /// (`PersistenceRequired`).
    pub(super) fn mob_persistence_required(&self, mob: MobRef) -> Option<bool> {
        self.native_mob_flag(mob, MobFlag::Persistent)
    }

    /// `Mob.setPersistenceRequired`, which sets the flag and cannot clear it.
    /// False for an unavailable mob.
    pub(super) fn mob_set_persistence_required(&mut self, mob: MobRef) -> bool {
        self.native_mob_set_flag(mob, MobFlag::Persistent, true)
    }

    /// `Mob.requiresCustomPersistence` and its species overrides: passengers,
    /// leashed mobs and species-specific reasons to stay, such as an enderman
    /// carrying a block. Independent of `PersistenceRequired` and `NoAI`.
    /// None for an unavailable mob or the separately modeled dragon.
    pub(super) fn mob_requires_custom_persistence(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        self.mobs
            .iter()
            .find(|m| m.id == id)
            .map(mobs::ai::requires_custom_persistence)
    }

    /// `Mob.isNoAi`: the mob stands where it is and its goals do not run,
    /// while what NoAI keeps running in vanilla (the daylight burn of the
    /// undead, a horse's tail) still does. Saved with it as `NoAI`. None
    /// for anything but a mob.
    pub(super) fn mob_no_ai(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(self.mobs.iter().find(|m| m.id == id && !m.removed)?.no_ai)
    }

    /// `Mob.setNoAi`: whether the mob's own AI runs. With no AI the mob
    /// stands (what NoAI keeps in vanilla still runs), and the flag is
    /// saved with it as `NoAI`. False for anything but a mob.
    pub(super) fn mob_set_no_ai(&mut self, mob: MobRef, no_ai: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.no_ai = no_ai)
            .is_some()
    }

    /// `LivingEntity.getNoActionTime`: the ticks since the mob last had a
    /// reason to stay (a player near, a hit). Despawning and a raid's
    /// recruiting read it. None for a stale or unavailable mob, the dragon,
    /// or on a client.
    pub(super) fn mob_no_action_time(&self, mob: MobRef) -> Option<i32> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .ai
                .no_action_time,
        )
    }

    /// `LivingEntity.setNoActionTime`: the count the mob's own tick carries
    /// on from (a far-away mob's despawn chance starts past 600). Not saved,
    /// as in vanilla. False for a stale or unavailable mob, the dragon, or
    /// on a client.
    pub(super) fn mob_set_no_action_time(&mut self, mob: MobRef, ticks: i32) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.ai.no_action_time = ticks)
            .is_some()
    }

    /// `Mob.setBaby` through the species' setter: ageable mobs use their
    /// baby start age (`-48000` for sniffers, otherwise `-24000`) or `0`,
    /// including age-boundary attribute and AI setup changes. Non-ageable
    /// babies, such as zombies, keep their non-growing baby state instead.
    /// Leaves `NoAI` and `PersistenceRequired` unchanged. False, with no
    /// changes, for an unavailable mob or a species with no baby form.
    pub(super) fn mob_set_baby(&mut self, mob: MobRef, baby: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        if !mobs::species::can_be_a_baby(mob.species()) {
            return false;
        }
        mobs::finalize::set_baby(mob, baby);
        true
    }

    /// `LookControl.setLookAt`, by the mob's `lookAt` path: the wanted
    /// position its look control rotates the head toward over the next
    /// ticks. As `move_to`'s path, the mob's own AI — a goal's look, or a
    /// brain species' `LookAtTargetSink`, which writes through this same
    /// control — may issue its own and replace this one; no species refuses.
    /// A creaking being looked at, an angry bee and an axolotl playing dead
    /// hold their look control, as in vanilla: the wanted position waits
    /// until it runs again. False for non-finite coordinates, a stale or
    /// unavailable mob, a client, and the dragon.
    pub(super) fn mob_look_at(&mut self, mob: MobRef, at: [f64; 3]) -> bool {
        if at.iter().any(|v| !v.is_finite()) {
            return false;
        }
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.look_at(at))
            .is_some()
    }

    /// `JumpControl.jump`: the mob jumps on its next own tick by its own
    /// jump path — no velocity is written, and the control consumes the
    /// wish on that tick (a rabbit's jump control reads it too). The mob's
    /// own wishes (a slime's hop, a swimming brain's) are the same flag; no
    /// species refuses. A creaking being looked at holds its jump control,
    /// as in vanilla: the wish waits until it runs again. False for a stale
    /// or unavailable mob, a client, and the dragon.
    pub(super) fn mob_jump(&mut self, mob: MobRef) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.ai.jump = true)
            .is_some()
    }

    /// `Mob.restrictTo(pos, radius)`: the home area the mob keeps to — its
    /// own stroll and wander goals pick their positions inside it, as a
    /// lead's and a guardian's home do. Radius `-1` clears the restriction
    /// while keeping `pos` as the center, `clear_restriction`'s shape. The
    /// area is saved with the mob (`home_pos`, `home_radius`). False for a
    /// stale or unavailable mob, a client, and the dragon.
    pub(super) fn mob_restrict_to(&mut self, mob: MobRef, at: [i32; 3], radius: i32) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.ai.restore_home(at, radius))
            .is_some()
    }

    /// `Mob.getRestrictCenter`: the restriction's block position, kept
    /// after `clear_restriction` and after a load (`[0, 0, 0]` for a mob
    /// that never had one, vanilla's `ZERO`). None for a stale or
    /// unavailable mob, a client, and the dragon.
    pub(super) fn mob_restrict_center(&self, mob: MobRef) -> Option<[i32; 3]> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .ai
                .home_position(),
        )
    }

    /// `Mob.getRestrictRadius`: `-1` while the mob has no restriction.
    /// None for a stale or unavailable mob, a client, and the dragon.
    pub(super) fn mob_restrict_radius(&self, mob: MobRef) -> Option<i32> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .ai
                .home
                .map_or(-1, |(_, radius)| radius),
        )
    }

    /// `Mob.hasRestriction`. None for a stale or unavailable mob, a client,
    /// and the dragon.
    pub(super) fn mob_has_restriction(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .ai
                .home
                .is_some(),
        )
    }

    /// `Mob.clearRestriction`: the restriction goes, the center stays (its
    /// own goals read no restriction, as vanilla's `-1` radius). False for
    /// a stale or unavailable mob, a client, and the dragon.
    pub(super) fn mob_clear_restriction(&mut self, mob: MobRef) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.ai.clear_home())
            .is_some()
    }

    /// `Mob.isWithinRestriction(pos)`: whether the block position is inside
    /// the home area; a mob without a restriction is within everywhere, as
    /// in vanilla. None for a stale or unavailable mob, a client, and the
    /// dragon.
    pub(super) fn mob_is_within_restriction(&self, mob: MobRef, at: [i32; 3]) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .ai
                .is_within_home(at),
        )
    }

    /// `Mob.isLeftHanded`: whether the mob holds things in its left hand
    /// (the mob flags in its entity data). None for anything but a mob.
    pub(super) fn mob_left_handed(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .extra
                .left_handed,
        )
    }

    /// `Mob.setLeftHanded`: sets the mob flag the players around get with
    /// the mob's next entity data and see in its swing and held items.
    /// Saved with the mob as `LeftHanded`. False for anything but a mob.
    pub(super) fn mob_set_left_handed(&mut self, mob: MobRef, left_handed: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.extra.left_handed = left_handed)
            .is_some()
    }

    /// `Mob.canPickUpLoot`, with the overrides of an allay (holding
    /// something) and a sulfur cube (with no block). None for anything but
    /// a mob.
    pub(super) fn mob_can_pick_up_loot(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .can_pick_up_loot(),
        )
    }

    /// `Mob.setCanPickUpLoot`: whether the mob picks up the items it sees
    /// (saved with it as `CanPickUpLoot`; clients are not told, as in
    /// vanilla). An allay and a sulfur cube decide for themselves. False
    /// for anything but a mob.
    pub(super) fn mob_set_can_pick_up_loot(&mut self, mob: MobRef, can_pick_up: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.extra.pick_up_loot = can_pick_up)
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use horizon_client::{entities::ArmorStand, mobs::Mob};
    use horizon_core::geometry::Vec3;
    use mod_api::WorldEpoch;

    fn add_mob(game: &mut Game<'_>, id: u32, name: &str) -> MobRef {
        let kind = game.res.mob_types().get(name).unwrap().clone();
        game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
        game.mob(game.entity(id).unwrap()).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn redirected_fake_target_is_refused_with_the_original_target_intact() {
        use std::sync::{Arc, Mutex};
        struct Redirect(Arc<Mutex<Option<LivingRef>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Redirect {
            fn living_change_target(
                &self,
                host: &dyn mod_api::Host,
                event: &mut mod_api::LivingChangeTargetEvent<'_>,
            ) -> mod_api::Flow {
                assert!(event.mob.living().health(host).is_some());
                *event.new_target = *self.0.lock().unwrap();
                mod_api::Flow::Continue
            }
        }
        let redirect = Arc::new(Mutex::new(None));
        let mods = vec![mod_api::ModEntry::new(
            "target-test",
            Box::new(Redirect(redirect.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                super::super::native_item_places::tests::open_to_lan(game);
                let level = game.level(game.dim.id()).unwrap();
                let fake = game.fake_player(level, 1234, "fake").unwrap();
                *redirect.lock().unwrap() = Some(LivingRef::new(fake.entity()));
                let mob = add_mob(game, 12, "minecraft:zombie");
                let existing = add_mob(game, 13, "minecraft:sheep");
                game.mobs[0].ai.target = Some(Who::Mob(13));
                assert!(!mob.set_target(game, None));
                assert_eq!(
                    game.mobs[0].ai.target,
                    Some(Who::Mob(existing.entity().id()))
                );
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn leash_holder_reads_live_state() {
        use super::super::{boats::Boat, lan::LanSettings};
        use horizon_client::lan::{
            Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig, playerdata::uuid_ints,
        };
        use horizon_protocol::packets::configuration::ClientInformation;
        use mobs::leash::{Holder, Pending};
        use mod_api::LeashHolder;
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };

        with_game(|game| {
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.level_shapes,
                level_seeds: game.levels.runtime.seeds(),
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 20,
                    uuid: 20,
                    name: "holder".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                20,
                [0.0, 0.0, 0.0],
                10,
            );
            guest.placed = true;
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            let cow = add_mob(game, 12, "minecraft:cow");
            let other = add_mob(game, 13, "minecraft:cow");
            game.boats
                .push(Boat::new(14, "oak".into(), Vec3::ZERO, 0.0));
            let boat = game.entity(14).unwrap();
            assert_eq!(
                cow.entity().leash_holder(game),
                Some(LeashHolder::Unleashed)
            );
            let motion =
                horizon_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(20, |guest| guest.motion = Some(Box::new(motion)))
                .unwrap();
            game.as_guest(20, |game| {
                assert!(game.interact_leash(0, "minecraft:lead"));
                assert_eq!(
                    cow.entity().leash_holder(game),
                    Some(LeashHolder::Entity(game.entity(20).unwrap()))
                );
            })
            .unwrap();
            for (holder, expected) in [
                (Holder::Mob(13), LeashHolder::Entity(other.entity())),
                (Holder::Boat(14), LeashHolder::Entity(boat)),
                (Holder::Knot([1, 2, 3]), LeashHolder::Knot([1, 2, 3])),
            ] {
                game.mobs[0].extra.leash = Some(Holder::Player);
                game.mobs[0].extra.leash_player = Some(uuid_ints(20));
                game.as_guest(20, |game| {
                    assert!(game.tie_player_leads(glam::DVec3::ZERO, holder, None));
                    assert_eq!(cow.entity().leash_holder(game), Some(expected));
                })
                .unwrap();
            }
            game.mobs[0].extra.leash = None;
            game.mobs[0].extra.leash_pending = Some(Pending::Uuid([0, 0, 0, 99]));
            assert_eq!(cow.entity().leash_holder(game), None);
            let stale = EntityRef::new(WorldEpoch::new(999), 12);
            assert_eq!(stale.leash_holder(game), None);
            game.enter_level(super::super::Dim::Nether).unwrap();
            assert_eq!(cow.entity().leash_holder(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_portal_eligibility_matches_gameplay() {
        with_game(|game| {
            let cow = add_mob(game, 12, "minecraft:cow");
            let wither = add_mob(game, 13, "minecraft:wither");
            let check = |game: &Game<'_>, handle: MobRef, expected| {
                let mob = game
                    .mobs
                    .iter()
                    .find(|m| m.id == handle.entity().id())
                    .unwrap();
                assert_eq!(game.mob_portal_eligible(mob), expected);
                let host: &dyn mod_api::Host = game;
                assert_eq!(handle.can_use_portal(host), Some(expected));
            };
            check(game, cow, true);
            check(game, wither, false);
            game.mobs[0].extra.mount = Some(99);
            check(game, cow, false);
            game.mobs[0].extra.mount = None;
            game.riding = Some(12);
            check(game, cow, false);
            game.riding = None;
            check(game, cow, true);
            let stale = MobRef::new(mod_api::EntityRef::new(WorldEpoch::new(999), 12));
            let host: &dyn mod_api::Host = game;
            assert_eq!(stale.can_use_portal(host), None);
            game.enter_command_level(super::super::Dim::Nether).unwrap();
            let host: &dyn mod_api::Host = game;
            assert_eq!(cow.can_use_portal(host), None);
            game.enter_command_level(super::super::Dim::Overworld)
                .unwrap();
            check(game, cow, true);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_flags_custom_persistence_is_independent_of_stored_flags() {
        with_game(|game| {
            let mob = add_mob(game, 12, "minecraft:cow");
            for no_ai in [false, true] {
                assert!(mob.set_no_ai(game, no_ai));
                for (persistent, leash, expected) in [
                    (false, None, false),
                    (false, Some(mobs::leash::Holder::Knot([5, 64, 5])), true),
                    (true, None, false),
                    (true, Some(mobs::leash::Holder::Knot([5, 64, 5])), true),
                ] {
                    game.mobs[0].persistent = persistent;
                    game.mobs[0].extra.leash = leash;
                    assert_eq!(mob.requires_custom_persistence(game), Some(expected));
                    assert_eq!(mob.persistence_required(game), Some(persistent));
                    assert_eq!(mob.no_ai(game), Some(no_ai));
                }
            }
            let enderman = add_mob(game, 13, "minecraft:enderman");
            assert_eq!(enderman.requires_custom_persistence(game), Some(false));
            game.mobs[1].extra.carried = Some(
                game.pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap(),
            );
            assert_eq!(enderman.requires_custom_persistence(game), Some(true));
            assert_eq!(enderman.persistence_required(game), Some(false));
            game.mobs[1].extra.carried = None;
            assert!(enderman.set_persistence_required(game));
            assert_eq!(enderman.requires_custom_persistence(game), Some(false));
            assert_eq!(enderman.persistence_required(game), Some(true));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_flags_baby_setter_uses_species_age_and_attribute_boundaries() {
        use mobs::finalize::{Attr, base_value};

        with_game(|game| {
            let mut id = 12;
            for (no_ai, persistent) in [(false, false), (false, true), (true, false), (true, true)]
            {
                for (name, baby_age, damage) in [
                    ("minecraft:sniffer", -48000, None),
                    ("minecraft:goat", -24000, Some((1.0, 2.0))),
                    ("minecraft:hoglin", -24000, Some((0.5, 6.0))),
                    // Non-ageable babies keep a sentinel age instead of growing up.
                    ("minecraft:zombie", -1073741824, None),
                    ("minecraft:piglin", -1073741824, None),
                ] {
                    let mob = add_mob(game, id, name);
                    id += 1;
                    assert!(mob.set_no_ai(game, no_ai));
                    if persistent {
                        assert!(mob.set_persistence_required(game));
                    }
                    for baby in [true, false, true, false] {
                        assert!(mob.set_baby(game, baby), "{name}");
                        let state = game.mobs.last().unwrap();
                        assert_eq!(state.extra.age, if baby { baby_age } else { 0 }, "{name}");
                        assert_eq!(mob.living().baby(game), Some(baby), "{name}");
                        if let Some((baby_damage, adult_damage)) = damage {
                            assert_eq!(
                                base_value(state, Attr::AttackDamage),
                                if baby { baby_damage } else { adult_damage },
                                "{name}"
                            );
                        }
                        assert_eq!(mob.no_ai(game), Some(no_ai), "{name}");
                        assert_eq!(mob.persistence_required(game), Some(persistent), "{name}");
                    }
                }
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_flags_baby_setter_runs_ghastling_setup_in_both_directions() {
        use mobs::{control::MoveKind, nav::NavKind};

        with_game(|game| {
            let ghast = add_mob(game, 12, "minecraft:happy_ghast");
            assert!(ghast.set_no_ai(game, true));
            assert!(ghast.set_persistence_required(game));
            assert!(game.mobs[0].ai.goals.has("HappyGhastFloatGoal"));
            for baby in [true, false, true, false] {
                assert!(ghast.set_baby(game, baby));
                let state = &game.mobs[0];
                assert_eq!(ghast.living().baby(game), Some(baby));
                assert_eq!(state.ai.goals.has("HappyGhastFloatGoal"), !baby);
                assert_eq!(state.ai.goals.has("RandomFloatAroundGoal"), !baby);
                if baby {
                    assert!(matches!(state.ai.nav.kind, NavKind::Flying));
                    assert!(matches!(state.ai.mov.kind, MoveKind::Flying { .. }));
                } else {
                    assert!(matches!(state.ai.nav.kind, NavKind::Ground));
                    assert!(matches!(state.ai.mov.kind, MoveKind::Custom));
                }
                assert_eq!(ghast.no_ai(game), Some(true));
                assert_eq!(ghast.persistence_required(game), Some(true));
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_flags_baby_setter_refuses_no_baby_species_and_unavailable_mobs() {
        with_game(|game| {
            for (id, name) in [(12, "minecraft:skeleton"), (13, "minecraft:parrot")] {
                let mob = add_mob(game, id, name);
                assert!(mob.set_no_ai(game, true));
                assert!(mob.set_persistence_required(game));
                for baby in [true, false] {
                    assert!(!mob.set_baby(game, baby), "{name}");
                    assert_eq!(game.mobs.last().unwrap().extra.age, 0, "{name}");
                    assert_eq!(mob.living().baby(game), Some(false), "{name}");
                    assert_eq!(mob.no_ai(game), Some(true), "{name}");
                    assert_eq!(mob.persistence_required(game), Some(true), "{name}");
                }
            }
            let mob = add_mob(game, 14, "minecraft:sniffer");
            let stale = MobRef::new(EntityRef::new(
                WorldEpoch::new(mob.entity().epoch().get() + 1),
                14,
            ));
            assert_eq!(stale.requires_custom_persistence(game), None);
            assert!(!stale.set_baby(game, true));
            game.held_out.push(14);
            assert_eq!(mob.requires_custom_persistence(game), None);
            assert!(!mob.set_baby(game, true));
            game.held_out.pop();
            game.mobs.last_mut().unwrap().removed = true;
            assert_eq!(mob.requires_custom_persistence(game), None);
            assert!(!mob.set_baby(game, true));
            assert_eq!(game.mobs.last().unwrap().extra.age, 0);
            game.mobs.last_mut().unwrap().removed = false;
            let taken = game.mobs.lift();
            assert_eq!(mob.requires_custom_persistence(game), None);
            assert!(!mob.set_baby(game, true));
            game.mobs = taken;
            assert!(mob.set_baby(game, true));

            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 15;
            game.dragon.replace(dragon);
            let dragon = game.mob(game.entity(15).unwrap()).unwrap();
            assert_eq!(dragon.requires_custom_persistence(game), None);
            assert!(!dragon.set_baby(game, true));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_target_round_trips_through_the_mob_binding() {
        use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        use horizon_protocol::packets::configuration::ClientInformation;
        use std::sync::{Arc, atomic::AtomicI32, mpsc};

        with_game(|game| {
            let settings = super::super::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                announce: false,
                ..NetConfig::lan(0, "MobRefTest".into(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 10,
                simulation_distance: 10,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, _rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 1,
                    uuid: 123,
                    name: "guest".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                20,
                [0.0; 3],
                8,
            );
            guest.placed = true;
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            let mob = game.mob(game.entity(12).unwrap()).unwrap();
            let player = game.living(game.entity(20).unwrap()).unwrap();
            assert!(mob.set_target(game, Some(player)));
            assert_eq!(mob.target(game), Some(player));
            assert_eq!(game.lan.as_ref().unwrap().mob_players.get(&12), Some(&1));
            // `ZombifiedPiglin.setTarget` does not impersonate a player hit:
            // existing kill credit and its remaining lifetime are unchanged.
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombified_piglin")
                .unwrap()
                .clone();
            let mut piglin_state = Mob::new(13, kind, Vec3::ZERO, 0.0);
            piglin_state.ai.last_hurt_by_player = 42;
            game.mobs.push(piglin_state);
            let piglin = game.mob(game.entity(13).unwrap()).unwrap();
            assert!(piglin.set_target(game, Some(player)));
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| m.id == 13)
                    .unwrap()
                    .ai
                    .last_hurt_by_player,
                42
            );
            assert!(!game.last_hurt_by_player.contains_key(&13));
            assert!(piglin.set_target(game, None));
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| m.id == 13)
                    .unwrap()
                    .ai
                    .last_hurt_by_player,
                42
            );
            let level = game.level(game.dim.id()).unwrap();
            let fake = game.fake_player(level, 9999, "fake").unwrap();
            let fake_living = LivingRef::new(fake.entity());
            assert!(!piglin.set_target(game, Some(fake_living)));
            assert_eq!(piglin.target(game), None);
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| m.id == 13)
                    .unwrap()
                    .ai
                    .last_hurt_by_player,
                42
            );
            // The alert recipient is allied with the host, not with the
            // unteamed guest targeted by the source piglin.
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombified_piglin")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(14, kind, Vec3::ZERO, 0.0));
            game.persist.uuids.insert(14, [0, 0, 0, 14]);
            let host_name = game.acting_player_name().to_owned();
            game.score.board.add_team("red");
            game.score.board.add_to_team(&host_name, "red");
            game.score
                .board
                .add_to_team("00000000-0000-0000-0000-00000000000e", "red");
            let recipient = game.mob(game.entity(14).unwrap()).unwrap();
            assert!(piglin.set_target(game, Some(player)));
            let alert = || {
                mobs::MobEvent::PiglinAlertTarget(mobs::undead::PiglinAlert {
                    mob: 14,
                    source: 13,
                    target: Who::Player,
                })
            };
            game.handle_mob_events(vec![alert()]).unwrap();
            assert_eq!(recipient.target(game), Some(player));
            assert!(recipient.set_target(game, None));
            game.score.board.add_to_team("guest", "red");
            game.handle_mob_events(vec![alert()]).unwrap();
            assert_eq!(recipient.target(game), None);
            {
                let value = 1;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.game_mode = value);
            }
            assert!(!mob.set_target(game, Some(player)));
            assert_eq!(mob.target(game), None);
            assert_eq!(game.mobs[0].ai.target, Some(Who::Player));
            {
                let value = 0;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.game_mode = value);
            }
            {
                let value = "minecraft:the_nether".into();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            assert!(!mob.set_target(game, Some(player)));
            assert_eq!(game.mobs[0].ai.target, Some(Who::Player));
            game.lan.as_mut().unwrap().mob_players.clear();
            assert_eq!(mob.target(game), None);
            assert_eq!(game.mobs[0].ai.target, Some(Who::Player));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn fox_enderman_and_zombified_piglin_take_targets_with_their_side_effects() {
        use horizon_core::random::{LegacyRandom, RandomSource};
        with_game(|game| {
            for (id, name) in [
                (12, "minecraft:fox"),
                (13, "minecraft:enderman"),
                (14, "minecraft:zombified_piglin"),
                (15, "minecraft:zombie"),
            ] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let handle = |game: &Game<'_>, id| game.mob(game.entity(id).unwrap()).unwrap();
            let (fox, enderman, piglin) = (handle(game, 12), handle(game, 13), handle(game, 14));
            let other = game.living(game.entity(15).unwrap()).unwrap();
            let state =
                |game: &Game<'_>, id| game.mobs.iter().find(|m| m.id == id).unwrap().clone();

            // Fox.setTarget: a target leaves defending as it is; clearing ends it.
            game.mobs
                .iter_mut()
                .find(|m| m.id == 12)
                .unwrap()
                .extra
                .ints[0] |= mobs::fox::DEFENDING;
            mobs::fox::add_trusted_entity(
                game.mobs.iter_mut().find(|m| m.id == 12).unwrap(),
                [1, 2, 3, 4],
            );
            assert!(fox.set_target(game, Some(other)));
            assert_eq!(fox.target(game), Some(other));
            assert!(mobs::fox::flag(&state(game, 12), mobs::fox::DEFENDING));
            assert!(fox.set_target(game, None));
            assert_eq!(fox.target(game), None);
            assert!(!mobs::fox::flag(&state(game, 12), mobs::fox::DEFENDING));
            assert!(mobs::fox::trusts(&state(game, 12), [1, 2, 3, 4]));

            // Enderman.setTarget: the open mouth, targetChangeTime and the
            // attack speed modifier follow the target.
            let enderman_state = game.mobs.iter_mut().find(|m| m.id == 13).unwrap();
            enderman_state.age = 1234;
            enderman_state.extra.flag2 = true;
            game.audio.capture = Some(Vec::new());
            assert!(enderman.set_target(game, Some(other)));
            let events =
                std::mem::take(&mut game.mobs.iter_mut().find(|m| m.id == 13).unwrap().outbox);
            game.handle_mob_events(events).unwrap();
            assert_eq!(
                game.audio.heard.get("minecraft:entity.enderman.stare"),
                Some(&1)
            );
            assert!(
                game.audio.capture.as_ref().unwrap().is_empty(),
                "stare stays client-local"
            );
            let m = state(game, 13);
            assert!(m.extra.flag, "DATA_CREEPY");
            assert_eq!(m.extra.counter, 1234, "targetChangeTime");
            assert!(mobs::enderman::attacking_speed(&m) > 0.0);
            assert!(m.extra.flag2, "retargeting preserves DATA_STARED_AT");
            assert!(m.movement_speed() > f64::from(0.3f32));
            assert!(enderman.set_target(game, None));
            let m = state(game, 13);
            assert!(!m.extra.flag);
            assert_eq!(m.extra.counter, 0);
            assert!(!m.extra.flag2, "clearing resets DATA_STARED_AT");
            assert_eq!(mobs::enderman::attacking_speed(&m), 0.0);

            // ZombifiedPiglin.setTarget samples both delays immediately;
            // changing a nonempty target does not resample either.
            let mut expected = LegacyRandom::new(2026);
            let sound_delay = expected.next_int_bound(21).unwrap();
            let alert_delay = 80 + expected.next_int_bound(41).unwrap();
            game.mobs.iter_mut().find(|m| m.id == 14).unwrap().random = horizon_core::random::EntityRandom::own(LegacyRandom::new(2026));
            assert!(piglin.set_target(game, Some(other)));
            assert_eq!(piglin.target(game), Some(other));
            let first = state(game, 14);
            assert_eq!(first.extra.counter, sound_delay);
            assert_eq!(first.extra.counter2, alert_delay);
            assert!(piglin.set_target(game, Some(other)));
            let changed = state(game, 14);
            assert_eq!(changed.extra.counter, first.extra.counter);
            assert_eq!(changed.extra.counter2, first.extra.counter2);
            assert!(piglin.set_target(game, None));
            assert_eq!(piglin.target(game), None);
        });
    }

    /// `ZombifiedPiglin.setTarget`, `customServerAiStep`, and `alertOthers`:
    /// a host write is consumed by real mob ticks, audible to the player and
    /// propagated to another piglin. Seed selection fixes the first sound at
    /// one tick; advancing the alert timer avoids waiting four seconds.
    #[test]
    #[ignore = "daily"]
    fn piglin_target_write_drives_anger_sound_and_alert() {
        use horizon_core::random::{LegacyRandom, RandomSource};
        with_game(|game| {
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = horizon_client::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            for (id, kind, x) in [
                (12, "minecraft:zombified_piglin", 0.0),
                (13, "minecraft:zombified_piglin", 2.0),
                (14, "minecraft:iron_golem", 8.0),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let mut mob = Mob::new(id, kind, Vec3::new(x, 100.0, 0.0), 0.0);
                mobs::ai::register(&mut mob);
                game.mobs.push(mob);
            }
            let seed = (0..10_000)
                .find(|&seed| LegacyRandom::new(seed).next_int_bound(21) == Ok(1))
                .unwrap();
            game.mobs[0].random = horizon_core::random::EntityRandom::own(LegacyRandom::new(seed));
            let actor = game.mob(game.entity(12).unwrap()).unwrap();
            let other = game.mob(game.entity(13).unwrap()).unwrap();
            let target = game.living(game.entity(14).unwrap()).unwrap();
            assert!(actor.set_target(game, Some(target)));
            assert_eq!(game.mobs[0].extra.counter, 1);
            game.mobs[0].extra.counter2 = 0;
            game.audio.capture = Some(Vec::new());
            game.tick_mobs().unwrap();
            assert_eq!(other.target(game), Some(target));
            game.tick_mobs().unwrap();
            assert!(
                game.audio
                    .capture
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|sound| sound.event == "minecraft:entity.zombified_piglin.angry")
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn brain_species_take_a_written_attack_target_memory() {
        with_game(|game| {
            for (id, name) in [
                (12, "minecraft:warden"),
                (13, "minecraft:breeze"),
                (14, "minecraft:axolotl"),
                (15, "minecraft:piglin"),
                (16, "minecraft:hoglin"),
                (17, "minecraft:creaking"),
                (18, "minecraft:zoglin"),
                (19, "minecraft:frog"),
                (20, "minecraft:zombie"),
                (21, "minecraft:iron_golem"),
            ] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let handle = |game: &Game<'_>, id| game.mob(game.entity(id).unwrap()).unwrap();
            let golem = game.living(game.entity(21).unwrap()).unwrap();
            // Each brain keeps the memory where its behaviours read it, and the
            // read handle answers it back; clearing erases it.
            for id in [12, 13, 14, 15, 16, 17, 18] {
                let mob = handle(game, id);
                assert!(mob.set_brain_target(game, Some(golem)), "mob {id}");
                assert_eq!(mob.target(game), Some(golem), "mob {id}");
                assert_eq!(
                    mobs::ai::brain_target(game.mobs.iter().find(|m| m.id == id).unwrap()),
                    Some(Some(Who::Mob(21))),
                    "mob {id}"
                );
                assert!(mob.set_brain_target(game, None), "mob {id}");
                assert_eq!(mob.target(game), None, "mob {id}");
            }
            // A plain memory write: the warden's own setAttackTarget effects
            // (the sonic-boom cooldown among them) do not run.
            let warden = handle(game, 12);
            assert!(warden.set_brain_target(game, Some(golem)));
            let st = game.mobs.iter().find(|m| m.id == 12).unwrap();
            assert!(
                !st.extra
                    .warden
                    .as_ref()
                    .unwrap()
                    .brain
                    .mem
                    .sonic_boom_cooldown
                    .present()
            );
            // The brains act on it: a tick keeps a hoglin's and a breeze's
            // written target (their target-validity behaviours accept an iron
            // golem), as a memory their own sensing set would be kept.
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = horizon_client::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            for id in [13, 16] {
                assert!(handle(game, id).set_brain_target(game, Some(golem)));
            }
            game.tick_mobs().unwrap();
            for id in [13, 16] {
                assert_eq!(
                    handle(game, id).target(game),
                    Some(golem),
                    "mob {id} after a tick"
                );
            }
            // The frog's target belongs to its tongue attack; a mob without
            // an attack-target brain takes `set_target` instead.
            assert!(!handle(game, 19).set_brain_target(game, Some(golem)));
            assert!(!handle(game, 20).set_brain_target(game, Some(golem)));
            assert!(handle(game, 20).set_target(game, Some(golem)));
            // The ordinary setter still refuses the brain species.
            assert!(!handle(game, 13).set_target(game, Some(golem)));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_handles_read_write_and_refuse_unavailable_entities() {
        with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie.clone(), Vec3::ZERO, 0.0));
            game.mobs.push(Mob::new(13, zombie, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(14, Vec3::ZERO, 0.0));
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            // Reserve the dragon's eight consecutive part IDs away from the mobs.
            dragon.id = 40;
            game.dragon.replace(dragon);
            assert!(
                game.mob(game.entity(40).unwrap()).is_some(),
                "the dragon is a Mob"
            );
            let entity = game.entity(12).unwrap();
            let mob = game.mob(entity).unwrap();
            let other = game.living(game.entity(13).unwrap()).unwrap();
            let stand = game.living(game.entity(14).unwrap()).unwrap();
            for (id, name) in [
                (16, "minecraft:creaking"),
                (17, "minecraft:creeper"),
                (18, "minecraft:goat"),
                (19, "minecraft:warden"),
                (21, "minecraft:ghast"),
            ] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let brain = game.mob(game.entity(16).unwrap()).unwrap();
            mobs::monster_brain::set_attack_target(
                game.mobs.iter_mut().find(|m| m.id == 16).unwrap(),
                Some(Who::Mob(13)),
            );
            assert_eq!(brain.target(game), Some(other));
            assert!(!brain.set_target(game, None));
            assert_eq!(brain.target(game), Some(other));
            let warden = game.mob(game.entity(19).unwrap()).unwrap();
            let ghast = game.living(game.entity(21).unwrap()).unwrap();
            let warden_mob = game.mobs.iter_mut().find(|m| m.id == 19).unwrap();
            warden_mob
                .extra
                .warden
                .as_mut()
                .unwrap()
                .brain
                .mem
                .attack_target
                .set(Who::Mob(21));
            assert_eq!(
                warden.target(game),
                Some(ghast),
                "warden can target a ghast, unlike Mob.canAttack"
            );
            assert!(!warden.set_target(game, None));
            let creeper = game.mob(game.entity(17).unwrap()).unwrap();
            let goat = game.living(game.entity(18).unwrap()).unwrap();
            assert!(creeper.set_target(game, Some(other)));
            assert!(!creeper.set_target(game, Some(goat)));
            assert_eq!(creeper.target(game), Some(other));
            assert!(game.mob(stand.entity()).is_none());
            assert_eq!(mob.living().entity(), entity);
            assert_eq!(game.mob_is_aggressive(mob), Some(false));
            assert!(game.mob_set_aggressive(mob, true));
            assert_eq!(game.mob_is_aggressive(mob), Some(true));
            assert_eq!(game.mob_persistence_required(mob), Some(false));
            assert!(game.mob_set_persistence_required(mob));
            assert_eq!(game.mob_persistence_required(mob), Some(true));
            assert!(mob.set_target(game, Some(other)));
            assert_eq!(mob.target(game), Some(other));
            assert!(!game.mob_set_target(mob, Some(stand)));
            assert_eq!(game.mob_target(mob), Some(other));
            let stale = MobRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                12,
            ));
            assert!(game.mob(stale.entity()).is_none());
            assert!(!game.mob_set_target(stale, None));
            assert_eq!(game.mob_is_aggressive(stale), None);
            let taken = game.mobs.lift();
            assert_eq!(game.mob_target(mob), None);
            assert!(!game.mob_set_target(mob, None));
            assert_eq!(taken[0].ai.target, Some(Who::Mob(13)));
            game.mobs = taken;
            assert!(game.mob_set_target(mob, None));
            assert_eq!(game.mob_target(mob), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn look_and_jump_go_through_the_mobs_own_controls() {
        with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie, Vec3::new(3.5, 65.0, 3.5), 0.0));
            let creaking = game
                .res
                .mob_types()
                .get("minecraft:creaking")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(16, creaking, Vec3::new(9.5, 65.0, 9.5), 0.0));
            let mob = game.mob(game.entity(12).unwrap()).unwrap();
            assert!(game.mob_look_at(mob, [10.5, 66.0, -2.5]));
            let (wanted, cooldown, speed) = {
                let control = &game.mobs[0].ai.look;
                (control.wanted, control.cooldown, control.y_max_rot_speed)
            };
            assert_eq!(wanted, [10.5, 66.0, -2.5]);
            assert_eq!(cooldown, 2);
            assert_eq!(speed, game.mobs[0].kind.head_rot_speed);
            assert!(!game.mob_look_at(mob, [f64::NAN, 66.0, 0.0]));
            assert_eq!(game.mobs[0].ai.look.wanted, [10.5, 66.0, -2.5]);
            // A brain species writes through the same look control, so its
            // handle takes the wish too.
            let brain = game.mob(game.entity(16).unwrap()).unwrap();
            assert!(game.mob_look_at(brain, [0.0, 66.0, 0.0]));
            assert_eq!(game.mobs[1].ai.look.wanted, [0.0, 66.0, 0.0]);
            assert!(game.mob_jump(brain));
            assert!(game.mobs[1].ai.jump);
            assert!(game.mob_jump(mob));
            assert!(game.mobs[0].ai.jump);
            // JumpControl.tick consumes the wish as the next tick's jump.
            game.mobs[0].jump_control_tick();
            assert!(game.mobs[0].ai.jumping);
            assert!(!game.mobs[0].ai.jump);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_restriction_round_trips_through_the_handle_and_saves() {
        use horizon_core::nbt::Tag;
        with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie, Vec3::new(3.5, 65.0, 3.5), 0.0));
            let mob = game.mob(game.entity(12).unwrap()).unwrap();
            assert_eq!(game.mob_restrict_center(mob), Some([0, 0, 0]));
            assert_eq!(game.mob_restrict_radius(mob), Some(-1));
            assert_eq!(game.mob_has_restriction(mob), Some(false));
            assert_eq!(
                game.mob_is_within_restriction(mob, [900, 90, 900]),
                Some(true),
                "an unrestricted mob is within everywhere"
            );
            assert!(game.mob_restrict_to(mob, [10, 64, -5], 6));
            assert_eq!(game.mob_has_restriction(mob), Some(true));
            assert_eq!(game.mob_restrict_center(mob), Some([10, 64, -5]));
            assert_eq!(game.mob_restrict_radius(mob), Some(6));
            assert_eq!(
                game.mob_is_within_restriction(mob, [12, 64, -2]),
                Some(true)
            );
            assert_eq!(
                game.mob_is_within_restriction(mob, [20, 64, 0]),
                Some(false)
            );
            // The home area is saved with the mob.
            let saved =
                horizon_client::entity_nbt::mob_to_nbt(&game.mobs[0], [0; 4], None, None, |_| None);
            assert_eq!(
                saved.get("home_pos"),
                Some(&Tag::IntArray(vec![10, 64, -5]))
            );
            assert_eq!(saved.get("home_radius"), Some(&Tag::Int(6)));
            // clearRestriction keeps the center; every position is within
            // again.
            assert!(game.mob_clear_restriction(mob));
            assert_eq!(game.mob_has_restriction(mob), Some(false));
            assert_eq!(game.mob_restrict_center(mob), Some([10, 64, -5]));
            assert_eq!(game.mob_restrict_radius(mob), Some(-1));
            assert_eq!(game.mob_is_within_restriction(mob, [20, 64, 0]), Some(true));
            let saved =
                horizon_client::entity_nbt::mob_to_nbt(&game.mobs[0], [0; 4], None, None, |_| None);
            assert!(saved.get("home_pos").is_none());
            assert!(saved.get("home_radius").is_none());
            // restrictTo with radius -1 clears while keeping the new center.
            assert!(game.mob_restrict_to(mob, [1, 2, 3], -1));
            assert_eq!(game.mob_has_restriction(mob), Some(false));
            assert_eq!(game.mob_restrict_center(mob), Some([1, 2, 3]));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_extreme_restriction_answers_as_vanilla() {
        with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie, Vec3::new(3.5, 65.0, 3.5), 0.0));
            let mob = game.mob(game.entity(12).unwrap()).unwrap();
            // `Vec3i.distSqr` in doubles: the far corner of the int range is
            // outside, with no overflow on the way.
            assert!(game.mob_restrict_to(mob, [i32::MIN, 0, 0], 6));
            assert_eq!(
                game.mob_is_within_restriction(mob, [i32::MAX, 0, 0]),
                Some(false)
            );
            assert!(game.mob_restrict_to(mob, [i32::MIN, i32::MIN, i32::MIN], 6));
            assert_eq!(
                game.mob_is_within_restriction(mob, [i32::MAX, i32::MAX, i32::MAX]),
                Some(false)
            );
            // The radius squares as a Java `int`: 65 536² wraps to 0, so not
            // even the center is within.
            assert!(game.mob_restrict_to(mob, [0, 0, 0], 65_536));
            assert_eq!(game.mob_is_within_restriction(mob, [0, 0, 0]), Some(false));
            // The largest radius that squares without wrapping still holds.
            assert!(game.mob_restrict_to(mob, [0, 0, 0], 46_340));
            assert_eq!(
                game.mob_is_within_restriction(mob, [46_339, 0, 0]),
                Some(true)
            );
            assert_eq!(
                game.mob_is_within_restriction(mob, [46_340, 0, 0]),
                Some(false)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn hand_and_loot_flags_live_on_the_mob_handle() {
        use horizon_core::nbt::Tag;
        with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 15;
            game.dragon.replace(dragon);
            let entity = game.entity(12).unwrap();
            let mob = game.mob(entity).unwrap();
            assert_eq!(game.mob_left_handed(mob), Some(false));
            assert!(game.mob_set_left_handed(mob, true));
            assert!(game.mobs[0].extra.left_handed && game.mobs[0].main_arm_left());
            assert_eq!(game.mob_left_handed(mob), Some(true));
            assert!(game.mob_set_can_pick_up_loot(mob, true));
            assert!(game.mobs[0].extra.pick_up_loot);
            assert_eq!(game.mob_can_pick_up_loot(mob), Some(true));
            // Saved as LeftHanded / CanPickUpLoot.
            let saved =
                horizon_client::entity_nbt::mob_to_nbt(&game.mobs[0], [0; 4], None, None, |_| None);
            assert_eq!(saved.get("LeftHanded"), Some(&Tag::Byte(1)));
            assert_eq!(saved.get("CanPickUpLoot"), Some(&Tag::Byte(1)));
            assert!(game.mob_set_left_handed(mob, false));
            assert!(game.mob_set_can_pick_up_loot(mob, false));
            assert_eq!(game.mob_left_handed(mob), Some(false));
            assert_eq!(game.mob_can_pick_up_loot(mob), Some(false));
            // The dragon is not a Mob for these.
            let dragon = game.mob(game.entity(15).unwrap()).unwrap();
            assert_eq!(game.mob_left_handed(dragon), None);
            assert!(!game.mob_set_left_handed(dragon, true));
            assert_eq!(game.mob_can_pick_up_loot(dragon), None);
            assert!(!game.mob_set_can_pick_up_loot(dragon, true));
            assert!(!game.mob_look_at(dragon, [0.0; 3]));
            assert!(!game.mob_jump(dragon));
            assert!(!game.mob_restrict_to(dragon, [0, 0, 0], 5));
            assert_eq!(game.mob_has_restriction(dragon), None);
            // A stale handle changes no other entity.
            let stale = MobRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                12,
            ));
            assert_eq!(game.mob_left_handed(stale), None);
            assert!(!game.mob_set_left_handed(stale, true));
            assert_eq!(game.mob_can_pick_up_loot(stale), None);
            assert!(!game.mob_set_can_pick_up_loot(stale, true));
            assert!(!game.mob_look_at(stale, [0.0; 3]));
            assert!(!game.mob_jump(stale));
            assert!(!game.mob_restrict_to(stale, [0, 0, 0], 5));
            assert!(!game.mob_clear_restriction(stale));
            assert_eq!(game.mob_restrict_center(stale), None);
            assert_eq!(game.mob_restrict_radius(stale), None);
            assert_eq!(game.mob_has_restriction(stale), None);
            assert_eq!(game.mob_is_within_restriction(stale, [0, 0, 0]), None);
            assert!(!game.mobs[0].extra.left_handed);
            assert!(!game.mobs[0].extra.pick_up_loot);
        });
    }
}
