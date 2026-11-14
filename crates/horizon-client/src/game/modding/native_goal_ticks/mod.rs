//! The mod goal context of a mob's tick (arismoko/horizon#1077): the reads a
//! goal makes through `GoalContext::host` answer the ticking mob itself, the
//! player the tick sees and the tick's snapshot of the other entities — the
//! entity lookups and snapshot-backed reads are the view's own (generated
//! below), entity data the snapshot does not carry is refused its typed
//! refusal, and every other backend read — the level's own (blocks, tags,
//! recipes, settings), time, height, weather, block entities — forwards to
//! the real backend of the game host the view carries. The writes a goal
//! makes apply to the mob in hand through the engine's own paths
//! (`nav_move_to`, `ai.target`), behind the same input boundaries the mob
//! handles take, and the write handles a `ServerHost` takes do not exist here.
use super::Game;
use crate::mobs::{
    self, Mob, MobWorld, Seen, TargetInfo, Who,
    part_goals::{BehaviorOp, GoalOp},
};
use horizon_core::geometry::Axis;
use mod_api::{
    BehaviorCxBackend, EntityRef, GoalCxBackend, Host, LegacyRandom, LivingRef, MobRef, PlayerRef,
    content::crash::{ModCall, enter_native_mod},
};

#[cfg(test)]
mod interception_tests;
mod live_backend;
mod live_entity;
mod live_living;
mod live_merchant;
mod live_mob;
#[cfg(test)]
mod live_read_tests;
mod live_support;
mod live_vehicle;
#[cfg(test)]
mod redirect_tests;

/// What a mod goal runs with: the mob in hand, the tick's `MobWorld` (the
/// [`mobs::part_goals::GoalHost`], the snapshot and the player view ride on
/// it) and the game host behind the tick, for the reads that are the level's
/// own. The writes apply to the mob itself.
pub(super) struct GoalCx<'a, 'w> {
    m: &'a mut Mob,
    w: &'a MobWorld<'w>,
    host: &'a dyn Host,
}

