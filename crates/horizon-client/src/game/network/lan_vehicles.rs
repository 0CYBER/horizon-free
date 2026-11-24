//! Boats and minecarts for a server's players: LAN guests, players on a
//! dedicated server, and single player's window on its server thread.
//!
//! A player aims at a vehicle, item frame, armor stand or painting as at a
//! mob (`ServerGamePacketListenerImpl.handleInteract`), and the game's own
//! code for it runs as that player (`AbstractBoat.interact` seats it,
//! `VehicleEntity.hurtServer` breaks the vehicle). A boat is driven by its
//! rider's client (`handleMoveVehicle`, `handlePaddleBoat`); a minecart moves
//! here with the rider's input (`ServerPlayer.getLastClientMoveIntent`).
//! Sneaking gets out (`Player.rideTick`: `wantsToStopRiding`), and the client
//! is teleported where the player lands (`ServerPlayer.dismountTo`).
use super::{Dim, Game, Keys, lan::LanHost, minecarts::SEAT};
use horizon_client::collisions::SceneCollisions;
use horizon_client::lan::guest::Guest;
use horizon_client::lan::hub::no_blocks_in;
use horizon_client::mobs::equine;
use horizon_core::geometry::{Aabb, Axis, Vec3};
use horizon_protocol::{
    packets::play::{self, Clientbound as C},
    text::translatable,
};
use horizon_world::entity_move::MoverType;

/// `PlayerInput` flags, as the server player's client last sent them.
pub(super) fn input_keys(flags: u8) -> Keys {
    Keys {
        forward: flags & 1 != 0,
        backward: flags & 2 != 0,
        left: flags & 4 != 0,
        right: flags & 8 != 0,
        jump: flags & 16 != 0,
        shift: flags & 32 != 0,
        sprint: flags & 64 != 0,
        ..Keys::default()
    }
}

fn corners(b: Aabb) -> ([f64; 3], [f64; 3]) {
    let (lo, hi) = (b.min_position(), b.max_position());
    ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
}

/// A boat or minecart a server player rode, followed for
/// `native_vehicle_moved`.
pub(super) struct WatchedVehicle {
    id: u32,
    dim: Dim,
    /// The server player it carries now.
    rider: Option<u128>,
    /// Where the hook last heard of it.
    at: [f64; 3],
    /// Ticks it has stood still.
    still: u32,
}

