//! An entity's motion and shape for native server mods, through an
//! `EntityRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387):
//! velocity, ground, box, eyes and view. They read the entity a handle names
//! as `native_entity_refs.rs` finds it, and return `None` (or `false`) for a
//! stale handle. On a client (API @API:client_entity_refs@) they read the
//! entities the server shows the window, and nothing moves one.
use super::entity::{Entity as _, EntityHandle, with_entity, with_entity_mut};
use super::{Game, commands::Target};
use horizon_core::geometry::{Aabb, Axis, Vec3};
use horizon_core::position::BlockPos;
use mod_api::{ClipBlock, ClipFluid, ClipHit, EntityRef};

/// `Entity.calculateViewVector` for a look turned `pitch_offset` degrees down: the unit
/// vector along `yaw` and `pitch + pitch_offset`.
fn turned_view_vector(yaw: f32, pitch: f32, pitch_offset: f32) -> [f64; 3] {
    let v = horizon_world::view_direction::view_vector(pitch + pitch_offset, yaw);
    [v.x, v.y, v.z]
}

/// `Entity.getOnPos()`: the supporting block when there is one, else the block `1.0E-5F`
/// under the feet at `position`.
pub(super) fn on_block_position(support: Option<BlockPos>, position: Vec3) -> [i32; 3] {
    match support {
        Some(p) => [p.x, p.y, p.z],
        None => [
            position.x.floor() as i32,
            (position.y - f64::from(1.0e-5_f32)).floor() as i32,
            position.z.floor() as i32,
        ],
    }
}

/// The end of a ray `distance` blocks from `from` along the unit vector `look`
/// (`from.add(look.scale(distance))`).
fn ray_end(from: [f64; 3], look: [f64; 3], distance: f64) -> [f64; 3] {
    std::array::from_fn(|i| from[i] + look[i] * distance)
}

fn corners(b: Aabb) -> [[f64; 3]; 2] {
    let axes = [Axis::X, Axis::Y, Axis::Z];
    [axes.map(|a| b.min(a)), axes.map(|a| b.max(a))]
}