/// What a handle names in the goal view: the ticking mob itself, the player
/// the tick sees, another mob in the snapshot, or the separately simulated
/// dragon. The ticking mob's own id never consults the taken list — the
/// exception that makes any mob handle resolve in its own tick (#29's limit).
enum Resolved<'a> {
    Own,
    Player,
    Live(&'a Mob),
    Seen(&'a Seen),
    Dragon(TargetInfo),
}

impl GoalCx<'_, '_> {
    /// Resolve `entity` in the tick's view, or None for a stale or foreign
    /// handle, an entity the snapshot does not carry, or a player the tick
    /// does not stand in: the identity the tick captured where it lent its
    /// player view names it — the game's own player, or the guest a server's
    /// mob sees — and a fake player resolves as none.
    fn resolve(&self, entity: EntityRef) -> Option<Resolved<'_>> {
        let host = self.w.goal_host?;
        if host.epoch() != Some(entity.epoch()) {
            return None;
        }
        let id = entity.id();
        if id == self.m.id {
            return Some(Resolved::Own);
        }
        if self.w.selected_player().is_some()
            && self
                .w
                .selected_player_identity()
                .is_some_and(|(_, player)| player == id)
        {
            return Some(Resolved::Player);
        }
        if let Some((dragon, info)) = self.w.dragon
            && dragon == id
        {
            return Some(Resolved::Dragon(info));
        }
        if let Some(mobs) = self.w.live_mobs {
            // When the tick has a live view, it is authoritative. Falling
            // back to an older snapshot here would resurrect a stale handle.
            return mobs.find(id).filter(|mob| !mob.removed).map(Resolved::Live);
        }
        let others = self.w.others;
        let seen = match self.w.others_index {
            Some(index) => index.get(&id).map(|&slot| &others[slot as usize]),
            None => others.iter().find(|s| s.id == id),
        }?;
        Some(Resolved::Seen(seen))
    }

    // --- the entity lookups the generated Host impl forwards to ----------

    /// The ticking mob's own handle, as both the goal and the behavior
    /// context build it: the goal host's epoch around the mob in hand.
    fn mob_handle(&self) -> MobRef {
        let host = self
            .w
            .goal_host
            .expect("a goal context is built only where a goal host is");
        let epoch = host.epoch().expect("a goal host is built only in a world");
        MobRef::new(EntityRef::new(epoch, self.m.id))
    }

    /// `Host::entity`: the id resolves in the tick's view — the ticking mob,
    /// the player it sees, a mob of the snapshot, the dragon — or none.
    pub(crate) fn lookup_entity(&self, id: u32) -> Option<EntityRef> {
        let host = self.w.goal_host?;
        let epoch = host.epoch()?;
        let entity = EntityRef::new(epoch, id);
        match self.resolve(entity) {
            Some(
                Resolved::Own
                | Resolved::Player
                | Resolved::Live(_)
                | Resolved::Seen(_)
                | Resolved::Dragon(_),
            ) => Some(entity),
            None => self.host.entity(id),
        }
    }

    /// `Host::living`: the handle resolves in the tick's view.
    pub(crate) fn lookup_living(&self, entity: EntityRef) -> Option<LivingRef> {
        self.resolve(entity)
            .map(|_| LivingRef::new(entity))
            .or_else(|| self.host.living(entity))
    }

    /// `Host::mob`: the handle resolves in the tick's view as a mob — the
    /// ticking mob, a snapshot mob, the dragon — but never a player: a player
    /// is no `MobRef`, as the write handle's own narrowing refuses it.
    pub(crate) fn lookup_mob(&self, entity: EntityRef) -> Option<MobRef> {
        match self.resolve(entity) {
            Some(Resolved::Player) => None,
            Some(_) => Some(MobRef::new(entity)),
            None => self.host.mob(entity),
        }
    }

    /// `Host::player`: resolve any online player, including the one the tick
    /// sees, by UUID.
    pub(crate) fn lookup_player(&self, uuid: u128) -> Option<PlayerRef> {
        self.host.player(uuid).or_else(|| {
            let (uuid_seen, id) = self.w.selected_player_identity()?;
            let epoch = self.w.goal_host?.epoch()?;
            (self.w.selected_player().is_some()
                && uuid == horizon_client::lan::playerdata::ints_uuid(uuid_seen))
            .then(|| mod_api::__private::game::player_ref(EntityRef::new(epoch, id), uuid))
        })
    }

    /// `Host::item_entity`: items remain in the game during mob ticks.
    pub(crate) fn lookup_item_entity(&self, entity: EntityRef) -> Option<mod_api::ItemEntityRef> {
        self.host.item_entity(entity)
    }

    /// `Host::block_entity`: block entities live in the world's chunks, which
    /// the tick does not take out — the game host answers for them.
    pub(crate) fn lookup_block_entity(
        &self,
        level: mod_api::LevelRef,
        pos: [i32; 3],
    ) -> Option<mod_api::BlockEntityRef> {
        self.host.block_entity(level, pos)
    }

    /// The entity's position, or None where unresolvable.
    fn position(&self, entity: EntityRef) -> Option<horizon_core::geometry::Vec3> {
        match self.resolve(entity)? {
            Resolved::Own => Some(self.m.position),
            Resolved::Live(mob) => Some(mob.position),
            Resolved::Player => self.w.selected_player().map(|p| p.position),
            Resolved::Seen(seen) => Some(seen.position),
            Resolved::Dragon(info) => Some(info.position),
        }
    }

    // --- the snapshot-backed reads the generated view forwards to ----------

    fn r_entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        self.position(entity).map(|p| [p.x, p.y, p.z])
    }

    fn r_entity_block_position(&self, entity: EntityRef) -> Option<[i32; 3]> {
        self.position(entity)
            .map(|p| [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32])
    }

    fn r_entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]> {
        match self.resolve(entity)? {
            Resolved::Own => Some(self.m.velocity),
            Resolved::Live(mob) => Some(mob.velocity),
            Resolved::Player => self.w.selected_player().map(|p| p.velocity),
            Resolved::Seen(seen) => Some(seen.velocity),
            Resolved::Dragon(_) => None,
        }
        .map(|v| [v.x, v.y, v.z])
    }

    fn r_entity_rotation(&self, entity: EntityRef) -> Option<[f32; 2]> {
        match self.resolve(entity)? {
            // `getYRot` and `getXRot`: the snapshot carries no pitch and a
            // player's view no pitch either, so only the mob in hand answers.
            Resolved::Own => Some([self.m.yaw, self.m.head_pitch]),
            Resolved::Live(mob) => Some([mob.yaw, mob.head_pitch]),
            _ => None,
        }
    }

    fn r_entity_head_yaw(&self, entity: EntityRef) -> Option<f32> {
        match self.resolve(entity)? {
            Resolved::Own => Some(self.m.head_yaw),
            Resolved::Live(mob) => Some(mob.head_yaw),
            Resolved::Player => self.w.selected_player().map(|p| p.yaw),
            // `Seen::yaw` is the body's, not the head's.
            Resolved::Seen(_) | Resolved::Dragon(_) => None,
        }
    }

    fn r_entity_bounding_box(&self, entity: EntityRef) -> Option<[[f64; 3]; 2]> {
        let bounds = match self.resolve(entity)? {
            Resolved::Own => self.m.bounds(),
            Resolved::Live(mob) => mob.bounds(),
            Resolved::Player => self.w.selected_player().map(|p| p.bounds)?,
            Resolved::Seen(seen) => seen.bounds,
            Resolved::Dragon(info) => info.bounds,
        };
        let axes = [Axis::X, Axis::Y, Axis::Z];
        Some([axes.map(|a| bounds.min(a)), axes.map(|a| bounds.max(a))])
    }

    fn r_entity_eye_height(&self, entity: EntityRef) -> Option<f64> {
        match self.resolve(entity)? {
            Resolved::Own => Some(self.m.eye_y() - self.m.position.y),
            Resolved::Live(mob) => Some(mob.eye_y() - mob.position.y),
            Resolved::Player => self.w.selected_player().map(|p| p.eye[1] - p.position.y),
            Resolved::Seen(seen) => Some(seen.eye_y - seen.position.y),
            Resolved::Dragon(info) => Some(info.eye_y - info.position.y),
        }
    }

    fn r_entity_eye_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        match self.resolve(entity)? {
            Resolved::Own => {
                let y = self.m.eye_y();
                Some([self.m.position.x, y, self.m.position.z])
            }
            Resolved::Player => self.w.selected_player().map(|p| p.eye),
            Resolved::Live(mob) => Some([mob.position.x, mob.eye_y(), mob.position.z]),
            Resolved::Seen(seen) => Some([seen.position.x, seen.eye_y, seen.position.z]),
            Resolved::Dragon(info) => Some([info.position.x, info.eye_y, info.position.z]),
        }
    }

    fn r_entity_distance_to_sqr(&self, entity: EntityRef, pos: [f64; 3]) -> Option<f64> {
        self.position(entity).map(|p| {
            (p.x - pos[0]).mul_add(p.x - pos[0], 0.0)
                + (p.y - pos[1]).mul_add(p.y - pos[1], 0.0)
                + (p.z - pos[2]).mul_add(p.z - pos[2], 0.0)
        })
    }

    fn r_entity_on_ground(&self, entity: EntityRef) -> Option<bool> {
        match self.resolve(entity)? {
            Resolved::Own => Some(self.m.on_ground),
            Resolved::Live(mob) => Some(mob.on_ground),
            // Not carried by the snapshot or a player's view.
            _ => None,
        }
    }

    fn r_entity_kind(&self, entity: EntityRef) -> Option<mod_api::EntityTypeKey> {
        match self.resolve(entity)? {
            // The snapshot carries the species, not its type id.
            Resolved::Own => mod_api::__private::resolved_entity_type_key(&self.m.kind.id),
            Resolved::Live(mob) => mod_api::__private::resolved_entity_type_key(&mob.kind.id),
            _ => None,
        }
    }

    fn r_living_health(&self, living: LivingRef) -> Option<f32> {
        match self.resolve(living.entity())? {
            Resolved::Own => Some(self.m.health),
            Resolved::Live(mob) => Some(mob.health),
            Resolved::Seen(seen) => Some(seen.health),
            // A player's view carries no health.
            _ => None,
        }
    }

    fn r_living_max_health(&self, living: LivingRef) -> Option<f32> {
        match self.resolve(living.entity())? {
            Resolved::Own => Some(self.m.attrs.max_health),
            Resolved::Live(mob) => Some(mob.attrs.max_health),
            // Not carried by the snapshot.
            _ => None,
        }
    }

    fn r_living_alive(&self, living: LivingRef) -> Option<bool> {
        match self.resolve(living.entity())? {
            Resolved::Own => Some(self.m.alive()),
            Resolved::Live(mob) => Some(mob.alive()),
            Resolved::Player => self.w.selected_player().map(|p| p.alive),
            Resolved::Seen(seen) => Some(seen.alive),
            Resolved::Dragon(info) => Some(info.alive),
        }
    }
}