impl Game<'_> {
    /// Where the boat or minecart `id` of this level is.
    fn watched_vehicle_position(&self, id: u32) -> Option<[f64; 3]> {
        let at = |v: Vec3| [v.x, v.y, v.z];
        self.boats
            .iter()
            .find(|b| b.id == id)
            .map(|b| at(b.position))
            .or_else(|| {
                self.carts
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| at(c.position))
            })
    }

    /// `VehicleMoveEvent` for the boats and minecarts server players ride
    /// (and left less than ten seconds ago): once a tick, after they moved.
    pub(super) fn tick_vehicle_watch(&mut self) {
        let riders: Vec<(u32, u32, u128)> = self.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter_map(|g| Some((g.vehicle?, g.conn, g.uuid)))
                .collect()
        });
        if riders.is_empty() && self.watched_vehicles.is_empty() {
            return;
        }
        let dim = self.dim;
        for &(id, _, uuid) in &riders {
            let Some(at) = self.watched_vehicle_position(id) else {
                continue;
            };
            match self.watched_vehicles.iter_mut().find(|w| w.id == id) {
                Some(w) => (w.dim, w.rider) = (dim, Some(uuid)),
                None => self.watched_vehicles.push(WatchedVehicle {
                    id,
                    dim,
                    rider: Some(uuid),
                    at,
                    still: 0,
                }),
            }
        }
        let mut kept = Vec::new();
        // (rider's connection and UUID, vehicle type, from, to)
        let mut moved = Vec::new();
        for mut w in std::mem::take(&mut self.watched_vehicles) {
            // Another level's vehicle is told of on that level's tick.
            if w.dim != dim {
                kept.push(w);
                continue;
            }
            // Broken, or gone from this level.
            let Some(now) = self.watched_vehicle_position(w.id) else {
                continue;
            };
            let rider = riders.iter().find(|r| r.0 == w.id);
            w.rider = rider.map(|r| r.2);
            if now != w.at {
                let from = std::mem::replace(&mut w.at, now);
                w.still = 0;
                let kind = self.target_type(
                    self.level_entity(w.id as i32)
                        .unwrap_or(super::commands::Target::Boat(w.id)),
                );
                moved.push((rider.map(|r| (r.1, r.2)), kind, from, now));
            } else {
                w.still += 1;
                if w.rider.is_none() && w.still > 200 {
                    continue;
                }
            }
            kept.push(w);
        }
        self.watched_vehicles = kept;
        // The rider's state is swapped in for the call, so the helpers reach
        // them; a vehicle nobody rides is told of with nobody swapped in.
        for (rider, kind, from, to) in moved {
            let Some((conn, uuid)) = rider else {
                self.native_vehicle_moved(None, &kind, from, to);
                continue;
            };
            let hook = |game: &mut Self| game.native_vehicle_moved(Some(uuid), &kind, from, to);
            if self.lan_acting == Some(conn) {
                hook(self);
            } else {
                self.as_guest(conn, hook);
            }
        }
    }

    /// The box of a pickable non-mob object, and
    /// the point an interact packet's location is measured from (the
    /// client's `remote_object_hit` uses the same points).
    pub(super) fn object_target(&self, id: i32) -> Option<([f64; 3], [f64; 3], [f64; 3])> {
        let at = |v: Vec3| [v.x, v.y, v.z];
        let centre = |p: [i32; 3]| p.map(|v| f64::from(v) + 0.5);
        if let Some(i) = self.interactions.get(&id) {
            let (lo, hi) = corners(i.bounds());
            return Some((lo, hi, at(i.position)));
        }
        let id = u32::try_from(id).ok()?;
        // The level's own `minecraft:interaction`: on a server this game holds the
        // entity itself, with the width and height it was summoned with.
        if let Some(o) = self.others.iter().find(|o| o.id == id)
            && let Some(state) = o.interaction
        {
            let (lo, hi) = corners(state.bounds(o.position));
            return Some((lo, hi, at(o.position)));
        }
        if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            let (lo, hi) = corners(b.bounds());
            return Some((lo, hi, at(b.position)));
        }
        if let Some(c) = self.carts.iter().find(|c| c.id == id) {
            let (lo, hi) = corners(c.bounds());
            return Some((lo, hi, at(c.position)));
        }
        if let Some(tnt) = self.tnts.iter().find(|tnt| tnt.id == id) {
            let (lo, hi) = corners(tnt.bounds());
            return Some((lo, hi, at(tnt.position)));
        }
        if let Some(a) = self.armor_stands.iter().find(|a| a.id == id) {
            let (lo, hi) = corners(a.bounds());
            return Some((lo, hi, at(a.position)));
        }
        if let Some(c) = self.cushions.iter().find(|c| c.id == id) {
            let (lo, hi) = corners(Aabb::new(
                Vec3::new(c.position.x - 0.5, c.position.y, c.position.z - 0.5),
                Vec3::new(c.position.x + 0.5, c.position.y + 0.25, c.position.z + 0.5),
            ));
            return Some((lo, hi, at(c.position)));
        }
        if let Some(f) = self.item_frames.iter().find(|f| f.id == id) {
            let (lo, hi) = f.bounds();
            return Some((lo, hi, centre(f.pos)));
        }
        let p = self.paintings.iter().find(|p| p.id == id)?;
        let (lo, hi) = p.bounds();
        Some((lo, hi, centre(p.pos)))
    }

    /// The vehicles the server's players ride: they move with their riders,
    /// not in `tick_boats` and `tick_carts`.
    pub(super) fn guest_vehicles(&self) -> Vec<u32> {
        self.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub.guests.iter().filter_map(|g| g.vehicle).collect()
        })
    }

    /// Returns the live vehicle for guest `conn`, checking suspended outer turns in `lan_outer_ridings`.
    pub(super) fn guest_current_vehicle(&self, conn: u32) -> Option<u32> {
        if let Some(pos) = self.lan_guest_swaps.iter().position(|&c| c == conn) {
            if pos + 1 < self.lan_outer_ridings.len() {
                return self.lan_outer_ridings[pos + 1];
            } else if Some(conn) == self.lan_acting {
                return self.riding;
            }
        }
        self.lan.as_ref()?.hub.guest(conn)?.vehicle
    }

    /// Returns the live vehicle for the host.
    pub(super) fn host_current_vehicle(&self) -> Option<u32> {
        if self.lan_acting.is_some() {
            self.lan_outer_ridings.first().copied().flatten()
        } else {
            self.riding
        }
    }

    /// Whether a server player other than the one acting rides `vehicle`
    /// (`Entity.canAddPassenger`: a boat or minecart seats one player here).
    pub(super) fn ridden_by_guest(&self, vehicle: u32) -> bool {
        let other_guest = self.lan.as_ref().is_some_and(|h| {
            h.hub
                .guests
                .iter()
                .any(|g| {
                    Some(g.conn) != self.lan_acting
                        && self.guest_current_vehicle(g.conn) == Some(vehicle)
                })
        });
        let host_rides = self.lan_acting.is_some() && self.host_current_vehicle() == Some(vehicle);
        other_guest || host_rides
    }
    /// Whether a server player rides `vehicle` (`Entity.isPassenger`: a
    /// passenger does not use a portal on its own, the vehicle carries it
    /// across, `Entity.teleportCrossDimension`).
    pub(super) fn guest_rides(&self, vehicle: u32) -> bool {
        self.lan
            .as_ref()
            .is_some_and(|h| h.hub.guests.iter().any(|g| g.vehicle == Some(vehicle)))
    }

    /// `Player.rideTick` for a server player, before its moves this tick:
    /// it stays in its seat, steers its minecart, or gets out.
    pub(super) fn guest_vehicle_tick(&mut self, conn: u32) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &host.hub.guests[gi];
        let Some(vehicle) = g.vehicle else {
            return;
        };
        if self.persist.removed_vehicles.contains_key(&vehicle) {
            // A canceled destruction dismount still owns the removed object.
            // Retry in the player's tick, not by silently dropping the edge.
            self.as_guest(conn, |game| game.stop_riding());
            return;
        }
        let (keys, dead) = (input_keys(g.input), g.dead || g.health <= 0.0);
        let boat = self.boats.iter().position(|b| b.id == vehicle);
        // Choose the seat before swapping this guest into the window player's slot.
        let boat_seat = usize::from(boat.is_some() && !self.guest_controls_vehicle(conn, vehicle));
        let cart = self.carts.iter().any(|c| c.id == vehicle);
        let mount = self.mobs.iter().any(|m| m.id == vehicle);
        let cushion = self.cushions.iter().any(|c| c.id == vehicle);
        let seat = self.seat(vehicle).is_some();
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        // The vehicle is gone (broken, in another level) or the rider died:
        // its client drops it with the entity or the death screen.
        if dead || (boat.is_none() && !cart && !mount && !cushion && !seat) {
            host.hub.guests.update(gi, |g| {
                (g.vehicle, g.vehicle_good) = (None, None);
            });
            return;
        }
        let mut ejected = false;
        if let Some(i) = boat {
            // `Entity.ejectPassengers`: every player in the boat must see
            // the request; the common boat tick clears it once nobody rides.
            ejected = self.boats[i].eject_riders;
        }
        // ServerGamePacketListenerImpl.tick: the vehicle's last good position
        // is where it is now, so a move the server made (a piston, a push)
        // is not held against the rider's next move, which would fail, be
        // sent back, and fail again on every answer.
        let at = boat.map(|i| self.boats[i].position).or_else(|| {
            self.mobs
                .iter()
                .find(|m| m.id == vehicle && equine::moved_by_client(m))
                .map(|m| m.position)
        });
        if let Some(p) = at {
            {
                let value = Some([p.x, p.y, p.z]);
                host.hub
                    .guests
                    .update(gi, |guest| guest.vehicle_good = value);
            }
        }
        // Getting out goes through leave_guest, which tells the client and
        // teleports it (ServerPlayer.dismountTo).
        let seat = self.as_guest(conn, |game| {
            // A cushion does not move: the rider sits on it until it sneaks
            // (`Cushion.removePassenger` plays the get-up sound).
            if game.cushions.iter().any(|c| Some(c.id) == game.riding) {
                if keys.shift {
                    game.tick_cushion_riding(keys);
                    return None;
                }
                let c = game.cushions.iter().find(|c| Some(c.id) == game.riding)?;
                let p = c.seat();
                return Some([p.x, p.y, p.z]);
            }
            // An armor stand, a display, a marker...: the rider keeps its seat as the vehicle is
            // moved (/tp, a mod) until it sneaks.
            if game.riding.is_some_and(|id| game.seat(id).is_some()) {
                let p = game.tick_seat_riding(keys)?;
                return Some([p.x, p.y, p.z]);
            }
            if let Some(i) = game.boats.iter().position(|b| Some(b.id) == game.riding) {
                if keys.shift || ejected {
                    game.dismount(i);
                    return None;
                }
                let p = game.boat_seat(i, boat_seat);
                return Some([p.x, p.y, p.z]);
            }
            // The minecart moves with the rider's input, and lets it out when
            // it sneaks or the cart breaks. An animal takes the rider's input
            // for its own tick (`travelRidden`), or lets it off.
            if !game.tick_cart_riding(keys) && game.tick_mount_riding(keys) {
                game.riding?;
                game.position_rider();
            }
            game.riding?;
            let p = game.player.as_ref()?.position;
            Some([p.x, p.y, p.z])
        });
        if let Some(Some(at)) = seat
            && let Some(host) = self.lan.as_mut()
        {
            host.hub.guests.update(gi, |g| {
                g.previous_position = g.position;
                g.position = at;
            });
        }
    }

    /// The maximum passenger capacity of `vehicle`.
    pub(super) fn vehicle_capacity(&self, vehicle: u32) -> usize {
        if let Some(b) = self.boats.iter().find(|b| b.id == vehicle) {
            if b.chest { 1 } else { 2 }
        } else if let Some(m) = self.mobs.iter().find(|m| m.id == vehicle) {
            Self::mount_capacity(m.species())
        } else {
            1
        }
    }

    /// After an interaction: a player that got into a vehicle sits in it,
    /// and its last good vehicle position is where the vehicle is.
    pub(super) fn guest_boarded(&mut self, conn: u32, before: Option<u32>) {
        let Some((gi, vehicle)) = self.lan.as_ref().and_then(|host| {
            let gi = host.hub.guests.iter().position(|g| g.conn == conn)?;
            let vehicle = host.hub.guests[gi].vehicle?;
            Some((gi, vehicle))
        }) else {
            return;
        };
        if before == Some(vehicle) {
            return;
        }
        let capacity = self.vehicle_capacity(vehicle);
        let host_riders = usize::from(self.host_current_vehicle() == Some(vehicle));
        let guest_riders = self.lan.as_ref().map_or(0, |host| {
            host.hub
                .guests
                .iter()
                .filter(|g| self.guest_current_vehicle(g.conn) == Some(vehicle))
                .count()
        });
        let mob_riders = self
            .mobs
            .iter()
            .filter(|m| m.extra.mount == Some(vehicle) && !m.removed && m.alive())
            .count();
        if host_riders + guest_riders + mob_riders > capacity {
            if let Some(host) = self.lan.as_mut() {
                let value = before;
                host.hub.guests.update(gi, |guest| guest.vehicle = value);
            }
            return;
        }
        let at = self
            .boats
            .iter()
            .position(|b| b.id == vehicle)
            .map(|i| {
                let seat = usize::from(!self.guest_controls_vehicle(conn, vehicle));
                (self.boats[i].position, self.boat_seat(i, seat))
            })
            .or_else(|| {
                self.carts
                    .iter()
                    .find(|c| c.id == vehicle)
                    .map(|c| (c.position, c.position.add_vector(Vec3::new(0.0, SEAT, 0.0))))
            })
            .or_else(|| {
                self.cushions
                    .iter()
                    .find(|c| c.id == vehicle)
                    .map(|c| (c.position, c.seat()))
            })
            .or_else(|| self.seat(vehicle).map(|s| (s.position, s.player_seat())));
        if let Some((p, seat)) = at {
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            host.hub.guests.update(gi, |g| {
                g.vehicle_good = Some([p.x, p.y, p.z]);
                g.previous_position = g.position;
                g.position = [seat.x, seat.y, seat.z];
                g.on_ground = false;
            });
            return;
        }
        // An animal: the rider sits where `positionRider` puts it.
        let seat = self.as_guest(conn, |game| {
            game.position_rider();
            game.player
                .as_ref()
                .map(|p| [p.position.x, p.position.y, p.position.z])
        });
        if let Some(Some(at)) = seat
            && let Some(host) = self.lan.as_mut()
        {
            host.hub.guests.update(gi, |g| {
                g.previous_position = g.position;
                g.position = at;
                g.on_ground = false;
            });
        }
    }

    /// `ServerGamePacketListenerImpl.handleMoveVehicle`: the boat or the
    /// saddled horse its rider drives goes where the client says, unless it
    /// moved too quickly or wrongly; then it stays, and the client is told
    /// where it is.
    pub(super) fn guest_move_vehicle(&mut self, conn: u32, p: &play::MoveVehicleRequest) {
        let controls_vehicle = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .and_then(|g| g.vehicle)
            .is_none_or(|v| self.guest_controls_vehicle(conn, v));
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let [x, y, z] = p.position;
        if contains_invalid_values(p.position, p.yaw, p.pitch) {
            self.lan.as_mut().unwrap().hub.kick(
                conn,
                translatable("multiplayer.disconnect.invalid_vehicle_movement", &[]),
            );
            return;
        }
        let g = &host.hub.guests[gi];
        if g.awaiting_teleport.is_some() || !g.client_loaded() {
            return;
        }
        // Only the vehicle's controlling passenger moves it: the move of a
        // player in a boat's second seat is ignored (#11541).
        if !controls_vehicle {
            return;
        }
        let owner = host.hub.singleplayer_owner == Some(g.uuid);
        let allow_flight = host.hub.config.allow_flight;
        let (vehicle, good) = (g.vehicle, g.vehicle_good);
        let back = |host: &mut LanHost, position: Vec3, yaw: f32| {
            let packet = play::MoveVehicle {
                position: [position.x, position.y, position.z],
                yaw,
                pitch: 0.0,
            };
            host.hub.send_to(conn, &C::MoveVehicle(packet));
        };
        // Only a boat, and a horse its rider's client moves, are driven by
        // their rider (getControllingPassenger); a minecart that is elsewhere
        // on the client is put back.
        let boat = vehicle.and_then(|v| self.boats.iter().position(|b| b.id == v));
        let mount = vehicle.and_then(|v| {
            self.mobs
                .iter()
                .position(|m| m.id == v && equine::moved_by_client(m))
        });
        // The vehicle's id, kind, position, rotation, motion, box, step height
        // and ground.
        let (id, old, yaw, velocity, bounds, step, on_ground) = if let Some(i) = boat {
            let b = &self.boats[i];
            (
                b.id,
                b.position,
                b.yaw,
                b.velocity,
                b.bounds(),
                0.0,
                b.on_ground,
            )
        } else if let Some(i) = mount {
            let m = &self.mobs[i];
            let step = m.kind.step_height as f32;
            (
                m.id,
                m.position,
                m.yaw,
                m.velocity,
                m.bounds(),
                step,
                m.on_ground,
            )
        } else {
            if let Some(c) = vehicle.and_then(|v| self.carts.iter().find(|c| c.id == v))
                && c.position.distance_squared(Vec3::new(x, y, z)) > 1.0
            {
                back(self.lan.as_mut().unwrap(), c.position, c.yaw);
            }
            return;
        };
        let good = good.map_or(old, |g| Vec3::new(g[0], g[1], g[2]));
        let target = Vec3::new(
            x.clamp(-3.0E7, 3.0E7),
            y.clamp(-2.0E7, 2.0E7),
            z.clamp(-3.0E7, 3.0E7),
        );
        // Moved too quickly: more than 10 blocks past its own motion.
        let moved = target.distance_squared(old) - velocity.length_squared();
        if moved > 100.0 && !owner {
            log::warn!("vehicle {id} moved too quickly! {moved}");
            back(self.lan.as_mut().unwrap(), old, yaw);
            return;
        }
        // A living mount on a climbable has no fall, before its move: a move
        // put back below keeps that.
        if let Some(i) = mount
            && self.mobs[i].on_climbable(&self.mob_world_base(&[]))
        {
            self.mobs[i].fall_distance = 0.0;
        }
        // vehicle.move(PLAYER, target - lastGood) with the vehicle's
        // collisions: where it can't follow the client (a quarter block off,
        // the height aside), it moved wrongly. A Player-cause move through
        // the move gate (arismoko/horizon#2414); PR 1a records the movement unchanged.
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let wanted = target.subtract(good);
        // The vehicle's type name only if a listener asks for it (arismoko/horizon#2414: no
        // string on a move nobody listens to).
        let kind = || match (boat, mount) {
            (Some(i), _) => std::borrow::Cow::Owned(self.boats[i].entity_type()),
            (None, Some(i)) => std::borrow::Cow::Borrowed(&*self.mobs[i].kind.id),
            // Unreachable: the vehicle was a boat or a mount, or it returned.
            (None, None) => std::borrow::Cow::Borrowed(""),
        };
        let mut gate = horizon_client::move_gate::armed_with(
            &self.loop_context,
            id,
            kind,
            MoverType::Player,
            wanted,
        );
        let wanted = gate.delta();
        // A listener's decision that is not the request (arismoko/horizon#2414, ruling 5):
        // the driver is put back whatever the distance - the flag comes from
        // the gate, not from the numbers.
        let overridden = gate.overridden();
        let resolved = horizon_client::move_gate::scene_resolve(
            &mut gate, &mut scene, bounds, wanted, step, on_ground,
        )
        .unwrap_or(wanted);
        let reached = old.add_vector(resolved);
        let wrongly =
            overridden || (target.x - reached.x).powi(2) + (target.z - reached.z).powi(2) > 0.0625;
        let free_before = scene.unobstructed(bounds).unwrap_or(true);
        // isEntityCollidingWithAnythingNew: a box that was free is not there.
        let moved_box = bounds.move_by(target.subtract(old));
        let free_after = scene
            .unobstructed(moved_box.deflate(Vec3::new(1.0E-5, 1.0E-5, 1.0E-5)))
            .unwrap_or(true);
        // Entity.move in `vehicle.move`, before the checks: a long move through
        // water or a reset block resets the fall, which a move put back keeps
        // (`absSnapTo` restores only the position).
        if let Some(i) = mount {
            self.mobs[i].fall_distance =
                self.vehicle_fall_reset(self.mobs[i].fall_distance, old, resolved);
        } else if let Some(i) = boat {
            let reset = self.vehicle_fall_reset(self.boats[i].fall_distance, old, resolved);
            self.boats.update(i, |boat| boat.fall_distance = reset);
        }
        // An overridden decision is put back from wherever the vehicle starts:
        // the free-old-box exemption is the distance test's (review round 1),
        // like the hub's unobstructed-start check.
        if overridden || (free_before && (wrongly || !free_after)) {
            log::warn!("vehicle {id} moved wrongly");
            back(self.lan.as_mut().unwrap(), old, yaw);
            return;
        }
        // clientVehicleIsFloating: the move didn't go down a step, the vehicle hadn't come down onto
        // something on its last move (`verticalCollisionBelow` before this one), and nothing but air is
        // around its box grown by 0.0625 and stretched 0.55 down (water holds a boat up). A flying or
        // no-gravity mount, and a server that allows flight, never float.
        let (gravity, no_gravity) = match mount {
            Some(i) => (0.08, self.mobs[i].no_gravity),
            None => (0.04, false),
        };
        let around = moved_box
            .inflate(Vec3::new(0.0625, 0.0625, 0.0625))
            .expand_towards(Vec3::new(0.0, -0.55, 0.0));
        let corner = |max: bool| {
            [Axis::X, Axis::Y, Axis::Z].map(|a| if max { around.max(a) } else { around.min(a) })
        };
        let no_blocks = no_blocks_in(
            &self.world,
            corner(false),
            corner(true),
            &self.lan.as_ref().unwrap().hub.encoder,
        );
        self.lan.as_mut().unwrap().hub.guests.update(gi, |g| {
            // `ServerChunkCache.move` after `absSnapTo` of the accepted move.
            g.release_tickets();
            g.vehicle_good = Some([target.x, target.y, target.z]);
            let rested = std::mem::replace(
                &mut g.vehicle_rests,
                wanted.y < 0.0 && resolved.y != wanted.y,
            );
            g.vehicle_floating =
                wanted.y >= -0.03125 && !rested && !allow_flight && !no_gravity && no_blocks;
            g.vehicle_gravity = gravity;
            // handleMoveVehicle's `handlePlayerKnownMovement(clientDeltaMovement)`:
            // a rider sends no position of its own, so the move it gave its vehicle
            // is the player's known movement (ServerPlayer.getKnownSpeed), which a
            // spear charge reads as the speed it stabs with. `ClientTickEnd` keeps
            // it for the tick like any other move.
            g.known_movement = [target.x - old.x, target.y - old.y, target.z - old.z];
            g.movement_this_tick = true;
        });
        if let Some(i) = mount {
            let mut m = self.mobs[i].clone();
            let mut events = Vec::new();
            let mut lent = horizon_client::health::LentLevel::take(&mut self.world);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let w = self.mob_world_base(&[]);
                // absSnapTo: where and how its rider's client turned it; the
                // rider sits on it from its own ride tick.
                let delta = target.subtract(m.position);
                m.position = target;
                m.yaw = p.yaw;
                m.head_pitch = p.pitch;
                // setOnGroundWithMovement and doCheckFallDamage with the client's
                // ground and move: a landing hurts the horse and its riders.
                let mut level = horizon_client::health::DamageLevel::lent(
                    &self.world,
                    self.pack.assets.registry,
                    &mut lent,
                    Some(self.items()),
                    Some(&self.pack.block_registries),
                    &self.damage_hooks,
                );
                m.client_moved_fall(&w, &mut level, delta, p.on_ground, &mut events);
            }));
            lent.give_back(&mut self.world);
            if let Err(panic) = outcome {
                std::panic::resume_unwind(panic);
            }
            self.mobs[i] = m;
            // propagateFallToPassengers reaches the guests through `host`: it is out
            // of `self.lan` while their packet is handled.
            let falls: Vec<(u32, f64, f32)> = events
                .iter()
                .filter_map(|e| match *e {
                    horizon_client::mobs::MobEvent::FallToPassengers {
                        mob,
                        distance,
                        modifier,
                    } => Some((mob, distance, modifier)),
                    _ => None,
                })
                .collect();
            // LivingEntity.checkFallDamage's landing crumbs go to every player
            // (`sendParticles`), the guests too.
            let (dust, events): (Vec<_>, Vec<_>) = events
                .into_iter()
                .partition(|e| matches!(e, horizon_client::mobs::MobEvent::LandingDust { .. }));
            for e in dust {
                if let horizon_client::mobs::MobEvent::LandingDust {
                    state,
                    at,
                    count,
                    speed,
                } = e
                {
                    self.landing_dust(state, Vec3::new(at[0], at[1], at[2]), count, speed);
                }
            }
            let _ = self.handle_mob_events(events);
            for (mob, distance, modifier) in falls {
                let riders: Vec<u32> = self
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guests
                    .iter()
                    .filter(|g| g.vehicle == Some(mob))
                    .map(|g| g.conn)
                    .collect();
                for conn in riders {
                    self.guest_hit_by(conn, |g| g.rider_fall_damage(distance, modifier));
                }
            }
            return;
        }
        let Some(i) = boat else {
            return;
        };
        self.boats.update(i, |b| {
            b.previous = old;
            b.velocity = target.subtract(old);
            b.position = target;
            b.previous_yaw = b.yaw;
            b.yaw = p.yaw;
            b.on_ground = p.on_ground;
        });
        // handleMoveVehicle's `vehicle.move` counts the fall
        // (AbstractBoat.checkFallDamage); the server's baseTick resets it in
        // water (updateFluidInteraction), which this boat's skipped tick
        // doesn't run.
        use super::entity::Entity as _;
        let mut probe = self.boats[i].clone();
        let in_water = probe
            .push_by_fluid(&self.world, &self.res.motion().fluids, &self.loop_context)
            .0
            > 0.0;
        // The long move reset ran before collision validation.
        let fall = if in_water {
            0.0
        } else {
            self.boat_fall(&self.boats[i], resolved.y)
        };
        self.boats.update(i, |b| b.fall_distance = fall);
        let seat = self.player_boat_seat(i);
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        host.hub.guests.update(gi, |g| {
            g.previous_position = g.position;
            g.position = [seat.x, seat.y, seat.z];
        });
    }

    /// `Entity.getControllingPassenger` of a vehicle a server player rides: a
    /// boat has two seats, and only its first player passenger drives it. The
    /// window player sits first when it rides; of the server's players it is
    /// the one that got in first.
    pub(super) fn guest_controls_vehicle(&self, conn: u32, vehicle: u32) -> bool {
        self.riding != Some(vehicle)
            && self
                .lan
                .as_ref()
                .is_some_and(|host| vehicle_driver(&host.hub.guests, vehicle) == Some(conn))
    }

    /// `handlePaddleBoat`: the paddles of the boat the player controls
    /// (`player.getControlledVehicle()`); a passenger in the second seat moves
    /// no paddle.
    pub(super) fn guest_paddle_boat(&mut self, conn: u32, paddles: [bool; 2]) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let vehicle = host
            .hub
            .guests
            .iter()
            .find(|g| g.conn == conn)
            .and_then(|g| g.vehicle);
        let Some(vehicle) = vehicle.filter(|&v| self.guest_controls_vehicle(conn, v)) else {
            return;
        };
        if let Some(i) = self.boats.iter().position(|b| b.id == vehicle) {
            self.boats.update(i, |b| {
                b.paddling = paddles;
                b.paddle_controller = true;
            });
        }
    }
}

