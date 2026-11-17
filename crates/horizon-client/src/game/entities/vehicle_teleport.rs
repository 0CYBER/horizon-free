//! The writers `/tp` and `/rotate` share with a vehicle's riders: a vehicle
//! that is teleported takes its passengers along in the same operation, by the
//! vehicle's offset and the angles it was placed at (`Entity.teleport` ->
//! `teleportSameDimension` -> `calculatePassengerTransition`), and `/rotate`
//! turns an entity without teleporting it (`RotateCommand` ->
//! `Entity.forceSetRotation`).
use super::Game;
use super::commands::Target;
use super::entity::{Entity as _, with_entity, with_entity_mut};
use horizon_core::geometry::Vec3;
use horizon_core::nbt::Tag;
use horizon_protocol::packets::play::{self, Clientbound as C};

fn is_horse(species: horizon_client::mobs::Species) -> bool {
    use horizon_client::mobs::Species::*;
    matches!(
        species,
        Horse
            | Donkey
            | Mule
            | SkeletonHorse
            | ZombieHorse
            | Camel
            | CamelHusk
            | Llama
            | TraderLlama
    )
}

/// `Entity.calculatePassengerTransition` with no relative axes: the passenger
/// keeps its offset from the vehicle.
pub(super) fn passenger_destination(old_vehicle: Vec3, new_vehicle: Vec3, passenger: Vec3) -> Vec3 {
    Vec3::new(
        new_vehicle.x + (passenger.x - old_vehicle.x),
        new_vehicle.y + (passenger.y - old_vehicle.y),
        new_vehicle.z + (passenger.z - old_vehicle.z),
    )
}

/// `PositionMoveRotation.calculateAbsolute` then `Entity.setXRot`: the pitch
/// is clamped to -90..90 first (so 360 and +infinity are +90), and NaN is
/// discarded (`None`).
pub(super) fn absolute_pitch(pitch: f32) -> Option<f32> {
    let clamped = pitch.clamp(-90.0, 90.0);
    (!clamped.is_nan()).then(|| (clamped % 360.0).clamp(-90.0, 90.0))
}

/// `Entity.calculatePassengerTransition` for the angles: a rider turns by what
/// the vehicle turned, keeping `passenger.getYRot() - getYRot()`. An absolute
/// angle adds that offset to the vehicle's new one; a relative one (`~`) is a
/// delta the rider adds to its own: the same sum. `after` is the angle before
/// the vehicle's pitch clamp; the rider's pitch is clamped on its own
/// (`Entity.setXRot`), and an axis the command did not set leaves the rider's.
pub(super) fn passenger_rotation(before: [f32; 2], after: [f32; 2], rider: [f32; 2]) -> [f32; 2] {
    let axis = |i: usize| {
        if after[i].is_finite() {
            after[i] + (rider[i] - before[i])
        } else {
            rider[i]
        }
    };
    [axis(0), absolute_pitch(axis(1)).unwrap_or(rider[1])]
}

/// The angles a command turned a vehicle to, with the angles it had before:
/// what `Entity.calculatePassengerTransition` needs for the riders.
#[derive(Clone, Copy)]
pub(super) struct PlacedRotation {
    pub before: [f32; 2],
    pub after: [f32; 2],
}

/// What a teleport's `TeleportTransition` does with the entity's motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum TeleportMotion {
    /// `Entity.teleportTo(ServerLevel, ..)`, as `/tp` calls it: no delta and no
    /// `Relative.DELTA`, so the entity arrives still.
    Stop,
    /// A portal's `Relative.DELTA` with no delta of its own: the entity keeps
    /// the motion it had (`PositionMoveRotation.of(this)`), unturned.
    Keep,
}

/// `AbstractMinecart.getKnownMovement` with `OldMinecartBehavior`: the cart's
/// motion clamped to 0.4 either way along x and z (or zero when any component
/// is NaN), what a teleport's `PositionMoveRotation.of(this)` carries over.
pub(super) fn cart_known_movement(velocity: Vec3) -> Vec3 {
    if velocity.x.is_nan() || velocity.y.is_nan() || velocity.z.is_nan() {
        return Vec3::ZERO;
    }
    Vec3::new(
        velocity.x.clamp(-0.4, 0.4),
        velocity.y,
        velocity.z.clamp(-0.4, 0.4),
    )
}

/// The entity id a vehicle's riders point at, for the targets that carry some.
fn vehicle_id(t: Target) -> Option<u32> {
    super::native_entities::target_id(t)
}