impl Game<'_> {
    /// The simulated entity (one of the shared entity walk's lists) with `id`.
    pub(super) fn native_walk_entity(&self, id: u32) -> Option<EntityHandle> {
        self.entity_handles()
            .find(|h| with_entity!(self, *h, e => e.id() == id))
    }
    /// This window's own player, when a client's target names it.
    fn native_client_player(&self, t: Target) -> Option<&horizon_client::motion::PlayerMotion> {
        self.remote.as_ref()?;
        (t == Target::Player).then_some(())?;
        self.player.as_ref()
    }
    /// The server's copy of the player a target names.
    pub(super) fn native_target_guest(
        &self,
        t: Target,
    ) -> Option<&horizon_client::lan::guest::Guest> {
        match t {
            Target::Player => self.command_guest(self.lan_acting?),
            Target::Guest(conn) => self.command_guest(conn),
            _ => None,
        }
    }
    /// The entity's velocity in blocks a tick (`Entity.getDeltaMovement`). A
    /// player's is the move its client last reported (`getKnownMovement`); an
    /// entity Horizon doesn't simulate (a display, a painting) has none, so
    /// zero. On a client, another entity's is the motion the server last
    /// sent it (`Entity.lerpMotion`). API 379.
    pub(super) fn entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let t = self.native_entity_read_target(entity)?;
        if !self.is_logical_server() {
            // The client's copy: its own player's motion, and the motion the
            // server sent for an entity it tracks.
            let v = match t {
                Target::Player => self.player.as_ref()?.velocity,
                _ => match self.native_walk_entity(entity.id()) {
                    Some(h) => with_entity!(self, h, e => e.velocity()),
                    None => Vec3::ZERO,
                },
            };
            return Some([v.x, v.y, v.z]);
        }
        if let Some(g) = self.native_target_guest(t) {
            return Some(std::array::from_fn(|i| {
                g.position[i] - g.previous_position[i]
            }));
        }
        let v = match self.native_walk_entity(entity.id()) {
            Some(h) => with_entity!(self, h, e => e.velocity()),
            None => Vec3::ZERO,
        };
        Some([v.x, v.y, v.z])
    }
    /// `Entity.getKnownSpeed`, in blocks a tick: a player's reported move; a
    /// vehicle with a live controlling player, that player's; else what the
    /// entity moved in its last tick (`computeSpeed`: position minus the
    /// previous tick's). Every list the walk keeps has a previous position.
    /// A frame, painting, crystal, spawner, cloud, fangs or marker never
    /// moves, so zero; the dragon uses its own previous position.
    pub(super) fn native_entity_known_speed(&self, entity: EntityRef) -> Option<Vec3> {
        let t = self.native_entity_target(entity)?;
        if let Some(g) = self.native_target_guest(t) {
            let [x, y, z] = g.known_movement;
            return Some(Vec3::new(x, y, z));
        }
        if matches!(t, Target::Player) {
            return Some(self.known_player_movement());
        }
        if let Some(rider) = self.native_controlling_player(t, entity) {
            return self.native_entity_known_speed(rider);
        }
        if let Target::Dragon(_) = t {
            let d = self.dragon.as_ref()?;
            return Some(d.position.subtract(d.previous));
        }
        Some(match self.native_walk_entity(entity.id()) {
            Some(h) => with_entity!(self, h, e => e.position().subtract(e.previous_position())),
            None => Vec3::ZERO,
        })
    }
    /// Whether `Entity.getControllingPassenger` can be a player: a boat's
    /// first passenger, or a mount's rider as `equine::controlled` decides
    /// (saddled; a pig or strider's rider holding the stick; a harnessed
    /// happy ghast), the rule the mount code steers by.
    fn native_steered(&self, t: Target) -> bool {
        match t {
            Target::Boat(_) => true,
            Target::Mob(id) => self
                .mobs
                .iter()
                .any(|m| m.id == id && horizon_client::mobs::equine::controlled(m)),
            _ => false,
        }
    }
    /// `Entity.getControllingPassenger` when it is a player.
    fn native_controlling_player(&self, t: Target, entity: EntityRef) -> Option<EntityRef> {
        let steered = self.native_steered(t);
        let first = *self.entity_passengers(entity).ok_or(()).ok()?.first()?;
        let player = matches!(
            self.native_entity_target(first)?,
            Target::Player | Target::Guest(_)
        );
        (steered && player).then_some(first)
    }
    /// Sets the entity's velocity (`Entity.setDeltaMovement`); clients see it
    /// as they see the entity move. False for a player (its client moves
    /// it), an entity Horizon doesn't simulate, a value that isn't finite,
    /// or on a client. API 379.
    pub(super) fn entity_set_velocity(&mut self, entity: EntityRef, velocity: [f64; 3]) -> bool {
        if self.remote.is_some()
            || !velocity.iter().all(|v| v.is_finite())
            || self.native_entity_target(entity).is_none()
        {
            return false;
        }
        let Some(h) = self.native_walk_entity(entity.id()) else {
            return false;
        };
        let [x, y, z] = velocity;
        with_entity_mut!(self, h, e => *e.velocity_mut() = Vec3::new(x, y, z));
        if let EntityHandle::Mob(i) = h {
            self.mobs[i].needs_sync = true;
        }
        true
    }
    /// `Entity.resetFallDistance`: how far the entity has fallen is forgotten, so its next
    /// landing hurts it as a step does (a fan's or a launcher's push, a mod's soft block).
    /// Players too, the acting one and guests. False for a stale handle or an entity not in
    /// the level in place; true for a kind that keeps no fall distance, which has none to forget.
    pub(super) fn entity_reset_fall_distance(&mut self, entity: EntityRef) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        if matches!(target, Target::Player | Target::Guest(_)) {
            let motion = match target {
                Target::Player => self.player.as_mut(),
                Target::Guest(conn) => self
                    .lan
                    .as_mut()
                    .and_then(|host| host.hub.guests.motion_mut(conn)),
                _ => None,
            };
            let Some(motion) = motion else {
                return false;
            };
            motion.fall_distance = 0.0;
            return true;
        }
        let Some(h) = self.native_walk_entity(entity.id()) else {
            return false;
        };
        with_entity_mut!(self, h, e => e.reset_fall_distance());
        true
    }
    /// Sets whether a server player or guest is swimming (`Entity.setSwimming`): the flag
    /// the next water update reads, as `Player` keeps it. False for a mob or another kind,
    /// and for a stale handle.
    pub(super) fn entity_set_swimming(&mut self, entity: EntityRef, swimming: bool) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        let motion = match target {
            Target::Player => self.player.as_mut(),
            Target::Guest(conn) => self
                .lan
                .as_mut()
                .and_then(|host| host.hub.guests.motion_mut(conn)),
            _ => None,
        };
        let Some(motion) = motion else {
            return false;
        };
        motion.swimming = swimming;
        true
    }

    /// Sets `Entity.fallDistance` of a server player, guest or mob, as Paper's
    /// `Entity#setFallDistance` does. False for another kind, a client, a stale handle, or a
    /// value that is not finite.
    pub(super) fn entity_set_fall_distance(&mut self, entity: EntityRef, distance: f64) -> bool {
        if !distance.is_finite() {
            return false;
        }
        match self.native_entity_target(entity) {
            Some(Target::Mob(id)) => self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                m.fall_distance = distance;
                true
            }),
            Some(target @ (Target::Player | Target::Guest(_))) => {
                let motion = match target {
                    Target::Player => self.player.as_mut(),
                    Target::Guest(conn) => self
                        .lan
                        .as_mut()
                        .and_then(|host| host.hub.guests.motion_mut(conn)),
                    _ => None,
                };
                motion.is_some_and(|motion| {
                    motion.fall_distance = distance;
                    true
                })
            }
            _ => false,
        }
    }
    /// Whether the entity stands on a block (`Entity.onGround`).
    /// Whether the entity stands on a block (`Entity.onGround`). API
    /// 379.
    pub(super) fn entity_on_ground(&self, entity: EntityRef) -> Option<bool> {
        let t = self.native_entity_target(entity)?;
        if let Some(p) = self.native_client_player(t) {
            return Some(p.ground.on_ground);
        }
        if let Some(g) = self.native_target_guest(t) {
            return Some(g.on_ground);
        }
        Some(
            self.native_walk_entity(entity.id())
                .is_some_and(|h| with_entity!(self, h, e => e.on_ground())),
        )
    }
    /// The entity's box (`Entity.getBoundingBox`): its lowest and highest
    /// corner. A display's corners both equal its position. None for the kinds
    /// whose box Horizon doesn't keep (markers, End crystals, the dragon); a
    /// dragon part's is its own. API 379.
    pub(super) fn entity_bounding_box(&self, entity: EntityRef) -> Option<[[f64; 3]; 2]> {
        if !self.is_logical_server()
            && let Some(bounds) = self.native_client_display_bounds(entity)
        {
            return Some(corners(bounds));
        }
        let t = self.native_entity_read_target(entity)?;
        if let Some(p) = self.native_client_player(t) {
            return Some(corners(p.bounds()));
        }
        if let Some(g) = self.native_target_guest(t) {
            return Some(corners(g.bounds()));
        }
        if let Some(h) = self.native_walk_entity(entity.id()) {
            return Some(corners(with_entity!(self, h, e => e.bounds())));
        }
        let (lo, hi) = match t {
            Target::Other(id) => {
                let entity = self.others.iter().find(|entity| entity.id == id)?;
                return entity.display_bounds().map(corners);
            }
            Target::DragonPart(id) => {
                let (dragon, part) = self.resident_dragon_part(id)?;
                return Some(corners(dragon.part_geometry(part).1));
            }
            Target::Frame(id) => self.item_frames.iter().find(|f| f.id == id)?.bounds(),
            Target::Painting(id) => self.paintings.iter().find(|p| p.id == id)?.bounds(),
            _ => return None,
        };
        Some([lo, hi])
    }
    /// The height of the entity's eyes above its feet (`Entity.getEyeHeight`).
    /// API 379.
    pub(super) fn entity_eye_height(&self, entity: EntityRef) -> Option<f64> {
        Some(self.target_eye_height(self.native_entity_read_target(entity)?))
    }
    /// Where the entity's eyes are (`Entity.getEyePosition`). API
    /// 379.
    pub(super) fn entity_eye_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let [x, y, z] = self.entity_position(entity)?;
        Some([x, y + self.entity_eye_height(entity)?, z])
    }
    /// The unit vector the entity looks along (`Entity.getViewVector`, from
    /// its pitch and yaw). API 379.
    pub(super) fn entity_view_vector(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let [yaw, pitch] = self.entity_rotation(entity)?;
        Some(turned_view_vector(yaw, pitch, 0.0))
    }
    /// The unit vector that points up out of the entity's head
    /// (`Entity.getUpVector`): its view vector turned a quarter of a turn
    /// toward the zenith.
    pub(super) fn entity_up_vector(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let [yaw, pitch] = self.entity_rotation(entity)?;
        Some(turned_view_vector(yaw, pitch, -90.0))
    }
    /// `Entity.mainSupportingBlockPos` of an entity on the ground: the player's own, kept
    /// as it moved, or the one `Entity.checkSupportingBlock` finds under the box of any
    /// other that stands.
    fn native_supporting_block(&self, entity: EntityRef, t: Target) -> Option<BlockPos> {
        if t == Target::Player
            && let Some(player) = &self.player
        {
            return player.ground.supporting_block;
        }
        if !self.entity_on_ground(entity)? {
            return None;
        }
        let (position, bounds) = match self.native_target_guest(t) {
            Some(g) => (
                Vec3::new(g.position[0], g.position[1], g.position[2]),
                g.bounds(),
            ),
            None => {
                with_entity!(self, self.native_walk_entity(entity.id())?, e => (e.position(), e.bounds()))
            }
        };
        self.supporting_block(position, bounds)
    }
    /// The block the entity stands on (`Entity.getOnPos`). API 379.
    pub(super) fn entity_on_block(&self, entity: EntityRef) -> Option<[i32; 3]> {
        let t = self.native_entity_target(entity)?;
        let [x, y, z] = self.entity_position(entity)?;
        Some(on_block_position(
            self.native_supporting_block(entity, t),
            Vec3::new(x, y, z),
        ))
    }
    /// What the entity looks at within `distance` blocks of its eyes: `Entity.pick`'s block
    /// ray along the view vector, outlines and no fluids.
    pub(super) fn entity_pick(&self, entity: EntityRef, distance: f64) -> Option<ClipHit> {
        if !(distance.is_finite() && distance >= 0.0) {
            return None;
        }
        let from = self.entity_eye_position(entity)?;
        let to = ray_end(from, self.entity_view_vector(entity)?, distance);
        self.level_clip(
            self.entity_level(entity)?,
            from,
            to,
            ClipBlock::Outline,
            ClipFluid::None,
        )
    }
    /// The square of the distance from the entity's feet to `pos`
    /// (`Entity.distanceToSqr`). API 379.
    pub(super) fn entity_distance_to_sqr(&self, entity: EntityRef, pos: [f64; 3]) -> Option<f64> {
        let p = self.entity_position(entity)?;
        let [dx, dy, dz] = std::array::from_fn(|i| p[i] - pos[i]);
        Some(dx * dx + dy * dy + dz * dz)
    }
}