impl GoalCxBackend for GoalCx<'_, '_> {
    fn host(&self) -> &dyn Host {
        self
    }

    fn mob(&self) -> MobRef {
        self.mob_handle()
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.m.random
    }

    fn move_to(&mut self, at: [f64; 3], speed: f64) -> bool {
        // The navigation handle's own boundary: numbers the pathfinder does
        // not take refuse here as they refuse there.
        if !super::native_mob_navigation::navigation_move_accepts(at, speed) {
            return false;
        }
        self.m.nav_move_to(self.w, at, speed)
    }

    fn stop_navigation(&mut self) {
        self.m.nav_stop();
    }

    fn look_at(&mut self, at: [f64; 3]) {
        // The look control's boundary, as the navigation handle takes it.
        if super::native_mob_navigation::navigation_look_accepts(at) {
            self.m.look_at(at);
        }
    }

    fn set_target(&mut self, target: Option<LivingRef>) -> bool {
        // The write handle's own setter refusals, the one rule it asks too
        // (#1175): the brain-backed species and the three whose setter side
        // effects are not modeled.
        if self.m.refuses_set_target() {
            return false;
        }
        let who = match target {
            None => None,
            Some(living) => match self.resolve(living.entity()) {
                Some(Resolved::Own) => {
                    // The write handle applies its rules to any target, the
                    // mob itself included: the same facts, from the mob in
                    // hand.
                    if !self.m.valid_target(
                        self.w,
                        &mobs::goals::TargetFacts::Mob {
                            id: self.m.id,
                            species: self.m.species(),
                            alive: self.m.alive(),
                            invulnerable: self.m.invulnerable_flag,
                            baby: self.m.is_baby(),
                        },
                    ) {
                        return false;
                    }
                    Some(Who::Mob(self.m.id))
                }
                Some(Resolved::Player) => {
                    // The write handle's player rules over the tick's view,
                    // through the same valid-target rule set (#1175).
                    let Some(p) = self.w.selected_player() else {
                        return false;
                    };
                    if !self.m.valid_target(
                        self.w,
                        &mobs::goals::TargetFacts::Player {
                            alive: p.alive,
                            invulnerable: p.invulnerable,
                            creative: p.creative,
                            spectator: p.spectator,
                            uuid: p.uuid,
                        },
                    ) {
                        return false;
                    }
                    Some(Who::Player)
                }
                Some(Resolved::Seen(seen)) => {
                    // `asValidTarget` over the snapshot's facts of the target,
                    // the same rule set the write handle asks.
                    if !self.m.valid_target(
                        self.w,
                        &mobs::goals::TargetFacts::Mob {
                            id: seen.id,
                            species: seen.species,
                            alive: seen.alive,
                            invulnerable: seen.invulnerable,
                            baby: seen.baby,
                        },
                    ) {
                        return false;
                    }
                    Some(Who::Mob(seen.id))
                }
                Some(Resolved::Live(mob)) => {
                    if !self.m.valid_target(
                        self.w,
                        &mobs::goals::TargetFacts::Mob {
                            id: mob.id,
                            species: mob.species(),
                            alive: mob.alive(),
                            invulnerable: mob.invulnerable_flag,
                            baby: mob.is_baby(),
                        },
                    ) {
                        return false;
                    }
                    Some(Who::Mob(mob.id))
                }
                Some(Resolved::Dragon(_)) | None => return false,
            },
        };
        matches!(
            self.m.set_ai_target(self.w, who),
            mobs::TargetChange::Set(_)
        )
    }