#[cfg(test)]
#[path = "vehicle_teleport/teleport_cross_tests.rs"]
mod teleport_cross_tests;

impl Game<'_> {
    /// `AbstractMinecart.getDismountLocationForPassenger` for a standing mob.
    pub(super) fn cart_dismount_point(&self, vehicle: u32, rider: u32) -> Option<Vec3> {
        self.cart_dismount_point_from(self.cart_object(vehicle)?, self.mob_object(rider)?)
    }

    pub(super) fn cart_dismount_mob(
        &self,
        vehicle: u32,
        rider: &horizon_client::mobs::Mob,
    ) -> Option<Vec3> {
        self.cart_dismount_point_from(self.cart_object(vehicle)?, rider)
    }

    /// The live tick snapshot may have moved beyond the cart's owning slot.
    pub(super) fn cart_dismount_point_from(
        &self,
        cart: &super::minecarts::Minecart,
        rider: &horizon_client::mobs::Mob,
    ) -> Option<Vec3> {
        use horizon_core::geometry::Aabb;
        let direction =
            (f64::from(cart.yaw) / 90.0 + 0.5).floor() as i32 + if cart.flipped { 3 } else { 1 };
        let (fx, fz) = [(0, 1), (-1, 0), (0, -1), (1, 0)][(direction & 3) as usize];
        let (rx, rz) = (-fz, fx);
        let offsets = [
            (rx, rz),
            (-rx, -rz),
            (rx - fx, rz - fz),
            (-rx - fx, -rz - fz),
            (fx + rx, fz + rz),
            (fx - rx, fz - rz),
            (-fx, -fz),
            (fx, fz),
        ];
        let [bx, by, bz] =
            [cart.position.x, cart.position.y, cart.position.z].map(|v| v.floor() as i32);
        let half = f64::from(rider.width().min(1.0) / 2.0);
        for dy in [0, 1, -1] {
            for (dx, dz) in offsets {
                let p = [bx + dx, by + dy, bz + dz];
                let shape = self.block_collision_shape(p);
                let below = self.block_collision_shape([p[0], p[1] - 1, p[2]]);
                let floor = horizon_core::geometry::block_floor_height(&shape, || &below);
                if !floor.is_infinite() && floor < 1.0 {
                    let at = Vec3::new(
                        f64::from(p[0]) + 0.5,
                        f64::from(p[1]) + floor,
                        f64::from(p[2]) + 0.5,
                    );
                    let bounds = Aabb::new(
                        Vec3::new(at.x - half, at.y, at.z - half),
                        Vec3::new(at.x + half, at.y + f64::from(rider.height()), at.z + half),
                    );
                    if self.can_dismount_to(bounds) {
                        return Some(at);
                    }
                }
            }
        }
        Some(Vec3::new(
            cart.position.x,
            cart.position.y + super::minecarts::HEIGHT,
            cart.position.z,
        ))
    }

    /// `AbstractHorse.getDismountLocationForPassenger`: try either side
    /// through the mount's height, then fall back to its feet.
    pub(super) fn horse_dismount_point(&self, vehicle: u32, rider: u32) -> Option<Vec3> {
        self.horse_dismount_mobs(self.mob_object(vehicle)?, self.mob_object(rider)?)
    }

    /// The horse search for a mount and rider currently lent out of the mob list.
    pub(super) fn horse_dismount_mobs(
        &self,
        mount: &horizon_client::mobs::Mob,
        rider: &horizon_client::mobs::Mob,
    ) -> Option<Vec3> {
        use horizon_core::geometry::{Aabb, trig};
        if !is_horse(mount.species()) {
            return None;
        }
        let turn = if rider.main_arm_left() { -90.0 } else { 90.0 };
        let reach =
            (f64::from(mount.width()) + f64::from(rider.width()) + f64::from(1e-5_f32)) / 2.0;
        let limit = mount.bounds().max_position().y + 0.75;
        for turn in [turn, -turn] {
            let radians = f64::from((mount.yaw + turn) * 0.017_453_292);
            let (dx, dz) = (-trig::sin(radians), trig::cos(radians));
            let scale = f64::from(dx.abs().max(dz.abs()));
            let x = mount.position.x + f64::from(dx) * reach / scale;
            let z = mount.position.z + f64::from(dz) * reach / scale;
            let mut y = mount.position.y.floor() as i32;
            loop {
                let p = [x.floor() as i32, y, z.floor() as i32];
                let shape = self.block_collision_shape(p);
                let below = self.block_collision_shape([p[0], p[1] - 1, p[2]]);
                let floor = horizon_core::geometry::block_floor_height(&shape, || &below);
                if f64::from(y) + floor > limit {
                    break;
                }
                if !floor.is_infinite() && floor < 1.0 {
                    let at = Vec3::new(x, f64::from(y) + floor, z);
                    let half = f64::from(rider.width()) / 2.0;
                    let bounds = Aabb::new(
                        Vec3::new(x - half, at.y, z - half),
                        Vec3::new(x + half, at.y + f64::from(rider.height()), z + half),
                    );
                    if self.can_dismount_to(bounds) {
                        return Some(at);
                    }
                }
                y += 1;
                if f64::from(y) >= limit {
                    break;
                }
            }
        }
        Some(mount.position)
    }

    /// `Entity.teleportCrossDimension` retains its ordered passenger list before
    /// ejecting riders. Dismounting also seats their own passengers again.
    fn prepare_vehicle_transition(&mut self, root: u32) {
        let riders = self.mob_passenger_ids(root);
        if riders.is_empty() {
            return;
        }
        let slots = self.ordered_passenger_slots(root);
        for &rider in riders.iter().rev() {
            self.dismount_mob(rider);
        }
        for rider in riders {
            self.prepare_vehicle_transition(rider);
            if let Some(m) = self.mob_object_mut(rider) {
                // Preserve the tree for serialization after all transitions
                // have used the positions left by ejectPassengers.
                m.extra.mount = Some(root);
                m.extra.passenger = true;
            }
        }
        // Dismounting removes live slots. Restoring only mob mounts would append
        // those riders after every other kind when the transfer tree is captured.
        self.persist.passenger_slots.insert(root, slots);
    }

    /// `Entity.teleportPassengers` on a mob dismounted during a transition.
    pub(super) fn snap_mob_passengers(&mut self, root: u32) {
        use horizon_client::mobs::riders;
        let Some(vehicle) = self.mob_object(root) else {
            return;
        };
        let (position, yaw, width, vehicle_box) = (
            vehicle.position,
            vehicle.yaw,
            vehicle.width(),
            vehicle.bounds(),
        );
        let children = self.seated_passengers(root);
        for (seat, &(_, id, player)) in children.iter().enumerate() {
            if player {
                continue;
            }
            let Some(m) = self.mob_object(id) else {
                continue;
            };
            let attachment = Self::mob_passenger_attachment(
                self.mob_object(root).unwrap(),
                m,
                seat,
                children.len(),
            );
            let offset = riders::vehicle_attachment(m, width);
            let m = self.mob_object_mut(id).unwrap();
            m.seated(root, offset, vehicle_box);
            m.position = riders::seat(attachment, offset, position, yaw);
            m.previous = m.position;
        }
        for (_, child, player) in children {
            if !player {
                self.snap_mob_passengers(child);
            }
        }
    }

    /// `PositionMoveRotation.of(entity).deltaMovement()`: the motion a
    /// transition that keeps it (`TeleportMotion::Keep`) carries over, a
    /// minecart's clamped (`cart_known_movement`). None for a player, whose
    /// client holds its motion, and an entity no level holds.
    pub(super) fn transition_motion(&mut self, t: Target) -> Option<Vec3> {
        let id = super::native_entities::target_id(t)?;
        let motion = self
            .in_target_level(t, |g| {
                let h = g.native_walk_entity(id)?;
                Some(with_entity!(g, h, e => e.velocity()))
            })
            .flatten()?;
        Some(if matches!(t, Target::Cart(_)) {
            cart_known_movement(motion)
        } else {
            motion
        })
    }
    /// After a teleport placed `t` in its level: `motion` is its motion again
    /// (`Entity.teleportSetPosition` with `Relative.DELTA`).
    pub(super) fn restore_transition_motion(&mut self, t: Target, motion: Vec3) {
        let Some(id) = super::native_entities::target_id(t) else {
            return;
        };
        let _ = self.in_target_level(t, |g| {
            if let Some(h) = g.native_walk_entity(id) {
                with_entity_mut!(g, h, e => *e.velocity_mut() = motion);
            }
        });
    }

    /// The cross-level teleport of a command's or a mod's target: a general
    /// entity tree carries itself across (identity kept, join identity new)
    /// and always answers; an entity that changes level alone answers; a
    /// vehicle tree answers only when it moved. `None` leaves the target to
    /// the caller's player path.
    pub(in crate::game) fn teleport_target_across(
        &mut self,
        target: Target,
        destination: super::Dim,
        at: [f64; 3],
        rotation: Option<[f32; 2]>,
        turn_riders: bool,
        motion: TeleportMotion,
    ) -> horizon_client::Result<Option<bool>> {
        if let Target::General(id) = target {
            return Ok(Some(self.teleport_general_tree_across(
                id,
                destination,
                at,
            )?));
        }
        if let Some(moved) = self.teleport_lone_across(target, destination, at, rotation, motion)? {
            return Ok(Some(moved));
        }
        Ok(self
            .teleport_vehicle_across(target, destination, at, rotation, turn_riders, motion)?
            .then_some(true))
    }

    /// `Entity.teleportCrossDimension` for an entity that changes level alone
    /// (`lone_traveller_box`: an item, a primed TNT, a trident, a firework, an
    /// experience orb, an arrow or another saved projectile): saved, removed
    /// from its level, and recreated at `at` in `destination` under a new id,
    /// its stack, owner, thrower and portal cooldown kept (`restoreFrom`), its
    /// motion as `motion` says, turned to `rotation` when there is one. None
    /// for a target of another kind; Some(false) for one no level holds, one
    /// already in `destination`, one with no saved form, or one carrying mobs;
    /// Some(true) when it moved, or a mod canceled the travel
    /// (`entity_travel_to_dimension`) and it stayed.
    fn teleport_lone_across(
        &mut self,
        target: Target,
        destination: super::Dim,
        at: [f64; 3],
        rotation: Option<[f32; 2]>,
        motion: TeleportMotion,
    ) -> horizon_client::Result<Option<bool>> {
        let (Target::Item(id)
        | Target::Tnt(id)
        | Target::Trident(id)
        | Target::Rocket(id)
        | Target::Orb(id)
        | Target::Projectile(id)
        | Target::Stand(id)
        | Target::Frame(id)
        | Target::Painting(id)
        | Target::Falling(id)
        | Target::Cushion(id)
        | Target::Other(id)
        | Target::Fangs(id)
        | Target::Crystal(id)
        | Target::Dragon(id)
        | Target::Spawner(id)
        | Target::Cloud(id)) = target
        else {
            return Ok(None);
        };
        self.in_target_level(target, |game| {
            let source = game.dim;
            // Entity.teleportCrossDimension carries the riders; nothing rides
            // these kinds but a mob a server seats on a primed TNT.
            if source == destination
                || game.lone_traveller_box(id).is_none()
                || game.mobs.iter().any(|m| m.extra.mount == Some(id))
            {
                return Ok(Some(false));
            }
            if !game.entity_may_travel(id, destination, super::server_travel::TravelCause::Teleport)
            {
                return Ok(Some(true));
            }
            // Open the destination before removing the entity.
            game.enter_level(destination)?;
            game.enter_level(source)?;
            let cooldown = game.portals.entities.get(&id).map_or(0, |s| s.cooldown);
            let Some((mut tag, state)) = game.take_lone_traveller(id) else {
                return Ok(Some(false));
            };
            game.native_leaves_flush();
            let [yaw, pitch] = horizon_client::entity_nbt::rotation(&tag);
            // PositionMoveRotation.calculateAbsolute: an axis the teleport
            // does not set keeps the entity's own, the pitch is clamped.
            let [yaw, pitch] = rotation.map_or([yaw, pitch], |[y, p]| {
                [
                    if y.is_finite() { y } else { yaw },
                    absolute_pitch(p).unwrap_or(pitch),
                ]
            });
            let kept = match motion {
                TeleportMotion::Keep => horizon_client::entity_nbt::motion(&tag),
                TeleportMotion::Stop => Vec3::ZERO,
            };
            let to = Vec3::new(at[0], at[1], at[2]);
            super::nether_portal::set_arrival_pose(&mut tag, to, kept, [yaw, pitch], 0.0);
            if matches!(
                target,
                Target::Frame(_) | Target::Painting(_) | Target::Cushion(_)
            ) && let Tag::Compound(c) = &mut tag
            {
                c.insert(
                    "block_pos".into(),
                    Tag::IntArray(at.map(|v| v.floor() as i32).to_vec()),
                );
            }
            if let Tag::Compound(c) = &mut tag {
                c.insert("PortalCooldown".into(), Tag::Int(cooldown));
            }
            // ServerLevel.addDuringTeleport: a fresh add, not loaded from disk.
            super::persist::mark_fresh(&mut tag);
            game.enter_level(destination)?;
            game.spawn_teleported_entity(Self::chunk_of(to), tag.into(), Some(state));
            game.enter_level(source)?;
            Ok(Some(true))
        })
        .unwrap_or(Ok(Some(false)))
    }

    /// `Entity.teleportCrossDimension`: carry a command's entire vehicle tree
    /// into the destination level, retaining UUIDs and the passengers' mounts,
    /// the root's motion as `motion` says.
    pub(super) fn teleport_vehicle_across(
        &mut self,
        target: Target,
        destination: super::Dim,
        at: [f64; 3],
        rotation: Option<[f32; 2]>,
        turn_riders: bool,
        motion: TeleportMotion,
    ) -> horizon_client::Result<bool> {
        let Some(root) = vehicle_id(target) else {
            return Ok(false);
        };
        self.in_target_level(target, |game| {
            let source = game.dim;
            if source == destination {
                return Ok(false);
            }
            // Refused passengers are dismounted and remain in the source level.
            if !game.tree_may_travel(root, destination) {
                return Ok(true);
            }
            // Open the destination before changing or removing any entity.
            game.enter_level(destination)?;
            game.enter_level(source)?;
            if game
                .mobs
                .iter()
                .any(|m| m.id == root && m.extra.mount.is_some())
            {
                game.dismount_mob(root);
            }
            game.prepare_vehicle_transition(root);
            let kept = match motion {
                TeleportMotion::Keep => game.transition_motion(target),
                TeleportMotion::Stop => None,
            };
            let from = game.place_teleport_in_level(target, at, rotation, [false; 3], turn_riders);
            if let Some(from) = from
                && game.options.dedicated.is_some()
            {
                game.native_entity_teleport(game.dim.id(), root, from, at);
            }
            match kept {
                Some(kept) => game.restore_transition_motion(target, kept),
                None => game.teleport_stops(target),
            }
            let tree = game.passenger_tree(root);
            // NBT and runtime ownership come from the same ordered object walk,
            // including off-level mobs and riders of every other live kind.
            let (tag, state) = game
                .take_vehicle_tree_transfer(root)
                .ok_or("missing vehicle object tree")?;
            let mut players = Vec::new();
            // A command runs with its source player swapped out of the hub.
            if game.riding.is_some_and(|id| tree.contains(&id))
                && let (Some(conn), Some(player)) = (game.lan_acting, game.player.as_ref())
            {
                players.push((
                    conn,
                    [player.position.x, player.position.y, player.position.z],
                    [game.yaw, game.pitch],
                    game.riding.unwrap(),
                ));
            }
            if let Some(host) = game.lan.as_ref() {
                for guest in &host.hub.guests {
                    if Some(guest.conn) != game.lan_acting
                        && guest.vehicle.is_some_and(|id| tree.contains(&id))
                    {
                        players.push((
                            guest.conn,
                            guest.position,
                            [guest.yaw, guest.pitch],
                            guest.vehicle.unwrap(),
                        ));
                    }
                }
            }
            game.mobs.retain(|m| !tree.contains(&m.id));
            game.persist.off_level.retain(|id, _| !tree.contains(id));
            game.boats.retain(|b| !tree.contains(&b.id));
            game.carts.retain(|c| !tree.contains(&c.id));
            // Mobs, boats and carts above, the other riders and every id's
            // container, lead and identity here.
            game.take_passenger_entities(&tree, true);
            game.native_leaves_flush();
            game.enter_level(destination)?;
            // restoreFrom reloads saved state into fresh entities, rather than
            // retaining navigation paths or goals from the old level.
            let restored_objects = game
                .restore_vehicle_tree_state(tag, state)
                .ok_or("could not restore vehicle tree")?;
            // AbstractHorse.addPassenger snaps a reattached passenger to the
            // new horse's getViewYRot(0) / getViewXRot(0). A fresh living
            // entity's previous head yaw is still zero, before its first tick.
            let new_root = restored_objects.root;
            let mounts: Vec<_> = game.mob_objects().map(|m| (m.id, m.extra.mount)).collect();
            let restored = super::persist::vehicle_tree(new_root, &mounts);
            let horses: Vec<_> = game
                .mob_objects()
                .filter(|m| restored.contains(&m.id) && is_horse(m.species()))
                .map(|m| (m.id, m.head_pitch))
                .collect();
            for (horse, pitch) in horses {
                for id in game.mob_passenger_ids(horse) {
                    let Some(rider) = game.mob_object_mut(id) else {
                        continue;
                    };
                    rider.yaw = 0.0;
                    rider.previous_yaw = 0.0;
                    rider.head_pitch = pitch;
                    rider.previous_head_pitch = pitch;
                }
            }
            for (conn, at, rotation, mount) in players {
                game.level_teleports
                    .push(super::server_travel::LevelTeleport {
                        conn,
                        dim: destination,
                        at,
                        rotation: Some(rotation),
                        vehicle: restored_objects.by_source.get(&mount).copied(),
                    });
            }
            game.native_joins_flush(false, 0);
            game.enter_level(source)?;
            Ok(true)
        })
        .unwrap_or(Ok(false))
    }

    /// Whether the vehicle `root` and the server players riding it (or what it
    /// carries) may all travel to `destination` by a teleport
    /// (`entity_travel_to_dimension`).
    fn tree_may_travel(&mut self, root: u32, destination: super::Dim) -> bool {
        self.entity_may_travel(
            root,
            destination,
            super::server_travel::TravelCause::Teleport,
        )
    }

    /// `BlockAttachedEntity.setPos`: a frame or painting moves to the block
    /// holding the position; its bounds follow from it.
    pub(super) fn teleport_hanging(&mut self, t: Target, v: Vec3) {
        let pos = [v.x.floor() as i32, v.y.floor() as i32, v.z.floor() as i32];
        match t {
            Target::Frame(id) => {
                if let Some(f) = self.item_frames.iter_mut().find(|f| f.id == id) {
                    f.pos = pos;
                }
            }
            Target::Painting(id) => {
                if let Some(p) = self.paintings.iter_mut().find(|p| p.id == id) {
                    p.pos = pos;
                }
            }
            _ => {}
        }
    }

    /// `Entity.teleportSameDimension`: after `t` moved from `from` to `to`, its
    /// riders move by the same offset, and to the angles the vehicle was
    /// placed at when it turned (`calculatePassengerTransition`), staying
    /// seated and recursive. The player in place and the hub's guests (a guest
    /// also gets its own position packet, as `ServerPlayer.teleport` sends one
    /// for a passenger).
    pub(super) fn teleport_passengers(
        &mut self,
        t: Target,
        from: [f64; 3],
        to: [f64; 3],
        rotation: Option<PlacedRotation>,
    ) {
        let Some(id) = vehicle_id(t) else {
            return;
        };
        let (old, new) = (
            Vec3::new(from[0], from[1], from[2]),
            Vec3::new(to[0], to[1], to[2]),
        );
        self.move_riders(id, old, new, rotation);
    }
    fn move_riders(
        &mut self,
        vehicle: u32,
        old: Vec3,
        new: Vec3,
        rotation: Option<PlacedRotation>,
    ) {
        let mut riders = self.ordered_live_passengers(vehicle);
        for id in self.mob_passenger_ids(vehicle) {
            if !riders.contains(&id) {
                riders.push(id);
            }
        }
        let mut guest_steps = Vec::new();
        for id in riders {
            let mut carried = None;
            if self.mob_object(id).is_none() {
                // A rider that is not a mob: moved by its own kind's writer.
                if let Some(target) = self.level_entity(id as i32)
                    && let Some(at) = self.target_position(target)
                {
                    let at = passenger_destination(old, new, Vec3::new(at[0], at[1], at[2]));
                    let turn = rotation.map(|placed| {
                        passenger_rotation(
                            placed.before,
                            placed.after,
                            self.target_rotation(target),
                        )
                    });
                    if matches!(target, Target::Frame(_) | Target::Painting(_)) {
                        self.place_saved_passenger(target, at);
                        self.move_riders(id, old, new, rotation);
                    } else {
                        // This existing writer also carries this rider's descendants.
                        self.teleport_to_in_level(target, [at.x, at.y, at.z], turn, [false; 3], true);
                    }
                }
                continue;
            }
            if let Some(m) = self.mob_object_mut(id) {
                let from = m.position;
                m.position = passenger_destination(old, new, m.position);
                m.previous = m.position;
                carried = Some((from, m.position));
                // Entity.calculatePassengerTransition turns the rider with the
                // vehicle, then Entity.teleportSetPosition keeps the old angles
                // for the interpolation.
                if let Some(placed) = rotation {
                    let [yaw, pitch] =
                        passenger_rotation(placed.before, placed.after, [m.yaw, m.head_pitch]);
                    m.yaw = yaw;
                    m.body_yaw = yaw;
                    m.head_yaw = yaw;
                    m.head_pitch = pitch;
                    m.previous_yaw = yaw;
                    m.previous_head_pitch = pitch;
                }
            }
            // Each rider's `teleportSetPosition` is a position update of its own.
            if let Some((from, to)) = carried {
                self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
            }
            self.move_riders(id, old, new, rotation);
        }
        if self.riding == Some(vehicle)
            && let Some(p) = self.player.as_mut()
        {
            let from = p.position;
            p.position = passenger_destination(old, new, p.position);
            p.teleported = true;
            self.previous_position = p.position;
            let (to, own) = (p.position, self.own_entity_id());
            if let Ok(own) = u32::try_from(own) {
                self.entity_section_step(own, [from.x, from.y, from.z], [to.x, to.y, to.z]);
            }
        }
        if let Some(placed) = rotation
            && self.riding == Some(vehicle)
        {
            let [yaw, pitch] =
                passenger_rotation(placed.before, placed.after, [self.yaw, self.pitch]);
            if yaw.is_finite() {
                self.yaw = yaw;
            }
            if pitch.is_finite() {
                self.pitch = pitch;
            }
        }
        let [yaw, pitch] = self.vehicle_rotation(vehicle);
        if let Some(host) = self.lan.as_mut() {
            for i in 0..host.hub.guests.len() {
                if host.hub.guests[i].vehicle != Some(vehicle) {
                    continue;
                }
                // Entity.sendTeleportTransitionToRidingPlayers: the rider's
                // client moves the vehicle it drives itself, so it hears the
                // vehicle's teleport (else its next MoveVehicle undoes it).
                host.hub.guests[i].send(&C::TeleportEntity(play::TeleportEntity {
                    id: vehicle as i32,
                    position: [new.x, new.y, new.z],
                    delta: [0.0; 3],
                    yaw,
                    pitch,
                    relatives: 0,
                    on_ground: false,
                }));
                if let Some(placed) = rotation {
                    host.hub.guests.update(i, |g| {
                        let [yaw, pitch] =
                            passenger_rotation(placed.before, placed.after, [g.yaw, g.pitch]);
                        g.yaw = yaw;
                        g.head_yaw = yaw;
                        g.pitch = pitch;
                    });
                }
                let id = host.hub.guests[i].entity_id as u32;
                let g = host.hub.guests[i].position;
                let from = Vec3::new(g[0], g[1], g[2]);
                let at = passenger_destination(old, new, Vec3::new(g[0], g[1], g[2]));
                if rotation.is_some() {
                    // A rider turned with its vehicle: its position packet
                    // carries the new rotation and keeps its motion.
                    host.hub.teleport_rider(i, [at.x, at.y, at.z]);
                } else {
                    host.hub.teleport_keeping_motion(i, [at.x, at.y, at.z]);
                }
                guest_steps.push((id, from, at));
            }
        }
        for (id, from, to) in guest_steps {
            self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
        }
    }

    /// A vehicle's yaw and pitch, for the riders' teleport packet.
    fn vehicle_rotation(&self, id: u32) -> [f32; 2] {
        if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            [b.yaw, b.pitch]
        } else if let Some(c) = self.carts.iter().find(|c| c.id == id) {
            [c.yaw, c.pitch]
        } else if let Some(m) = self.mob_object(id) {
            [m.yaw, m.head_pitch]
        } else {
            [0.0, 0.0]
        }
    }

    /// `Entity.forceSetRotation`: yaw, head yaw, pitch and their old values
    /// only. Position, motion, the vehicle and the riders stay.
    pub(super) fn set_rotation(&mut self, t: Target, rotation: [f32; 2]) {
        let _ = self.in_target_level(t, |g| g.set_rotation_in_level(t, rotation));
    }
    fn set_rotation_in_level(&mut self, t: Target, [yaw, pitch]: [f32; 2]) {
        let pitch = absolute_pitch(pitch);
        match t {
            Target::Guest(_) => {
                self.for_player(t, |g| {
                    g.set_rotation_in_level(Target::Player, [yaw, pitch.unwrap_or(f32::NAN)])
                });
            }
            Target::Player => {
                if yaw.is_finite() {
                    self.yaw = yaw;
                }
                if let Some(pitch) = pitch {
                    self.pitch = pitch;
                }
                // A server's player hears the turn alone when its turn ends.
                self.rotation_forced = self.lan_acting.is_some();
            }
            Target::Mob(id) => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                    if yaw.is_finite() {
                        m.yaw = yaw;
                        m.body_yaw = yaw;
                        m.head_yaw = yaw;
                    }
                    if let Some(pitch) = pitch {
                        m.head_pitch = pitch;
                    }
                    m.previous_yaw = m.yaw;
                    m.previous_head_pitch = m.head_pitch;
                }
            }
            Target::Other(id) => {
                if let Some(i) = self.others.iter().position(|e| e.id == id) {
                    self.others.update(i, |o| {
                        if yaw.is_finite() {
                            o.yaw = yaw;
                        }
                        if let Some(pitch) = pitch {
                            o.pitch = pitch;
                        }
                    });
                }
            }
            Target::Stand(id) if yaw.is_finite() => {
                if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id) {
                    a.yaw = yaw;
                }
            }
            Target::Boat(id) if yaw.is_finite() => {
                if let Some(i) = self.boats.iter().position(|b| b.id == id) {
                    self.boats.update(i, |b| {
                        b.yaw = yaw;
                        if let Some(pitch) = pitch {
                            b.pitch = pitch;
                        }
                    });
                }
            }
            Target::Cart(id) if yaw.is_finite() => {
                if let Some(i) = self.carts.iter().position(|c| c.id == id) {
                    self.carts.update(i, |c| {
                        c.yaw = yaw;
                        if let Some(pitch) = pitch {
                            c.pitch = pitch;
                        }
                    });
                }
            }
            Target::Fangs(id) if yaw.is_finite() => {
                if let Some(i) = self.fangs.list.iter().position(|e| e.id == id) {
                    self.fangs.list.update(i, |f| {
                        f.yaw = yaw;
                    });
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passenger_keeps_its_offset() {
        let at = passenger_destination(
            Vec3::new(0.0, 64.0, 0.0),
            Vec3::new(100.0, 70.0, -20.0),
            Vec3::new(0.0, 64.5, 0.25),
        );
        assert_eq!((at.x, at.y, at.z), (100.0, 70.5, -19.75));
    }
    #[test]
    fn pitch_clamps_before_the_setter() {
        assert_eq!(absolute_pitch(360.0), Some(90.0));
        assert_eq!(absolute_pitch(-360.0), Some(-90.0));
        assert_eq!(absolute_pitch(f32::INFINITY), Some(90.0));
        assert_eq!(absolute_pitch(f32::NEG_INFINITY), Some(-90.0));
        assert_eq!(absolute_pitch(45.0), Some(45.0));
        assert_eq!(absolute_pitch(f32::NAN), None);
    }
    #[test]
    fn an_absolute_turn_keeps_the_riders_angle_to_the_vehicle() {
        // A rider 30 degrees off a vehicle that turns from 0 to 90 ends at
        // 120: it keeps the 30 degrees it had on it.
        assert_eq!(
            passenger_rotation([0.0, 0.0], [90.0, 0.0], [30.0, 0.0]),
            [120.0, 0.0]
        );
        // The pitch keeps its offset the same way, through Entity.setXRot's
        // clamp: 40 - 10 = 30 degrees off a vehicle placed at 70 would be 100.
        assert_eq!(
            passenger_rotation([10.0, 10.0], [0.0, 70.0], [0.0, 40.0]),
            [-10.0, 90.0]
        );
    }
    #[test]
    fn a_rider_turns_from_the_vehicles_unclamped_pitch() {
        // `~ ~100` on a level vehicle: the vehicle stops at 90, a rider 50
        // below it adds the whole 100 (calculatePassengerTransition) and ends at 50.
        assert_eq!(
            passenger_rotation([0.0, 0.0], [0.0, 100.0], [0.0, -50.0]),
            [0.0, 50.0]
        );
    }
    #[test]
    fn an_axis_the_command_did_not_set_leaves_the_riders_own() {
        assert!(passenger_rotation([0.0, 0.0], [f32::NAN, 0.0], [30.0, 0.0])[0] == 30.0);
        assert!(passenger_rotation([0.0, 0.0], [0.0, f32::NAN], [0.0, 40.0])[1] == 40.0);
    }
    #[test]
    fn cart_known_movement_zeros_nan_and_clamps_horizontal() {
        let clamped = cart_known_movement(Vec3::new(1.25, -0.75, -0.9));
        assert_eq!((clamped.x, clamped.y, clamped.z), (0.4, -0.75, -0.4));

        for nan_vec in [
            Vec3::new(f64::NAN, 0.1, 0.2),
            Vec3::new(0.1, f64::NAN, 0.2),
            Vec3::new(0.1, 0.2, f64::NAN),
        ] {
            let zeroed = cart_known_movement(nan_vec);
            assert_eq!((zeroed.x, zeroed.y, zeroed.z), (0.0, 0.0, 0.0));
        }
    }
}
