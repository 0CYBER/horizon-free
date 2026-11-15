//! Shared physical mob seating; specialized vehicle seats stay authoritative.
use super::*;

impl Game<'_> {
    pub(in crate::game) fn position_mob_rider(&mut self, i: usize, index: &HashMap<u32, usize>) {
        let Some(v) = self.mobs[i].extra.mount else {
            return;
        };
        let Some(&j) = index.get(&v) else {
            // Boats, minecarts, primed TNT and cushions seated their
            // riders above, each in its own seat and from the
            // vehicle's drawn position; re-seating them here from the
            // shared seat 0 yanked boat riders between two places.
            if self.seats_own_riders(v) {
                return;
            }
            // A mob on a text display: NPC plugins seat their sitting
            // NPCs on one. The server's position for a passenger
            // gives way to its seat, as `rideTick` places it.
            let me = self.remote.as_ref().map(|r| r.entity_id as u32);
            let own_seat = self.player.as_ref().map(|p| {
                p.position
                    .add_vector(Vec3::new(0.0, f64::from(p.dimensions.height), 0.0))
            });
            let Some(at) = self.passenger_riding_position(v, me, own_seat, index) else {
                return;
            };
            let seat = Vec3::new(at.x, at.y, at.z);
            let vehicle_box = self
                .local_passenger_vehicle_bounds(v)
                .unwrap_or_else(|| Aabb::new(seat, seat));
            let vehicle_width =
                (vehicle_box.max_position().x - vehicle_box.min_position().x) as f32;
            let before = self
                .text_displays
                .get(&(v as i32))
                .map(|d| d.seat().1)
                .unwrap_or_else(|| {
                    let old = self.mobs[i].previous;
                    glam::DVec3::new(
                        old.x,
                        old.y + riders::vehicle_attachment(&self.mobs[i], vehicle_width),
                        old.z,
                    )
                });
            let m = &mut self.mobs[i];
            let offset = riders::vehicle_attachment(m, vehicle_width);
            m.seated(v, offset, vehicle_box);
            m.position = Vec3::new(at.x, at.y - offset, at.z);
            m.previous = Vec3::new(before.x, before.y - offset, before.z);
            m.velocity = Vec3::ZERO;
            m.fall_distance = 0.0;
            m.on_ground = false;
            m.extra.passenger = true;
            m.ai.nav.stop();
            return;
        };
        let vehicle = &self.mobs[j];
        let mut place = (0, 1);
        if horizon_client::mobs::camel::is_camel(vehicle.species()) {
            // Camel.getPassengerAttachmentPoint: the seat follows the
            // rider's place in `getPassengers` (load order).
            let mut aboard: Vec<usize> = (0..self.mobs.len())
                .filter(|&k| self.mobs[k].extra.mount == Some(v))
                .collect();
            let player_aboard = vehicle.rider.is_some() || self.riding == Some(v);
            let (seat, count) = if player_aboard {
                let mob_seat = aboard.iter().position(|&k| k == i).unwrap_or(0);
                (mob_seat + 1, aboard.len() + 1)
            } else {
                if let Some(driver) = vehicle.extra.rider
                    && let Some(pos) =
                        aboard.iter().position(|&k| self.mobs[k].id == driver)
                    && pos != 0
                {
                    let driver_idx = aboard.remove(pos);
                    aboard.insert(0, driver_idx);
                }
                let seat = aboard.iter().position(|&k| k == i).unwrap_or(0);
                (seat, aboard.len())
            };
            let (seat, count) = if self.persist.off_level.is_empty() {
                (seat, count)
            } else {
                let seats = self.seated_passengers(v);
                (
                    seats
                        .iter()
                        .position(|&(_, id, _)| id == self.mobs[i].id)
                        .unwrap_or(seat),
                    seats.len(),
                )
            };
            place = (seat, count);
        }
        if vehicle.species() == horizon_client::mobs::Species::HappyGhast
            || vehicle.kind.passenger_attachments.len() > 1
        {
            // Built-in and content seats follow the rider's actual
            // place in getPassengers, including retained objects.
            // `Entity.addPassenger` puts a player first, so a mob that
            // boards after one sits at index 1.
            let player_aboard = vehicle.rider.is_some() || self.riding == Some(v);
            let seat = usize::from(player_aboard)
                + self.mobs[..i]
                    .iter()
                    .filter(|r| r.extra.mount == Some(v))
                    .count();
            let seat = if self.persist.off_level.is_empty() {
                seat
            } else {
                self.seated_passengers(v)
                    .iter()
                    .position(|&(_, id, _)| id == self.mobs[i].id)
                    .unwrap_or(seat)
            };
            place = (seat, 1);
        }
        let attach =
            Self::mob_passenger_attachment(vehicle, &self.mobs[i], place.0, place.1);
        let (pos, prev) = (vehicle.position, vehicle.previous);
        let (yaw, prev_yaw) = (vehicle.yaw, vehicle.previous_yaw);
        let (body, prev_body) = (vehicle.body_yaw, vehicle.previous_body_yaw);
        let rider = &self.mobs[i];
        let controlling = riders::controls_vehicle(
            vehicle,
            rider,
            vehicle.rider.is_some() || self.riding == Some(v),
            self.res
                .mob_data()
                .entity_has("minecraft:non_controlling_rider", &rider.kind.id),
        );
        let takes_body = riders::takes_vehicle_body(vehicle, rider, controlling);
        let vehicle_width = vehicle.width();
        let vehicle_box = vehicle.bounds();
        let m = &mut self.mobs[i];
        let offset = riders::vehicle_attachment(m, vehicle_width);
        m.seated(v, offset, vehicle_box);
        m.position = riders::seat(attach, offset, pos, yaw);
        m.previous = riders::seat(attach, offset, prev, prev_yaw);
        m.velocity = Vec3::ZERO;
        m.fall_distance = 0.0;
        m.on_ground = false;
        if takes_body {
            m.body_yaw = body;
            m.previous_body_yaw = prev_body;
        }
        m.extra.vehicle_body = Some((v, prev_body, body));
        m.extra.passenger = true;
        // A rider never walks off on its own.
        m.ai.nav.stop();
    }

    /// A specialized vehicle may itself have moved earlier in the saved
    /// tree. Re-seat its mobs through the existing per-vehicle implementation.
    pub(in crate::game) fn position_saved_mob_rider(
        &mut self,
        i: usize,
        index: &HashMap<u32, usize>,
    ) {
        if let Some(vehicle) = self.mobs[i].extra.mount {
            let aboard = self
                .mobs
                .iter()
                .enumerate()
                .filter_map(|(j, m)| (m.extra.mount == Some(vehicle)).then_some(j))
                .collect();
            let riders = Riders::from([(vehicle, aboard)]);
            self.seat_boat_mobs(&riders);
            self.seat_cart_mobs(&riders);
            self.seat_tnt_mobs(&riders);
            self.seat_cushion_mobs(&riders);
            self.seat_static_mobs(&riders);
        }
        self.position_mob_rider(i, index);
    }

    /// Default Entity passenger attachment: the top of its current box.
    /// Registered dimensions cover live carriers without an Entity handle.
    pub(in crate::game) fn local_passenger_vehicle_bounds(&self, id: u32) -> Option<Aabb> {
        use crate::game::entity::{Entity, with_entity};
        if self.remote.is_some() {
            return None;
        }
        let target = self.level_entity(id as i32)?;
        let at = self.target_position(target)?;
        let [width, height] = self.registered_entity_dimensions(&self.target_type(target))?;
        let half = f64::from(width) / 2.0;
        let mut bounds = Aabb::new(
            Vec3::new(at[0] - half, at[1], at[2] - half),
            Vec3::new(at[0] + half, at[1] + f64::from(height), at[2] + half),
        );
        for handle in self.entity_handles() {
            with_entity!(self, handle, entity => {
                if entity.id() == id { bounds = entity.bounds(); }
            });
        }
        Some(bounds)
    }
}