    fn jump(&mut self) {
        // `JumpControl.jump`: the wish the control consumes on the mob's next
        // own tick (it assigns `jumping` from `jump` there), as the mob jump
        // handle writes it.
        self.m.ai.jump = true;
    }
}

impl BehaviorCxBackend for GoalCx<'_, '_> {
    fn host(&self) -> &dyn Host {
        self
    }

    fn mob(&self) -> MobRef {
        self.mob_handle()
    }

    fn walk_target(&self) -> Option<mod_api::BrainWalkTarget> {
        mobs::mod_behaviors::walk_target(self.m, self.w)
    }

    fn look_target(&self) -> Option<[f64; 3]> {
        mobs::mod_behaviors::look_target(self.m, self.w)
    }

    fn attack_target(&self) -> Option<LivingRef> {
        // The brain's own target read through the engine's path, the handle
        // resolved in the tick's view.
        let who = mobs::ai::brain_target(self.m)??;
        let host = self.w.goal_host?;
        let epoch = host.epoch()?;
        let id = match who {
            Who::Mob(id) => id,
            Who::Player => self.w.player_identity?.1,
        };
        let entity = EntityRef::new(epoch, id);
        self.resolve(entity)?;
        Some(LivingRef::new(entity))
    }

    fn set_brain_target(&mut self, target: Option<LivingRef>) -> bool {
        // The write handle's own setter rules, over the tick's view: the mob
        // itself, the player it sees, or another live/snapshot mob.
        let who = match target {
            None => None,
            Some(living) => match self.resolve(living.entity()) {
                Some(Resolved::Own) => Some(Who::Mob(self.m.id)),
                Some(Resolved::Player) => Some(Who::Player),
                Some(Resolved::Seen(seen)) => Some(Who::Mob(seen.id)),
                Some(Resolved::Live(mob)) => Some(Who::Mob(mob.id)),
                Some(Resolved::Dragon(_)) | None => return false,
            },
        };
        mobs::ai::set_brain_target(self.m, who)
    }