/// `Entity.getControllingPassenger` of a vehicle a server player rides: a
/// boat has two seats, and only its first player passenger drives it. The
/// window player sits first when it rides — `guest_boarded` keeps a guest off
/// a vehicle the window player rides — and of the server's players it is the
/// one that got in first.
pub(super) fn vehicle_driver(guests: &[Guest], vehicle: u32) -> Option<u32> {
    guests
        .iter()
        .filter(|g| g.vehicle == Some(vehicle))
        .min_by_key(|g| g.vehicle_order)
        .map(|g| g.conn)
}

/// `ServerPlayer.getKnownMovement` of a rider whose vehicle one of the
/// window's players drives: that driver's move (`Entity.getKnownSpeed`), the
/// move its client sent for the vehicle (`handleMoveVehicle` →
/// `handlePlayerKnownMovement`). `None` when `conn` is that driver itself, or
/// rides nothing: then its own move is its known movement.
///
/// What [`vehicle_driver`] counts is the first of the server's players on the
/// vehicle, which vanilla's `Entity.getControllingPassenger` is not whenever
/// it hands no player the reins: a minecart (`AbstractMinecart` has none), a
/// pig or a strider whose rider holds no carrot on a stick
/// (`Pig.getControllingPassenger`, `Strider.getControllingPassenger`), a mount
/// nobody saddled or anything else a `/ride` puts a player on. Vanilla reads
/// `Entity.getKnownSpeed`'s own `lastKnownSpeed` there, the vehicle's move per
/// tick, which the caller has to find: `Game::known_player_movement` reads a
/// mob's with `Game::mob_speed` and keeps no source for a boat or a minecart
/// yet. The caller must check that a player actually controls the vehicle
/// before borrowing the first guest's client movement.
pub(super) fn ridden_known_movement(guests: &[Guest], conn: u32, vehicle: u32) -> Option<Vec3> {
    let driver = vehicle_driver(guests, vehicle).filter(|&driver| driver != conn)?;
    let [x, y, z] = guests.iter().find(|g| g.conn == driver)?.known_movement;
    Some(Vec3::new(x, y, z))
}