#[cfg(test)]
mod tests {
    use horizon_core::geometry::{Aabb, Vec3};
    use mod_api::EntityRef;

    fn assert_near(got: [f64; 3], want: [f64; 3]) {
        for i in 0..3 {
            assert!((got[i] - want[i]).abs() < 1.0e-4, "{got:?} is not {want:?}");
        }
    }

    /// Unit vectors worked out by hand from `Entity.calculateViewVector`
    /// (`x = -sin(yaw) cos(pitch)`, `y = -sin(pitch)`, `z = cos(yaw) cos(pitch)`),
    /// the look turned 90 degrees up for the up vector.
    #[test]
    fn the_up_vector_tilts_with_the_look() {
        let half = std::f64::consts::FRAC_1_SQRT_2;
        // Level and facing south: straight up.
        assert_near(super::turned_view_vector(0.0, 0.0, -90.0), [0.0, 1.0, 0.0]);
        // Looking down 45 degrees, the top of the head leans forward (south).
        assert_near(
            super::turned_view_vector(0.0, 45.0, -90.0),
            [0.0, half, half],
        );
        // The same look facing west (yaw 90): forward is -x.
        assert_near(
            super::turned_view_vector(90.0, 45.0, -90.0),
            [-half, half, 0.0],
        );
        // Looking straight up, the head's top points back (north when facing south).
        assert_near(
            super::turned_view_vector(0.0, -90.0, -90.0),
            [0.0, 0.0, -1.0],
        );
        // No turn is the view vector itself: south, and down 30 degrees.
        assert_near(super::turned_view_vector(0.0, 0.0, 0.0), [0.0, 0.0, 1.0]);
        assert_near(
            super::turned_view_vector(0.0, 30.0, 0.0),
            [0.0, -0.5, 3.0_f64.sqrt() / 2.0],
        );
    }

