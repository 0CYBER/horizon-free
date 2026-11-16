//! Passenger reads over the mob tick's disjoint live entity loan.
use super::GoalCx;
use mod_api::{EntityRef, LivingRef};

impl GoalCx<'_, '_> {
    pub(super) fn r_entity_dismount_location(
        &self,
        vehicle: EntityRef,
        passenger: LivingRef,
    ) -> Option<[f64; 3]> {
        if self.lookup_entity(vehicle.id())? != vehicle
            || self.lookup_living(passenger.entity()).is_none()
        {
            return None;
        }
        self.w.goal_host?.mob_dismount_location(
            self.entity_mob(vehicle),
            vehicle.id(),
            self.entity_mob(passenger.entity()),
            passenger.entity(),
        )
    }
    /// Resolve vehicle links through the loan before asking the game for an
    /// entity that remains in place (such as a player or a boat).
    fn scoped_root_vehicle(&self, entity: EntityRef) -> Option<EntityRef> {
        if self.lookup_entity(entity.id())? != entity {
            return None;
        }
        let mut root = entity;
        for _ in 0..64 {
            let vehicle = if self.entity_mob(root).is_some() {
                self.r_entity_vehicle(root)
            } else {
                mod_api::__private::backend_read(self.host, |backend| backend.entity_vehicle(root))
                    .flatten()
            };
            match vehicle {
                Some(vehicle) => root = vehicle,
                None => break,
            }
        }
        Some(root)
    }

    pub(super) fn r_entity_root_vehicle(&self, entity: EntityRef) -> Option<EntityRef> {
        self.scoped_root_vehicle(entity)
    }

    pub(super) fn r_entity_is_passenger_of_same_vehicle(
        &self,
        entity: EntityRef,
        other: EntityRef,
    ) -> Option<bool> {
        Some(self.scoped_root_vehicle(entity)? == self.scoped_root_vehicle(other)?)
    }

    pub(super) fn r_entity_passengers(&self, entity: EntityRef) -> Option<Vec<EntityRef>> {
        self.entity_mob(entity)?;
        let live = self.w.live_mobs?;
        let host = self.w.goal_host?;
        let mut seats = host.player_passenger_seats(entity.id());
        let mut add_mob = |mob: &crate::mobs::Mob| {
            if mob.extra.mount == Some(entity.id()) && !mob.removed && mob.alive() {
                seats.push((mob.extra.vehicle_order, mob.id));
            }
        };
        add_mob(self.m);
        live.for_each(&mut add_mob);
        seats.sort_by_key(|&(order, _)| order);
        Some(
            seats
                .into_iter()
                .filter_map(|(_, id)| self.lookup_entity(id))
                .collect(),
        )
    }

    pub(super) fn r_entity_is_vehicle(&self, entity: EntityRef) -> Option<bool> {
        Some(!self.r_entity_passengers(entity)?.is_empty())
    }

    pub(super) fn r_entity_indirect_passengers(&self, entity: EntityRef) -> Option<Vec<EntityRef>> {
        let mut all = Vec::new();
        let mut next = self.r_entity_passengers(entity)?;
        next.reverse();
        while let Some(passenger) = next.pop() {
            if passenger == entity || all.contains(&passenger) {
                continue;
            }
            all.push(passenger);
            let children = if self.entity_mob(passenger).is_some() {
                self.r_entity_passengers(passenger)
            } else {
                mod_api::__private::backend_read(self.host, |backend| {
                    backend.entity_passengers(passenger)
                })
                .flatten()
            };
            if let Some(riders) = children {
                next.extend(riders.into_iter().rev());
            }
        }
        Some(all)
    }

    pub(super) fn r_entity_controlling_passenger(&self, entity: EntityRef) -> Option<LivingRef> {
        let vehicle = self.entity_mob(entity)?;
        let first = *self.r_entity_passengers(entity)?.first()?;
        if let Some(rider) = self.entity_mob(first) {
            let tagged = self
                .w
                .data
                .entity_has("minecraft:non_controlling_rider", &rider.kind.id);
            return (!vehicle.no_ai && !tagged).then(|| LivingRef::new(first));
        }
        // The game only lets a player control this kind when the relevant
        // saddle, steering item or harness is present.
        if self.host.mob(first).is_some() || self.host.living(first).is_none() {
            return None;
        }
        self.w
            .goal_host?
            .mob_player_controls(vehicle, first.id())
            .then(|| LivingRef::new(first))
    }
}
