//! Vehicles and passengers for native server mods, through an `EntityRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387): what an entity
//! rides, who rides it, getting off, and who holds its lead. Players ride
//! boats, minecarts and mounts; mobs ride mobs, boats and minecarts. Reads
//! return `None` for a stale handle or on a client.
use super::{Game, commands::Target, mounts::steering_item};
use horizon_client::{
    lan::playerdata::ints_uuid,
    mobs::{Mob, Species, equine, happy_ghast, leash::Holder, nautilus},
};
use mod_api::{EntityRef, LeashHolder, LivingRef};

impl Game<'_> {
    /// The dismount search with a vehicle or passenger lent out of `self.mobs`.
    pub(super) fn scoped_mob_dismount_location(
        &self,
        vehicle: Option<&Mob>,
        vehicle_id: u32,
        rider: Option<&Mob>,
        passenger: EntityRef,
    ) -> Option<[f64; 3]> {
        let at = if let Some(rider) = rider {
            if let Some(boat) = self.boats.iter().find(|boat| boat.id == vehicle_id) {
                Some(self.boat_dismount_target(boat, f64::from(rider.width()), rider.yaw))
            } else if let Some(mount) = vehicle {
                Some(
                    self.horse_dismount_mobs(mount, rider)
                        .unwrap_or_else(|| super::mounts::on_top(mount)),
                )
            } else {
                self.cart_dismount_mob(vehicle_id, rider)
            }
        } else if let Some(mount) = vehicle {
            match self.native_entity_target(passenger)? {
                Target::Player => {
                    Some(self.player_dismount_from_lent_mob(mount, self.yaw, self.player.as_ref()?))
                }
                Target::Guest(conn) => {
                    let guest = self.command_guest(conn)?;
                    Some(self.player_dismount_from_lent_mob(
                        mount,
                        guest.yaw,
                        guest.motion.as_deref()?,
                    ))
                }
                _ => None,
            }
        } else {
            let vehicle = self.entity(vehicle_id)?;
            return self.entity_dismount_location(vehicle, LivingRef::new(passenger));
        };
        if let Some(at) = at {
            return Some([at.x, at.y, at.z]);
        }
        let vehicle = self.entity(vehicle_id)?;
        let [x, _, z] = self.entity_position(vehicle)?;
        let [_, [_, top, _]] = self.entity_bounding_box(vehicle)?;
        Some([x, top, z])
    }

    /// The id of the entity `t` rides, if it rides one.
    fn native_vehicle_id(&self, t: Target) -> Option<u32> {
        match t {
            Target::Player => self.riding,
            Target::Guest(conn) => self.command_guest(conn)?.vehicle,
            Target::Mob(id) => self.mob_object(id).filter(|m| !m.removed)?.extra.mount,
            // A general row rides the row its tree loaded with; a row queued
            // for removal rides nothing, as a removed mob does not.
            Target::General(id) => self
                .resident_entities
                .general
                .rows
                .iter()
                .find(|row| row.id == id && !self.persist.removals.contains(&Target::General(id)))
                .and_then(|row| row.mount),
            _ => None,
        }
    }
    /// The entity's vehicle as a handle, if it rides one in this level.
    fn native_vehicle(&self, entity: EntityRef) -> Option<Option<EntityRef>> {
        let t = self.native_entity_target(entity)?;
        Some(self.native_vehicle_id(t).and_then(|v| self.entity(v)))
    }
    /// What the entity rides (`Entity.getVehicle`): `None` when it rides
    /// nothing, its actual vehicle is off-level, or the input handle is stale.
    pub(super) fn entity_vehicle(&self, entity: EntityRef) -> Option<EntityRef> {
        self.native_vehicle(entity).flatten()
    }
    /// Whether the entity rides another object, including a retained off-level vehicle.
    pub(super) fn entity_is_passenger(&self, entity: EntityRef) -> Option<bool> {
        Some(
            self.native_vehicle_id(self.native_entity_target(entity)?)
                .is_some(),
        )
    }
    /// The lowest vehicle under the entity (`Entity.getRootVehicle`): the
    /// entity itself when it rides nothing. Off-level intermediaries are traversed;
    /// an off-level root or a cycle returns no handle, never a descendant substitute.
    pub(super) fn entity_root_vehicle(&self, entity: EntityRef) -> Option<EntityRef> {
        let target = self.native_entity_target(entity)?;
        self.entity(self.native_root_vehicle_id(entity.id(), target)?)
    }

    /// Follow full object links, returning no root for a cycle. Only mobs can
    /// be intermediate vehicles. Their object count plus the non-Mob endpoints
    /// bounds the walk without truncating valid trees or allocating a visited set.
    pub(super) fn native_root_vehicle_id(&self, id: u32, target: Target) -> Option<u32> {
        let mut root = id;
        let mut next = self.native_vehicle_id(target);
        for _ in 0..=self.mobs.len() + self.persist.off_level.len() + 1 {
            let Some(vehicle) = next else {
                return Some(root);
            };
            root = vehicle;
            next = self.native_vehicle_id(Target::Mob(vehicle));
        }
        None
    }
    /// A live portal root carries a direct or indirect mount through complete objects.
    pub(super) fn vehicle_tree_contains_mount(&self, root: u32, mount: u32) -> bool {
        self.entity(root).is_some()
            && (self.mob_object(mount).is_some_and(|mob| !mob.removed)
                || self.entity(mount).is_some())
            && self.native_root_vehicle_id(mount, Target::Mob(mount)) == Some(root)
    }

    /// The entities riding this one (`Entity.getPassengers`), in `addPassenger`'s order.
    pub(super) fn entity_passengers(&self, entity: EntityRef) -> Option<Vec<EntityRef>> {
        self.native_entity_target(entity)?;
        let seats = self.seated_passengers(entity.id()).into_iter();
        Some(seats.filter_map(|(_, id, _)| self.entity(id)).collect())
    }
    /// Whether any object rides the entity, including retained off-level passengers.
    pub(super) fn entity_is_vehicle(&self, entity: EntityRef) -> Option<bool> {
        self.native_entity_target(entity)?;
        Some(!self.seated_passengers(entity.id()).is_empty())
    }
    /// Every entity riding this one, at any depth (`Entity.getIndirectPassengers`): each
    /// passenger in boarding order, followed by those riding it. Traverse off-level
    /// intermediaries but return only resident handles.
    pub(super) fn entity_indirect_passengers(&self, entity: EntityRef) -> Option<Vec<EntityRef>> {
        self.native_entity_target(entity)?;
        let mut all: Vec<EntityRef> = Vec::new();
        // This vector allocates only when a retained, unrepresentable object is
        // actually encountered; ordinary resident-only trees keep two buffers.
        let mut visited_off_level = Vec::new();
        let mut next: Vec<u32> = self
            .seated_passengers(entity.id())
            .into_iter()
            .rev()
            .map(|(_, id, _)| id)
            .collect();
        while let Some(passenger) = next.pop() {
            if passenger == entity.id()
                || all.iter().any(|r| r.id() == passenger)
                || visited_off_level.contains(&passenger)
            {
                continue;
            }
            if let Some(handle) = self.entity(passenger) {
                all.push(handle);
            } else {
                visited_off_level.push(passenger);
            }
            next.extend(
                self.seated_passengers(passenger)
                    .into_iter()
                    .rev()
                    .map(|(_, id, _)| id),
            );
        }
        Some(all)
    }
    /// Whether the player `t` holds the stick that steers `species` in either hand.
    fn native_player_steers(&self, species: Species, t: Target) -> bool {
        let (main, offhand) = match t {
            Target::Player => (self.held(), self.offhand.as_ref()),
            Target::Guest(conn) => {
                let Some(guest) = self.command_guest(conn) else {
                    return false;
                };
                (
                    guest.inventory.get(guest.selected).and_then(Option::as_ref),
                    guest.offhand.as_ref(),
                )
            }
            _ => return false,
        };
        steering_item(
            species,
            main.map(|s| s.item.as_ref()),
            offhand.map(|s| s.item.as_ref()),
        )
    }
    /// The overrides of `getControllingPassenger` on `mob`: whether the player `rider`, its
    /// first passenger, steers it.
    pub(super) fn native_player_controls(&self, mob: &Mob, rider: Target) -> bool {
        let species = mob.species();
        match species {
            Species::Pig | Species::Strider => {
                mob.extra.saddled() && self.native_player_steers(species, rider)
            }
            Species::HappyGhast => {
                happy_ghast::wearing_harness(mob) && !happy_ghast::on_still_timeout(mob)
            }
            s if equine::is_equine(s) || nautilus::is_nautilus(s) => mob.extra.saddled(),
            _ => false,
        }
    }
    /// The first passenger, when it steers the entity (`Entity.getControllingPassenger`): a
    /// player in a boat, a player on a mount that is ready for one, or a mob on a mob with AI
    /// unless `#non_controlling_rider` names it. `None` when nothing steers it, or for a stale
    /// handle.
    pub(super) fn entity_controlling_passenger(&self, entity: EntityRef) -> Option<LivingRef> {
        let vehicle = self.native_entity_target(entity)?;
        let first_id = self.seated_passengers(entity.id()).first()?.1;
        let first = self.entity(first_id)?;
        let rider = self.native_entity_target(first)?;
        let is_player = matches!(rider, Target::Player | Target::Guest(_));
        match vehicle {
            Target::Boat(_) if is_player => Some(LivingRef::new(first)),
            Target::Mob(id) => {
                let mob = self.mobs.iter().find(|m| m.id == id)?;
                if is_player {
                    return self
                        .native_player_controls(mob, rider)
                        .then(|| LivingRef::new(first));
                }
                let rider = self.mobs.iter().find(|m| m.id == first.id())?;
                let tagged = self
                    .res
                    .mob_data()
                    .entity_has("minecraft:non_controlling_rider", &rider.kind.id);
                (!mob.no_ai && !tagged).then(|| LivingRef::new(first))
            }
            _ => None,
        }
    }
    /// Whether both entities share a root vehicle (`Entity.isPassengerOfSameVehicle`): true for
    /// an entity and itself, false for two entities riding nothing. `None` for a stale handle.
    pub(super) fn entity_is_passenger_of_same_vehicle(
        &self,
        entity: EntityRef,
        other: EntityRef,
    ) -> Option<bool> {
        let target = self.native_entity_target(entity)?;
        let other_target = self.native_entity_target(other)?;
        Some(
            self.native_root_vehicle_id(entity.id(), target)?
                == self.native_root_vehicle_id(other.id(), other_target)?,
        )
    }
    /// Where getting off this vehicle puts `passenger` as it stands now, riding it or not. This
    /// is the game's own placement, from the code `stop_riding` runs, so it can differ from
    /// `Entity.getDismountLocationForPassenger`. A player leaves a boat ahead of them or on top
    /// of it, a cart beside or on top of it, a strider by vanilla's search, a happy ghast on
    /// top, and any other mob beside or on top of it. A mob leaves a boat as a player does, a
    /// cart by vanilla's search, a horse, camel or llama at its side or feet, and any other mob
    /// on top: vanilla's searches for `Animal` and the strider don't reach mobs yet, and those
    /// for horses, `Animal` and the cart don't reach players. Nothing rides anything else, so
    /// there it is the top of the box. Reads only: no `mob_ride` or dismount veto is asked and
    /// the passenger's pose stays. `None` for a stale handle, a passenger other than a player
    /// or a mob, a player with no motion state, or a vehicle whose box Horizon does not keep.
    pub(super) fn entity_dismount_location(
        &self,
        vehicle: EntityRef,
        passenger: LivingRef,
    ) -> Option<[f64; 3]> {
        self.native_entity_target(vehicle)?;
        let id = vehicle.id();
        let at = match self.native_entity_target(passenger.entity())? {
            Target::Mob(rider) => self.mob_dismount_point(id, rider),
            Target::Player => self.player_dismount_point(id, self.yaw, self.player.as_ref()?),
            Target::Guest(conn) => {
                let guest = self.command_guest(conn)?;
                self.player_dismount_point(id, guest.yaw, guest.motion.as_deref()?)
            }
            _ => return None,
        };
        if let Some(at) = at {
            return Some([at.x, at.y, at.z]);
        }
        let [x, _, z] = self.entity_position(vehicle)?;
        let [_, [_, top, _]] = self.entity_bounding_box(vehicle)?;
        Some([x, top, z])
    }
    /// A lead's live holder as a handle: `None` when it is out of reach.
    fn native_leash_holder(
        &self,
        holder: Option<Holder>,
        player: Option<[i32; 4]>,
    ) -> Option<LeashHolder> {
        Some(match holder {
            None => LeashHolder::Unleashed,
            Some(Holder::Knot(pos)) => LeashHolder::Knot(pos),
            Some(Holder::Mob(id)) => {
                let entity = self.entity(id)?;
                if !matches!(self.native_entity_target(entity), Some(Target::Mob(_))) {
                    return None;
                }
                LeashHolder::Entity(entity)
            }
            Some(Holder::Boat(id)) => {
                let entity = self.entity(id)?;
                if !matches!(self.native_entity_target(entity), Some(Target::Boat(_))) {
                    return None;
                }
                LeashHolder::Entity(entity)
            }
            Some(Holder::Player) => LeashHolder::Entity(self.player(ints_uuid(player?))?.entity()),
        })
    }
    /// The live holder of a mob's or boat's lead (`Leashable.getLeashHolder`), or `Unleashed`
    /// when no lead exists. `None` means an entity that takes no lead, or that the entity,
    /// holder or saved holder is unavailable, stale or in a parked level. Does not load a level.
    pub(super) fn entity_leash_holder(&self, entity: EntityRef) -> Option<LeashHolder> {
        match self.native_entity_target(entity)? {
            Target::Mob(id) => {
                let mob = self.mobs.iter().find(|m| m.id == id)?;
                if mob.extra.leash_pending.is_some() {
                    return None;
                }
                self.native_leash_holder(mob.extra.leash, mob.extra.leash_player)
            }
            Target::Boat(id) => {
                let boat = self.boats.iter().find(|b| b.id == id)?;
                if boat.leash_pending.is_some() {
                    return None;
                }
                self.native_leash_holder(boat.leash, self.boat_leash_player(boat))
            }
            _ => None,
        }
    }
    /// Gives the holder the mob's or boat's lead (`Leashable.setLeashedTo`): the entity lets go
    /// of any other holder and gets off its vehicle, where a mod's `mob_ride` lets it. As
    /// there, neither `canBeLeashed` nor the distance is asked: a holder too far away snaps
    /// the lead on the next tick. False for a stale handle, the entity itself, a holder other
    /// than a player, mob or boat, or an entity that takes no lead.
    pub(super) fn entity_leash_to(&mut self, entity: EntityRef, holder: EntityRef) -> bool {
        let (Some(target), Some(by)) = (
            self.native_entity_target(entity),
            self.native_entity_target(holder),
        ) else {
            return false;
        };
        if entity == holder {
            return false;
        }
        let (holder, player) = match by {
            Target::Mob(id) => (Holder::Mob(id), None),
            Target::Boat(id) => (Holder::Boat(id), None),
            Target::Player | Target::Guest(_) => match self.native_player_facts(by) {
                Some(facts) => (Holder::Player, Some(facts.uuid)),
                None => return false,
            },
            _ => return false,
        };
        self.native_set_leash(entity, target, holder, player)
    }
    /// Ties the mob's or boat's lead to the fence at `pos`, as `entity_leash_to` does to
    /// an entity; the knot is there while a lead hangs on it. False for a stale handle, an
    /// entity that takes no lead, or no fence at `pos`.
    pub(super) fn entity_leash_to_knot(&mut self, entity: EntityRef, pos: [i32; 3]) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        let block = self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]));
        if !self.named_block_in_tag("minecraft:fences", &block.name) {
            return false;
        }
        self.native_set_leash(entity, target, Holder::Knot(pos), None)
    }
    /// `Leashable.setLeashedTo` from a handle: the lead first, then off the vehicle.
    fn native_set_leash(
        &mut self,
        entity: EntityRef,
        target: Target,
        holder: Holder,
        player: Option<[i32; 4]>,
    ) -> bool {
        let leashed = self.set_leash(target, holder, player);
        if leashed {
            self.entity_stop_riding(entity);
        }
        leashed
    }
    /// Takes the lead off a mob or boat: the lead falls as an item with `drop_lead`
    /// (`Leashable.dropLeash`), and is gone without it (`removeLeash`). A mob's removal is heard
    /// by `entity_unleash`. False when no holder has the lead, for a stale handle, or for an
    /// entity that takes no lead. A holder still unresolved from a save is left alone.
    pub(super) fn entity_detach_leash(&mut self, entity: EntityRef, drop_lead: bool) -> bool {
        self.native_entity_target(entity)
            .is_some_and(|target| self.detach_leash(target, drop_lead))
    }
    /// Attempts one mount, shared by handle writes and the ride command.
    pub(super) fn start_riding_target(
        &mut self,
        rider: Target,
        vehicle: Target,
        force: bool,
    ) -> bool {
        if let Target::Guest(conn) = rider {
            if self
                .command_guest(conn)
                .is_none_or(|guest| guest.dimension != self.dim.id())
            {
                return false;
            }
            return self
                .for_player(rider, |game| {
                    game.start_riding_target(Target::Player, vehicle, force)
                })
                .unwrap_or(false);
        }
        let Some(vehicle_id) = self.native_riding_preconditions(rider, vehicle, force) else {
            return false;
        };
        let previous_vehicle = self.native_vehicle_id(rider);
        let allowed = match rider {
            Target::Player => self.native_may_board(vehicle_id),
            Target::Mob(id) => self.native_mob_may_ride(id, vehicle_id, true),
            _ => false,
        };
        if !allowed
            || self.native_vehicle_id(rider) != previous_vehicle
            || self
                .native_riding_preconditions(rider, vehicle, force)
                .is_none()
        {
            return false;
        }
        if previous_vehicle.is_some() {
            match rider {
                Target::Player => self.stop_riding(),
                Target::Mob(id) => {
                    if previous_vehicle.is_some_and(|v| {
                        self.persist.mount_notifications.contains(&(id, v, false))
                            && self.persist.removals.iter().any(|target| {
                                matches!(*target, Target::Boat(n) | Target::Cart(n) if n == v)
                            })
                    }) {
                        // This explicit callback mutation completes the in-flight edge,
                        // rather than recursively notifying it a second time.
                        self.dismount_mob_reentrant(id);
                    } else {
                        self.dismount_mob(id);
                    }
                }
                _ => return false,
            }
            if self.native_vehicle_id(rider).is_some() {
                return false;
            }
        }
        // Dismount dispatch can also discard, remount or fill the target vehicle.
        if self
            .native_riding_preconditions(rider, vehicle, force)
            .is_none()
        {
            return false;
        }
        match rider {
            Target::Player => {
                match vehicle {
                    // `ServerPlayer.startRiding` teleports with `Relative.ROTATION`: the
                    // rider keeps their view, whatever the vehicle's yaw.
                    Target::Boat(_) => {}
                    Target::Mob(id) => {
                        if let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) {
                            mob.extra.flag = false;
                            mob.extra.counter = 0;
                        }
                        self.mount.jump_ticks = 0;
                        self.mount.jump_scale = 0.0;
                    }
                    Target::Stand(id) | Target::Other(id) => {
                        let Some(seat) = self.seat(id) else {
                            return false;
                        };
                        self.board_seat(id, seat);
                        return self.native_vehicle_id(rider) == Some(vehicle_id);
                    }
                    _ => {}
                }
                self.riding = Some(vehicle_id);
                self.seat_acting_player(vehicle_id);
                self.trigger_player("minecraft:started_riding");
            }
            Target::Mob(id) => {
                horizon_client::mobs::riders::link(&mut self.mobs, id, vehicle_id);
                self.seat_mob(id);
                // `Mob.startRiding`: `dropLeash()`, before the rider is seated.
                self.break_leash(id, true, false);
            }
            _ => return false,
        }
        self.native_vehicle_id(rider) == Some(vehicle_id)
    }

    /// Validate the captured relationship at each callback boundary.
    fn native_riding_preconditions(
        &self,
        rider: Target,
        vehicle: Target,
        force: bool,
    ) -> Option<u32> {
        let vehicle_id = match vehicle {
            Target::Mob(id)
            | Target::Boat(id)
            | Target::Cart(id)
            | Target::Stand(id)
            | Target::Other(id) => id,
            _ => return None,
        };
        if self.entity(vehicle_id).is_none()
            || matches!(rider, Target::Mob(id) if self.mobs.iter().all(|m| m.id != id || m.removed))
            || !matches!(rider, Target::Player | Target::Mob(_))
            || rider == vehicle
            || self.native_vehicle_id(rider) == Some(vehicle_id)
        {
            return None;
        }
        // Intermediate vehicles can be retained objects, not ordinary handles.
        let mut current = Some(vehicle_id);
        for _ in 0..=self.mobs.len() + self.persist.off_level.len() + 1 {
            let Some(id) = current else { break };
            if rider == Target::Mob(id) {
                return None;
            }
            current = self.native_vehicle_id(Target::Mob(id));
        }
        if current.is_some() {
            return None;
        }
        if !force {
            match rider {
                Target::Player if self.keys.held(horizon_client::actions::Action::Sneak) => {
                    return None;
                }
                Target::Mob(id)
                    if self.mobs.iter().find(|m| m.id == id).is_none_or(|m| {
                        m.extra.boarding_cooldown > 0
                            || matches!(
                                m.species(),
                                horizon_client::mobs::Species::Warden
                                    | horizon_client::mobs::Species::Wither
                            )
                    }) =>
                {
                    return None;
                }
                _ => {}
            }
        }
        match vehicle {
            Target::Boat(id) => {
                let index = self.boats.iter().position(|b| b.id == id)?;
                if self.boat_passengers(id) >= self.boats[index].max_passengers()
                    || (!force && !self.boat_has_room(index))
                {
                    return None;
                }
            }
            Target::Cart(id) => {
                if !self.carts.iter().any(|cart| cart.id == id)
                    || self.riding == Some(id)
                    || self.ridden_by_guest(id)
                    || !self.seated_passengers(id).is_empty()
                {
                    return None;
                }
            }
            Target::Mob(id) => {
                let mob = self.mobs.iter().find(|m| m.id == id && m.alive())?;
                let seats = self.seated_passengers(id);
                let players = seats
                    .iter()
                    .filter(|(_, _, player)| *player)
                    .count()
                    .max(usize::from(mob.rider.is_some()));
                let passengers = players + seats.iter().filter(|(_, _, player)| !*player).count();
                if !force && !horizon_client::mobs::riders::can_add_passenger(mob, passengers) {
                    return None;
                }
            }
            Target::Stand(id) | Target::Other(id) => {
                if self.seat(id).is_none() || (!force && !self.seated_passengers(id).is_empty()) {
                    return None;
                }
            }
            _ => return None,
        }
        Some(vehicle_id)
    }

    /// Attempts to board a vehicle through the shared server mount path.
    ///
    /// Players and mobs can ride a mob, boat, minecart or generic seat entity. Returns false for
    /// unsupported entity pairs, stale or held-out handles, a full stored
    /// vehicle, a cycle, or a canceled mount or dismount. Normal mob capacity
    /// follows its species and is bypassed by force; minecarts store one passenger and boats store one
    /// mob and up to two passengers in total. Generic seats admit one passenger unless forced.
    /// `force` bypasses normal admission checks, not these storage limits.
    /// Player boarding cooldown and the strider's
    /// eye-in-lava admission check are not yet represented by the game.
    pub(super) fn entity_start_riding(
        &mut self,
        entity: EntityRef,
        vehicle: EntityRef,
        force: bool,
    ) -> bool {
        let Some(rider) = self.native_entity_target(entity) else {
            return false;
        };
        let Some(vehicle) = self.native_entity_target(vehicle) else {
            return false;
        };
        self.start_riding_target(rider, vehicle, force)
    }

    /// Gets the entity off its vehicle (`Entity.stopRiding`), where the
    /// vehicle lets it out, as `/ride ... dismount` does. False when it rode
    /// nothing, a mod's `mob_ride` hook kept it on, or for a stale handle.
    pub(super) fn entity_stop_riding(&mut self, entity: EntityRef) -> bool {
        let Some(t) = self.native_entity_target(entity) else {
            return false;
        };
        let Some(vehicle) = self.native_vehicle_id(t) else {
            return false;
        };
        match t {
            Target::Mob(id) => self.dismount_mob(id),
            Target::Player | Target::Guest(_) => {
                self.for_player(t, |g| g.stop_riding());
            }
            // A general row gets off by its mount link: the row keeps its
            // identity and place, and the level's next walk sees it as a
            // root (`Entity.stopRiding` clears the mount and nothing else).
            // The mount rewrite also retires the rows' rider index, which
            // the list's own write stamp does not see.
            Target::General(id) => {
                let Some(row) = self
                    .resident_entities
                    .general
                    .rows
                    .iter_mut()
                    .find(|row| row.id == id)
                else {
                    return false;
                };
                row.mount = None;
                self.resident_entities.general.mount_writes += 1;
            }
            _ => return false,
        }
        let stopped = self.native_vehicle_id(t).is_none();
        if stopped {
            self.release_removed_vehicle(vehicle);
        }
        stopped
    }
    /// Gets every passenger off the entity (`Entity.ejectPassengers`), last
    /// first as vanilla walks the list, so `mob_ride` hears them in its
    /// order. False when nothing rode it, or for a stale handle.
    pub(super) fn entity_eject_passengers(&mut self, entity: EntityRef) -> bool {
        let Some(passengers) = self.entity_passengers(entity) else {
            return false;
        };
        let mut any = false;
        for p in passengers.into_iter().rev() {
            any |= self.entity_stop_riding(p);
        }
        any
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Game, commands::Target};
    use horizon_client::chunk::Chunk;
    use horizon_client::mobs::Mob;
    use horizon_core::geometry::Vec3;
    use mod_api::{
        self, EntityMountEvent, EntityRef, EntityUnleashEvent, Flow, Hooks, LivingRef, ServerHost,
    };
    use std::sync::{Arc, Mutex};

    struct MountObserver(Arc<Mutex<Vec<bool>>>);
    #[mod_api::__subscriptions]
    impl Hooks for MountObserver {
        fn entity_mount(&self, _: &mut dyn ServerHost, event: &mut EntityMountEvent<'_>) -> Flow {
            self.0.lock().unwrap().push(event.mounting);
            if event.mounting {
                Flow::Continue
            } else {
                Flow::Cancel
            }
        }
    }

    #[derive(Debug, PartialEq)]
    struct SeatMountPayload {
        entity: u32,
        kind: String,
        vehicle: u32,
        vehicle_kind: String,
        mounting: bool,
    }

    struct SeatMountObserver {
        events: Arc<Mutex<Vec<SeatMountPayload>>>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for SeatMountObserver {
        fn entity_mount(&self, _: &mut dyn ServerHost, event: &mut EntityMountEvent<'_>) -> Flow {
            self.events.lock().unwrap().push(SeatMountPayload {
                entity: event.entity,
                kind: event.kind.into(),
                vehicle: event.vehicle,
                vehicle_kind: event.vehicle_kind.into(),
                mounting: event.mounting,
            });
            if self.cancel.load(std::sync::atomic::Ordering::SeqCst) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn generic_seat_mount_payloads_cancellation_and_capacity() {
        use horizon_client::entities::ArmorStand;
        use std::sync::atomic::{AtomicBool, Ordering};

        let events = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "seat_mount_observer",
            Box::new(SeatMountObserver {
                events: events.clone(),
                cancel: cancel.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                for (index, vehicle_kind) in [
                    "minecraft:armor_stand",
                    "minecraft:block_display",
                    "minecraft:item_display",
                    "minecraft:text_display",
                    "minecraft:marker",
                    "minecraft:interaction",
                ]
                .into_iter()
                .enumerate()
                {
                    let id = 200 + index as u32 * 3;
                    if vehicle_kind == "minecraft:armor_stand" {
                        game.armor_stands.push(ArmorStand::new(id, Vec3::ZERO, 0.0));
                    } else {
                        game.others.push(super::super::commands::OtherEntity {
                            attachment_lifetime: Default::default(),
                            knot_check: None,
                            eye: None,
                            join: Default::default(),
                            item_transformation: None,
                            interaction: None,
                            id,
                            kind: vehicle_kind.into(),
                            position: Vec3::ZERO,
                            yaw: 0.0,
                            pitch: 0.0,
                        });
                    }
                    let vehicle = game.entity(id).unwrap();
                    let rider = add_mob(game, id + 1, "minecraft:zombie", Vec3::ZERO);
                    let second = add_mob(game, id + 2, "minecraft:skeleton", Vec3::ZERO);
                    let payload = |entity, kind: &str, mounting| SeatMountPayload {
                        entity,
                        kind: kind.into(),
                        vehicle: id,
                        vehicle_kind: vehicle_kind.into(),
                        mounting,
                    };
                    events.lock().unwrap().clear();
                    if vehicle_kind == "minecraft:marker" {
                        // Marker.couldAcceptPassenger refuses even a forced mount,
                        // before the cancellable mount notification.
                        assert!(!game.entity_start_riding(rider, vehicle, false));
                        assert!(!game.entity_start_riding(rider, vehicle, true));
                        assert_eq!(rider.vehicle(game), None);
                        assert_eq!(vehicle.passengers(game), Some(vec![]));
                        assert!(events.lock().unwrap().is_empty());
                        continue;
                    }

                    // A canceled handle write reports the type but changes neither side.
                    cancel.store(true, Ordering::SeqCst);
                    assert!(!game.entity_start_riding(rider, vehicle, false));
                    assert_eq!(rider.vehicle(game), None);
                    assert_eq!(vehicle.passengers(game), Some(vec![]));
                    assert_eq!(
                        *events.lock().unwrap(),
                        [payload(id + 1, "minecraft:zombie", true)]
                    );

                    cancel.store(false, Ordering::SeqCst);
                    assert!(game.entity_start_riding(rider, vehicle, false));
                    assert_eq!(rider.vehicle(game), Some(vehicle));
                    // Default Entity.canAddPassenger requires an empty seat. No hook for refusal.
                    assert!(!game.entity_start_riding(second, vehicle, false));
                    assert_eq!(second.vehicle(game), None);
                    assert_eq!(vehicle.passengers(game), Some(vec![rider]));
                    assert_eq!(events.lock().unwrap().len(), 2);
                    // /ride uses the same forced path, which permits more generic passengers.
                    assert!(game.entity_start_riding(second, vehicle, true));
                    assert_eq!(second.vehicle(game), Some(vehicle));
                    assert_eq!(vehicle.passengers(game), Some(vec![rider, second]));

                    cancel.store(true, Ordering::SeqCst);
                    assert!(!game.entity_stop_riding(rider));
                    assert_eq!(rider.vehicle(game), Some(vehicle));
                    assert_eq!(vehicle.passengers(game), Some(vec![rider, second]));
                    cancel.store(false, Ordering::SeqCst);
                    assert!(game.entity_stop_riding(rider));
                    assert!(game.entity_stop_riding(second));
                    assert_eq!(rider.vehicle(game), None);
                    assert_eq!(second.vehicle(game), None);
                    assert_eq!(vehicle.passengers(game), Some(vec![]));
                    assert_eq!(
                        *events.lock().unwrap(),
                        [
                            payload(id + 1, "minecraft:zombie", true),
                            payload(id + 1, "minecraft:zombie", true),
                            payload(id + 2, "minecraft:skeleton", true),
                            payload(id + 1, "minecraft:zombie", false),
                            payload(id + 1, "minecraft:zombie", false),
                            payload(id + 2, "minecraft:skeleton", false),
                        ]
                    );
                }
            });
        });
    }

    struct UnleashObserver(Arc<Mutex<u32>>);
    #[mod_api::__subscriptions]
    impl Hooks for UnleashObserver {
        fn entity_unleash(&self, _: &mut dyn ServerHost, _: &mut EntityUnleashEvent<'_>) {
            *self.0.lock().unwrap() += 1;
        }
    }

    #[test]
    #[ignore = "daily"]
    fn canceled_dismount_keeps_old_mount_and_mount_hook_fires_once() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "mount_observer",
            Box::new(MountObserver(events.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                for id in [40, 41, 42] {
                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
                }
                assert!(game.start_riding_target(Target::Mob(40), Target::Mob(41), true));
                assert!(!game.start_riding_target(Target::Mob(40), Target::Mob(42), true));
                assert_eq!(game.mobs[0].extra.mount, Some(41));
                assert_eq!(*events.lock().unwrap(), [true, true, false]);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn full_vehicle_checks_distinguish_forced_mobs_from_storage_limits() {
        use super::super::boats::Boat;
        let events = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "mount_observer",
            Box::new(MountObserver(events.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.ui.registries.permission = 4;
                for (id, name) in [
                    (40, "minecraft:zombie"),
                    (41, "minecraft:pig"),
                    (42, "minecraft:sheep"),
                    (43, "minecraft:cow"),
                ] {
                    let kind = game.res.mob_types().get(name).unwrap().clone();
                    game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
                }
                game.boats
                    .push(Boat::new(44, "oak".into(), Vec3::ZERO, 0.0));
                let pig =
                    "ride @e[type=minecraft:zombie,limit=1] mount @e[type=minecraft:pig,limit=1]";
                let boat = "ride @e[type=minecraft:zombie,limit=1] mount @e[type=minecraft:oak_boat,limit=1]";
                assert!(game.run_command(pig));
                game.mobs[0].extra.mount = None;
                game.mobs[1].extra.rider = None;
                assert!(game.run_command(boat));
                game.mobs[0].extra.mount = None;
                game.mobs[2].extra.mount = Some(41);
                game.mobs[1].extra.rider = Some(42);
                game.mobs[3].extra.mount = Some(44);
                game.riding = Some(44);
                events.lock().unwrap().clear();
                let rider = game.entity(40).unwrap();
                for id in [41, 44] {
                    let vehicle = game.entity(id).unwrap();
                    assert!(!game.entity_start_riding(rider, vehicle, false));
                }
                // Boats keep their storage limit even for commands and forced handles.
                assert!(!game.run_command(boat));
                let vehicle = game.entity(44).unwrap();
                assert!(!game.entity_start_riding(rider, vehicle, true));
                assert!(events.lock().unwrap().is_empty());
                // /ride forces mob boarding: capacity alone must not suppress its hook.
                assert!(game.run_command(pig));
                assert_eq!(*events.lock().unwrap(), [true]);
                assert_eq!(game.mobs[0].extra.mount, Some(41));
                assert_eq!(game.mobs[2].extra.mount, Some(41));
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn camel_admits_multiple_mob_passengers_but_not_a_fourth() {
        super::super::native_entity_refs::tests::with_game(|game| {
            for (id, name) in [
                (70, "minecraft:camel"),
                (71, "minecraft:camel_husk"),
                (72, "minecraft:happy_ghast"),
            ] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            for id in 73..=76 {
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            // Capacity counts real player bodies, not an unbound `riding` pointer.
            let player = super::super::native_player_death::tests::add_guest(game);
            let player_id = player.entity().id() as i32;
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.vehicle = Some(70));
            assert!(game.start_riding_target(Target::Mob(73), Target::Mob(70), false));
            assert!(game.start_riding_target(Target::Mob(74), Target::Mob(70), false));
            assert!(!game.start_riding_target(Target::Mob(75), Target::Mob(70), false));
            let mut riding = horizon_client::lan::entity::Riding::default();
            game.lan_riding(&[(70, player_id)], &mut riding);
            assert_eq!(riding.passengers(70), &[player_id, 73, 74]);
            // Force bypasses mob capacity, without losing any existing passenger.
            assert!(game.start_riding_target(Target::Mob(75), Target::Mob(70), true));
            game.lan_riding(&[(70, player_id)], &mut riding);
            assert_eq!(riding.passengers(70), &[player_id, 73, 74, 75]);
            game.dismount_mob(73);
            game.dismount_mob(74);
            game.dismount_mob(75);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.vehicle = None);
            for vehicle in [70, 71, 72] {
                let capacity = if vehicle == 72 { 4 } else { 3 };
                // Entity.removePassenger sets a 60-tick boarding cooldown. Fresh
                // riders isolate canAddPassenger from that unrelated refusal.
                let first = 100 + vehicle * 10;
                for id in first..=first + capacity {
                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
                }
                for rider in first..first + capacity {
                    assert!(
                        game.start_riding_target(Target::Mob(rider), Target::Mob(vehicle), false),
                        "ordinary passenger {rider} must fit vehicle {vehicle}"
                    );
                }
                assert!(
                    !game.start_riding_target(
                        Target::Mob(first + capacity),
                        Target::Mob(vehicle),
                        false
                    ),
                    "a fresh passenger must not fit beyond vehicle {vehicle}'s capacity"
                );
                for rider in first..first + capacity {
                    game.dismount_mob(rider);
                }
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn boat_and_cart_interactions_use_shared_mount_path() {
        use super::super::{
            InteractTarget,
            boats::Boat,
            minecarts::{CartKind, Minecart},
        };
        use horizon_client::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.yaw = 10.0;
            game.boats
                .push(Boat::new(60, "oak".into(), Vec3::ZERO, 45.0));
            assert!(game.interact_boat(Some(InteractTarget {
                id: 60,
                location: [0.0; 3]
            })));
            assert_eq!(game.riding, Some(60));
            assert_eq!(game.yaw, 10.0, "mounting keeps the rider's view");
            game.riding = None;
            game.carts
                .push(Minecart::new(61, CartKind::Rideable, Vec3::ZERO));
            assert!(game.interact_cart(Some(InteractTarget {
                id: 61,
                location: [0.0; 3]
            })));
            assert_eq!(game.riding, Some(61));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_rider_boards_another_cart_or_boat_straight_from_its_vehicle() {
        use super::super::{
            InteractTarget,
            boats::Boat,
            minecarts::{CartKind, Minecart},
        };
        use horizon_client::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.carts
                .push(Minecart::new(61, CartKind::Rideable, Vec3::ZERO));
            game.carts
                .push(Minecart::new(62, CartKind::Rideable, Vec3::ZERO));
            let mut chest_boat = Boat::new(63, "oak".into(), Vec3::ZERO, 0.0);
            chest_boat.chest = true;
            game.boats.push(chest_boat);
            let at = |id| {
                Some(InteractTarget {
                    id,
                    location: [0.0; 3],
                })
            };
            assert!(game.interact_cart(at(61)));
            assert_eq!(game.riding, Some(61));
            // Minecart.interact -> Entity.startRiding, which leaves the old
            // vehicle first: the rider changes carts in one click.
            assert!(game.interact_cart(at(62)));
            assert_eq!(game.riding, Some(62));
            // AbstractChestBoat.interact tries AbstractBoat.interact's
            // boarding first; the chest only opens when that passes.
            assert!(game.interact_boat(at(63)));
            assert_eq!(game.riding, Some(63));
            assert_eq!(game.container, None, "the chest stays closed");
            assert!(game.interact_cart(at(61)));
            assert_eq!(game.riding, Some(61), "and from a boat into a cart");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn handles_reject_held_out_and_stale_entities() {
        super::super::native_entity_refs::tests::with_game(|game| {
            for id in [50, 51] {
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let rider = game.entity(50).unwrap();
            let vehicle = game.entity(51).unwrap();
            let stale = mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(rider.epoch().get().wrapping_add(1)),
                rider.id(),
            );
            assert!(!game.entity_start_riding(stale, vehicle, true));
            game.held_out.push(51);
            assert!(!game.entity_start_riding(rider, vehicle, true));
            game.held_out.pop();
            assert!(game.entity_start_riding(rider, vehicle, true));
            assert_eq!(game.mobs[0].extra.mount, Some(51));
        });
    }

    fn add_mob(game: &mut Game<'_>, id: u32, name: &str, at: Vec3) -> EntityRef {
        let kind = game.res.mob_types().get(name).unwrap().clone();
        game.mobs.push(Mob::new(id, kind, at, 0.0));
        game.entity(id).unwrap()
    }
    fn stale(entity: EntityRef) -> EntityRef {
        EntityRef::new(
            mod_api::WorldEpoch::new(entity.epoch().get().wrapping_add(1)),
            entity.id(),
        )
    }
    /// A floor of stone at y 63 around the origin, with air above it.
    fn lay_floor(game: &mut Game<'_>) {
        for (x, z) in [(-1, -1), (-1, 0), (0, -1), (0, 0)] {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in -4..=4 {
            for z in -4..=4 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_nested_tree_lists_passengers_depth_first_and_shares_its_root() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let camel = add_mob(game, 80, "minecraft:camel", Vec3::ZERO);
            let first = add_mob(game, 81, "minecraft:pig", Vec3::ZERO);
            let second = add_mob(game, 82, "minecraft:pig", Vec3::ZERO);
            let top = add_mob(game, 83, "minecraft:zombie", Vec3::ZERO);
            let apart = add_mob(game, 84, "minecraft:cow", Vec3::ZERO);
            for (rider, vehicle) in [(81, 80), (82, 80), (83, 81)] {
                assert!(game.start_riding_target(Target::Mob(rider), Target::Mob(vehicle), true));
            }
            assert_eq!(camel.passengers(game), Some(vec![first, second]));
            assert_eq!(
                camel.indirect_passengers(game),
                Some(vec![first, top, second])
            );
            assert_eq!(first.indirect_passengers(game), Some(vec![top]));
            assert_eq!(top.indirect_passengers(game), Some(Vec::new()));

            assert_eq!(top.is_passenger_of_same_vehicle(game, second), Some(true));
            assert_eq!(top.is_passenger_of_same_vehicle(game, camel), Some(true));
            assert_eq!(top.is_passenger_of_same_vehicle(game, apart), Some(false));
            assert_eq!(apart.is_passenger_of_same_vehicle(game, apart), Some(true));
            assert_eq!(top.is_passenger_of_same_vehicle(game, stale(camel)), None);
            assert_eq!(stale(camel).indirect_passengers(game), None);
            assert_eq!(stale(camel).controlling_passenger(game), None);
            // Dying is not removal: a dead first passenger still owns its place.
            game.mob_object_mut(81).unwrap().health = 0.0;
            assert_eq!(camel.passengers(game), Some(vec![first, second]));
            assert_eq!(
                camel.controlling_passenger(game).map(|r| r.entity()),
                Some(first)
            );
            game.mob_object_mut(81).unwrap().health = 1.0;
            let index = game.mobs.iter().position(|m| m.id == 81).unwrap();
            let ghost = game.mobs.remove(index);
            game.persist
                .off_level
                .insert(81, super::super::persist::OffLevel::Mob(ghost));
            assert!(game.entity(81).is_none());
            assert_eq!(camel.passengers(game), Some(vec![second]));
            assert_eq!(
                camel.controlling_passenger(game),
                None,
                "never skip the actual first rider"
            );
            assert_eq!(
                top.vehicle(game),
                None,
                "off-level vehicle has no public handle"
            );
            assert_eq!(top.is_passenger(game), Some(true));
            assert_eq!(top.root_vehicle(game), Some(camel));
            assert_eq!(camel.indirect_passengers(game), Some(vec![top, second]));
            assert_eq!(
                first.is_passenger(game),
                None,
                "retained objects cannot revive stale handles"
            );
            let index = game.mobs.iter().position(|m| m.id == 82).unwrap();
            let ghost = game.mobs.remove(index);
            game.persist
                .off_level
                .insert(82, super::super::persist::OffLevel::Mob(ghost));
            assert_eq!(camel.passengers(game), Some(Vec::new()));
            assert_eq!(
                camel.is_vehicle(game),
                Some(true),
                "ghost-only children still ride"
            );
            assert_eq!(camel.indirect_passengers(game), Some(vec![top]));
            let index = game.mobs.iter().position(|m| m.id == 80).unwrap();
            let ghost = game.mobs.remove(index);
            game.persist
                .off_level
                .insert(80, super::super::persist::OffLevel::Mob(ghost));
            assert_eq!(
                top.root_vehicle(game),
                None,
                "an off-level root is not its resident descendant"
            );
            game.mob_object_mut(84).unwrap().extra.mount = Some(80);
            assert_eq!(top.is_passenger_of_same_vehicle(game, apart), Some(true));
            assert_eq!(apart.root_vehicle(game), None);
            assert_eq!(top.is_passenger_of_same_vehicle(game, top), Some(true));
            assert_eq!(stale(top).root_vehicle(game), None);
            assert_eq!(stale(top).is_passenger(game), None);
            assert_eq!(stale(top).is_vehicle(game), None);
            let other_root = add_mob(game, 85, "minecraft:cow", Vec3::ZERO);
            let index = game
                .mobs
                .iter()
                .position(|m| m.id == other_root.id())
                .unwrap();
            let ghost = game.mobs.remove(index);
            game.persist
                .off_level
                .insert(85, super::super::persist::OffLevel::Mob(ghost));
            game.persist.uuids.insert(80, [1, 2, 3, 4]);
            game.persist.uuids.insert(85, [1, 2, 3, 4]);
            game.mob_object_mut(84).unwrap().extra.mount = Some(85);
            assert_eq!(
                top.is_passenger_of_same_vehicle(game, apart),
                Some(false),
                "equal UUIDs are not the same object root"
            );
            // The topology walk must not truncate a valid tree at an arbitrary depth.
            let root = add_mob(game, 200, "minecraft:pig", Vec3::ZERO);
            let mut leaf = root;
            for id in 201..=270 {
                let rider = add_mob(game, id, "minecraft:pig", Vec3::ZERO);
                game.mob_object_mut(id).unwrap().extra.mount = Some(leaf.id());
                leaf = rider;
            }
            assert_eq!(leaf.root_vehicle(game), Some(root));
            game.mob_object_mut(200).unwrap().extra.mount = Some(leaf.id());
            assert_eq!(leaf.root_vehicle(game), None, "cycles have no root");
            assert_eq!(root.indirect_passengers(game).unwrap().len(), 70);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn passengers_keep_the_order_they_boarded_in_not_the_mob_list() {
        use horizon_core::nbt::Tag;
        super::super::native_entity_refs::tests::with_game(|game| {
            let camel = add_mob(game, 80, "minecraft:camel", Vec3::ZERO);
            let first = add_mob(game, 81, "minecraft:pig", Vec3::ZERO);
            let second = add_mob(game, 82, "minecraft:pig", Vec3::ZERO);
            let top = add_mob(game, 83, "minecraft:zombie", Vec3::ZERO);
            for (rider, vehicle) in [(82, 80), (81, 80), (83, 81)] {
                assert!(game.start_riding_target(Target::Mob(rider), Target::Mob(vehicle), true));
            }
            assert_eq!(camel.passengers(game), Some(vec![second, first]));
            assert_eq!(
                camel.indirect_passengers(game),
                Some(vec![second, first, top])
            );

            // The first rider going leaves the next in line first, whatever the mob list says.
            let driver = add_mob(game, 85, "minecraft:zombie", Vec3::ZERO);
            let vehicle = add_mob(game, 86, "minecraft:camel", Vec3::ZERO);
            let next = add_mob(game, 87, "minecraft:skeleton", Vec3::ZERO);
            let last = add_mob(game, 88, "minecraft:husk", Vec3::ZERO);
            for rider in [last, next, driver] {
                assert!(game.entity_start_riding(rider, vehicle, true));
            }
            assert_eq!(vehicle.passengers(game), Some(vec![last, next, driver]));
            // `Entity.saveWithoutId` writes `getPassengers`, so a reload keeps the order.
            let index = game.mobs.iter().position(|m| m.id == 86).unwrap();
            let saved = game.mob_tree_nbt(index);
            let Some(Tag::List(riders)) = saved.get("Passengers") else {
                panic!("no Passengers in {saved:?}");
            };
            let kinds: Vec<_> = riders.iter().map(|r| r.get("id").cloned()).collect();
            let kind = |name: &str| Some(Tag::String(name.into()));
            assert_eq!(
                kinds,
                [
                    kind("minecraft:husk"),
                    kind("minecraft:skeleton"),
                    kind("minecraft:zombie")
                ]
            );
            assert_eq!(
                vehicle.controlling_passenger(game),
                Some(LivingRef::new(last))
            );
            assert!(game.entity_stop_riding(last));
            assert_eq!(game.mobs[index].extra.rider, Some(87));
            assert_eq!(
                vehicle.controlling_passenger(game),
                Some(LivingRef::new(next))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_player_boards_ahead_of_a_mob_and_behind_a_player_as_add_passenger_says() {
        use horizon_client::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_game(|game| {
            lay_floor(game);
            let at = Vec3::new(0.5, 64.0, 0.5);
            let one = super::super::native_player_death::tests::add_guest(game).entity();
            let mut guest = super::super::native_player_death::tests::new_guest(2, 456);
            guest.entity_id = 21;
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.guests.push(guest);
            for conn in [1, 2] {
                let motion = PlayerMotion::new(game.motion, at).unwrap();
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.update_guest(conn, |g| g.motion = Some(Box::new(motion)))
                    .unwrap();
            }
            let two = game.player(456).unwrap().entity();
            let ghast = add_mob(game, 140, "minecraft:happy_ghast", at);
            let cow = add_mob(game, 141, "minecraft:cow", at);
            let pig = add_mob(game, 142, "minecraft:pig", at);

            // Ahead of a mob, behind the mobs once a player is first: `[one, cow, pig, two]`.
            assert!(game.entity_start_riding(cow, ghast, true));
            assert!(game.entity_start_riding(one, ghast, true));
            assert_eq!(ghast.passengers(game), Some(vec![one, cow]));
            assert!(game.entity_start_riding(pig, ghast, true));
            assert_eq!(ghast.passengers(game), Some(vec![one, cow, pig]));
            // On its own turn too, before the turn's end writes the record back.
            game.as_guest(2, |game| {
                assert!(game.start_riding_target(Target::Player, Target::Mob(140), true));
                assert_eq!(ghast.passengers(game), Some(vec![one, cow, pig, two]));
            })
            .unwrap();
            assert_eq!(ghast.passengers(game), Some(vec![one, cow, pig, two]));

            // The first player leaving leaves a mob first, and the next player goes ahead of it.
            assert!(game.entity_stop_riding(one));
            assert_eq!(ghast.passengers(game), Some(vec![cow, pig, two]));
            assert!(game.entity_start_riding(one, ghast, true));
            assert_eq!(ghast.passengers(game), Some(vec![one, cow, pig, two]));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_first_passenger_controls_only_what_it_can_steer() {
        use super::super::boats::Boat;
        use horizon_client::items::ItemStack;
        super::super::native_entity_refs::tests::with_game(|game| {
            let ravager = add_mob(game, 90, "minecraft:ravager", Vec3::ZERO);
            let zombie = add_mob(game, 91, "minecraft:zombie", Vec3::ZERO);
            assert_eq!(ravager.controlling_passenger(game), None);
            assert!(game.start_riding_target(Target::Mob(91), Target::Mob(90), true));
            assert_eq!(
                ravager.controlling_passenger(game),
                Some(LivingRef::new(zombie))
            );
            game.mobs[0].no_ai = true;
            assert_eq!(ravager.controlling_passenger(game), None);
            game.mobs[0].no_ai = false;

            // `#non_controlling_rider` keeps a mob from steering.
            let cow = add_mob(game, 92, "minecraft:cow", Vec3::ZERO);
            let slime = add_mob(game, 93, "minecraft:slime", Vec3::ZERO);
            assert!(game.start_riding_target(Target::Mob(93), Target::Mob(92), true));
            assert_eq!(cow.passengers(game), Some(vec![slime]));
            assert_eq!(cow.controlling_passenger(game), None);

            // A player steers a boat at once, a pig only saddled and with the stick in hand.
            let player = super::super::native_player_death::tests::add_guest(game).entity();
            let conn = 1;
            game.boats
                .push(Boat::new(95, "oak".into(), Vec3::ZERO, 0.0));
            let boat = game.entity(95).unwrap();
            // A mob in a boat steers nothing, and a player boarding after it goes first.
            let sailor = add_mob(game, 88, "minecraft:zombie", Vec3::ZERO);
            assert!(game.start_riding_target(Target::Mob(88), Target::Boat(95), true));
            assert_eq!(boat.passengers(game), Some(vec![sailor]));
            assert_eq!(boat.controlling_passenger(game), None);
            let pig = add_mob(game, 96, "minecraft:pig", Vec3::ZERO);
            let ride = |game: &mut Game<'_>, vehicle| {
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |g| g.vehicle = Some(vehicle))
                    .unwrap();
            };
            ride(game, 95);
            assert_eq!(
                boat.controlling_passenger(game),
                Some(LivingRef::new(player))
            );
            ride(game, 96);
            assert_eq!(pig.passengers(game), Some(vec![player]));
            assert_eq!(pig.controlling_passenger(game), None);
            let index = game.mobs.iter().position(|m| m.id == 96).unwrap();
            game.mobs[index].extra.set_saddled(true);
            assert_eq!(pig.controlling_passenger(game), None);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |g| {
                    g.offhand = Some(ItemStack::new("minecraft:carrot_on_a_stick", 1));
                })
                .unwrap();
            let steered = Some(LivingRef::new(player));
            assert_eq!(pig.controlling_passenger(game), steered);

            // A horse and a nautilus ask for the saddle alone, a strider for its own stick too,
            // and a happy ghast for a harness and no still timeout.
            let mount = |game: &mut Game<'_>, id, name| {
                let entity = add_mob(game, id, name, Vec3::ZERO);
                ride(game, id);
                assert_eq!(entity.controlling_passenger(game), None, "{name}");
                (entity, game.mobs.iter().position(|m| m.id == id).unwrap())
            };
            for (id, name) in [(97, "minecraft:horse"), (98, "minecraft:nautilus")] {
                let (entity, index) = mount(game, id, name);
                game.mobs[index].extra.set_saddled(true);
                assert_eq!(entity.controlling_passenger(game), steered, "{name}");
            }
            let (strider, index) = mount(game, 99, "minecraft:strider");
            game.mobs[index].extra.set_saddled(true);
            assert_eq!(strider.controlling_passenger(game), None);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |g| {
                    g.offhand = Some(ItemStack::new("minecraft:warped_fungus_on_a_stick", 1));
                })
                .unwrap();
            assert_eq!(strider.controlling_passenger(game), steered);
            let (ghast, index) = mount(game, 89, "minecraft:happy_ghast");
            game.mobs[index]
                .extra
                .set_body(Some("minecraft:white_harness".into()));
            assert_eq!(ghast.controlling_passenger(game), steered);
            game.mobs[index].extra.counter = 10;
            assert_eq!(ghast.controlling_passenger(game), None);
            // Both endpoints can be non-Mobs: even with no Mob objects, the
            // root walk must visit the player's boat before its terminal step.
            game.mobs.clear();
            ride(game, 95);
            assert_eq!(player.root_vehicle(game), Some(boat));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn entity_drops_gates_a_dying_mobs_lead_in_the_tick_not_in_detach() {
        use horizon_client::{
            gamerules::Value,
            mobs::{MobEvent, leash::Holder},
            motion::PlayerMotion,
        };
        super::super::native_entity_refs::tests::with_game(|game| {
            let at = Vec3::new(0.5, 64.0, 0.5);
            let cow = add_mob(game, 160, "minecraft:cow", at);
            let holder = add_mob(game, 161, "minecraft:sheep", at);
            game.mobs[0].health = 0.0;
            let tick_cleanup = |game: &mut Game<'_>| {
                game.mobs[0].extra.leash = Some(Holder::Mob(161));
                // What `leash::tick` sends for a mob that is not alive.
                let event = MobEvent::LeashBreak {
                    mob: 160,
                    drop: true,
                    sound: false,
                    at: None,
                };
                game.handle_mob_events(vec![event]).unwrap();
                assert_eq!(game.mobs[0].extra.leash, None);
            };

            // `tickLeash`: `removeLeash` with the rule off, `dropLeash` with it on.
            game.ui.rules.set("entity_drops", Value::Bool(false));
            tick_cleanup(game);
            assert!(game.dropped.is_empty());
            game.ui.rules.set("entity_drops", Value::Bool(true));
            tick_cleanup(game);
            take_one_lead(game, at);

            // `Leashable.dropLeash` asks no rule: the lead falls as promised.
            game.ui.rules.set("entity_drops", Value::Bool(false));
            assert!(cow.leash_to(game, holder));
            assert!(cow.detach_leash(game, true));
            take_one_lead(game, at);

            // Neither does `Entity.dropAllLeashConnections`, which calls it for a player's mobs.
            let player = super::super::native_player_death::tests::add_guest(game).entity();
            let motion = PlayerMotion::new(game.motion, at).unwrap();
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.update_guest(1, |g| {
                g.motion = Some(Box::new(motion));
                g.position = [at.x, at.y, at.z];
            })
            .unwrap();
            assert!(cow.leash_to(game, player));
            let dropped = game.as_guest(1, |game| game.drop_all_leash_connections());
            assert_eq!(dropped, Some(true));
            assert_eq!(game.mobs[0].extra.leash, None);
            take_one_lead(game, at);
        });
    }

    /// Asks where `rider` gets off `vehicle`, on foot and riding, then has it really get off:
    /// all three must agree. Returns the spot.
    fn get_off(game: &mut Game<'_>, rider: EntityRef, vehicle: EntityRef) -> [f64; 3] {
        let living = LivingRef::new(rider);
        let asked = vehicle.dismount_location(game, living).unwrap();
        assert!(game.entity_start_riding(rider, vehicle, true));
        assert_eq!(vehicle.dismount_location(game, living), Some(asked));
        assert!(game.entity_stop_riding(rider));
        assert_eq!(game.entity_position(rider), Some(asked));
        asked
    }
    /// The level holds exactly one dropped item, a single lead at `at`; takes it away.
    fn take_one_lead(game: &mut Game<'_>, at: Vec3) {
        assert_eq!(game.dropped.len(), 1);
        let item = game.dropped.iter().next().unwrap();
        assert_eq!(&*item.stack.item, "minecraft:lead");
        assert_eq!(item.stack.count, 1);
        assert_eq!(item.position, at);
        game.dropped.clear();
    }
    fn assert_near(at: [f64; 3], expected: [f64; 3]) {
        let near = at.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-4);
        assert!(near, "{at:?} is not {expected:?}");
    }

    #[test]
    #[ignore = "daily"]
    fn a_mob_gets_off_where_the_query_says() {
        use super::super::{
            boats::Boat,
            minecarts::{CartKind, Minecart},
        };
        super::super::native_entity_refs::tests::with_game(|game| {
            lay_floor(game);
            let at = Vec3::new(0.5, 64.0, 0.5);
            let rider = add_mob(game, 100, "minecraft:zombie", at);
            // On top of every mob with no rule of its own, the floor beside it free or not: an
            // `Animal` and a strider included, whose vanilla searches mobs don't use yet.
            for (id, name, height) in [
                (101, "minecraft:cow", 1.4),
                (102, "minecraft:strider", 1.7),
                (103, "minecraft:spider", 0.9),
                (104, "minecraft:happy_ghast", 4.0),
            ] {
                let vehicle = add_mob(game, id, name, at);
                assert_near(get_off(game, rider, vehicle), [0.5, 64.0 + height, 0.5]);
            }
            // At a right-handed rider's right, `(1.3964844 + 0.6 + 1e-5) / 2` from a horse's centre.
            let index = game.mobs.iter().position(|m| m.id == 100).unwrap();
            game.mobs[index].extra.left_handed = false;
            let horse = add_mob(game, 105, "minecraft:horse", at);
            assert_near(get_off(game, rider, horse), [-0.49825, 64.0, 0.5]);
            // Ahead of it past a boat's side, `(1.375 * sqrt 2 + 0.6 + 1e-5) / 2` from its centre.
            game.boats.push(Boat::new(106, "oak".into(), at, 0.0));
            let boat = game.entity(106).unwrap();
            assert_near(get_off(game, rider, boat), [0.5, 64.0, 1.77229]);
            // A cart facing south lets it off to the north.
            game.carts.push(Minecart::new(107, CartKind::Rideable, at));
            let cart = game.entity(107).unwrap();
            assert_near(get_off(game, rider, cart), [0.5, 64.0, -0.5]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_player_gets_off_where_the_query_says() {
        use super::super::{
            boats::Boat,
            minecarts::{CartKind, Minecart},
        };
        use horizon_client::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_game(|game| {
            lay_floor(game);
            let at = Vec3::new(0.5, 64.0, 0.5);
            let player = super::super::native_player_death::tests::add_guest(game).entity();
            let motion = PlayerMotion::new(game.motion, at).unwrap();
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.update_guest(1, |g| g.motion = Some(Box::new(motion)))
                .unwrap();

            // Animal searches neighbouring block centres, then the other side past a wall.
            let cow = add_mob(game, 120, "minecraft:cow", at);
            assert_near(get_off(game, player, cow), [-0.5, 64.0, 0.5]);
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            for y in [64, 65] {
                game.world.set(-1, y, 0, stone).unwrap();
            }
            assert_near(get_off(game, player, cow), [1.5, 64.0, 0.5]);
            for y in [64, 65] {
                game.world.set(-1, y, 0, air).unwrap();
            }

            // A strider's search starts along the player's facing, a block's centre on the floor.
            let strider = add_mob(game, 121, "minecraft:strider", at);
            assert_near(get_off(game, player, strider), [0.5, 64.0, 1.5]);
            let ghast = add_mob(game, 122, "minecraft:happy_ghast", at);
            assert_near(get_off(game, player, ghast), [0.5, 68.0, 0.5]);
            game.boats.push(Boat::new(123, "oak".into(), at, 0.0));
            let boat = game.entity(123).unwrap();
            assert_near(get_off(game, player, boat), [0.5, 64.0, 1.77229]);
            // As for a mob, a cart facing south lets it off to the north.
            game.carts.push(Minecart::new(124, CartKind::Rideable, at));
            let cart = game.entity(124).unwrap();
            assert_near(get_off(game, player, cart), [0.5, 64.0, -0.5]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_player_gets_off_along_their_own_yaw_on_their_turn_or_not() {
        use super::super::boats::Boat;
        use horizon_client::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_game(|game| {
            lay_floor(game);
            let at = Vec3::new(0.5, 64.0, 0.5);
            let player = super::super::native_player_death::tests::add_guest(game).entity();
            let motion = PlayerMotion::new(game.motion, at).unwrap();
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.update_guest(1, |g| {
                g.motion = Some(Box::new(motion));
                g.yaw = 45.0;
            })
            .unwrap();
            // Boarding turns the player to the vehicle, so it faces the same way.
            let mut boat = Boat::new(150, "oak".into(), at, 45.0);
            boat.previous_yaw = 45.0;
            game.boats.push(boat);
            let boat = game.entity(150).unwrap();
            let strider = add_mob(game, 151, "minecraft:strider", at);
            game.mobs[0].yaw = 45.0;

            // `getCollisionHorizontalEscapeVector` at 45 degrees: the longer axis reaches the
            // distance, so both do, `(1.375 * sqrt 2 + 0.6 + 1e-5) / 2` out from a boat's centre.
            let past_boat = [-0.77229, 64.0, 1.77229];
            // `(0.9 + 0.6 + 1e-5) / 2` from a strider's: the centre of the block there.
            let past_strider = [-0.5, 64.0, 1.5];
            assert_near(get_off(game, player, boat), past_boat);
            assert_near(get_off(game, player, strider), past_strider);
            game.as_guest(1, |game| {
                assert_eq!(game.native_entity_target(player), Some(Target::Player));
                assert_near(get_off(game, player, boat), past_boat);
                assert_near(get_off(game, player, strider), past_strider);
            })
            .unwrap();
        });
    }

    #[test]
    #[ignore = "daily"]
    fn dismount_location_answers_without_a_ride_or_a_live_handle() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let at = Vec3::new(0.5, 64.0, 0.5);
            let cow = add_mob(game, 130, "minecraft:cow", at);
            let rider = add_mob(game, 131, "minecraft:zombie", at);
            let living = LivingRef::new(rider);
            // Nothing rides a player: the top of its box.
            let player = super::super::native_player_death::tests::add_guest(game).entity();
            let [_, [_, top, _]] = game.entity_bounding_box(player).unwrap();
            let [x, _, z] = game.entity_position(player).unwrap();
            assert_eq!(player.dismount_location(game, living), Some([x, top, z]));

            assert_eq!(stale(cow).dismount_location(game, living), None);
            assert_eq!(
                cow.dismount_location(game, LivingRef::new(stale(rider))),
                None
            );
            // A player with no motion state has nothing to measure.
            assert_eq!(cow.dismount_location(game, LivingRef::new(player)), None);
            // An entity Horizon doesn't simulate keeps no box to stand on.
            game.others.push(super::super::commands::OtherEntity {
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                item_transformation: None,
                interaction: None,
                id: 132,
                kind: "minecraft:marker".into(),
                position: at,
                yaw: 0.0,
                pitch: 0.0,
            });
            let marker = game.entity(132).unwrap();
            assert_eq!(marker.dismount_location(game, living), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn leash_writes_change_the_holder_and_tell_the_mods() {
        use super::super::boats::Boat;
        use horizon_client::{mobs::leash::Pending, motion::PlayerMotion};
        use mod_api::LeashHolder;
        let unleashed = Arc::new(Mutex::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "unleash_observer",
            Box::new(UnleashObserver(unleashed.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                lay_floor(game);
                let registry = game.pack.assets.registry;
                let fence = registry.default_state("minecraft:oak_fence").unwrap();
                game.world.set(2, 64, 2, fence).unwrap();
                let at = Vec3::new(0.5, 64.0, 0.5);
                let cow = add_mob(game, 110, "minecraft:cow", at);
                let holder = add_mob(game, 111, "minecraft:sheep", at);
                let vehicle = add_mob(game, 112, "minecraft:pig", at);
                game.boats.push(Boat::new(113, "oak".into(), at, 0.0));
                let boat = game.entity(113).unwrap();

                // A mob's lead goes to a mob, and the leashed mob gets off its vehicle.
                assert!(game.entity_start_riding(cow, vehicle, true));
                assert!(cow.leash_to(game, holder));
                assert_eq!(cow.leash_holder(game), Some(LeashHolder::Entity(holder)));
                assert_eq!(cow.vehicle(game), None);

                // Taking the lead over is no removal; a fence holds it by position.
                assert!(cow.leash_to(game, boat));
                assert_eq!(cow.leash_holder(game), Some(LeashHolder::Entity(boat)));
                assert!(cow.leash_to_knot(game, [2, 64, 2]));
                assert_eq!(cow.leash_holder(game), Some(LeashHolder::Knot([2, 64, 2])));
                assert_eq!(*unleashed.lock().unwrap(), 0);

                // Refused: the entity itself, a block that is no fence, a stale side.
                assert!(!cow.leash_to(game, cow));
                assert!(!cow.leash_to_knot(game, [0, 63, 0]));
                assert!(!cow.leash_to(game, stale(holder)));
                assert!(!stale(cow).leash_to(game, holder));
                assert!(!stale(cow).leash_to_knot(game, [2, 64, 2]));
                assert!(!stale(cow).detach_leash(game, true));
                assert_eq!(stale(cow).leash_holder(game), None);
                assert_eq!(cow.leash_holder(game), Some(LeashHolder::Knot([2, 64, 2])));

                // `removeLeash` leaves no item, `dropLeash` drops one; both tell `entity_unleash`.
                assert!(cow.detach_leash(game, false));
                assert!(game.dropped.is_empty());
                assert_eq!(cow.leash_holder(game), Some(LeashHolder::Unleashed));
                assert_eq!(*unleashed.lock().unwrap(), 1);
                assert!(!cow.detach_leash(game, false));
                assert!(cow.leash_to(game, holder));
                assert!(cow.detach_leash(game, true));
                // At the cow's feet, on top of the pig it was taken off.
                take_one_lead(game, Vec3::new(0.5, 64.0 + f64::from(0.9_f32), 0.5));
                assert_eq!(*unleashed.lock().unwrap(), 2);

                // A boat is leashable too, and a player is not.
                assert_eq!(boat.leash_holder(game), Some(LeashHolder::Unleashed));
                assert!(boat.leash_to(game, holder));
                assert_eq!(boat.leash_holder(game), Some(LeashHolder::Entity(holder)));
                assert!(boat.detach_leash(game, false));
                assert_eq!(boat.leash_holder(game), Some(LeashHolder::Unleashed));
                assert!(game.dropped.is_empty());
                assert!(boat.leash_to(game, holder));
                assert!(boat.detach_leash(game, true));
                assert_eq!(boat.leash_holder(game), Some(LeashHolder::Unleashed));
                take_one_lead(game, at);

                // A player holds it by UUID, whether in the hub's record or on their own turn.
                let player = super::super::native_player_death::tests::add_guest(game).entity();
                assert!(cow.leash_to(game, player));
                assert_eq!(cow.leash_holder(game), Some(LeashHolder::Entity(player)));
                assert!(!player.leash_to(game, cow));
                assert_eq!(player.leash_holder(game), None);
                let motion = PlayerMotion::new(game.motion, at).unwrap();
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.update_guest(1, |g| g.motion = Some(Box::new(motion)))
                    .unwrap();
                assert!(cow.detach_leash(game, false));
                game.as_guest(1, |game| {
                    assert!(cow.leash_to(game, player));
                    assert_eq!(cow.leash_holder(game), Some(LeashHolder::Entity(player)));
                })
                .unwrap();
                assert_eq!(cow.leash_holder(game), Some(LeashHolder::Entity(player)));
                assert!(cow.detach_leash(game, false));

                // A holder unresolved from a save reads as unknown, and `dropLeash` leaves it;
                // `setLeashedTo` replaces it.
                let index = game.mobs.iter().position(|m| m.id == 110).unwrap();
                game.mobs[index].extra.leash_pending = Some(Pending::Uuid([0, 0, 0, 99]));
                assert_eq!(cow.leash_holder(game), None);
                assert!(!cow.detach_leash(game, true));
                assert!(game.mobs[index].extra.leash_pending.is_some());
                assert!(cow.leash_to(game, holder));
                assert_eq!(cow.leash_holder(game), Some(LeashHolder::Entity(holder)));
                let index = game.boats.iter().position(|b| b.id == 113).unwrap();
                game.boats.update(index, |boat| {
                    boat.leash_pending = Some(Pending::Uuid([0, 0, 0, 99]));
                });
                assert_eq!(boat.leash_holder(game), None);
                assert!(!boat.detach_leash(game, true));
                assert!(game.boats[index].leash_pending.is_some());
                assert!(boat.leash_to(game, holder));
                assert_eq!(boat.leash_holder(game), Some(LeashHolder::Entity(holder)));
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mount_rejects_cycles_and_respects_cooldown_unless_forced() {
        super::super::native_entity_refs::tests::with_game(|game| {
            for id in [30, 31, 32] {
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            game.mobs[0].extra.boarding_cooldown = 10;
            assert!(!game.start_riding_target(Target::Mob(30), Target::Mob(31), false));
            assert!(game.start_riding_target(Target::Mob(30), Target::Mob(31), true));
            assert!(!game.start_riding_target(Target::Mob(30), Target::Mob(31), true));
            assert!(!game.start_riding_target(Target::Mob(31), Target::Mob(30), true));
            assert!(!game.start_riding_target(Target::Mob(32), Target::Mob(31), false));
            assert!(game.start_riding_target(Target::Mob(32), Target::Mob(31), true));
            assert_eq!(game.mobs[0].extra.mount, Some(31));
            assert_eq!(game.mobs[1].extra.rider, Some(30));
            assert_eq!(game.mobs[2].extra.mount, Some(31));
            game.dismount_mob(30);
            assert_eq!(game.mobs[0].extra.mount, None);
            assert_eq!(game.mobs[0].extra.boarding_cooldown, 60);
            assert!(!game.start_riding_target(Target::Mob(30), Target::Mob(32), false));
            assert!(game.start_riding_target(Target::Mob(30), Target::Mob(32), true));
            assert_eq!(game.mobs[0].extra.mount, Some(32));
        });
    }
}
