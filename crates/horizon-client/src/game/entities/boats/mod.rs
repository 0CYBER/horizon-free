//! Boats and rafts (`AbstractBoat`): water/land/air status, floating,
//! paddling controls, collisions, riding (`positionRider`, `clampRotation`,
//! dismounting), `BoatItem` placement, `VehicleEntity.hurtServer` breaking,
//! saving and `AbstractBoatRenderer` with animated paddles.
mod below_world;
#[cfg(test)]
mod friction_tests;
#[cfg(test)]
mod lifetime_tests;
mod placement;
pub(super) use placement::OutlineFluidRay;

pub(super) use horizon_client_gameplay::boat_models::BoatModels;
pub(super) use horizon_gameplay::boats::*;

use super::{Game, InteractTarget, entity::Entity, entity_culling::at, view_vector};
use horizon_client::actions::Action;
use horizon_client::entity_rules::EntityRules;
use horizon_client::{
    blocks::Fluid,
    collisions::{EntityCollisions, SceneCollisions},
    items::ItemStack,
    mesher::Vertex,
    mobs::anim::Poses,
    motion::{BubbleColumn, RiddenBoat, bubble_column_at},
};
use horizon_core::{
    geometry::{Aabb, ShapeOp, Vec3, VoxelShape, trig},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::collision_context::CollisionContext;
use std::{f32::consts::PI, sync::Arc};

/// The push the paddles give along the boat's yaw (`tick_boat`'s
/// controlBoat): the sine and cosine of the yaw in float radians from `Mth`'s
/// sine table, times the acceleration as floats, widened to double.
fn paddle_push(yaw: f32, acceleration: f32) -> (f64, f64) {
    (
        f64::from(trig::sin(f64::from(-yaw * trig::DEG_TO_RAD)) * acceleration),
        f64::from(trig::cos(f64::from(yaw * trig::DEG_TO_RAD)) * acceleration),
    )
}
/// What `PowderSnowBlock.entityInside` passes to `makeStuckInBlock`.
fn powder_snow_stuck() -> Vec3 {
    Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32))
}
/// The cells a box reaches into (deflated by 1e-5, as `Entity.checkInsideBlocks` does),
/// in x, y, z order.
fn reached_cells(bounds: Aabb, is: impl Fn([i32; 3]) -> bool) -> Vec<[i32; 3]> {
    let e = f64::from(1.0e-5_f32);
    let bounds = bounds.deflate(Vec3::new(e, e, e));
    let (lo, hi) = (bounds.min_position(), bounds.max_position());
    let mut cells = Vec::new();
    for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
        for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
            for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                if is([x, y, z]) {
                    cells.push([x, y, z]);
                }
            }
        }
    }
    cells
}
/// The snowflake `PowderSnowBlock.entityInside` sends for an entity whose known movement is
/// `known`: none unless it moved horizontally and a coin falls so, then `Mth.randomBetween(-1, 1)`
/// over 12 sideways and 0.05 up.
fn snowflake_velocity(known: Vec3, random: &mut impl RandomSource) -> Option<[f64; 3]> {
    if (known.x == 0.0 && known.z == 0.0) || !random.next_boolean() {
        return None;
    }
    let xa = (random.next_float() * 2.0 - 1.0) * 0.083_333_336_f32;
    let za = (random.next_float() * 2.0 - 1.0) * 0.083_333_336_f32;
    Some([f64::from(xa), f64::from(0.05_f32), f64::from(za)])
}
impl Game<'_> {
    fn fluid_height(&self, x: i32, y: i32, z: i32) -> Option<(f32, bool)> {
        self.fluid_height_of(x, y, z, Some(Fluid::Water))
    }
    /// The horizontal water plane around the camera for the water mirror: the median
    /// standing-water surface of a grid of columns 48 blocks around, each scanned down from
    /// the camera to its ground. A lake or the sea has one plane; the mirror shows only the
    /// water on it. With no water in reach the last plane found stays, so the mirror never
    /// switches off as the camera moves.
    pub(super) fn water_plane(&self, camera: glam::DVec3) -> Option<f32> {
        const RADIUS: i32 = 48;
        const STEP: usize = 4;
        const DEPTH: i32 = 96;
        let (cx, cz) = (camera.x.floor() as i32, camera.z.floor() as i32);
        let top = camera.y.floor() as i32 + 2;
        // Scanned again once the camera moves 8 blocks, or every 5 seconds: a scan reads up
        // to 60 000 blocks on the game thread, and the plane rarely changes.
        let scan = (
            [cx >> 3, top >> 3, cz >> 3],
            self.world.game_time.div_euclid(100),
        );
        if self.water_plane_scan.replace(Some(scan)) == Some(scan) {
            return self.water_plane_memo.get();
        }
        let mut heights = Vec::with_capacity(625);
        for x in (cx - RADIUS..=cx + RADIUS).step_by(STEP) {
            for z in (cz - RADIUS..=cz + RADIUS).step_by(STEP) {
                // The top of the column's water: the highest standing water under the
                // camera (a full block's water continues above it).
                for y in (top - DEPTH..=top).rev() {
                    if let Some((h, source)) = self.fluid_height(x, y, z) {
                        if source && h < 1.0 {
                            heights.push(y as f32 + h);
                        }
                        break;
                    }
                    if self.res.facts().get(self.world.block(x, y, z)).solid_render {
                        break;
                    }
                }
            }
        }
        if heights.is_empty() {
            return self.water_plane_memo.get();
        }
        heights.sort_by(f32::total_cmp);
        let found = heights[heights.len() / 2];
        // The plane in use stays while a quarter of the columns are still on it, so two
        // lakes at different heights do not make the mirror flicker between them.
        let plane = match self.water_plane_memo.get() {
            Some(last)
                if 4 * heights.iter().filter(|h| (*h - last).abs() < 0.05).count()
                    >= heights.len() =>
            {
                last
            }
            _ => found,
        };
        self.water_plane_memo.set(Some(plane));
        Some(plane)
    }
    /// `FluidState.getHeight` and whether it is a source, for `fluid` or,
    /// with `None`, any fluid (`ClipContext.Fluid.ANY`).
    pub(super) fn fluid_height_of(
        &self,
        x: i32,
        y: i32,
        z: i32,
        fluid: Option<Fluid>,
    ) -> Option<(f32, bool)> {
        let f = self.res.facts().get(self.world.block(x, y, z));
        if f.fluid == Fluid::None || fluid.is_some_and(|want| f.fluid != want) {
            return None;
        }
        let above = self
            .res
            .facts()
            .get(self.world.block(x, y.wrapping_add(1), z));
        // FluidState.getHeight: 1 under the same fluid, else amount / 9.
        let height = if above.fluid == f.fluid {
            1.0
        } else {
            f32::from(f.fluid_level) / 9.0
        };
        Some((height, f.fluid_level == 8 && !f.fluid_falling))
    }
    /// `getWaterLevelAbove`: the top of the water column the hull is in.
    /// A float, as vanilla's: the block's y plus its water height in float.
    fn water_level_above(&self, b: &Boat) -> f32 {
        let bb = b.bounds();
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let (y0, y1) = (hi.y.floor() as i32, hi.y.ceil() as i32);
        'rows: for y in y0..y1 {
            let mut height = 0.0f32;
            for x in lo.x.floor() as i32..hi.x.ceil() as i32 {
                for z in lo.z.floor() as i32..hi.z.ceil() as i32 {
                    if let Some((h, _)) = self.fluid_height(x, y, z) {
                        height = height.max(h);
                    }
                    if height >= 1.0 {
                        continue 'rows;
                    }
                }
            }
            if height < 1.0 {
                return y as f32 + height;
            }
        }
        (i64::from(y1) + 1) as f32
    }
    fn boat_status(&self, b: &mut Boat) -> Status {
        let bb = b.bounds();
        let (lo, hi) = (bb.min_position(), bb.max_position());
        // isUnderwater
        let max_y = hi.y + 0.001;
        let mut under = false;
        for x in lo.x.floor() as i32..hi.x.ceil() as i32 {
            for y in hi.y.floor() as i32..max_y.ceil() as i32 {
                for z in lo.z.floor() as i32..hi.z.ceil() as i32 {
                    if let Some((h, source)) = self.fluid_height(x, y, z)
                        && max_y < f64::from(y as f32 + h)
                    {
                        if !source {
                            b.water_level = hi.y;
                            return Status::UnderFlowingWater;
                        }
                        under = true;
                    }
                }
            }
        }
        if under {
            b.water_level = hi.y;
            return Status::UnderWater;
        }
        // checkInWater
        let mut in_water = false;
        b.water_level = -f64::MAX;
        for x in lo.x.floor() as i32..hi.x.ceil() as i32 {
            for y in lo.y.floor() as i32..(lo.y + 0.001).ceil() as i32 {
                for z in lo.z.floor() as i32..hi.z.ceil() as i32 {
                    if let Some((h, _)) = self.fluid_height(x, y, z) {
                        // `float height = y + getHeight`: summed in float.
                        let level = f64::from(y as f32 + h);
                        b.water_level = b.water_level.max(level);
                        in_water |= lo.y < level;
                    }
                }
            }
        }
        if in_water {
            return Status::InWater;
        }

        let friction = self.boat_ground_friction(b, bb);
        if friction > 0.0 {
            b.land_friction = friction;
            Status::OnLand
        } else {
            Status::InAir
        }
    }
    /// `AbstractBoat.getGroundFriction`: average friction of the blocks whose
    /// collision shape meets a thin slab just under the hull (lily pads skipped).
    fn boat_ground_friction(&self, b: &Boat, bb: Aabb) -> f32 {
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let ground = Aabb::new(
            Vec3::new(lo.x, lo.y - 0.001, lo.z),
            Vec3::new(hi.x, lo.y, hi.z),
        );
        let (g_lo, g_hi) = (ground.min_position(), ground.max_position());
        let (x0, x1, y0, y1, z0, z1) = (
            (g_lo.x.floor() - 1.0) as i32,
            (g_hi.x.ceil() + 1.0) as i32,
            (g_lo.y.floor() - 1.0) as i32,
            (g_hi.y.ceil() + 1.0) as i32,
            (g_lo.z.floor() - 1.0) as i32,
            (g_hi.z.ceil() + 1.0) as i32,
        );
        let slab = VoxelShape::from_box(ground).unwrap_or_else(|_| VoxelShape::empty());
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let (mut friction, mut count) = (0.0f32, 0);
        for x in x0..x1 {
            for z in z0..z1 {
                let edges = i32::from(x == x0 || x == x1 - 1) + i32::from(z == z0 || z == z1 - 1);
                if edges == 2 {
                    continue;
                }
                for y in y0..y1 {
                    if edges == 1 && (y == y0 || y == y1 - 1) {
                        continue;
                    }
                    let state = self.world.block(x, y, z);
                    let f = self.res.facts().get(state);
                    if f.name.as_ref() == "minecraft:air" || f.has_class("LilyPadBlock") {
                        continue;
                    }
                    let Ok(shape) = self.collision.collision_shape_with_context(
                        &mut scene,
                        state,
                        BlockPos::new(x, y, z),
                        &CollisionContext::empty(false),
                    ) else {
                        continue;
                    };
                    let world_shape =
                        shape.moved(Vec3::new(f64::from(x), f64::from(y), f64::from(z)));
                    if world_shape
                        .join_is_not_empty(&slab, ShapeOp::AND)
                        .unwrap_or(false)
                    {
                        friction +=
                            self.boat_friction(b, BlockPos::new(x, y, z), state, f.friction);
                        count += 1;
                    }
                }
            }
        }
        if count > 0 {
            friction / count as f32
        } else {
            0.0
        }
    }
    /// `Entity.move`'s fall reset for an entity that keeps a fall and moves on
    /// by `moved` from `from`: a move of a block or more casts up to 8 blocks
    /// of it (`ClipContext.Block.FALLDAMAGE_RESETTING`, `ClipContext.Fluid.WATER`)
    /// and a hit resets the fall, before the move's own drop counts.
    pub(super) fn vehicle_fall_reset(&self, fall: f64, from: Vec3, moved: Vec3) -> f64 {
        let length = moved.length_squared();
        if fall == 0.0 || length < 1.0 {
            return fall;
        }
        let reset = &self.res.motion().fall_reset;
        let fluids = &self.res.motion().fluids;
        let to = from.add_vector(moved.normalize().scale(length.sqrt().min(8.0)));
        let hit = horizon_core::geometry::traverse_blocks(from, to, 1_000_000, |p| {
            // A tagged block clips as a full block, not its collider.
            let name = &self.res.facts().get(self.world.block(p.x, p.y, p.z)).name;
            if reset.contains(&**name) && VoxelShape::block().clip(from, to, p).is_some() {
                return Some(());
            }
            // The water's shape, through the shared fluid shape cache.
            horizon_client::entities::water_clip(
                &self.loop_context,
                &self.world,
                fluids,
                from,
                to,
                p,
            )
            .then_some(())
        });
        if matches!(hit, Ok(Some(()))) {
            0.0
        } else {
            fall
        }
    }
    /// `AbstractBoat.checkFallDamage` after a move that went `moved_y` (with
    /// `b` already where it landed): on the ground the fall resets; a move
    /// down over anything but water (the block under its `blockPosition`)
    /// adds to it.
    pub(super) fn boat_fall(&self, b: &Boat, moved_y: f64) -> f64 {
        if b.on_ground {
            return 0.0;
        }
        if moved_y < 0.0 {
            let below = self.res.facts().get(self.world.block(
                b.position.x.floor() as i32,
                (b.position.y.floor() as i32).wrapping_sub(1),
                b.position.z.floor() as i32,
            ));
            if below.fluid != Fluid::Water {
                return b.fall_distance - f64::from(moved_y as f32);
            }
        }
        b.fall_distance
    }
    /// `AbstractBoat.tick` for one boat (`controlled` when the player rides it).
    pub(super) fn tick_boat(&mut self, mut b: Boat, controlled: bool) -> Boat {
        b.previous = b.position;
        b.previous_yaw = b.yaw;
        // Entity.baseTick ends with Leashable.tickLeash, on a server only
        // (a ridden boat's lead ticks in `tick_boats`).
        if !controlled && self.remote.is_none() {
            self.tick_boat_leash(&mut b);
        }
        let had_status = std::mem::replace(&mut b.status_known, true);
        b.old_status = b.status;
        b.status = self.boat_status(&mut b);
        b.tick_submersion();
        // `tick_boats_now` wears it off for the boats it ticks, before they burn.
        if controlled {
            b.wear_off_hurt();
        }
        // A remote client gets the controller from the passenger packet, even when
        // this window occupies the second seat. Local worlds derive it here.
        if self.remote.is_none() {
            b.paddle_controller = controlled;
        }
        if !controlled {
            b.paddling = [false; 2];
            b.input = [false; 4];
        }
        // `Entity.baseTick`: the boat is immersed and pushed by the current it
        // sits in (`isPushedByFluid`), before `floatBoat` scales the velocity.
        // updateFluidInteraction: in water the fall resets.
        if b.push_by_fluid(&self.world, &self.res.motion().fluids, &self.loop_context)
            .0
            > 0.0
        {
            b.fall_distance = 0.0;
        }
        // floatBoat
        // `-getGravity()`: none with NoGravity.
        let mut vspeed = 0.0 - b.get_gravity();
        let mut buoyancy = 0.0;
        let inv_friction: f32;
        let obstacles = self.boat_move_obstacles(&b);
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if had_status
            && b.old_status == Status::InAir
            && b.status != Status::InAir
            && b.status != Status::OnLand
        {
            // getY(1.0): the top of its box.
            b.water_level = b.position.y + HEIGHT;
            // getWaterLevelAbove() - getBbHeight(): a float subtraction.
            let target = f64::from(self.water_level_above(&b) - HEIGHT as f32) + 0.101;
            let moved = Aabb::new(
                Vec3::new(
                    b.bounds().min_position().x,
                    target,
                    b.bounds().min_position().z,
                ),
                Vec3::new(
                    b.bounds().max_position().x,
                    target + HEIGHT,
                    b.bounds().max_position().z,
                ),
            );
            if scene.unobstructed(moved).unwrap_or(false) {
                b.position.y = target;
                b.velocity.y = 0.0;
            }
            b.status = Status::InWater;
        } else {
            match b.status {
                Status::InWater => {
                    buoyancy = (b.water_level - b.position.y) / HEIGHT;
                    inv_friction = 0.9;
                }
                Status::UnderFlowingWater => {
                    vspeed = -7.0e-4;
                    inv_friction = 0.9;
                }
                Status::UnderWater => {
                    buoyancy = f64::from(0.01_f32);
                    inv_friction = 0.45;
                }
                Status::InAir => inv_friction = 0.9,
                Status::OnLand => {
                    inv_friction = b.land_friction;
                    if controlled {
                        b.land_friction /= 2.0;
                    }
                }
            }
            let f = f64::from(inv_friction);
            b.velocity = Vec3::new(b.velocity.x * f, b.velocity.y + vspeed, b.velocity.z * f);
            b.delta_rotation *= inv_friction;
            if buoyancy > 0.0 {
                b.velocity.y = (b.velocity.y + buoyancy * (0.04 / 0.65)) * 0.75;
            }
        }
        // controlBoat
        if controlled {
            let [left, right, up, down] = b.input;
            let mut acceleration = 0.0f32;
            if left {
                b.delta_rotation -= 1.0;
            }
            if right {
                b.delta_rotation += 1.0;
            }
            if right != left && !up && !down {
                acceleration += 0.005;
            }
            b.yaw += b.delta_rotation;
            if up {
                acceleration += 0.04;
            }
            if down {
                acceleration -= 0.005;
            }
            let (x, z) = paddle_push(b.yaw, acceleration);
            b.velocity.x += x;
            b.velocity.z += z;
            b.paddling = [right && !left || up, left && !right || up];
        }
        // Entity.move: collisions zero the blocked components, the world border's
        // among them (`CollisionGetter.borderCollision`). The boat's own move
        // is a self move (arismoko/horizon#2414: vanilla's `AbstractBoat.tick` passes SELF);
        // PR 1a records the movement unchanged.
        let mut scene = EntityCollisions {
            blocks: scene,
            entities: &obstacles,
            border: Some((self.border.geometry(), [b.position.x, b.position.z])),
        };
        let movement = b.take_movement();
        let mut gate = horizon_client::move_gate::armed_with(
            &self.loop_context,
            b.id,
            || std::borrow::Cow::Owned(b.entity_type()),
            horizon_world::entity_move::MoverType::SelfMovement,
            movement,
        );
        let movement = gate.delta();
        if let Ok(moved) = horizon_client::move_gate::entities_resolve(
            &mut gate,
            &mut scene,
            b.bounds(),
            movement,
            0.0,
            b.on_ground,
        ) {
            b.fall_distance = self.vehicle_fall_reset(b.fall_distance, b.position, moved);
            b.position = Vec3::new(
                b.position.x + moved.x,
                b.position.y + moved.y,
                b.position.z + moved.z,
            );
            b.on_ground = movement.y < 0.0 && moved.y > movement.y;
            b.fall_distance = self.boat_fall(&b, moved.y);
            if (moved.x - movement.x).abs() > 1.0e-7 {
                b.velocity.x = 0.0;
            }
            if (moved.z - movement.z).abs() > 1.0e-7 {
                b.velocity.z = 0.0;
            }
            if (moved.y - movement.y).abs() > 1.0e-7 {
                b.velocity.y = 0.0;
            }
        }
        // AbstractBoat.tick's applyEffectsFromBlocks: powder snow or a cobweb makes the
        // boat stuck, which slows its next move, and resets its fall.
        self.boat_stuck_in_blocks(&mut b);
        // Entity.move's last step: soul sand and honey slow the boat too.
        let factor = crate::motion::block_speed_factor_at(
            &self.world,
            &self.motion.blocks,
            &self.loop_context,
            b.position,
            None,
        );
        b.velocity = horizon_world::entity_move::scale_by_block_speed(b.velocity, factor);
        Self::tick_boat_paddles(&mut b, &mut self.audio);
        b
    }
    /// `AbstractBoat.tick` advances the oars even when this client does not move the hull.
    pub(super) fn tick_boat_paddles(b: &mut Boat, audio: &mut super::Audio) {
        // Paddle sounds at the top of each stroke.
        for i in 0..2 {
            if b.paddle_state(i) {
                let p = b.paddles[i] % (2.0 * PI);
                if p <= PI / 4.0 && (b.paddles[i] + PI / 8.0) % (2.0 * PI) >= PI / 4.0 {
                    let sound = match b.status {
                        Status::InWater | Status::UnderWater | Status::UnderFlowingWater => {
                            Some("minecraft:entity.boat.paddle_water")
                        }
                        Status::OnLand => Some("minecraft:entity.boat.paddle_land"),
                        Status::InAir => None,
                    };
                    if let Some(sound) = sound {
                        let v = view_vector(b.yaw, 0.0);
                        let (dx, dz) = if i == 1 { (-v[2], v[0]) } else { (v[2], -v[0]) };
                        let pitch = 0.8 + 0.4 * b.random.next_float();
                        audio.play(
                            sound,
                            Some([b.position.x + dx, b.position.y, b.position.z + dz]),
                            1.0,
                            pitch,
                        );
                    }
                }
                b.paddles[i] += PI / 8.0;
            } else {
                b.paddles[i] = 0.0;
            }
        }
    }
    /// Refuse a removed tick target before carrying its slot into the clone.
    pub(super) fn store_boat_back(&mut self, index: usize, boat: Boat) -> bool {
        if self
            .persist
            .removals
            .contains(&super::commands::Target::Boat(boat.id))
        {
            return false;
        }
        self.boats.store_back(index, boat)
    }
    /// Every boat but the ridden one (which ticks with its rider).
    pub(super) fn tick_boats(&mut self) {
        self.vehicles_ticking(true);
        self.tick_boats_now();
        self.vehicles_ticking(false);
    }
    fn tick_boats_now(&mut self) {
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut round = self.native_tick_round();
        // A server player's boat is driven by its client (lan_vehicles.rs).
        let driven = self.guest_vehicles();
        // EntityTickEvent.Post comes once AbstractBoat.tick is over, after the
        // passes of `boat_server_passes` (lava, ejection, bubbles, contacts),
        // and before the next boat ticks, as `ServerLevel.tickNonPassenger`.
        // A pass (lava breaks a boat) or a mod's hook may remove boats: the next one is found

        // after this one by its id, so none is skipped or ticked twice.
        let mut i = 0;
        while i < self.boats.len() {
            let id = self.boats[i].id;

            'boat: {
                let p = self.boats[i].position;
                if Some(id) == self.riding
                    || driven.contains(&id)
                    || !ticking(p.x.floor() as i32, p.z.floor() as i32)
                {
                    // Its rider's client moves it, but the server still wears
                    // off its damage and then burns it.
                    if ticking(p.x.floor() as i32, p.z.floor() as i32) {
                        if driven.contains(&id) {
                            self.boats.update(i, |b| b.wear_off_hurt());
                        }
                        if self.boat_fire_tick(i) || self.boat_below_world(i) {
                            break 'boat;
                        }
                    }
                    // The lead still ticks too.
                    if ticking(p.x.floor() as i32, p.z.floor() as i32) && self.remote.is_none() {
                        self.with_entity_held_out(id, |game| {
                            let mut b = game.boats[i].clone();
                            game.tick_boat_leash(&mut b);
                            game.store_boat_back(i, b);
                        });
                    }
                    self.boat_server_passes(id, &driven, &ticking, &round);
                    break 'boat;
                }
                let kind = if round.listening() {
                    self.boats[i].entity_type()
                } else {
                    String::new()
                };
                if !self.native_tick_pre_one(&mut round, id, &kind) {
                    self.boat_server_passes(id, &driven, &ticking, &round);
                    break 'boat;
                }
                // The hook may have removed boats.
                let Some(i) = Some(i)
                    .filter(|&i| self.boats.get(i).is_some_and(|b| b.id == id))
                    .or_else(|| self.boats.iter().position(|b| b.id == id))
                else {
                    break 'boat;
                };
                self.boats.update(i, |b| b.wear_off_hurt());
                // Entity.baseTick burns it before it moves; the fire may break it.
                if !self.boat_fire_tick(i) && !self.boat_below_world(i) {
                    let section_step = self.with_entity_held_out(id, |game| {
                        let b = game.boats[i].clone();
                        let from = b.position;
                        let b = game.tick_boat(b, false);
                        let to = b.position;
                        game.store_boat_back(i, b).then_some((from, to))
                    });
                    if let Some((from, to)) = section_step {
                        self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
                    }
                    self.boat_server_passes(id, &driven, &ticking, &round);
                }
                self.native_tick_post_one(&round, id, &kind);
            }
            i = match self.boats.get(i) {
                Some(b) if b.id == id => i + 1,
                _ => self
                    .boats
                    .iter()
                    .position(|b| b.id == id)
                    .map_or(i, |k| k + 1),
            };
        }
    }
    /// The rest of `AbstractBoat.tick` on the server, for every boat, the
    /// client-driven ones too: `applyEffectsFromBlocks` (lava breaks a boat
    /// at once), the passengers of one under water too long get out, and it
    /// picks up or pushes what it touches.
    fn boat_server_passes(
        &mut self,
        id: u32,
        driven: &[u32],
        ticking: &impl Fn(i32, i32) -> bool,
        round: &super::native_ticks::TickRound,
    ) {
        let Some(i) = self.boats.iter().position(|b| b.id == id) else {
            return;
        };
        let p = self.boats[i].position;
        if !ticking(p.x.floor() as i32, p.z.floor() as i32) || round.canceled(id) {
            return;
        }
        if Some(id) != self.riding && self.boat_lava_tick(i) {
            return;
        }
        if self.remote.is_none() {
            if self.boat_fire_passes(i) {
                return;
            }
            self.boat_powder_snow_snowflakes(i);
        }
        // `AbstractBoat.tick` counts submersion on the server even when
        // its controlling client owns the boat's movement. Its
        // `applyEffectsFromBlocks` still runs here: a cobweb or powder snow
        // resets the fall of the boat the guest drives.
        if driven.contains(&id) {
            let mut b = self.boats[i].clone();
            self.boat_stuck_in_blocks(&mut b);
            self.boats.update(i, |boat| {
                boat.stuck = b.stuck;
                boat.fall_distance = b.fall_distance;
            });
            self.refresh_boat_status(i);
        }
        if self.boats[i].out_of_control >= 60.0 {
            self.eject_boat_riders(i);
        }
        self.boat_bubble_tick(i);
        self.boat_contacts(i);
    }
    /// `Entity.canSimulateMovement`: for a boat (`AUTHORITATIVE_SIDE`) a
    /// server leaves the boat its player drives to that player's client, and a
    /// client moves only the boat its player drives.
    fn boat_can_simulate_movement(&self, id: u32) -> bool {
        if self.remote.is_some() {
            self.controls_boat(id)
        } else {
            !self.guest_vehicles().contains(&id)
        }
    }
    /// `AbstractBoat.tick`: `status = getStatus()` for a boat this window
    /// does not tick (`tick_boat` sets it for the others): a client's boats
    /// other than the one it drives, and a server's boats its players drive.
    pub(super) fn refresh_boat_status(&mut self, i: usize) {
        let b = &self.boats[i];
        let mut probe = Boat::new(b.id, b.wood.clone(), b.position, b.yaw);
        let status = self.boat_status(&mut probe);
        self.boats.update(i, |b| b.status = status);
        // `oldStatus = status` runs on every boat, so the next tick has one.
        self.boats.update(i, |b| b.status_known = true);
        self.boats.update(i, Boat::tick_submersion);
    }
    /// `BubbleColumnBlock.entityInside`: the bubble columns boat `i` touches
    /// (its box deflated by 1e-5 as `Entity.checkInsideBlocks` does), in x, y,
    /// z order.
    pub(super) fn boat_bubble_columns(&self, i: usize) -> Vec<BubbleColumn> {
        let b = self.boats[i].bounds().deflate(Vec3::new(
            f64::from(1.0e-5f32),
            f64::from(1.0e-5f32),
            f64::from(1.0e-5f32),
        ));
        let (lo, hi) = (b.min_position(), b.max_position());
        let mut columns = Vec::new();
        // Entity.checkInsideBlocks floors both ends of the deflated box.
        // Narrowing ceil(max) before subtracting one wraps at i32::MIN,
        // turning a tiny box into a scan of the entire integer Y domain.
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    if let Some(column) = bubble_column_at(
                        &self.world,
                        self.pack.assets.registry,
                        self.res.facts(),
                        self.collision,
                        BlockPos::new(x, y, z),
                    ) {
                        columns.push(column);
                    }
                }
            }
        }
        columns
    }
    /// `Entity.handleOnInsideBubbleColumn`: a column under more water lifts
    /// boat `i` up to 0.7 or drags it down to -0.3 (`AbstractBoat` does not
    /// override `onInsideBubbleColumn`), applied by the column's own push below
    /// the surface.
    pub(super) fn boat_inside_bubble_column(&mut self, i: usize, column: BubbleColumn) {
        if !self.boat_can_simulate_movement(self.boats[i].id) {
            return;
        }
        self.boats.update(i, |b| column.push(&mut b.velocity));
    }
    /// `AbstractBoat.onAboveBubbleColumn`: whether the splash comes up, one
    /// time in a hundred for a boat that is not under water.
    pub(super) fn boat_splash_roll(&mut self, i: usize) -> bool {
        !self.boats[i].under_water() && self.world.random.next_int_bound(100).unwrap_or(1) == 0
    }
    /// `AbstractBoat.onAboveBubbleColumn`: the client's half after the roll,
    /// the splash sound and particle at the boat.
    pub(super) fn boat_bubble_splash(&mut self, i: usize) {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let p = self.boats[i].position;
        let pitch = 0.8 + 0.4 * self.world.random.next_float();
        self.audio.play(
            "minecraft:entity.generic.splash",
            Some([p.x, p.y, p.z]),
            1.0,
            pitch,
        );
        let x = p.x + f64::from(self.world.random.next_float());
        let z = p.z + f64::from(self.world.random.next_float());
        self.particles.add(
            &ParticleOptions::simple(ParticleType::Splash),
            [x, p.y + 0.7, z],
            [0.0; 3],
        );
    }
    /// `AbstractBoat.onAboveBubbleColumn`: on the server (this window
    /// simulates the boat) the column is remembered, the countdown starts and
    /// the splash's game event goes to the listeners (its sound and particle
    /// are the client's).
    fn boat_above_bubble_column(&mut self, i: usize, drag_down: bool) {
        self.boats.update(i, |b| {
            b.above_bubble_column = true;
            b.bubble_down = drag_down;
            if b.bubble_time == 0 {
                b.bubble_time = BUBBLE_TIME;
            }
        });
        if !self.boat_splash_roll(i) {
            return;
        }
        // A window that is its own client hears and sees the splash too.
        if self.options.execution_role.is_client() {
            self.boat_bubble_splash(i);
        }
        let (id, p) = (self.boats[i].id, self.boats[i].position);
        // Entity.gameEvent(SPLASH, getControllingPassenger()): the window's
        // own player when it drives the boat.
        let ctx = if self.riding == Some(id) && self.player.is_some() {
            self.player_context(None)
        } else {
            self.who_context(None, None)
        };
        self.post_game_event("minecraft:splash", [p.x, p.y, p.z], ctx);
    }
    /// `AbstractBoat.handleBubbleColumnEffect`: an up column throws the boat
    /// up at 0.6, a down column pulls it down by 0.7.
    pub(super) fn handle_bubble_column_effect(&mut self, i: usize, drag_down: bool) {
        if !self.boat_can_simulate_movement(self.boats[i].id) {
            return;
        }
        self.boats.update(i, |b| {
            if drag_down {
                // movement.add(0.0, -0.7, 0.0)
                b.velocity.y += -0.7;
            } else {
                b.velocity.y = 0.6;
            }
        });
    }
    /// `Entity.ejectPassengers`: the mobs get out here, this window's player
    /// gets out, and a server's player is asked to
    /// (`guest_vehicle_tick` lets it out through its client).
    fn eject_boat_riders(&mut self, i: usize) {
        self.eject_boat_mobs(i, false);
        let id = self.boats[i].id;
        if self.riding == Some(id) && self.player.is_some() {
            self.dismount(i);
        } else if self.guest_rides(id) {
            self.boats.update(i, |b| b.eject_riders = true);
        }
    }
    /// `AbstractBoat.tickBubbleColumn`: on the server, a boat that stays over
    /// a column for `BUBBLE_TIME` ticks is thrown up or pulled down (a down
    /// column lets the riders out first), and every client is told.
    fn tick_boat_bubble_column(&mut self, i: usize) {
        if let Some(down) = self.boats.update(i, Boat::tick_bubble_countdown) {
            let (id, p) = (self.boats[i].id, self.boats[i].position);
            let event = if down {
                BUBBLE_DOWN_EVENT
            } else {
                BUBBLE_UP_EVENT
            };
            if down {
                self.eject_boat_riders(i);
            }
            self.lan_entity_event(id as i32, event, [p.x, p.y, p.z]);
            self.handle_bubble_column_effect(i, down);
        }
    }
    /// `AbstractBoat.tick`: the end of it on the server for boat `i`, the two
    /// `applyEffectsFromBlocks` that reach bubble columns (the lava is
    /// `boat_lava_tick`'s), then `tickBubbleColumn`; and the wobble when this
    /// window is its own client.
    fn boat_bubble_tick(&mut self, i: usize) {
        let id = self.boats[i].id;
        // An ejection nobody took (its player left the boat first) is void.
        if !self.guest_rides(id) {
            self.boats.update(i, |b| b.eject_riders = false);
        }
        let touches = self.boat_bubble_columns(i);
        // AbstractBoat.tick calls applyEffectsFromBlocks twice.
        for _ in 0..2 {
            for &column in &touches {
                if column.surface {
                    self.boat_above_bubble_column(i, column.drag);
                } else {
                    self.boat_inside_bubble_column(i, column);
                }
            }
        }
        self.tick_boat_bubble_column(i);
        if self.options.execution_role.is_client() {
            self.boats.update(i, Boat::tick_bubble_wobble);
        }
    }
    /// `PowderSnowBlock.entityInside` for a boat: the powder snow cells its box, and the box it
    /// had before its move, reach into (`Entity.checkInsideBlocks` along the move).
    fn boat_powder_snow_cells(&self, b: &Boat) -> Vec<[i32; 3]> {
        self.boat_reached_cells(b, |p| self.is_powder_snow(p))
    }
    /// The cells a boat's box, and the box it had before its move, reach into
    /// (`Entity.checkInsideBlocks` along the move), in x, y, z order.
    fn boat_reached_cells(&self, b: &Boat, is: impl Fn([i32; 3]) -> bool) -> Vec<[i32; 3]> {
        let mut cells = reached_cells(b.bounds(), &is);
        if b.previous != b.position {
            for p in reached_cells(Boat::bounds_at(b.previous), &is) {
                if !cells.contains(&p) {
                    cells.push(p);
                }
            }
        }
        cells
    }
    /// `Entity.makeStuckInBlock` from the powder snow (`PowderSnowBlock.entityInside`) and
    /// cobweb (`WebBlock.entityInside`) cells a boat's box reaches into: each resets its fall
    /// and the last one reached leaves the multiplier its next move spends.
    fn boat_stuck_in_blocks(&self, b: &mut Boat) {
        // `forEachBlockIntersectedBetween` visits the box before the move first and the box
        // after it last, so a block the boat moves into wins over one it leaves.
        let mut cells = Vec::new();
        if b.previous != b.position {
            cells.extend(reached_cells(Boat::bounds_at(b.previous), |_| true));
        }
        for cell in reached_cells(b.bounds(), |_| true) {
            if !cells.contains(&cell) {
                cells.push(cell);
            }
        }
        for cell in cells {
            let name = &*self
                .res
                .facts()
                .get(self.world.block(cell[0], cell[1], cell[2]))
                .name;
            b.stuck = match name {
                "minecraft:powder_snow" => powder_snow_stuck(),
                "minecraft:cobweb" => horizon_client::falling_block::web_multiplier(),
                _ => continue,
            };
            b.fall_distance = 0.0;
        }
    }
    /// `BaseFireBlock.fireDamage` of a fire block at `p`.
    fn fire_damage_at(&self, p: [i32; 3]) -> Option<f32> {
        match &*self
            .res
            .facts()
            .get(self.world.block(p[0], p[1], p[2]))
            .name
        {
            "minecraft:fire" => Some(1.0),
            "minecraft:soul_fire" => Some(2.0),
            _ => None,
        }
    }
    /// `Entity.baseTick` on the server: a burning boat takes 1 fire damage
    /// every 20 ticks (none in lava) and burns a tick down. True when the fire
    /// broke it.
    fn boat_fire_tick(&mut self, i: usize) -> bool {
        if self.remote.is_some() || self.boats[i].fire_ticks <= 0 {
            return false;
        }
        let burn = self.boats[i].fire_ticks % 20 == 0 && !self.in_lava(self.boats[i].bounds());
        self.boats.update(i, |b| b.fire_ticks -= 1);
        burn && self.burn_boat(i, 1.0, "minecraft:on_fire")
    }
    /// The fire of the two `applyEffectsFromBlocks` in `AbstractBoat.tick`.
    /// `Entity.checkInsideBlocks` meets the blocks of the box the boat started
    /// its move in first, then the ones it reached, each block once; in each
    /// step fire blocks light it (`BaseFireBlock.fireIgnite`, a cooldown tick
    /// at a time) and each burns it, then a burning boat melts the powder snow
    /// (with `mob_griefing`) and water and powder snow put it out. Rain puts
    /// it out too, and a boat neither burning nor lit has its cooldown
    /// cleared. True when the fire broke it.
    fn boat_fire_passes(&mut self, i: usize) -> bool {
        let b = &self.boats[i];
        let mut steps = vec![Boat::bounds_at(b.previous)];
        if b.previous != b.position {
            steps.push(b.bounds());
        }
        // `Entity.collidedWithFluid` sweeps the box along the whole move:
        // water reaches it down to the lower of its two bottoms.
        let low = b.previous.y.min(b.position.y);
        let (at, top) = (b.position, b.bounds().max_position().y);
        let rain = self.in_rain(at, top);
        for _ in 0..2 {
            if self.boat_cactus_pass(i, &steps) {
                return true;
            }
            let before = self.boats[i].fire_ticks;
            let mut in_snow = false;
            let mut seen = Vec::new();
            for &bounds in &steps {
                let cells = reached_cells(bounds, |p| !seen.contains(&p));
                seen.extend_from_slice(&cells);
                let fires: Vec<f32> = cells
                    .iter()
                    .filter_map(|&p| self.fire_damage_at(p))
                    .collect();
                if !fires.is_empty() {
                    self.boats.update(i, |b| {
                        if b.fire_ticks < 0 {
                            b.fire_ticks += 1;
                        }
                        if b.fire_ticks >= 0 {
                            // igniteForSeconds(8)
                            b.fire_ticks = b.fire_ticks.max(160);
                        }
                    });
                    for &damage in &fires {
                        if self.burn_boat(i, damage, "minecraft:in_fire") {
                            return true;
                        }
                    }
                }
                let water = cells.iter().any(|&p| self.water_reaches(p, low));
                let snow: Vec<[i32; 3]> = cells
                    .iter()
                    .copied()
                    .filter(|&p| self.is_powder_snow(p))
                    .collect();
                in_snow |= !snow.is_empty();
                if self.boats[i].fire_ticks > 0
                    && self
                        .ui
                        .rules
                        .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING)
                {
                    for &cell in &snow {
                        self.powder_snow_melt(BlockPos::new(cell[0], cell[1], cell[2]));
                    }
                }
                if water || !snow.is_empty() {
                    // clearFire
                    self.boats.update(i, |b| b.fire_ticks = b.fire_ticks.min(0));
                }
            }
            let now = self.boats.update(i, |b| {
                if in_snow {
                    b.ticks_frozen = (b.ticks_frozen + 1)
                        .min(horizon_client::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
                }
                if rain {
                    b.fire_ticks = b.fire_ticks.min(0);
                }
                let now = b.fire_ticks;
                if now <= 0 && now <= before {
                    b.fire_ticks = 0;
                }
                now
            });
            if before > 0 && now <= 0 {
                self.extinguish_sound([at.x, at.y, at.z]);
            }
        }
        false
    }
    /// Whether the water at cell `p` reaches a box whose bottom is `low`
    /// (`Entity.collidedWithFluid`).
    fn water_reaches(&self, p: [i32; 3], low: f64) -> bool {
        self.fluid_height_of(p[0], p[1], p[2], Some(Fluid::Water))
            .is_some_and(|(height, _)| low + 1.0e-5 < f64::from(p[1]) + f64::from(height))
    }
    /// `VehicleEntity.hurtServer` for fire (`inFire`, `onFire`) or a cactus,
    /// which an `Invulnerable` boat ignores. True when the boat broke.
    pub(super) fn burn_boat(&mut self, i: usize, damage: f32, damage_type: &str) -> bool {
        let id = self.boats[i].id;
        let invulnerable = self.saved_refuses_damage(id, damage_type, false);
        let kind = self.boats[i].entity_type();
        let source = horizon_client::health::HurtSource {
            damage_type,
            causing: 0,
            direct: 0,
            position: None,
        };
        !self.native_nonliving_invulnerable(id, &kind, source, invulnerable)
            && self.hurt_boat(i, damage, false)
    }
    /// `PowderSnowBlock.entityInside`'s `ServerLevel.sendParticles`: a boat that moved
    /// horizontally in powder snow throws snowflakes the players around see. `AbstractBoat.tick`
    /// calls `applyEffectsFromBlocks` twice.
    fn boat_powder_snow_snowflakes(&mut self, i: usize) {
        let b = &self.boats[i];
        let cells = self.boat_powder_snow_cells(b);
        if cells.is_empty() {
            return;
        }
        let known = b.position.subtract(b.previous);
        let (x, z) = (b.position.x, b.position.z);
        for _ in 0..2 {
            for cell in &cells {
                if let Some(velocity) = snowflake_velocity(known, &mut self.world.random) {
                    self.lan_snowflake([x, f64::from(cell[1] + 1), z], velocity);
                }
            }
        }
    }
    /// `LavaFluid.entityInside` (`lavaHurt`) from the two
    /// `applyEffectsFromBlocks` calls in `AbstractBoat.tick`: twice 40
    /// damage, so lava breaks a boat on its first tick inside, and the fire
    /// `lavaIgnite` lights never gets to burn it. True when the boat broke.
    fn boat_lava_tick(&mut self, i: usize) -> bool {
        if !self.in_lava(self.boats[i].bounds()) {
            return false;
        }
        for _ in 0..2 {
            let p = self.boats[i].position;
            let id = self.boats[i].id;
            let kind = self.boats[i].entity_type();
            let source = horizon_client::health::HurtSource {
                damage_type: "minecraft:lava",
                causing: 0,
                direct: 0,
                position: None,
            };
            let vanilla = self.saved_refuses_damage(id, "minecraft:lava", false);
            let broke = !self.native_nonliving_invulnerable(id, &kind, source, vanilla)
                && self.hurt_boat(i, 4.0, false);
            self.lava_hurt_sound([p.x, p.y, p.z]);
            if broke {
                return true;
            }
        }
        false
    }
    /// `Entity.lavaHurt`'s sound once the hurt went through.
    pub(super) fn lava_hurt_sound(&mut self, at: [f64; 3]) {
        let pitch = 2.0 + self.world.random.next_float() * 0.4;
        self.audio
            .play("minecraft:entity.generic.burn", Some(at), 0.4, pitch);
    }
    /// Whether a vehicle's box, deflated by 1e-5, reaches into lava below its
    /// surface (`Entity.collidedWithFluid`).
    pub(super) fn in_lava(&self, bb: Aabb) -> bool {
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let e = 1.0e-5;
        for x in (lo.x + e).floor() as i32..=(hi.x - e).floor() as i32 {
            for y in (lo.y + e).floor() as i32..=(hi.y - e).floor() as i32 {
                for z in (lo.z + e).floor() as i32..=(hi.z - e).floor() as i32 {
                    if let Some((height, _)) = self.fluid_height_of(x, y, z, Some(Fluid::Lava))
                        && lo.y + e < f64::from(y) + f64::from(height)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
    /// `AbstractBoat.modifyPassengerFluidInteractionBox`: the ridden boat's box
    /// and whether it is underwater, for `Entity.getFluidInteractionBox`. A boat
    /// keeps only the part of the passenger's box above its own top, so a boat
    /// bobbing on water never sinks the passenger's eyes or body.
    pub(super) fn riding_boat_fluid_box(&self) -> Option<RiddenBoat> {
        self.boats
            .iter()
            .find(|b| Some(b.id) == self.riding)
            .map(|b| RiddenBoat {
                under_water: b.under_water(),
                max_y: b.bounds().max_position().y,
            })
    }
    /// `getDismountLocationForPassenger`: in front of the player if there is
    /// floor there, else on top of the boat.
    pub(super) fn dismount(&mut self, i: usize) {
        let id = self.boats[i].id;
        if self.riding == Some(id) && !self.native_may_leave(id) {
            return;
        }
        self.riding = None;
        // `AbstractBoat.tick`'s non-authoritative branch: the copy of a boat
        // that no player of this side drives drops its cached motion
        // (`setDeltaMovement(Vec3.ZERO)`, `deltaRotation = 0`) and is not
        // moved, so a boat is at a standstill when its rider gets out instead
        // of coasting on the velocity of the ride that just ended.
        self.boats.update(i, |b| b.velocity = Vec3::ZERO);
        self.boats.update(i, |b| b.delta_rotation = 0.0);
        if self.dismount_in_portal(self.boats[i].position) {
            return;
        }
        let target = self.boat_player_dismount_target(i, self.yaw);
        if let Some(p) = self.player.as_mut() {
            p.position = target;
            p.velocity = Vec3::ZERO;
        }
    }
    /// Where the boat at `i` lets a player facing `yaw` off.
    pub(super) fn boat_player_dismount_target(&self, i: usize, yaw: f32) -> Vec3 {
        self.boat_dismount_target(&self.boats[i], 0.6, yaw)
    }
    /// `getDismountLocationForPassenger` for a passenger `width` wide
    /// facing `yaw`.
    pub(super) fn boat_dismount_target(&self, b: &Boat, width: f64, yaw: f32) -> Vec3 {
        let r = yaw.to_radians();
        let distance = (WIDTH * std::f64::consts::SQRT_2 + width + 1.0e-5) / 2.0;
        let (s, c) = (-(r.sin()) as f64, r.cos() as f64);
        let m = s.abs().max(c.abs());
        let (x, z) = (
            b.position.x + s * distance / m,
            b.position.z + c * distance / m,
        );
        let top = b.bounds().max_position().y;
        let (bx, by, bz) = (x.floor() as i32, top.floor() as i32, z.floor() as i32);
        let below = self.res.facts().get(self.world.block(bx, by - 1, bz));
        let here = self.res.facts().get(self.world.block(bx, by, bz));
        if below.fluid != Fluid::Water
            && !here.full_collision
            && (below.full_collision || below.legacy_solid)
        {
            Vec3::new(x, f64::from(by), z)
        } else {
            Vec3::new(b.position.x, top, b.position.z)
        }
    }
    /// `Entity.setYRot` on a boat just made (its old rotation moves with it).
    pub(super) fn set_boat_yaw(&mut self, id: u32, yaw: f32) {
        if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            self.boats.update(i, |b| {
                b.yaw = yaw;
                b.previous_yaw = yaw;
            });
        }
    }
    /// `BoatItem.use`: aim at a block or the water surface.
    pub(super) fn place_boat(&mut self, stack: &ItemStack) -> bool {
        let Some(boat) = self.boat_to_place(stack) else {
            return false;
        };
        self.next_entity += 1;
        let key = (
            boat.position.x.floor() as i32 >> 4,
            boat.position.z.floor() as i32 >> 4,
        );
        self.world.modified.insert(key);
        let (id, kind, yaw) = (boat.id, boat.entity_type(), boat.yaw);
        self.boats.push(boat);
        self.apply_item_stack_config(super::commands::Target::Boat(id), &kind, stack, true);
        // BoatItem.use: setYRot(player.getYRot()) after the stack's config.
        self.set_boat_yaw(id, yaw);
        if self.serving.is_some() || self.options.dedicated.is_some() {
            self.native_vehicle_create(
                self.boats.last().unwrap().entity_type().as_str(),
                [
                    self.boats.last().unwrap().position.x,
                    self.boats.last().unwrap().position.y,
                    self.boats.last().unwrap().position.z,
                ],
            );
        }
        // BoatItem.use: ENTITY_PLACE at the hit (the boat's position), by the player.
        let at = self
            .boats
            .last()
            .map(|b| [b.position.x, b.position.y, b.position.z]);
        if let Some(at) = at {
            let ctx = self.player_context(None);
            self.post_game_event("minecraft:entity_place", at, ctx);
        }
        if !self.creative {
            self.consume_held();
        }
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        true
    }
    /// The boat `BoatItem.use` would place for `stack` (its SUCCESS), or
    /// None when it aims at nothing (PASS) or the spot is taken (FAIL).
    pub(super) fn boat_to_place(&self, stack: &ItemStack) -> Option<Boat> {
        let (wood, chest) = any_boat_wood(&stack.item)?;
        if !self.boat_models.has(wood, chest) {
            return None;
        }
        let hit = self.boat_item_hit()?;
        let mut boat = Boat::new(
            self.next_entity + 1,
            Arc::from(wood),
            Vec3::new(hit[0], hit[1], hit[2]),
            self.yaw,
        );
        if chest {
            boat = boat.with_chest();
        }
        let bb = boat.bounds();
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let e = 1.0e-7;
        let inner = Aabb::new(
            Vec3::new(lo.x + e, lo.y + e, lo.z + e),
            Vec3::new(hi.x - e, hi.y - e, hi.z - e),
        );
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if !scene.unobstructed(inner).unwrap_or(false) || self.boat_entity_collision(bb) {
            return None;
        }
        Some(boat)
    }
    pub(super) fn boat_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = self.entity_reach;
        let mut best: Option<(usize, f64)> = None;
        for (i, b) in self.boats.iter().enumerate() {
            if Some(b.id) == self.riding {
                continue;
            }
            let bb = b.bounds();
            let (lo, hi) = (bb.min_position(), bb.max_position());
            let (lo, hi) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
            let (mut t0, mut t1) = (0.0f64, reach);
            let mut hit = true;
            for a in 0..3 {
                if dir[a].abs() < 1e-9 {
                    hit &= eye[a] >= lo[a] && eye[a] <= hi[a];
                    continue;
                }
                let (mut ta, mut tb) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            if hit && t0 <= t1 && best.is_none_or(|b| t0 < b.1) {
                best = Some((i, t0));
            }
        }
        best
    }
    /// `AbstractBoat.interact`: board the boat under the crosshair, or the one
    /// a guest's interact packet named.
    pub(super) fn interact_boat(&mut self, target: Option<InteractTarget>) -> bool {
        let Some((i, d)) = (match target {
            Some(t) => Self::named_entity(&self.boats, t.id, |b| b.id).map(|i| (i, 0.0)),
            None => self.boat_hit(),
        }) else {
            return false;
        };
        let sneaking = self.keys.held(Action::Sneak);
        // Only a client's own pick is stopped by a block: the range check passed.
        if target.is_none() && !self.before_block(d) {
            return false;
        }
        // AbstractBoat.interact starts with Entity.interact: the lead.
        if self.interact_boat_leash(i) {
            self.swing_arm();
            return true;
        }
        // ChestBoat.interact: sneaking (or a full seat) opens the chest. A
        // player in another vehicle boards instead (`Entity.startRiding`
        // leaves the old one first).
        if self.boats[i].chest && (sneaking || !self.boat_has_room(i)) {
            self.open_chest_boat_container(i);
            self.swing_arm();
            return true;
        }
        if sneaking
            || self.boats[i].out_of_control >= 60.0
            || self.ridden_by_guest(self.boats[i].id)
            || !self.boat_has_room(i)
        {
            return false;
        }
        let id = self.boats[i].id;
        self.start_riding_target(
            super::commands::Target::Player,
            super::commands::Target::Boat(id),
            false,
        );
        true
    }
    /// `ContainerEntity.interactWithContainerVehicle` (`AbstractChestBoat`):
    /// the chest's 27 slots go under a placeholder key that the menu edits; a
    /// server hands them to the player as a `ChestMenu` (`as_guest` ->
    /// `lan_open_container`). `AbstractChestBoat.interact` fires
    /// `CONTAINER_OPEN` and angers the piglins that see it.
    pub(super) fn open_chest_boat_container(&mut self, i: usize) {
        let b = &self.boats[i];
        let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, b.id as i32);
        let at = [b.position.x, b.position.y, b.position.z];
        let items = b.items.clone();
        // `startOpen` shares one container, it does not reset it: a stand-in
        // another player's menu is still editing over stays as it is.
        if !self.world.block_entities.contains_key(&key) {
            self.world
                .block_entities
                .insert(key, horizon_client::containers::BlockEntity::Chest(items));
        }
        self.container = Some(key);
        self.set_screen(super::Screen::Inventory);
        self.capture(false);
        self.right = false;
        let ctx = self.player_context(None);
        self.post_game_event("minecraft:container_open", at, ctx);
        self.anger_piglins(true);
    }
    /// `AbstractChestBoat.openCustomInventoryScreen` (`HasCustomInventoryScreen`),
    /// the inventory key while riding a chest boat: its chest opens, not the
    /// player's inventory.
    pub(super) fn open_ridden_chest_boat(&mut self) -> bool {
        let Some(id) = self.riding else {
            return false;
        };
        let Some(i) = self.boats.iter().position(|b| b.id == id) else {
            return false;
        };
        if !self.boats[i].chest {
            return false;
        }
        self.open_chest_boat_container(i);
        true
    }
    /// Whether the player rides a chest boat: `HasCustomInventoryScreen`, so
    /// the inventory key sends `ServerboundPlayerCommandPacket.Action.OPEN_INVENTORY`
    /// (`ServerGamePacketListenerImpl.handlePlayerCommand` opens its screen).
    pub(super) fn riding_chest_boat(&self) -> bool {
        self.riding
            .and_then(|id| self.boats.iter().find(|b| b.id == id))
            .is_some_and(|b| b.chest)
    }
    /// `VehicleEntity.hurtServer` from the player's attack.
    pub(super) fn attack_boat(&mut self) -> bool {
        let Some((i, d)) = self.boat_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        self.strike_boat(i);
        self.swing_arm();
        true
    }
    /// The accepted boat target, shared by aimed attacks and headless parity.
    pub(super) fn strike_boat(&mut self, i: usize) {
        let area = self.boats[i].bounds();
        // Player.attack's full damage against the boat's entity type.
        let (kind, id) = (self.boats[i].entity_type(), self.boats[i].id as i32);
        let (damage, scale, _, magic) = self.melee_damage(&kind, false, Some(id as u32));
        let sweep = self.sweep_before_hit(&kind, false);
        let chest = self.boats[i].chest;
        let attacker = self.own_entity_id();
        let source = horizon_client::health::HurtSource {
            damage_type: "minecraft:player_attack",
            causing: attacker,
            direct: attacker,
            position: None,
        };
        let vanilla =
            self.saved_refuses_damage(id as u32, "minecraft:player_attack", self.creative);
        let invulnerable = crate::game::entity::damage_level_of!(self, scoped)
            .invulnerability_check(
                horizon_client::health::Hurting::Entity(id, &kind, source),
                vanilla,
            );
        let hurt = if invulnerable {
            None
        } else {
            self.hurt_boat_result(
                i,
                damage,
                self.creative,
                Some(crate::game::commands::Target::Player),
            )
        };
        // AbstractChestBoat.destroy -> ContainerEntity.chestVehicleDestroyed:
        // with entity_drops on, a player whose hit breaks a chest boat (not a
        // creative discard) angers the piglins that see it.
        if chest
            && hurt == Some(true)
            && !self.creative
            && self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::ENTITY_DROPS)
        {
            self.anger_piglins(true);
        }
        self.vehicle_attack_sounds(area, sweep, magic.then_some(id));
        if damage > 0.0
            && let Some(destroyed) = hurt
        {
            let (knockback, _) = self.extra_knockback(&kind, scale);
            if knockback > 0.0 {
                // Player.causeExtraKnockback uses Entity.push for a boat,
                // not LivingEntity.knockback (no resistance or velocity halving).
                if !destroyed {
                    let (x, z) = self.extra_knockback_direction();
                    self.boats
                        .update(i, |b| b.velocity.x += f64::from(-(x as f32) * knockback));
                    self.boats.update(i, |b| b.velocity.y += 0.1);
                    self.boats
                        .update(i, |b| b.velocity.z += f64::from(-(z as f32) * knockback));
                }
                self.special_attack_slowdown(knockback);
            }
            self.special_attack_exhaustion();
        }
        // Player.onAttack: the hit starts the attack strength again.
        self.attack_ticks = 0;
    }
    /// `VehicleEntity.hurtServer`: wobble, add the damage and break past 40
    /// (`creative`: a creative player's hit discards it straight away).
    /// `AbstractChestBoat.remove` drops the chest contents for every removal
    /// reason that destroys the boat (DISCARDED included), while the boat item
    /// itself only drops on the death path (`VehicleEntity.destroy`), not when
    /// a creative hit discards it. True when the boat was removed.
    pub(super) fn hurt_boat(&mut self, i: usize, damage: f32, creative: bool) -> bool {
        self.hurt_boat_by(i, damage, creative, None)
    }
    /// Whether a spectator must be refused this boat's menu.
    pub(super) fn boat_loot_pending(&self, id: u32) -> bool {
        matches!(
            self.persist.raw.get(&id),
            Some(horizon_core::nbt::Tag::Compound(map)) if map.contains_key(&"LootTable".into())
        )
    }

    /// Existing boat access routes all use the shared unpack operation.
    pub(super) fn unpack_boat_loot(&mut self, i: usize, player: bool) {
        let player = player.then(|| self.native_acting_player()).flatten();
        self.unpack_loot(
            super::container_loot::LootTarget::Boat(self.boats[i].id),
            player,
        );
    }

    pub(super) fn hurt_boat_result(
        &mut self,
        i: usize,
        mut damage: f32,
        creative: bool,
        by: Option<super::commands::Target>,
    ) -> Option<bool> {
        let id = self.boats[i].id;
        if self
            .persist
            .destroying_vehicles
            .contains(&super::commands::Target::Boat(id))
        {
            return Some(true);
        }
        let mut cancel = false;
        self.native_vehicle_damage("boat", &mut damage, &mut cancel);
        if cancel {
            return None;
        }
        let Some(i) = self.boats.iter().position(|b| {
            b.id == id
                && !self
                    .persist
                    .removals
                    .contains(&crate::game::commands::Target::Boat(id))
        }) else {
            return Some(true);
        };
        let (destroyed, at) = self.boats.update(i, |b| {
            b.hurt_dir = -b.hurt_dir;
            b.hurt_time = 10;
            b.damage += damage * 10.0;
            (
                creative || b.damage > 40.0,
                [b.position.x, b.position.y, b.position.z],
            )
        });
        self.vehicle_damage_event(at, by);
        if destroyed {
            self.native_vehicle_destroy("boat", at);
            if !self.boats.iter().any(|b| {
                b.id == id
                    && !self
                        .persist
                        .removals
                        .contains(&crate::game::commands::Target::Boat(id))
            }) {
                return Some(true);
            }
            if self
                .with_destroying_vehicle(super::commands::Target::Boat(id), |game| {
                    game.dismount_destroyed_vehicle_players(id);
                    if let Some(i) = game.boats.iter().position(|boat| boat.id == id) {
                        game.eject_boat_mobs(i, true);
                    }
                })
                .is_none()
            {
                return Some(true);
            }
            let Some(i) = self.boats.iter().position(|b| {
                b.id == id
                    && !self
                        .persist
                        .removals
                        .contains(&crate::game::commands::Target::Boat(id))
            }) else {
                return Some(true);
            };
            self.release_vehicle_container(id);
            self.unpack_boat_loot(i, false);
            let b = self.boats.remove(i);
            // `AbstractChestBoat.remove` spills the chest through
            // `Containers.dropContents` for every destroying removal reason
            // (the kill below, and a creative hit's discard); that has no
            // `entity_drops` gate of its own. A plain boat has no slots.
            self.drop_vehicle_contents(at, &b.items);
            // AbstractBoat.remove, after that: a destroyed boat drops its lead.
            self.boat_removed(&b);
            // `VehicleEntity.destroy(level, dropItem)`: `kill` (the boat's own
            // ENTITY_DIE after that removal), then the boat item on the death
            // path only, and only with `entity_drops` on, by `spawnAtLocation`
            // (an ItemEntity's own random motion).
            if !creative {
                self.entity_killed_event(at, false);
            }
            // `VehicleEntity.destroy(level, dropItem)` drops the boat item on
            // the death path only, and only with `entity_drops` on.
            if !creative
                && self
                    .ui
                    .rules
                    .bool_at(horizon_client::gamerules::vanilla::ENTITY_DROPS)
            {
                let item = b.entity_type();
                self.spawn_item_own(
                    ItemStack {
                        item: Arc::from(item.as_str()),
                        count: 1,
                        damage: 0,
                        extra: None,
                    },
                    at,
                    10,
                );
                // AbstractChestBoat.destroy: `chestVehicleDestroyed` drops the
                // emptied chest again, which only draws each slot's spot.
                if b.chest {
                    let empty = vec![None; b.items.len()];
                    self.drop_vehicle_contents(at, &empty);
                }
            }
            self.retain_removed_vehicle(id, super::persist::RemovedVehicle::Boat(b));
        }
        Some(destroyed)
    }
    /// AbstractBoatRenderer.
    pub(super) fn boat_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let a = alpha as f32;
        let range = super::entity_culling::default_range(
            glam::DVec3::new(WIDTH, HEIGHT, WIDTH),
            self.view_scale(),
        );
        for b in &self.boats {
            // LevelExtractor: the camera entity is not drawn from its own eyes.
            if self.hides_as_camera(b.id) {
                continue;
            }
            // EntityRenderer.shouldRender.
            let (previous, position) = (at(b.previous), at(b.position));
            let Some(sight) = self.standing_entity_sight(
                camera, previous, position, alpha, WIDTH, HEIGHT, range, 0.0,
            ) else {
                continue;
            };
            let culling = self.sighted_decision(sight, || self.entity_cull_decision(b, alpha));
            if culling.hides_entity() {
                continue;
            }
            let entity_start = out.len();
            self.mark_motion(entity_start, Self::MOTION_OBJECT | u64::from(b.id));
            let models = if b.chest {
                &self.boat_models.chest_models
            } else {
                &self.boat_models.models
            };
            let Some(model) = models.get(&*b.wood) else {
                continue;
            };
            let folder = if b.chest { "chest_boat" } else { "boat" };
            let Some(uv) = self
                .gui
                .atlas
                .sprites
                .get(&format!("entity/{folder}/{}", b.wood))
                .map(|r| [r.u0, r.v0, r.u1, r.v1])
            else {
                continue;
            };
            let p = glam::DVec3::new(
                b.previous.x + (b.position.x - b.previous.x) * alpha,
                b.previous.y + (b.position.y - b.previous.y) * alpha,
                b.previous.z + (b.position.z - b.previous.z) * alpha,
            ) - camera;
            let yaw = b.previous_yaw + (b.yaw - b.previous_yaw) * a;
            let hurt = b.hurt_time as f32 - a;
            let damage = (b.damage - a).max(0.0);
            let mut base =
                glam::Mat4::from_translation(p.as_vec3() + glam::Vec3::new(0.0, 0.375, 0.0))
                    * glam::Mat4::from_rotation_y((180.0 - yaw).to_radians());
            if hurt > 0.0 {
                base *= glam::Mat4::from_rotation_x(
                    (hurt.sin() * hurt * damage / 10.0 * b.hurt_dir as f32).to_radians(),
                );
            }
            // AbstractBoatRenderer.extractRenderState: bubbleAngle.
            let wobble = b.bubble_angle(a);
            // AbstractBoatRenderer.submit: a boat above water wobbles in a
            // bubble column, about the axis (1, 0, 1).
            if !b.under_water() && wobble.abs() >= 1.0e-5 {
                base *= glam::Mat4::from_axis_angle(
                    glam::Vec3::new(1.0, 0.0, 1.0).normalize(),
                    wobble * (std::f64::consts::PI / 180.0) as f32,
                );
            }
            base *= glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
                * glam::Mat4::from_rotation_y(90f32.to_radians());
            let mut poses = Poses::new(model);
            // AbstractBoatModel.animatePaddle
            for (side, name) in [(0usize, "left_paddle"), (1, "right_paddle")] {
                if !poses.has(name) {
                    continue;
                }
                let t = b.rowing_time(side, a);
                let lerp = |f: f32, lo: f32, hi: f32| lo + f.clamp(0.0, 1.0) * (hi - lo);
                let pose = poses.get(name);
                pose[3] = lerp(((-t).sin() + 1.0) / 2.0, -PI / 3.0, -PI / 12.0);
                pose[4] = lerp(((-t + 1.0).sin() + 1.0) / 2.0, -PI / 4.0, PI / 4.0);
                if side == 1 {
                    pose[4] = PI - pose[4];
                }
            }
            let l = self.world.light(
                b.position.x.floor() as i32,
                (b.position.y + 0.3).floor() as i32,
                b.position.z.floor() as i32,
            );
            let packed = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            horizon_client::mobs::render::draw(
                model,
                base,
                &poses,
                uv,
                0xFFFF_FFFF,
                packed,
                0,
                out,
            );
            // BoatRenderer.submitTypeAdditions: the water patch hides the water
            // inside the hull unless the boat is under water (RaftRenderer has none).
            if let Some(patch) = self.boat_models.water_patch.as_ref()
                && &*b.wood != "bamboo"
                && !matches!(b.status, Status::UnderWater | Status::UnderFlowingWater)
            {
                // Depth only, so its texture is never seen; one texel of the white sprite, the
                // same at every corner (no derivatives, so the base level at any distance), keeps
                // a shader pack's alpha test from throwing the patch away.
                let white = (self.gui.atlas.sprites.get("horizon/white")).map_or(uv, |r| {
                    let (u, v) = ((r.u0 + r.u1) / 2.0, (r.v0 + r.v1) / 2.0);
                    [u, v, u, v]
                });
                horizon_client::mobs::render::draw(
                    patch,
                    base,
                    &Poses::new(patch),
                    white,
                    0xFFFF_FFFF,
                    packed,
                    horizon_client::gfx::WATER_MASK_OVERLAY,
                    out,
                );
            }
            self.finish_lit_entity(culling, b, out, entity_start);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::nbt::Tag;

    fn boat() -> Boat {
        Boat::new(1, Arc::from("oak"), Vec3::ZERO, 0.0)
    }

    /// `controlBoat`'s push with `Mth`'s sine table (`(float) Math.sin(i /
    /// 10430.378350470453)`, indexed by `(long) (radians * 10430.378350470453)
    /// & 65535`) and float products. Expected values worked out apart from
    /// Horizon, in Python with float32 rounding; the platform's sine gives
    /// -0.011894993484020233 for the first x.
    #[test]
    fn paddling_pushes_along_the_sine_table() {
        for (yaw, acceleration, push) in [
            (17.3, 0.04, (-0.011893643997609615, 0.03819085285067558)),
            (-123.45, 0.04, (0.03337550163269043, -0.022047126665711403)),
            (250.0, 0.04, (0.03758755698800087, -0.013681206852197647)),
            (-6.736, 0.005, (0.0005863539990969002, 0.004965443629771471)),
        ] {
            assert_eq!(paddle_push(yaw, acceleration), push, "yaw {yaw}");
        }
    }

    #[test]
    #[ignore = "daily"]
    fn destruction_and_underwater_ejection_snap_rejected_passenger_descendants() {
        for broken in [false, true] {
            super::super::native_entity_refs::tests::with_game(|game| {
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let mut boat = Boat::new(71, "oak".into(), Vec3::new(10.0, 200.0, 20.0), 0.0);
                boat.status = Status::UnderWater;
                boat.out_of_control = 60.0;
                game.boats.push(boat);
                let mut rider = horizon_client::mobs::Mob::new(
                    72,
                    kind.clone(),
                    Vec3::new(5.0, 190.0, 6.0),
                    0.0,
                );
                rider.extra.mount = Some(71);
                rider.extra.passenger = true;
                game.persist
                    .off_level
                    .insert(72, super::super::persist::OffLevel::Mob(rider));
                let mut child = horizon_client::mobs::Mob::new(73, kind, Vec3::ZERO, 0.0);
                child.extra.mount = Some(72);
                child.age = 19;
                game.mobs.push(child);
                if broken {
                    assert!(game.hurt_boat(0, 5.0, true));
                    assert!(game.boats.is_empty());
                } else {
                    // The actual ejection entry used by AbstractBoat.tick at 60 submerged ticks.
                    game.eject_boat_riders(0);
                    assert_eq!(game.boats.len(), 1);
                }
                let rider = game.mob_object(72).unwrap();
                assert_eq!(rider.extra.mount, None);
                assert!(!rider.extra.passenger);
                assert_eq!(rider.extra.boarding_cooldown, 60);
                // Removed vehicle: retain x/z and clamp y to the boat's y.
                // Live underwater boat over air: getDismountLocation's box-top fallback.
                let expected = if broken {
                    Vec3::new(5.0, 200.0, 6.0)
                } else {
                    Vec3::new(10.0, 200.5625, 20.0)
                };
                assert_eq!(rider.position, expected);
                let child = game.mob_object(73).unwrap();
                assert_eq!(child.position.x, expected.x);
                assert_eq!(child.position.z, expected.z);
                assert!((child.position.y - expected.y - 0.86875).abs() < 1e-6);
                assert_eq!(child.previous, child.position);
                assert_eq!(child.age, 19);
                assert_eq!(child.extra.mount, Some(72));
            });
        }
    }

    /// `AbstractBoat.tickBubbleColumn` on a client: the wobble grows while the
    /// bubble time runs and fades after.
    #[test]
    fn the_wobble_grows_while_the_bubble_time_runs_and_fades_after() {
        let mut b = boat();
        b.bubble_time = BUBBLE_TIME;
        for _ in 0..30 {
            b.tick_bubble_wobble();
        }
        // 0.05 a tick, capped at 1.
        assert_eq!(b.bubble_multiplier, 1.0);
        // 10 * sin(0.5 * tickCount) * multiplier.
        let expected = 10.0 * (0.5 * 30.0f64).sin() as f32;
        assert!((b.bubble_angle - expected).abs() < 1.0e-4);
        // The render angle lerps between the last two ticks.
        let mid = b.bubble_angle(0.5);
        assert!((mid - (b.bubble_angle_old + b.bubble_angle) / 2.0).abs() < 1.0e-5);
        // Once the bubble time is 0 the multiplier fades by 0.1 a tick.
        b.bubble_time = 0;
        for _ in 0..4 {
            b.tick_bubble_wobble();
        }
        assert!((b.bubble_multiplier - 0.6).abs() < 1.0e-5);
        for _ in 0..10 {
            b.tick_bubble_wobble();
        }
        assert_eq!(b.bubble_multiplier, 0.0);
        assert_eq!(b.bubble_angle, 0.0);
    }

    /// `AbstractBoat.tickBubbleColumn` on the server: a boat held over a
    /// column fires on the 60th tick, with the column's direction, then the
    /// time restarts.
    #[test]
    fn the_bubble_countdown_fires_every_sixty_ticks_over_a_column() {
        for down in [false, true] {
            let mut b = boat();
            let mut fired = Vec::new();
            for tick in 1..=130 {
                // What boat_above_bubble_column does each tick.
                b.above_bubble_column = true;
                b.bubble_down = down;
                if b.bubble_time == 0 {
                    b.bubble_time = BUBBLE_TIME;
                }
                if let Some(d) = b.tick_bubble_countdown() {
                    fired.push((tick, d));
                }
                assert!(!b.above_bubble_column);
            }
            assert_eq!(fired, vec![(60, down), (120, down)]);
        }
    }

    /// Leaving the column resets the time: nothing fires, and a boat with no
    /// time left does nothing over a column until the column sets it.
    #[test]
    fn leaving_the_column_resets_the_bubble_countdown() {
        let mut b = boat();
        b.bubble_time = BUBBLE_TIME;
        for _ in 0..30 {
            b.above_bubble_column = true;
            assert_eq!(b.tick_bubble_countdown(), None);
        }
        assert_eq!(b.bubble_time, 30);
        // A tick away from the column.
        assert_eq!(b.tick_bubble_countdown(), None);
        assert_eq!(b.bubble_time, 0);
        b.above_bubble_column = true;
        assert_eq!(b.tick_bubble_countdown(), None);
        assert!(b.above_bubble_column);
    }

    #[test]
    fn a_boat_that_never_saw_a_column_does_not_wobble() {
        let mut b = boat();
        for _ in 0..10 {
            b.tick_bubble_wobble();
        }
        assert_eq!(b.bubble_angle, 0.0);
        assert_eq!(b.bubble_angle(0.5), 0.0);
    }

    /// #8156: the powder snow cells a boat's box reaches into, its box deflated by 1e-5.
    #[test]
    fn a_boat_reaches_the_powder_snow_cells_its_box_touches() {
        let b = Boat::new(1, "oak".into(), Vec3::new(10.5, 64.0, 10.5), 0.0);
        let snow = |cells: &'static [[i32; 3]]| move |p: [i32; 3]| cells.contains(&p);
        // Inside, a corner cell, and a cell the box only ends against.
        assert_eq!(
            reached_cells(b.bounds(), snow(&[[10, 64, 10]])),
            vec![[10, 64, 10]]
        );
        assert_eq!(
            reached_cells(b.bounds(), snow(&[[9, 64, 11], [12, 64, 10], [10, 65, 10]])),
            vec![[9, 64, 11]],
            "the cell beyond the edge and the one above the box are not reached"
        );
        let edge = Boat::new(1, "oak".into(), Vec3::new(11.3125, 64.0, 10.5), 0.0);
        assert!(reached_cells(edge.bounds(), snow(&[[12, 64, 10]])).is_empty());
        // x, y, z order.
        let all = reached_cells(b.bounds(), |_| true);
        assert_eq!(all.len(), 9);
        assert_eq!((all[0], all[8]), ([9, 64, 9], [11, 64, 11]));
    }

    /// `Entity.move`: a boat made stuck goes its velocity times the multiplier once, and
    /// loses its velocity; one that is not goes as it did.
    #[test]
    fn a_stuck_boat_moves_a_part_of_its_velocity_once() {
        let mut b = Boat::new(1, "oak".into(), Vec3::ZERO, 0.0);
        b.velocity = Vec3::new(1.0, 0.5, 2.0);
        assert_eq!(b.take_movement(), Vec3::new(1.0, 0.5, 2.0));
        assert_eq!(b.velocity, Vec3::new(1.0, 0.5, 2.0));
        b.stuck = powder_snow_stuck();
        let moved = b.take_movement();
        let slow = f64::from(0.9_f32);
        assert_eq!(moved, Vec3::new(slow, 0.75, 2.0 * slow));
        assert_eq!((b.velocity, b.stuck), (Vec3::ZERO, Vec3::ZERO));
        b.velocity = Vec3::new(0.0, 0.1, 1.0);
        assert_eq!(b.take_movement(), Vec3::new(0.0, 0.1, 1.0), "spent");
    }

    /// `Entity.saveWithoutId` writes `TicksFrozen` only while positive; `Entity.load` reads it.
    #[test]
    fn a_boat_saves_and_loads_ticks_frozen() {
        let mut b = Boat::new(
            1,
            std::sync::Arc::from("oak"),
            Vec3::new(0.0, 64.0, 0.0),
            0.0,
        );
        assert_eq!(b.to_nbt([0; 4], None).get("TicksFrozen"), None);
        b.ticks_frozen = 6;
        let saved = b.to_nbt([0; 4], None);
        assert_eq!(saved.get("TicksFrozen"), Some(&Tag::Int(6)));
        assert_eq!(saved_ticks_frozen(&saved), 6);
        b.ticks_frozen = 0;
        assert_eq!(b.to_nbt([0; 4], Some(&saved)).get("TicksFrozen"), None);
    }
    /// `PowderSnowBlock.entityInside`'s snowflake: only for an entity that moved sideways, and
    /// then only when a coin falls so.
    #[test]
    fn a_boat_moving_sideways_in_powder_snow_throws_snowflakes_now_and_then() {
        use horizon_core::random::LegacyRandom;
        // Still or only rising: no coin is drawn.
        let mut random = LegacyRandom::new(7);
        let mut untouched = LegacyRandom::new(7);
        assert_eq!(
            snowflake_velocity(Vec3::new(0.0, 0.4, 0.0), &mut random),
            None
        );
        assert_eq!(random.next_int(), untouched.next_int());
        let known = Vec3::new(0.1, 0.0, 0.0);
        // One stream, as the server's: about half the calls draw a snowflake. (Seeds a step
        // apart would not do, their first coin is nearly always the same.)
        let mut stream = LegacyRandom::new(42);
        let flakes: Vec<[f64; 3]> = (0..200)
            .filter_map(|_| snowflake_velocity(known, &mut stream))
            .collect();
        assert!(
            (60..=140).contains(&flakes.len()),
            "about half the seeds: {}",
            flakes.len()
        );
        let limit = f64::from(0.083_333_336_f32);
        for [x, y, z] in flakes {
            assert!(x.abs() <= limit && z.abs() <= limit);
            assert_eq!(y, f64::from(0.05_f32));
        }
        // Either horizontal axis is enough.
        assert!(
            (0..64)
                .any(|_| { snowflake_velocity(Vec3::new(0.0, 0.0, -0.1), &mut stream).is_some() })
        );
    }
}
#[cfg(test)]
mod cobweb_tests {
    use super::Boat;
    use crate::game::game_tests::tests::server_game;
    use horizon_client::chunk::Chunk;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    /// `AbstractBoat.tick`'s `applyEffectsFromBlocks` calls `WebBlock.entityInside`, which
    /// makes the boat stuck: its next move goes a quarter of the speed across and a
    /// twentieth down, its velocity is spent and its fall is reset. A boat outside the web
    /// moves as it did and keeps falling.
    #[test]
    #[ignore = "daily"]
    fn a_cobweb_slows_a_boat_and_resets_its_fall() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let web = game
            .pack
            .assets
            .registry
            .default_state("minecraft:cobweb")
            .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.world.set(8, 64, 8, web).unwrap();
        let boat = |x: f64| {
            let mut b = Boat::new(1, Arc::from("oak"), Vec3::new(x, 64.0, 8.5), 0.0);
            b.velocity = Vec3::new(1.0, 0.0, 0.0);
            b.fall_distance = 5.0;
            b
        };
        let (mut in_web, mut outside) = (boat(8.5), boat(12.5));
        // The tick in the web sets the multiplier and resets the fall.
        in_web = game.tick_boat(in_web, false);
        outside = game.tick_boat(outside, false);
        let (web_fall, free_fall) = (in_web.fall_distance, outside.fall_distance);
        // The next move spends the multiplier.
        let (web_from, free_from) = (in_web.position, outside.position);
        in_web = game.tick_boat(in_web, false);
        outside = game.tick_boat(outside, false);
        let (web_moved, free_moved) = (
            in_web.position.subtract(web_from),
            outside.position.subtract(free_from),
        );
        assert!(
            free_moved.x > 0.5,
            "the boat outside the web moves: {free_moved:?}"
        );
        assert_eq!(
            web_moved.x,
            free_moved.x * 0.25,
            "a quarter of the speed across"
        );
        assert_eq!(web_moved.z, 0.0);
        assert!(
            (web_moved.y - free_moved.y * f64::from(0.05_f32)).abs() < 1e-12,
            "a twentieth down: {} against {}",
            web_moved.y,
            free_moved.y
        );
        assert_eq!(
            in_web.velocity,
            Vec3::ZERO,
            "the multiplier and velocity are spent"
        );
        assert_eq!(web_fall, 0.0, "makeStuckInBlock resets the fall");
        assert!(
            free_fall > 5.0,
            "a boat outside the web keeps falling: {free_fall}"
        );
    }

    /// A boat whose box reaches a cobweb and, after the move, powder snow takes
    /// the multiplier of the block vanilla's traversal reaches last: the one
    /// after the move, not the one it leaves.
    #[test]
    #[ignore = "daily"]
    fn a_boat_leaving_a_cobweb_for_powder_snow_takes_the_powder_snow() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let (web, snow) = {
            let registry = &game.pack.assets.registry;
            (
                registry.default_state("minecraft:cobweb").unwrap(),
                registry.default_state("minecraft:powder_snow").unwrap(),
            )
        };
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.world.set(8, 64, 8, web).unwrap();
        game.world.set(10, 64, 8, snow).unwrap();
        let mut boat = Boat::new(1, Arc::from("oak"), Vec3::new(8.5, 64.0, 8.5), 0.0);
        // Far enough in one move to leave the cobweb's cell and reach the snow's.
        boat.velocity = Vec3::new(2.0, 0.0, 0.0);
        boat = game.tick_boat(boat, false);
        // The same boat without the stored multiplier: the next move is the full one.
        let mut free = boat.clone();
        free.stuck = Vec3::ZERO;
        let (from, free_from) = (boat.position, free.position);
        boat = game.tick_boat(boat, false);
        free = game.tick_boat(free, false);
        let (moved, free_moved) = (
            boat.position.subtract(from),
            free.position.subtract(free_from),
        );
        assert!(
            free_moved.x > 1.0,
            "the boat outside the blocks moves: {free_moved:?}"
        );
        assert!(
            (moved.x - free_moved.x * f64::from(0.9_f32)).abs() < 1e-12,
            "powder snow's across multiplier: {} against {}",
            moved.x,
            free_moved.x
        );
        assert!(
            (moved.y - free_moved.y * 1.5).abs() < 1e-12,
            "powder snow's down multiplier: {} against {}",
            moved.y,
            free_moved.y
        );
    }
}