    #[test]
    fn on_block_is_the_support_else_the_cell_just_under_the_feet() {
        use horizon_core::position::BlockPos;
        let feet = Vec3::new(0.9, 65.0, -0.2);
        // A support wins even where it is not under the feet's own column.
        assert_eq!(
            super::on_block_position(Some(BlockPos::new(1, 64, -1)), feet),
            [1, 64, -1]
        );
        // None: the cell under the feet; a hair under, so the block of a flush stand.
        assert_eq!(super::on_block_position(None, feet), [0, 64, -1]);
        assert_eq!(
            super::on_block_position(None, Vec3::new(0.5, 65.5, 0.5)),
            [0, 65, 0]
        );
    }

    #[test]
    fn a_pick_ray_ends_a_distance_along_the_look() {
        assert_eq!(
            super::ray_end([1.0, 2.0, 3.0], [0.0, -1.0, 0.0], 4.5),
            [1.0, -2.5, 3.0]
        );
        assert_eq!(
            super::ray_end([1.0, 2.0, 3.0], [0.6, 0.0, 0.8], 0.0),
            [1.0, 2.0, 3.0]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn on_block_picks_the_support_nearest_the_entity_and_pick_clips_blocks() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::{chunk::Chunk, mobs::Mob};
        use mod_api::BlockFace;
        use std::sync::Arc;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            // A zombie 0.6 wide straddling x = 1: its box is over blocks 0 and 1.
            let mut zombie = Mob::new(40, kind, Vec3::new(0.9, 65.0, 0.5), 0.0);
            zombie.on_ground = true;
            game.mobs.push(zombie);
            let entity = game.entity(40).unwrap();
            // Only block 1 is under the box: it supports, not the block of the feet's own column.
            game.world.set(1, 64, 0, stone).unwrap();
            assert_eq!(game.entity_on_block(entity), Some([1, 64, 0]));
            // With both, the one whose centre is nearest the feet: 0.4 against 0.6 away.
            game.world.set(0, 64, 0, stone).unwrap();
            assert_eq!(game.entity_on_block(entity), Some([0, 64, 0]));
            game.mobs[0].position = Vec3::new(1.1, 65.0, 0.5);
            assert_eq!(game.entity_on_block(entity), Some([1, 64, 0]));
            // Off the ground there is no support: the cell under the feet.
            game.mobs[0].position = Vec3::new(0.9, 65.0, 0.5);
            game.mobs[0].on_ground = false;
            assert_eq!(game.entity_on_block(entity), Some([0, 64, 0]));
            let air = game
                .pack
                .assets
                .registry
                .default_state("minecraft:air")
                .unwrap();
            game.world.set(0, 64, 0, air).unwrap();
            game.mobs[0].on_ground = true;
            assert_eq!(game.entity_on_block(entity), Some([1, 64, 0]));

            // Facing south (yaw 0) level, the eyes see a block two cells ahead.
            game.mobs[0].position = Vec3::new(0.5, 65.0, 0.5);
            let eyes = game.entity_eye_position(entity).unwrap();
            let wall = [0, eyes[1].floor() as i32, 2];
            game.world.set(wall[0], wall[1], wall[2], stone).unwrap();
            let hit = game.entity_pick(entity, 3.0).unwrap();
            assert!(!hit.miss);
            assert_eq!(hit.block, wall);
            assert_eq!(hit.face, BlockFace::North);
            assert_near(hit.location, [0.5, eyes[1], 2.0]);
            // Too short a ray misses, as does one over an empty column.
            assert!(game.entity_pick(entity, 1.0).unwrap().miss);
            game.mobs[0].position = Vec3::new(5.5, 65.0, 0.5);
            assert!(game.entity_pick(entity, 3.0).unwrap().miss);
            // Looking straight down from the stand hits the top of its support.
            game.mobs[0].position = Vec3::new(1.5, 65.0, 0.5);
            game.mobs[0].head_pitch = 90.0;
            let down = game.entity_pick(entity, 3.0).unwrap();
            assert_eq!((down.block, down.face), ([1, 64, 0], BlockFace::Up));
            assert_near(down.location, [1.5, 65.0, 0.5]);
            // Looking down 45 degrees tilts the up vector forward.
            game.mobs[0].head_pitch = 45.0;
            assert_near(
                game.entity_up_vector(entity).unwrap(),
                [
                    0.0,
                    std::f64::consts::FRAC_1_SQRT_2,
                    std::f64::consts::FRAC_1_SQRT_2,
                ],
            );
            // A ray without a length, or with none, or from a stale handle: nothing.
            assert!(game.entity_pick(entity, -1.0).is_none());
            assert!(game.entity_pick(entity, f64::NAN).is_none());
            let stale = EntityRef::new(mod_api::WorldEpoch::new(entity.epoch().get() + 1), 40);
            assert!(game.entity_pick(stale, 3.0).is_none());
            assert!(game.entity_on_block(stale).is_none());
            assert!(game.entity_up_vector(stale).is_none());
        });
    }

    #[test]
    fn corners_are_lowest_then_highest() {
        let b = Aabb::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        assert_eq!(super::corners(b), [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
    }

    #[test]
    #[ignore = "daily"]
    fn reset_fall_distance_forgets_a_fall_for_mobs_carts_and_players() {
        use super::super::minecarts::{CartKind, Minecart};
        use horizon_client::mobs::Mob;
        fn fake_fall<'a>(game: &'a mut super::super::Game<'_>) -> &'a mut f64 {
            let host = game.lan.as_mut().unwrap();
            let conn = host.hub.guests.iter().find(|g| g.uuid == 2).unwrap().conn;
            &mut host
                .hub
                .guests
                .motion_mut(conn)
                .expect("a fake has a motion")
                .fall_distance
        }
        super::super::native_entity_refs::tests::with_game(|game| {
            super::super::bossbars::audience_tests::add_real_and_fake(game);
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut zombie = Mob::new(40, kind, Vec3::ZERO, 0.0);
            zombie.fall_distance = 12.0;
            game.mobs.push(zombie);
            let mut cart = Minecart::new(41, CartKind::Rideable, Vec3::ZERO);
            cart.fall_distance = 7.0;
            game.carts.push(cart);
            let (zombie, cart) = (game.entity(40).unwrap(), game.entity(41).unwrap());
            assert!(game.entity_reset_fall_distance(zombie));
            assert!(game.entity_reset_fall_distance(cart));
            assert_eq!(game.mobs[0].fall_distance, 0.0);
            assert_eq!(game.carts[0].fall_distance, 0.0);
            // A player: the fake guest's own motion.
            let level = game.level(game.dim.id()).unwrap();
            let fake = game.fake_player(level, 2, "fake").unwrap().entity();
            *fake_fall(game) = 9.0;
            assert!(game.entity_reset_fall_distance(fake));
            assert_eq!(*fake_fall(game), 0.0);
            // An entity that is gone: nothing.
            game.mobs.clear();
            assert!(!game.entity_reset_fall_distance(zombie));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn entity_set_swimming_writes_a_players_flag_and_refuses_a_mob() {
        fn fake_swimming<'a>(game: &'a mut super::super::Game<'_>) -> &'a mut bool {
            let host = game.lan.as_mut().unwrap();
            let conn = host.hub.guests.iter().find(|g| g.uuid == 2).unwrap().conn;
            &mut host
                .hub
                .guests
                .motion_mut(conn)
                .expect("a fake has a motion")
                .swimming
        }
        super::super::native_entity_refs::tests::with_game(|game| {
            super::super::bossbars::audience_tests::add_real_and_fake(game);
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            use horizon_client::mobs::Mob;
            game.mobs.push(Mob::new(40, kind, Vec3::ZERO, 0.0));
            // A mob refuses: its swim is its own AI's.
            assert!(!game.entity_set_swimming(game.entity(40).unwrap(), true));
            // A fake guest takes the flag, then gives it up.
            let level = game.level(game.dim.id()).unwrap();
            let fake = game.fake_player(level, 2, "fake").unwrap().entity();
            assert!(game.entity_set_swimming(fake, true));
            assert!(*fake_swimming(game));
            assert!(game.entity_set_swimming(fake, false));
            assert!(!*fake_swimming(game));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn entity_set_fall_distance_writes_a_mob_or_guest_and_refuses_the_rest() {
        fn fake_fall<'a>(game: &'a mut super::super::Game<'_>) -> &'a mut f64 {
            let host = game.lan.as_mut().unwrap();
            let conn = host.hub.guests.iter().find(|g| g.uuid == 2).unwrap().conn;
            &mut host
                .hub
                .guests
                .motion_mut(conn)
                .expect("a fake has a motion")
                .fall_distance
        }
        super::super::native_entity_refs::tests::with_game(|game| {
            super::super::bossbars::audience_tests::add_real_and_fake(game);
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            use horizon_client::mobs::Mob;
            game.mobs.push(Mob::new(40, kind, Vec3::ZERO, 0.0));
            let zombie = game.entity(40).unwrap();
            assert!(game.entity_set_fall_distance(zombie, 3.5));
            assert_eq!(game.mobs[0].fall_distance, 3.5);
            // A value that is not finite is refused and changes nothing.
            assert!(!game.entity_set_fall_distance(zombie, f64::NAN));
            assert_eq!(game.mobs[0].fall_distance, 3.5);
            // A fake guest's own motion takes the value.
            let level = game.level(game.dim.id()).unwrap();
            let fake = game.fake_player(level, 2, "fake").unwrap().entity();
            assert!(game.entity_set_fall_distance(fake, 9.0));
            assert_eq!(*fake_fall(game), 9.0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn only_a_mount_its_rider_controls_takes_the_riders_speed() {
        use super::super::commands::Target;
        use horizon_client::mobs::{Mob, equine::Rider};
        super::super::native_entity_refs::tests::with_game(|game| {
            for (id, name) in [(30, "minecraft:horse"), (31, "minecraft:pig")] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                let mut m = Mob::new(id, kind, Vec3::ZERO, 0.0);
                m.rider = Some(Rider::default());
                game.mobs.push(m);
            }
            // A horse being broken in, unsaddled: its own speed (AbstractHorse.java:962).
            assert!(!game.native_steered(Target::Mob(30)));
            game.mobs[0].extra.set_saddled(true);
            assert!(game.native_steered(Target::Mob(30)));
            // A saddled pig whose rider holds no stick (Pig.java:97-100).
            game.mobs[1].extra.set_saddled(true);
            assert!(!game.native_steered(Target::Mob(31)));
            game.mobs[1].rider.as_mut().unwrap().steer = true;
            assert!(game.native_steered(Target::Mob(31)));
        });
    }
}