/// `ServerGamePacketListenerImpl.containsInvalidValues(DDDFF)`: a NaN coordinate or a rotation
/// that is not finite. An infinite coordinate is valid there: `handleMoveVehicle` clamps it and
/// goes on with the usual checks, so it does not disconnect.
fn contains_invalid_values([x, y, z]: [f64; 3], yaw: f32, pitch: f32) -> bool {
    x.is_nan() || y.is_nan() || z.is_nan() || !yaw.is_finite() || !pitch.is_finite()
}

#[cfg(test)]
mod controller_tests {
    use super::{ridden_known_movement, vehicle_driver};
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::chunk::Chunk;
    use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use horizon_core::geometry::Vec3;
    use horizon_core::position::ChunkPos;
    use horizon_protocol::packets::{configuration::ClientInformation, play};
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    fn guest(conn: u32, uuid: u128) -> Guest {
        guest_with_packets(conn, uuid).0
    }

    /// A guest and what is sent to it.
    fn guest_with_packets(
        conn: u32,
        uuid: u128,
    ) -> (Guest, mpsc::Receiver<horizon_client::lan::Out>) {
        let (tx, rx) = mpsc::channel();
        let mut g = Guest::new(
            Joined {
                conn,
                uuid,
                name: format!("Rider{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            1000 + conn as i32,
            [4.5, 200.0, 4.5],
            2,
        );
        g.dimension = "minecraft:overworld".into();
        g.placed = true;
        g.loaded = true;
        // PlayerLoaded came in: its moves count.
        g.load_timeout = 0;
        (g, rx)
    }

    /// `ServerPlayer.getKnownMovement` of a rider that is not at the reins: it
    /// takes the vehicle's (`Entity.getKnownSpeed`), the move its controlling
    /// passenger's client sent for it (`handleMoveVehicle` →
    /// `handlePlayerKnownMovement`). A rider sends no position of its own, so
    /// the second seat of a boat has nothing of its own to carry: a spear
    /// charge there reads the boat's speed, as the driver's does (#17092).
    #[test]
    fn a_passenger_that_does_not_drive_takes_its_vehicles_move() {
        let (mut passenger, _) = guest_with_packets(2, 21);
        let (mut driver, _) = guest_with_packets(3, 22);
        (driver.vehicle, driver.vehicle_order) = (Some(7), 1);
        (passenger.vehicle, passenger.vehicle_order) = (Some(7), 2);
        driver.known_movement = [0.3, 0.0, 0.0];
        assert_eq!(
            passenger.known_movement, [0.0; 3],
            "a rider that drives nothing sent no move of its own"
        );
        let guests = [passenger, driver];
        assert_eq!(
            vehicle_driver(&guests, 7),
            Some(3),
            "the one that got in first drives it"
        );
        assert_eq!(
            ridden_known_movement(&guests, 2, 7),
            Some(Vec3::new(0.3, 0.0, 0.0)),
            "the second seat carries the boat's move"
        );
        assert_eq!(
            ridden_known_movement(&guests, 3, 7),
            None,
            "the driver's known movement is the one it sent"
        );
        assert_eq!(
            ridden_known_movement(&guests, 2, 9),
            None,
            "a passenger of another vehicle takes nothing from it"
        );
    }

    /// `Game::known_player_movement` of a guest on a mob: a ride vanilla hands
    /// no player the reins of — no saddle, no carrot on a stick, whatever a
    /// `/ride` puts a player on — takes the mount's own move of the tick
    /// (`ServerPlayer.getKnownSpeed` → `Entity.getKnownSpeed` →
    /// `Entity.lastKnownSpeed`), where the rider carries nothing of its own; a
    /// ride it does steer keeps the move its client sent for the mount
    /// (#17092). The game is built from the prepared vanilla data
    /// `test_fixtures::vanilla` reads — `HORIZON_TEST_RESOURCES`, else the
    /// checkout's `.local` — so it runs in the daily prepared-resource tier.
    #[test]
    #[ignore = "daily"]
    fn a_mount_nobody_steers_carries_its_rider_with_its_own_move() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(std::env::temp_dir().join(format!(
            "horizon-unsaddled-mount-unit-{}",
            std::process::id()
        )));
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("summon minecraft:horse 4.5 200 4.5 {NoAI:1b,Tame:1b}"));
        let horse = game.mobs[0].id;
        // The move the horse made in the tick, `Entity.computeSpeed`'s
        // `lastKnownSpeed`: from where it stood to where it stands now, a
        // canter's 0.2 blocks a tick.
        game.mobs[0].previous = Vec3::new(4.5, 200.0, 4.5);
        game.mobs[0].position = Vec3::new(4.7, 200.0, 4.5);
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "unsaddled-mount-test".into(), 3)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 3,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let mut host = game.lan.take().unwrap();
        let (mut rider, _) = guest_with_packets(2, 21);
        (rider.vehicle, rider.vehicle_order) = (Some(horse), 1);
        // The move such a rider's client sends for its mount when it does
        // steer it (`handleMoveVehicle`).
        rider.known_movement = [0.3, 0.0, 0.0];
        host.hub.guests.push(rider);
        game.lan = Some(host);
        // Nobody steers an unsaddled horse (`AbstractHorse.getControllingPassenger`
        // is null), so the charge carries the horse's own move.
        let seen = game.as_guest(2, |g| g.known_player_movement()).unwrap();
        assert!(
            (seen.x - 0.2).abs() < 1.0e-9 && seen.y == 0.0 && seen.z == 0.0,
            "a mount nobody steers charges with its own move: {seen:?}"
        );
        // A second guest does not turn the first rider into a controlling
        // passenger while the horse is still unsaddled.
        let (mut passenger, _) = guest_with_packets(3, 22);
        (passenger.vehicle, passenger.vehicle_order) = (Some(horse), 2);
        game.lan.as_mut().unwrap().hub.guests.push(passenger);
        let seen = game.as_guest(3, |g| g.known_player_movement()).unwrap();
        assert!(
            (seen.x - 0.2).abs() < 1.0e-9 && seen.y == 0.0 && seen.z == 0.0,
            "the second passenger also takes the unsteered mount's own move: {seen:?}"
        );
        // Saddled, with its rider at the reins (`equine::controlled`): the move
        // that rider's client sent, not the mount's.
        game.mobs[0].extra.set_saddled(true);
        game.mobs[0].rider = Some(horizon_client::mobs::equine::Rider {
            client: true,
            ..Default::default()
        });
        let seen = game.as_guest(2, |g| g.known_player_movement()).unwrap();
        assert!(
            (seen.x - 0.3).abs() < 1.0e-9 && seen.y == 0.0 && seen.z == 0.0,
            "a rider at the reins charges with its own move: {seen:?}"
        );
        let seen = game.as_guest(3, |g| g.known_player_movement()).unwrap();
        assert!(
            (seen.x - 0.3).abs() < 1.0e-9 && seen.y == 0.0 && seen.z == 0.0,
            "the second passenger takes the controlling rider's client move: {seen:?}"
        );
    }

    /// vanilla's `containsInvalidValues`: only a NaN coordinate or a non-finite rotation is invalid.
    #[test]
    fn only_a_nan_coordinate_or_a_non_finite_rotation_is_invalid() {
        use super::contains_invalid_values as invalid;
        assert!(!invalid([1.0, 64.0, -3.5], 90.0, 10.0));
        for at in [f64::INFINITY, f64::NEG_INFINITY] {
            assert!(!invalid([at, 64.0, 0.0], 0.0, 0.0), "x {at}");
            assert!(!invalid([0.0, at, 0.0], 0.0, 0.0), "y {at}");
            assert!(!invalid([0.0, 64.0, at], 0.0, 0.0), "z {at}");
        }
        assert!(invalid([f64::NAN, 64.0, 0.0], 0.0, 0.0));
        assert!(invalid([0.0, f64::NAN, 0.0], 0.0, 0.0));
        assert!(invalid([0.0, 64.0, f64::NAN], 0.0, 0.0));
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(invalid([0.0, 64.0, 0.0], bad, 0.0), "yaw {bad}");
            assert!(invalid([0.0, 64.0, 0.0], 0.0, bad), "pitch {bad}");
        }
    }

    /// `Game::known_player_movement` of a passenger that is not its vehicle's
    /// controlling passenger: a spear charge in a boat's second seat carries
    /// the boat's move (`ServerPlayer.getKnownMovement` →
    /// `Entity.getKnownSpeed`), the one the driver's client sent, and not the
    /// nothing a rider sends of its own (#17092). The game is built from the
    /// prepared vanilla data `test_fixtures::vanilla` reads —
    /// `HORIZON_TEST_RESOURCES`, else the checkout's `.local` — so it runs in
    /// the daily prepared-resource tier.
    #[test]
    #[ignore = "daily"]
    fn a_second_seats_charge_carries_the_boats_move() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::env::temp_dir().join(format!("horizon-second-seat-unit-{}", std::process::id())),
        );
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("summon minecraft:oak_boat 4.5 200 4.5"));
        let boat = game.boats[0].id;
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "second-seat-test".into(), 3)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 3,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let mut host = game.lan.take().unwrap();
        let (mut passenger, _) = guest_with_packets(2, 21);
        let (mut driver, _) = guest_with_packets(3, 22);
        (passenger.vehicle, passenger.vehicle_order) = (Some(boat), 2);
        (driver.vehicle, driver.vehicle_order) = (Some(boat), 1);
        // What the driver's client sent for the boat (`handleMoveVehicle`).
        driver.known_movement = [0.3, 0.0, 0.0];
        host.hub.guests.push(passenger);
        host.hub.guests.push(driver);
        game.lan = Some(host);
        let seen = game.as_guest(2, |g| g.known_player_movement()).unwrap();
        assert!(
            (seen.x - 0.3).abs() < 1.0e-9 && seen.y == 0.0 && seen.z == 0.0,
            "the second seat charges with the boat's move: {seen:?}"
        );
    }

    /// Two players in one boat: the one that got in first drives it
    /// (`getControllingPassenger`), whichever joined the server first. Its
    /// client hears it sits first, and only its moves count; the second
    /// seat's client, which still sends where it sees the boat, used to put
    /// the boat back every tick (#11541).
    #[test]
    #[ignore = "daily"]
    fn the_first_rider_in_drives_the_boat() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::env::temp_dir().join(format!("horizon-boat-driver-unit-{}", std::process::id())),
        );
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0;
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone);
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("summon minecraft:oak_boat 4.5 200 4.5"));
        let boat = game.boats[0].id;
        let start = game.boats[0].position;
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "boat-driver-test".into(), 3)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 3,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let mut host = game.lan.take().unwrap();
        // The second seat's player joined the server first; the driver got
        // into the boat first.
        let (mut passenger, mut driver) = (guest(2, 21), guest(3, 22));
        (driver.vehicle, driver.vehicle_order) = (Some(boat), 1);
        (passenger.vehicle, passenger.vehicle_order) = (Some(boat), 2);
        let (passenger_id, driver_id) = (passenger.entity_id, driver.entity_id);
        host.hub.guests.push(passenger);
        host.hub.guests.push(driver);

        let mut seated: Vec<_> = host
            .hub
            .guests
            .iter()
            .filter_map(|g| Some((g.vehicle_order, g.vehicle?, g.entity_id)))
            .collect();
        seated.sort_by_key(|&(order, ..)| order);
        let guests: Vec<_> = seated
            .into_iter()
            .map(|(_, vehicle, id)| (vehicle, id))
            .collect();
        game.lan_riding(&guests, &mut host.hub.riding);
        assert_eq!(
            host.hub.riding.passengers(boat as i32),
            [driver_id, passenger_id],
            "the driver sits first"
        );

        let move_to = |x: f64| play::MoveVehicleRequest {
            position: [x, start.y, start.z],
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
        };
        game.lan = Some(host);
        game.guest_move_vehicle(2, &move_to(start.x + 0.3));
        assert_eq!(
            game.boats[0].position, start,
            "the second seat moves nothing"
        );
        game.guest_move_vehicle(3, &move_to(start.x + 0.3));
        assert!(
            (game.boats[0].position.x - (start.x + 0.3)).abs() < 1.0e-9,
            "the driver moves the boat: {:?}",
            game.boats[0].position
        );
        // `ServerPlayer.getKnownMovement` of the second seat: it does not
        // drive the boat, so it takes the vehicle's (`Entity.getKnownSpeed`)
        // — the move the driver's client sent. A rider sends no position of
        // its own, and a spear charge reads this as the speed it stabs with
        // (`KineticWeapon.getMotion`), so without it the second seat stabbed
        // as if it stood still.
        let seen = game.as_guest(2, |g| g.known_player_movement());
        let [x, y, z] = seen.map(|m| [m.x, m.y, m.z]).unwrap();
        assert!(
            (x - 0.3).abs() < 1.0e-9 && y == 0.0 && z == 0.0,
            "the second seat's charge carries the boat's move: {seen:?}"
        );
        // handleMoveVehicle's `vehicle.move` runs AbstractBoat.checkFallDamage:
        // a boat that lands on the ground (a fall loaded from its save, here)
        // has no fall left.
        game.boats.update(0, |b| b.fall_distance = 5.0);
        game.guest_move_vehicle(3, &move_to(start.x + 0.6));
        assert_eq!(
            game.boats[0].fall_distance, 0.0,
            "a guest's boat on the ground"
        );
        // `AbstractBoat.tick` still runs `applyEffectsFromBlocks` on the server
        // for the boat its guest drives: `WebBlock.entityInside` resets the fall
        // (`Entity.makeStuckInBlock`).
        let cobweb = game
            .pack
            .assets
            .registry
            .default_state("minecraft:cobweb")
            .unwrap();
        let p = game.boats[0].position;
        game.world
            .set(
                p.x.floor() as i32,
                p.y.floor() as i32,
                p.z.floor() as i32,
                cobweb,
            )
            .unwrap();
        game.boats.update(0, |b| b.fall_distance = 4.0);
        game.guest_move_vehicle(
            3,
            &play::MoveVehicleRequest {
                position: [p.x, p.y, p.z],
                yaw: 0.0,
                pitch: 0.0,
                on_ground: false,
            },
        );
        assert_eq!(game.boats[0].fall_distance, 4.0, "still falling");
        game.tick_boats();
        assert_eq!(
            game.boats[0].fall_distance, 0.0,
            "a cobweb resets the fall of a boat its guest drives"
        );
    }

    /// handleMoveVehicle for a horse its guest's client moves: `vehicle.move`
    /// leaves the fall alone (the server isn't the mount's authority), then
    /// `doCheckFallDamage` with the client's move and ground adds each drop and,
    /// on the ground, lands the horse: `AbstractHorse.causeFallDamage` of the
    /// whole fall. The horse used to keep no fall at all (#13042).
    #[test]
    #[ignore = "daily"]
    fn a_guests_horse_falls_and_lands() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::env::temp_dir().join(format!("horizon-horse-fall-unit-{}", std::process::id())),
        );
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0;
        let cobweb = game
            .pack
            .assets
            .registry
            .default_state("minecraft:cobweb")
            .unwrap()
            .0;
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone);
            }
        }
        // Off the fall's line: only the long moves at the end cross them.
        chunk.set(6, 205, 4, cobweb);
        chunk.set(8, 205, 4, cobweb);
        for y in 205..=207 {
            chunk.set(10, y, 4, stone);
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("summon minecraft:horse 4.5 210 4.5 {NoAI:1b,Tame:1b}"));
        let horse = game.mobs[0].id;
        game.mobs[0].extra.set_saddled(true);
        game.mobs[0].rider = Some(horizon_client::mobs::equine::Rider {
            client: true,
            ..Default::default()
        });
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "horse-fall-test".into(), 3)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 3,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();

        let (mut rider, packets) = guest_with_packets(2, 21);
        (rider.vehicle, rider.vehicle_order) = (Some(horse), 1);
        game.lan.as_mut().unwrap().hub.guests.push(rider);
        // What the landing should do: the horse's own fall damage for the whole drop.
        let mut twin = game.mobs[0].clone();
        twin.cause_fall_damage_from(
            &game.loop_context,
            &mut crate::health::TestLevel::new(0).level(),
            10.0,
            1.0,
            &mut Vec::new(),
        );
        assert!(
            twin.health < game.mobs[0].health,
            "a 10-block fall hurts a horse"
        );

        let move_to = |y: f64, on_ground: bool| play::MoveVehicleRequest {
            position: [4.5, y, 4.5],
            yaw: 0.0,
            pitch: 0.0,
            on_ground,
        };
        for y in [209.0, 207.0, 204.0] {
            game.guest_move_vehicle(2, &move_to(y, false));
        }
        assert_eq!(game.mobs[0].position.y, 204.0, "the guest moves the horse");
        assert_eq!(
            game.mobs[0].fall_distance, 6.0,
            "each drop adds to the fall"
        );
        game.guest_move_vehicle(2, &move_to(200.0, true));
        assert_eq!(game.mobs[0].fall_distance, 0.0, "the landing ends the fall");
        assert_eq!(
            game.mobs[0].health, twin.health,
            "the landing hurts as the fall does"
        );
        // propagateFallToPassengers: the rider's own causeFallDamage of the fall,
        // ceil((10 - 3) * 1) = 7.
        assert_eq!(
            game.lan.as_mut().unwrap().hub.guests[0].health,
            13.0,
            "the rider takes the fall too"
        );
        // LivingEntity.checkFallDamage sends the landing's block crumbs to every
        // player near, the rider too.
        let dust = packets.try_iter().any(|out| match out {
            horizon_client::lan::Out::Packet(body) => {
                let mut r = horizon_protocol::buf::Reader::new(&body);
                let id = r.varint().unwrap();
                matches!(
                    play::Clientbound::decode(id, r.rest()),
                    Ok(play::Clientbound::LevelParticles(_))
                )
            }
            _ => false,
        });
        assert!(dust, "the guest sees the landing dust");

        // Entity.move in `vehicle.move`: a move of a block or more casts along
        // it (FALLDAMAGE_RESETTING, WATER), and the cobweb it crosses resets
        // the fall (a fall loaded from its save, here) before the move counts.
        game.guest_move_vehicle(2, &move_to(205.5, false));
        assert_eq!(game.mobs[0].position.y, 205.5, "the guest lifts the horse");
        game.mobs[0].fall_distance = 4.0;
        game.guest_move_vehicle(
            2,
            &play::MoveVehicleRequest {
                position: [6.5, 205.5, 4.5],
                yaw: 0.0,
                pitch: 0.0,
                on_ground: false,
            },
        );
        assert_eq!(game.mobs[0].position.x, 6.5, "the guest moves the horse on");
        assert_eq!(
            game.mobs[0].fall_distance, 0.0,
            "a long move through a cobweb resets the fall"
        );
        // handleMoveVehicle moves the horse before it checks the move: one
        // into a wall is put back (moved wrongly), and keeps the reset.
        game.mobs[0].fall_distance = 4.0;
        game.guest_move_vehicle(
            2,
            &play::MoveVehicleRequest {
                position: [11.5, 205.5, 4.5],
                yaw: 0.0,
                pitch: 0.0,
                on_ground: false,
            },
        );
        assert_eq!(game.mobs[0].position.x, 6.5, "the move is put back");
        assert_eq!(
            game.mobs[0].fall_distance, 0.0,
            "a move put back keeps the cobweb's reset"
        );
    }

    /// handleMoveVehicle's `handlePlayerKnownMovement(clientDeltaMovement)`: the
    /// move a rider gives its horse is that player's known movement
    /// (`ServerPlayer.getKnownSpeed`), which a spear charge reads as the speed
    /// it stabs with (`Player.stabAttack`'s `getMotion`): a rider sends no
    /// position of its own, only the vehicle's. `handleClientTickEnd` keeps it
    /// for that tick, and zeroes it without a move.
    #[test]
    #[ignore = "daily"]
    fn a_guests_horse_move_is_its_riders_known_movement() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::env::temp_dir().join(format!("horizon-horse-move-unit-{}", std::process::id())),
        );
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0;
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone);
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("summon minecraft:horse 4.5 200 4.5 {NoAI:1b,Tame:1b}"));
        let horse = game.mobs[0].id;
        game.mobs[0].extra.set_saddled(true);
        game.mobs[0].rider = Some(horizon_client::mobs::equine::Rider {
            client: true,
            ..Default::default()
        });
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "horse-move-test".into(), 3)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 3,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let mut host = game.lan.take().unwrap();
        let (mut rider, _packets) = guest_with_packets(2, 21);
        (rider.vehicle, rider.vehicle_order) = (Some(horse), 1);
        host.hub.guests.push(rider);

        let move_to = |x: f64| play::MoveVehicleRequest {
            position: [x, 200.0, 4.5],
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
        };
        // The rider's client sends its vehicle's move and its tick end in one
        // client tick, and both go through the inbox: the hub hands the game the
        // MoveVehicle and queues the tick-end reset behind it, so the game sets
        // the rider's movement before the reset keeps it for the tick. Calling
        // guest_move_vehicle directly would apply the move ahead of the drain and
        // miss that order (#17206's review): the reset used to run inline before
        // the queued move, leaving the flag set and a stale movement kept into the
        // next tick.
        host.hub.guests.update(0, |g| {
            g.inbox.push_back(crate::lan::guest::InboxItem::Packet(
                play::Serverbound::MoveVehicle(move_to(4.9)),
                None,
            ));
            g.inbox.push_back(crate::lan::guest::InboxItem::Packet(
                play::Serverbound::ClientTickEnd(horizon_protocol::packets::Empty),
                None,
            ));
        });
        game.lan = Some(host);
        game.tick_lan_packets();
        host = game.lan.take().unwrap();
        assert_eq!(game.mobs[0].position.x, 4.9, "the guest moves the horse");
        let moved = host.hub.guests[0].known_movement;
        assert!(
            (moved[0] - 0.4).abs() < 1.0e-9 && moved[1] == 0.0 && moved[2] == 0.0,
            "the horse's move is its rider's known movement: {moved:?}"
        );
        assert!(
            !host.hub.guests[0].movement_this_tick,
            "the tick end cleared the flag after the queued move, not before it"
        );
        // The horse's own tick kept its position as the last one it had
        // (`Entity.computeSpeed`'s last known position, which a mob that its
        // rider's client moves never beats: `travelRidden` only stops it). Its
        // rider's speed is still the move the client sent, so a spear charge
        // on horseback carries the horse's speed instead of none.
        game.mobs[0].previous = game.mobs[0].position;
        game.lan = Some(host);
        let seen = game.as_guest(2, |g| g.known_player_movement());
        host = game.lan.take().unwrap();
        assert_eq!(
            seen.map(|m| [m.x, m.y, m.z]),
            Some(moved),
            "a spear charge on horseback carries the horse's speed"
        );
        // The next client tick: the level pass's packet gate opens again.
        game.lan = Some(host);
        game.tick_lan(false);
        host = game.lan.take().unwrap();
        // That tick has no move of either, so its tick end zeroes the movement
        // the last tick kept. With the reset ahead of the queued move the flag
        // stayed set, and this tick kept the stale move instead of zeroing it.
        host.hub.guests.update(0, |g| {
            g.inbox.push_back(crate::lan::guest::InboxItem::Packet(
                play::Serverbound::ClientTickEnd(horizon_protocol::packets::Empty),
                None,
            ));
        });
        game.lan = Some(host);
        game.tick_lan_packets();
        host = game.lan.take().unwrap();
        assert_eq!(
            host.hub.guests[0].known_movement,
            [0.0; 3],
            "a client tick with no move of either has none"
        );
        game.lan = Some(host);
    }
}