    fn set_walk_target(&mut self, at: [i32; 3], speed: f64, close: i32) {
        mobs::mod_behaviors::set_walk_target(self.m, at, speed, close)
    }
}

impl mobs::part_goals::GoalHost for Game<'_> {
    fn epoch(&self) -> Option<mod_api::WorldEpoch> {
        self.native_lifecycle.epoch()
    }

    fn merchant_is_live(&self, mob: &Mob) -> bool {
        self.is_logical_server() && !mob.removed
    }

    fn merchant_offers_lent(&self, mob: &Mob) -> Option<Vec<mod_api::MerchantOffer>> {
        use super::api_values::TryIntoApi;
        self.merchant_read_lent(mob, |offers, _, _| offers.try_into_api().ok())
    }

    fn merchant_trading_player_lent(&self, mob: &Mob) -> Option<mod_api::PlayerRef> {
        let conn = self.lan_trade_conn(mob.id)?;
        self.player(self.lan_trade_player(conn)?)
    }

    fn merchant_trade_state_lent(&self, mob: &Mob) -> (i32, i32) {
        self.merchant_read_lent(mob, |_, xp, level| (xp, level))
    }

    fn player_passenger_seats(&self, vehicle: u32) -> Vec<(i64, u32)> {
        self.seated_passengers(vehicle)
            .into_iter()
            .filter_map(|(order, id, player)| player.then_some((order, id)))
            .collect()
    }

    fn mob_dismount_location(
        &self,
        vehicle: Option<&Mob>,
        vehicle_id: u32,
        rider: Option<&Mob>,
        passenger: EntityRef,
    ) -> Option<[f64; 3]> {
        self.scoped_mob_dismount_location(vehicle, vehicle_id, rider, passenger)
    }

    fn mob_player_controls(&self, mob: &Mob, player: u32) -> bool {
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return false;
        };
        let Some(target) = self.native_entity_target(EntityRef::new(epoch, player)) else {
            return false;
        };
        matches!(
            target,
            super::commands::Target::Player | super::commands::Target::Guest(_)
        ) && self.native_player_controls(mob, target)
    }

    fn mob_tags(&self, mob: &Mob) -> Option<Vec<String>> {
        if self.remote.is_some() {
            return None;
        }
        Some(self.tags_of(super::commands::Target::Mob(mob.id)))
    }

    fn mob_name(&self, mob: &Mob, actor: &Mob, world: &MobWorld<'_>) -> String {
        let name = match self.mob_team_scoped(mob, actor, world) {
            Some(team) => team.formatted_name(mob.command_name()),
            None => mob.command_name(),
        };
        name.plain()
    }

    fn mob_added_to_level(&self, mob: &Mob) -> bool {
        self.persist
            .live
            .binary_search_by_key(&mob.id, |entry| entry.0)
            .is_ok()
    }

    fn mob_portal_cooldown(&self, mob: &Mob) -> i32 {
        self.portals.entities.get(&mob.id).map_or(0, |p| p.cooldown)
    }

    fn mob_invulnerable_to_damage(&self, mob: &Mob, damage_type: &str) -> bool {
        self.mob_query_invulnerable(mob, damage_type)
    }

    fn mob_can_use_portal(&self, mob: &Mob) -> bool {
        self.mob_portal_eligible(mob)
    }

    fn mob_is_climbing(&self, mob: &Mob) -> bool {
        self.native_mob_is_climbing(mob)
    }

    fn mob_player_credit(&self, mob: &Mob) -> Option<Option<u32>> {
        let credit = *self.last_hurt_by_player.get(&mob.id)?;
        Some(
            self.resolve_player_credit(credit)
                .and_then(|conn| self.player_entity_id_by_connection(conn))
                .and_then(|id| u32::try_from(id).ok()),
        )
    }

    fn mob_target(&self, mob: &Mob, actor: &Mob, world: &MobWorld<'_>) -> Option<LivingRef> {
        let target = mobs::ai::brain_target(mob).unwrap_or(mob.ai.target)?;
        match target {
            Who::Player => {
                // Preserve the game's per-mob guest binding: the selected
                // player for this tick need not be this mob's own target. A
                // listener's redirect, however, already changed the tick's
                // selection while the persisted binding is still borrowed.
                let redirected = world
                    .player_selection
                    .filter(|selection| mob.id == actor.id && selection.has_switched())
                    .and_then(|selection| selection.selected())
                    .map(|player| player.entity_id);
                let id = match redirected {
                    Some(id) => id,
                    None => {
                        let host = self.lan.as_ref()?;
                        let conn = host.mob_players.get(&mob.id)?;
                        u32::try_from(host.hub.guest(*conn)?.entity_id).ok()?
                    }
                };
                let entity = self.entity(id)?;
                self.native_mob_valid_target(mob, self.native_entity_target(entity)?)
                    .then(|| self.living(entity))?
            }
            Who::Mob(id) => {
                // Mob tick temporarily lends the list, so the regular game
                // lookup cannot see it. The disjoint loan holds live mobs.
                let other = if id == mob.id {
                    mob
                } else {
                    world.live_mobs?.find(id)?
                };
                if other.removed {
                    return None;
                }
                let valid = if mob.species() == mobs::Species::Warden {
                    let bounds = other.bounds();
                    let border = self.border.geometry();
                    other.alive()
                        && !other.invulnerable_flag
                        && other.species() != mobs::Species::Warden
                        && !horizon_client::scoreboard::teams_allied(
                            self.mob_team_scoped(mob, actor, world),
                            self.mob_team_scoped(other, actor, world),
                        )
                        && border.is_within_bounds(bounds.min(Axis::X), bounds.min(Axis::Z), 0.0)
                        && border.is_within_bounds(
                            bounds.max(Axis::X) - f64::from(1.0e-5_f32),
                            bounds.max(Axis::Z) - f64::from(1.0e-5_f32),
                            0.0,
                        )
                } else {
                    mob.valid_target(
                        world,
                        &mobs::goals::TargetFacts::Mob {
                            id,
                            species: other.species(),
                            alive: other.alive(),
                            invulnerable: other.invulnerable_flag,
                            baby: other.is_baby(),
                        },
                    )
                };
                let epoch = self.native_lifecycle.epoch()?;
                valid.then(|| LivingRef::new(EntityRef::new(epoch, id)))
            }
        }
    }

    fn mob_on_block(&self, mob: &Mob) -> [i32; 3] {
        let support = mob
            .on_ground
            .then(|| self.supporting_block(mob.position, mob.bounds()))
            .flatten();
        super::native_entity_motion::on_block_position(support, mob.position)
    }

    fn entities_team_damage_allowed(
        &self,
        entity: EntityRef,
        entity_mob: Option<&Mob>,
        source: EntityRef,
        source_mob: Option<&Mob>,
        actor: &Mob,
        world: &MobWorld<'_>,
    ) -> Option<bool> {
        let own = match entity_mob {
            Some(mob) => self.mob_team_scoped(mob, actor, world),
            None => self.native_target_team(self.native_entity_target(entity)?)?,
        };
        let other = match source_mob {
            Some(mob) => self.mob_team_scoped(mob, actor, world),
            None => self.native_target_team(self.native_entity_target(source)?)?,
        };
        Some(horizon_client::scoreboard::teams_allow_damage(own, other))
    }

    fn run_goal(
        &self,
        goal: &mut Box<dyn mod_api::Goal>,
        m: &mut Mob,
        w: &MobWorld<'_>,
        op: GoalOp,
        mod_id: &'static str,
        part: &'static str,
    ) -> Option<bool> {
        let call = match op {
            GoalOp::CanUse => "goal_can_use",
            GoalOp::CanContinue => "goal_can_continue",
            GoalOp::Start => "goal_start",
            GoalOp::Stop => "goal_stop",
            GoalOp::Tick => "goal_tick",
        };
        let mut gcx = GoalCx { m, w, host: self };
        let mut context = mod_api::GoalContext::new(&mut gcx);
        let _guard = enter_native_mod(ModCall {
            id: mod_id,
            hook: call,
            part: Some(part),
        });
        Some(match op {
            GoalOp::CanUse => goal.can_use(&mut context),
            GoalOp::CanContinue => goal.can_continue(&mut context),
            GoalOp::Start => {
                goal.start(&mut context);
                true
            }
            GoalOp::Stop => {
                goal.stop(&mut context);
                true
            }
            GoalOp::Tick => {
                goal.tick(&mut context);
                true
            }
        })
    }

    fn run_behavior(
        &self,
        behavior: &mut Box<dyn mod_api::Behavior>,
        m: &mut Mob,
        w: &MobWorld<'_>,
        op: BehaviorOp,
        mod_id: &'static str,
        part: &'static str,
    ) -> Option<bool> {
        let call = match op {
            BehaviorOp::CheckExtraStartConditions => "behavior_check_extra_start_conditions",
            BehaviorOp::Start => "behavior_start",
            BehaviorOp::CanStillUse => "behavior_can_still_use",
            BehaviorOp::Tick => "behavior_tick",
            BehaviorOp::Stop => "behavior_stop",
        };
        // The same view a goal runs on (#824): one boundary, the brain's
        // memories typed on top.
        let mut gcx = GoalCx { m, w, host: self };
        let mut context = mod_api::BehaviorCx::new(&mut gcx);
        let _guard = enter_native_mod(ModCall {
            id: mod_id,
            hook: call,
            part: Some(part),
        });
        Some(match op {
            BehaviorOp::CheckExtraStartConditions => {
                behavior.check_extra_start_conditions(&mut context)
            }
            BehaviorOp::Start => {
                behavior.start(&mut context);
                true
            }
            BehaviorOp::CanStillUse => behavior.can_still_use(&mut context),
            BehaviorOp::Tick => {
                behavior.tick(&mut context);
                true
            }
            BehaviorOp::Stop => {
                behavior.stop(&mut context);
                true
            }
        })
    }
}

