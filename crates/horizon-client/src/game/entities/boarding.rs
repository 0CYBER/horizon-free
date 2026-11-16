//! A player's `Entity.boardingCooldown` is always 0: `Player.removeVehicle`
//! zeroes the 60 ticks `Entity.removePassenger` just set (#11158), so
//! `Entity.canRide` only asks that the player is not sneaking, which each
//! vehicle's interaction checks. Mobs keep theirs in `extra.boarding_cooldown`
//! (`boat_passengers::BOARDING_COOLDOWN`).
use super::Game;

/// `Entity.getIndirectPassengersStream` for `vehicle`: the vehicle and every
/// vehicle riding it, directly or through another rider. `mounts` holds
/// `(vehicle, its vehicle)` for every mob.
pub(super) fn passenger_vehicles(vehicle: u32, mounts: &[(u32, Option<u32>)]) -> Vec<u32> {
    let mut ids = vec![vehicle];
    let mut i = 0;
    while i < ids.len() {
        let v = ids[i];
        i += 1;
        for &(id, mount) in mounts {
            if mount == Some(v) && !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

impl Game<'_> {
    /// `Entity.startRiding` for the acting player: the rider link, then
    /// `CriteriaTriggers.START_RIDING_TRIGGER` for every server player the
    /// vehicle carries.
    pub(super) fn start_riding(&mut self, vehicle: u32) {
        self.riding = Some(vehicle);
        self.seat_acting_player(vehicle);
        self.note_started_riding(vehicle);
    }

    /// `Entity.passengers` of `vehicle`, first passenger first: each one's place, id and whether
    /// it is a player. Places tie only for those that never boarded through `addPassenger` (a
    /// load keeps its list order), players ahead.
    pub(in crate::game) fn seated_passengers(&self, vehicle: u32) -> Vec<(i64, u32, bool)> {
        let acting = self.lan_acting.filter(|_| self.player.is_some());
        let mut acting_listed = false;
        let mut seats: Vec<(i64, u32, bool)> = self
            .lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .filter(|g| g.placed && g.dimension == self.dim.id())
            .filter(|g| {
                let rides = if acting == Some(g.conn) {
                    acting_listed = true;
                    self.riding
                } else {
                    g.vehicle
                };
                rides == Some(vehicle)
            })
            .map(|g| (g.vehicle_order, g.entity_id as u32, true))
            .collect();
        // An acting player absent from the placed list has no place kept: it still rides
        // through `riding`, at the front.
        if let Some(me) = self.lan_acting_entity
            && self.lan_acting_uuid.is_some()
            && !acting_listed
            && self.player.is_some()
            && self.riding == Some(vehicle)
        {
            seats.push((i64::MIN, me, true));
        }
        seats.extend(
            self.mob_objects()
                .filter(|m| m.extra.mount == Some(vehicle) && !m.removed)
                .map(|m| (m.extra.vehicle_order, m.id, false)),
        );
        seats.sort_by_key(|&(place, ..)| place);
        seats
    }
    /// `Entity.addPassenger`: the place of `rider` getting on `vehicle`. On a server a player
    /// goes ahead of a first passenger that is no player; everything else goes last.
    pub(in crate::game) fn boarding_order(&self, vehicle: u32, rider: u32, player: bool) -> i64 {
        let mut aboard = self.seated_passengers(vehicle);
        aboard.retain(|&(_, id, _)| id != rider);
        match (aboard.first(), aboard.last()) {
            (Some(&(first, _, false)), _) if player => first - 1,
            (_, Some(&(last, ..))) => last + 1,
            _ => 0,
        }
    }
    /// `addPassenger` for the mob `id`, linked to its vehicle just now.
    pub(in crate::game) fn seat_mob(&mut self, id: u32) {
        let mount = self.mob_object(id);
        let Some(vehicle) = mount.and_then(|m| m.extra.mount) else {
            return;
        };
        let order = self.boarding_order(vehicle, id, false);
        if let Some(m) = self.mob_object_mut(id) {
            m.extra.vehicle_order = order;
        }
    }
    /// `addPassenger` for the acting guest, now in `vehicle`.
    pub(in crate::game) fn seat_acting_player(&mut self, vehicle: u32) {
        let (Some(conn), Some(me)) = (self.lan_acting, self.lan_acting_entity) else {
            return;
        };
        let order = self.boarding_order(vehicle, me, true);
        if let Some(host) = self.lan.as_mut() {
            host.hub.update_guest(conn, |g| g.vehicle_order = order);
        }
    }

    /// A rider link changed on `vehicle` (`sendEventAndTriggers`). Every
    /// player aboard hears the trigger; a host's turn can fire it now, while
    /// a guest's turn (the hub held by its action) leaves it for
    /// [`Self::flush_started_riding`] once every player is back in place.
    pub(super) fn note_started_riding(&mut self, vehicle: u32) {
        self.started_riding.push(vehicle);
        if self.lan.is_some() && self.lan_acting.is_none() {
            self.flush_started_riding();
        }
    }

    /// Fires the queued `started_riding` triggers with every player in place.
    pub(super) fn flush_started_riding(&mut self) {
        for vehicle in std::mem::take(&mut self.started_riding) {
            self.trigger_started_riding(vehicle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "daily"]
    fn retained_passengers_occupy_seats_in_boarding_order() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            use horizon_client::mobs::Mob;
            use horizon_core::geometry::Vec3;
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            for (id, order) in [(74, 2), (72, 0)] {
                let mut m = Mob::new(id, pig.clone(), Vec3::ZERO, 0.0);
                m.extra.mount = Some(71);
                m.extra.vehicle_order = order;
                game.persist
                    .off_level
                    .insert(id, crate::game::persist::OffLevel::Mob(m));
            }
            let mut resident = Mob::new(73, pig, Vec3::ZERO, 0.0);
            resident.extra.mount = Some(71);
            resident.extra.vehicle_order = 1;
            game.mobs.push(resident);
            assert_eq!(
                game.seated_passengers(71),
                vec![(0, 72, false), (1, 73, false), (2, 74, false)]
            );
            assert_eq!(game.boarding_order(71, 75, false), 3);
            assert_eq!(game.boarding_order(71, 75, true), -1);
            game.seat_mob(72);
            assert_eq!(game.mob_object(72).unwrap().extra.vehicle_order, 3);
        });
    }

    #[test]
    fn started_riding_reaches_every_rider_of_the_vehicle() {
        // Entity.startRiding: START_RIDING_TRIGGER for every ServerPlayer in
        // entityToRide.getIndirectPassengersStream(). A mob between two
        // riders is still part of that stream, and another vehicle is not.
        let mounts = [(7, Some(5)), (8, Some(7)), (9, Some(4))];
        let aboard = passenger_vehicles(5, &mounts);
        assert!(aboard.contains(&5), "the rider on the vehicle itself");
        assert!(aboard.contains(&7), "the mob in the vehicle's seat");
        assert!(aboard.contains(&8), "the rider of that mob");
        assert!(!aboard.contains(&9), "a mob riding another vehicle");
        assert!(!aboard.contains(&6), "a mob riding nothing");
    }
}