impl Game<'_> {
    /// Dispatch a target change against the mob tick's scoped read view. The
    /// game has lent its mob list to the tick, so using `Game` itself as the
    /// host here would make the actor and other mob handles unresolvable.
    pub(super) fn native_mob_change_target_in_tick(
        &self,
        mob: &mut Mob,
        world: &MobWorld<'_>,
        is_brain: bool,
        target: Option<Who>,
    ) -> mobs::TargetChange {
        let vanilla = mobs::TargetChange::Set(target);
        if !mod_api::__private::listened(mod_api::HookId::LivingChangeTarget) {
            return vanilla;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return vanilla;
        };
        let original = match target {
            None => None,
            Some(Who::Mob(id)) => Some(LivingRef::new(EntityRef::new(epoch, id))),
            Some(Who::Player) => {
                let Some((_, id)) = world.selected_player_identity() else {
                    return vanilla;
                };
                Some(LivingRef::new(EntityRef::new(epoch, id)))
            }
        };
        let entity = MobRef::new(EntityRef::new(epoch, mob.id));
        let context = GoalCx {
            m: mob,
            w: world,
            host: self,
        };
        let mut new_target = original;
        let mut cancel = false;
        super::native_hooks::NativeHookHost { host: &context }.native_living_change_target(
            mod_api::__private::LivingChangeTargetCall {
                mob: entity,
                original_target: original,
                new_target: &mut new_target,
                is_brain,
                cancel: &mut cancel,
            },
        );
        if cancel {
            return mobs::TargetChange::Keep;
        }
        if new_target == original {
            return vanilla;
        }
        let Some(target) = new_target else {
            return mobs::TargetChange::Set(None);
        };
        // A listener can redirect to another real player in this level.
        // Select that player's stable tick view before the caller continues,
        // so `Who::Player` and subsequent AI reads name the same player.
        if target.entity().epoch() == epoch
            && let Some(selection) = world.player_selection
            && selection.get(target.entity().id()).is_some()
            && let Some(player) = self.native_entity_target(target.entity())
            && self.native_player_facts(player).is_some()
            && self.native_mob_valid_target(context.m, player)
            && selection.select(target.entity().id())
        {
            return mobs::TargetChange::Set(Some(Who::Player));
        }
        match context.resolve(target.entity()) {
            Some(Resolved::Player) => mobs::TargetChange::Set(Some(Who::Player)),
            Some(Resolved::Own) => mobs::TargetChange::Set(Some(Who::Mob(context.m.id))),
            Some(Resolved::Live(mob)) => mobs::TargetChange::Set(Some(Who::Mob(mob.id))),
            Some(Resolved::Seen(seen)) => mobs::TargetChange::Set(Some(Who::Mob(seen.id))),
            Some(Resolved::Dragon(_)) | None => mobs::TargetChange::Keep,
        }
    }
}

pub(super) mod generated {
    // The mob tick's read view (#1077), as the game generates it: the
    // snapshot-backed reads forward to the context's own methods, every other
    // backend read answers its typed refusal.
    include!(concat!(env!("OUT_DIR"), "/mod_api_goal_view_generated.rs"));
}

#[cfg(test)]
#[path = "native_goal_ticks_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "native_behavior_tests.rs"]
mod behavior_tests;
