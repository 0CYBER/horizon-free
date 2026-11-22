//! Nether portals for the player and the other entities:
//! `Entity.setAsInsidePortal` / `handlePortal` with `PortalProcessor`,
//! `NetherPortalBlock.getPortalTransitionTime` and `getPortalDestination`
//! (`PortalForcer` through `horizon_world::level::portal_travel`), and
//! `LocalPlayer.handlePortalTransitionEffect`. Items and mobs also go through
//! End portals here (`EndPortalBlock.entityInside` / `getPortalDestination`);
//! the player's End portals are game/world/end.rs's.
//!
//! A server keeps one level per dimension. Entity portal processes and
//! cooldowns follow their level; shared `Arrival`s hold each traveller's
//! saved compound until its destination chunks are loaded.
use super::entity::{Entity as _, entity_lists_of, for_each_entity};
use super::{Dim, Game, Result, Screen};
use horizon_client::fast_hash::FxHashMap;
use horizon_client::mobs::{Species, spawn_rules};
use horizon_client::nether_portal::{ENTITY_DELAY, PortalState};
use horizon_client::world::PendingEntity;
use horizon_core::{
    geometry::{Aabb, Axis, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::level::flags as level_flags;
use std::collections::HashMap;
use {
    horizon_core::nbt::Tag,
    horizon_world::level::portal_travel::{
        BorderBounds, FoundRectangle, closest_portal, collision_free_position, exit_position,
        largest_rectangle_around, relative_position,
    },
};

#[path = "nether_portal/respawn_attributes.rs"]
mod respawn_attributes;

#[cfg(test)]
#[path = "tests/portal_spawn_tests.rs"]
mod portal_spawn_tests;

/// `Player.getDimensionChangingDelay`.
pub(super) const PLAYER_DELAY: i32 = 10;
/// `VehicleEntity.getDimensionChangingDelay`: boats and minecarts.
const VEHICLE_DELAY: i32 = 10;
/// `Projectile.getDimensionChangingDelay`.
const PROJECTILE_DELAY: i32 = 2;

/// Where in its portal a traveller stood, and where its exit is to be found.
#[derive(Clone, Copy, Debug)]
pub(super) struct Plan {
    pub(super) dim: Dim,
    /// The border-clamped, scaled position (`approximateExitPos`).
    pub(super) approximate: BlockPos,
    entry_x_axis: bool,
    offset: Vec3,
    width: f32,
    height: f32,
    /// Through an End portal: `approximate` is the End's spawn point, or the
    /// Overworld's shared spawn.
    pub(super) end: bool,
}

impl Plan {
    /// `EndPortalBlock.getPortalDestination` for an entity other than a
    /// player in `from`: the End's spawn point, or out of the End the
    /// Overworld's shared spawn `world_spawn`.
    fn end_portal(from: Dim, world_spawn: BlockPos, width: f32, height: f32) -> Self {
        let (dim, approximate) = if from == Dim::End {
            (Dim::Overworld, world_spawn)
        } else {
            let [x, y, z] = super::end::END_SPAWN;
            (Dim::End, BlockPos::new(x, y, z))
        };
        Self {
            dim,
            approximate,
            entry_x_axis: true,
            offset: Vec3::ZERO,
            width,
            height,
            end: true,
        }
    }
}

/// An entity on its way to a dimension that isn't loaded.
#[derive(Debug)]
enum TransferredArrivalState {
    Entity(super::entity_initialization::TransferredEntityState),
    VehicleTree(super::persist::TransferredVehicleTree),
}

#[derive(Debug)]
struct Arrival {
    /// Teleport-only state; deliberately absent from arrival_tag.
    tnt_used_portal: bool,
    plan: Plan,
    tag: Tag,
    transferred: Option<TransferredArrivalState>,
}

/// The nether portal state of the world (`Game::portals`).
#[derive(Default)]
pub(super) struct Portals {
    /// The player's.
    pub(super) player: PortalState,
    /// `LocalPlayer.portalProcess.isInsidePortalThisTick`.
    client_inside: bool,
    /// Whether the last tick's `handlePortalTransitionEffect` found the
    /// player inside a nether portal: it closes any screen opened meanwhile.
    client_was_inside: bool,
    /// Items and mobs, by entity id.
    pub(super) entities: HashMap<u32, PortalState>,
    /// Positions before this tick's moves, by entity id.
    pub(super) before: FxHashMap<u32, Vec3>,
    arrivals: Vec<Arrival>,
    /// The vehicles server players ride that went through a portal this
    /// tick, in their level, with their entry: they carry their riders
    /// (`tick_server_travel`).
    carrying: Vec<(Dim, u32, BlockPos)>,
    /// What a carrying minecart moved by when its portal was handled, at the
    /// start of its tick: its motion is what crosses (`arrive_vehicle`).
    pub(super) carried_motion: HashMap<u32, Vec3>,
    /// The cooldown a carrying vehicle left with. Vanilla's copy arrives in
    /// the same tick (`Entity.restoreFrom`), so the source level ticking
    /// while the destination loads must not run it down.
    pub(super) carried_cooldown: HashMap<u32, i32>,
    /// Where a carrying vehicle stood then: its plan is taken from there
    /// (`vehicle_plan`), as vanilla's is, not from where its rider's own tick
    /// has moved it on to since.
    pub(super) carried_at: HashMap<u32, Vec3>,
    loaded: bool,
    dirty: bool,
    /// Per-chunk portal palette masks, kept across ticks (portal_sections).
    pub(super) sections: horizon_client::portal_sections::PortalSections,
    /// The attributes a respawn hands from the old player to the new one.
    pub(super) respawn_attributes: respawn_attributes::RespawnAttributes,
}

fn rotate_y(v: Vec3, degrees: f32) -> Vec3 {
    // Vec3.yRot((float)Math.toRadians(degrees))
    let angle = f64::from(degrees.to_radians());
    let (cos, sin) = (
        f64::from(horizon_core::geometry::trig::cos(angle)),
        f64::from(horizon_core::geometry::trig::sin(angle)),
    );
    Vec3::new(v.x * cos + v.z * sin, v.y, v.z * cos - v.x * sin)
}
/// Whether a projectile is an arrow, which goes through a portal like the
/// other entities.
pub(super) fn is_arrow(p: &horizon_client::projectiles::Projectile) -> bool {
    matches!(
        p.kind,
        horizon_client::projectiles::ProjectileKind::Arrow { .. }
    )
}
/// The projectiles besides arrows that a portal sends on: the thrown items
/// and fireballs a chunk saves. A pearl has its own rules (it keeps its owner's
/// dimension) and a wind charge is not saved, so both stay.
pub(super) fn is_portal_projectile(p: &horizon_client::projectiles::Projectile) -> bool {
    use horizon_client::projectiles::ProjectileKind;
    !is_arrow(p)
        && !matches!(
            p.kind,
            ProjectileKind::EnderPearl
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        )
        && horizon_client::entity_nbt::saved_with_chunk(p)
}
/// Width and height of a projectile that goes through a portal.
pub(super) fn projectile_box(p: &horizon_client::projectiles::Projectile) -> (f32, f32) {
    if is_arrow(p) {
        (
            horizon_client::arrows::WIDTH,
            horizon_client::arrows::HEIGHT,
        )
    } else {
        let size = p.kind.size() as f32;
        (size, size)
    }
}
/// `Entity.calculatePassengerTransition`: the riders under `tag`'s
/// `Passengers`, and theirs, keep their place on the vehicle: they move by
/// what it moved and turn by what it turned.
fn move_passengers(tag: &mut Tag, by: Vec3, turn: f32) {
    let Tag::Compound(c) = tag else {
        return;
    };
    let Some(Tag::List(riders)) = c.get_mut(&"Passengers".into()) else {
        return;
    };
    for rider in riders {
        if let Some(at) = horizon_client::entity_nbt::position(rider)
            && let Tag::Compound(r) = rider
        {
            r.insert(
                "Pos".into(),
                doubles([at.x + by.x, at.y + by.y, at.z + by.z]),
            );
            let [yaw, pitch] = horizon_client::entity_nbt::rotation(rider);
            if let Tag::Compound(r) = rider {
                r.insert(
                    "Rotation".into(),
                    Tag::List(vec![
                        Tag::Float((yaw + turn).to_bits()),
                        Tag::Float(pitch.to_bits()),
                    ]),
                );
            }
        }
        move_passengers(rider, by, turn);
    }
}
fn doubles(values: [f64; 3]) -> Tag {
    Tag::List(values.iter().map(|v| Tag::Double(v.to_bits())).collect())
}
/// `Entity.collidedWithShapeMovingFrom` for a dimension box. Kept with the
/// original mod-facing signature; the game's AABB path uses `_box` below.
#[allow(dead_code)] // mod-facing signature; the game path uses _box
fn touches_end_portal(from: Vec3, to: Vec3, width: f32, height: f32, pos: BlockPos) -> bool {
    let half = f64::from(width) / 2.0;
    let start = Aabb::new(
        Vec3::new(from.x - half, from.y, from.z - half),
        Vec3::new(from.x + half, from.y + f64::from(height), from.z + half),
    );
    touches_end_portal_box(from, to, start, pos)
}

/// Whether the box `start` (the entity's box at `from`) moving to `to` meets
/// the End portal at `pos`: `Entity.collidedWithShapeMovingFrom` against its
/// entity-inside shape, y 6/16..12/16 of the block.
fn touches_end_portal_box(from: Vec3, to: Vec3, start: Aabb, pos: BlockPos) -> bool {
    let (x, y, z) = (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
    let shape = Aabb::new(
        Vec3::new(x, y + 0.375, z),
        Vec3::new(x + 1.0, y + 0.75, z + 1.0),
    );
    start.collided_along_vector(to.subtract(from), &[shape])
}
/// `PositionMoveRotation.calculateAbsolute` for an End portal's transition
/// of an entity facing `[yaw, pitch]`: into the End it faces west and keeps
/// its pitch (`Relative.X_ROT`); out of it both turn by the world spawn's
/// angles (`Relative.ROTATION`). The pitch stays within ±90.
fn end_portal_rotation(into_end: bool, [yaw, pitch]: [f32; 2], spawn: [f32; 2]) -> [f32; 2] {
    let (yaw, pitch) = if into_end {
        // Direction.WEST.toYRot().
        (90.0, pitch)
    } else {
        (yaw + spawn[0], pitch + spawn[1])
    };
    [yaw, pitch.clamp(-90.0, 90.0)]
}
/// Apply a resolved destination to the entity tree without resetting its cooldown.
pub(super) fn set_arrival_pose(
    tag: &mut Tag,
    at: Vec3,
    motion: Vec3,
    [yaw, pitch]: [f32; 2],
    yaw_turn: f32,
) {
    if let Some(was) = horizon_client::entity_nbt::position(tag) {
        move_passengers(tag, at.subtract(was), yaw_turn);
    }
    if let Tag::Compound(c) = tag {
        c.insert("Pos".into(), doubles([at.x, at.y, at.z]));
        c.insert("Motion".into(), doubles([motion.x, motion.y, motion.z]));
        c.insert(
            "Rotation".into(),
            Tag::List(vec![Tag::Float(yaw.to_bits()), Tag::Float(pitch.to_bits())]),
        );
        // PortalCooldown keeps what setPortalCooldown gave it before
        // the teleport (`send_through_portal`), copied by restoreFrom.
    }
}

/// A waiting entity as `horizon_portal_arrivals.dat` keeps it.
fn arrival_tag(a: &Arrival) -> Tag {
    let p = &a.plan;
    Tag::compound([
        ("dimension", Tag::String(p.dim.id().into())),
        ("x", Tag::Int(p.approximate.x)),
        ("y", Tag::Int(p.approximate.y)),
        ("z", Tag::Int(p.approximate.z)),
        ("x_axis", Tag::Int(i32::from(p.entry_x_axis))),
        ("right", Tag::Double(p.offset.x.to_bits())),
        ("up", Tag::Double(p.offset.y.to_bits())),
        ("forward", Tag::Double(p.offset.z.to_bits())),
        ("width", Tag::Float(p.width.to_bits())),
        ("height", Tag::Float(p.height.to_bits())),
        ("end", Tag::Int(i32::from(p.end))),
        ("entity", a.tag.clone()),
    ])
}
/// The waiting entity `arrival_tag` wrote.
fn arrival_from_tag(a: &Tag) -> Option<Arrival> {
    let int = |k: &str| match a.get(k) {
        Some(Tag::Int(v)) => Some(*v),
        _ => None,
    };
    let float = |k: &str| match a.get(k) {
        Some(Tag::Float(v)) => Some(f32::from_bits(*v)),
        _ => None,
    };
    let double = |k: &str| match a.get(k) {
        Some(Tag::Double(v)) => Some(f64::from_bits(*v)),
        _ => None,
    };
    let dim = Dim::from_id(&horizon_client::save::text(a.get("dimension"))?);
    let (Some(x), Some(y), Some(z), Some(tag)) = (int("x"), int("y"), int("z"), a.get("entity"))
    else {
        return None;
    };
    Some(Arrival {
        tnt_used_portal: false,
        transferred: None,
        plan: Plan {
            dim,
            approximate: BlockPos::new(x, y, z),
            entry_x_axis: int("x_axis").unwrap_or(1) != 0,
            offset: Vec3::new(
                double("right").unwrap_or(0.5),
                double("up").unwrap_or(0.0),
                double("forward").unwrap_or(0.0),
            ),
            width: float("width").unwrap_or(0.25),
            height: float("height").unwrap_or(0.25),
            end: int("end").unwrap_or(0) != 0,
        },
        tag: tag.clone(),
    })
}

/// Keep an explicitly captured counter. Old archives without a positive counter
/// retain the historical fallback; their migration needs separate evidence.
fn preserve_arrival_cooldown(tag: &mut Tag) {
    if let Tag::Compound(fields) = tag
        && !matches!(fields.get(&"PortalCooldown".into()), Some(Tag::Int(delay)) if *delay > 0)
    {
        fields.insert("PortalCooldown".into(), Tag::Int(ENTITY_DELAY));
    }
}

impl Game<'_> {
    /// The world border's extent.
    pub(super) fn border_bounds(&self) -> BorderBounds {
        self.border_bounds_of(self.dim)
    }
    /// The extent of `dim`'s own border (`ServerLevel.getWorldBorder`), the level in place or a
    /// parked one.
    fn border_bounds_of(&self, dim: Dim) -> BorderBounds {
        let border = if dim == self.dim {
            &self.border
        } else {
            self.levels.parked_border(dim).unwrap_or(&self.border)
        };
        let [min_x, min_z, max_x, max_z] = border.bounds();
        BorderBounds {
            min_x,
            min_z,
            max_x,
            max_z,
        }
    }
    /// `ServerLevel.isAllowedToEnterPortal`.
    pub(super) fn may_enter(&self, target: Dim) -> bool {
        target != Dim::Nether
            || self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::ALLOW_ENTERING_NETHER_USING_PORTALS)
    }
    /// The plan of an entity of `width` by `height` at `at` going through the
    /// portal at `entry` (`getDimensionTransitionFromExit` and the scaled,
    /// clamped exit of `getPortalDestination`).
    pub(super) fn portal_plan(
        &self,
        entry: BlockPos,
        at: Vec3,
        width: f32,
        height: f32,
        reset_forward: bool,
    ) -> Plan {
        let target = if self.dim == Dim::Nether {
            Dim::Overworld
        } else {
            Dim::Nether
        };
        let state = self.world.block(entry.x, entry.y, entry.z);
        let b = self.res.behaviours();
        let (entry_x_axis, offset) = match b.portal_axis(state) {
            Some(x_axis) => {
                let area = largest_rectangle_around(
                    entry,
                    if x_axis { Axis::X } else { Axis::Z },
                    21,
                    Axis::Y,
                    21,
                    |p| self.world.block(p.x, p.y, p.z) == state,
                );
                let mut offset = relative_position(area, x_axis, at, width, height);
                if reset_forward {
                    offset.z = 0.0;
                }
                (x_axis, offset)
            }
            None => (true, Vec3::new(0.5, 0.0, 0.0)),
        };
        // DimensionType.getTeleportationScale: the coordinate scales of the two levels.
        let scale = horizon_world::dimension_type::DimensionFacts::teleportation_scale(
            self.dimension_type(),
            self.dimension_type_of(target),
        );
        Plan {
            dim: target,
            // getPortalDestination clamps to the border of the level it leads to.
            approximate: self
                .border_bounds_of(target)
                .clamp(at.x * scale, at.y, at.z * scale),
            entry_x_axis,
            offset,
            width,
            height,
            end: false,
        }
    }
    /// Whether entity `id` may take `plan`'s portal: the rule on entering its
    /// dimension, then the mods (`entity_travel_to_dimension`).
    fn may_travel_through(&mut self, id: u32, plan: &Plan) -> bool {
        self.may_enter(plan.dim)
            && self.entity_may_travel(
                id,
                plan.dim,
                super::server_travel::TravelCause::of_plan(plan),
            )
    }
    /// The plan of an item or mob going through the portal block at `entry`:
    /// an End portal's fixed destination, else the Nether portal's.
    fn entity_plan(
        &self,
        entry: BlockPos,
        at: Vec3,
        width: f32,
        height: f32,
        reset_forward: bool,
    ) -> Plan {
        if !self.is_end_portal(self.world.block(entry.x, entry.y, entry.z)) {
            return self.portal_plan(entry, at, width, height, reset_forward);
        }
        let spawn = self.level.world_spawn.unwrap_or_else(|| {
            self.spawn.map_or([0, 64, 0], |s| {
                [s.x.floor() as i32, s.y.floor() as i32, s.z.floor() as i32]
            })
        });
        Plan::end_portal(
            self.dim,
            BlockPos::new(spawn[0], spawn[1], spawn[2]),
            width,
            height,
        )
    }
    fn is_end_portal(&self, state: horizon_world::registry::BlockStateId) -> bool {
        &*self.res.facts().get(state).name == "minecraft:end_portal"
    }
    /// Where an item or mob through an End portal lands once `plan`'s level
    /// has loaded around it: the bottom centre of the End's spawn point,
    /// above the platform built below it (`EndPlatformFeature.createEndPlatform`);
    /// out of the End, the top of the shared spawn's column
    /// (`Entity.adjustSpawnLocation`).
    fn end_portal_exit(&mut self, plan: &Plan) -> Vec3 {
        let p = plan.approximate;
        let centre = Vec3::new(f64::from(p.x) + 0.5, f64::from(p.y), f64::from(p.z) + 0.5);
        if plan.dim == Dim::End {
            // In the End the arrival only builds the platform: no spawn search.
            if let Err(e) = self.end_arrival(centre, &mut None) {
                log::error!("End platform: {e}");
            }
            return centre;
        }
        let y = self.motion_top(p.x, p.z);
        Vec3::new(centre.x, f64::from(y), centre.z)
    }
    /// The nether portal blocks of the chunks within `radius` of `center`
    /// (the points of interest `PortalForcer` searches): loaded chunks as
    /// they are, the others as saved (each read once until it is saved again);
    /// chunks never generated have none.
    fn portal_blocks(&self, center: BlockPos, radius: i32) -> Vec<BlockPos> {
        let b = self.res.behaviours();
        let is_portal = |state| b.portal_axis(state).is_some();
        let chunk_radius = radius.div_euclid(16) + 1;
        let (cx, cz) = (center.x >> 4, center.z >> 4);
        let mut out = Vec::new();
        for z in cz - chunk_radius..=cz + chunk_radius {
            for x in cx - chunk_radius..=cx + chunk_radius {
                match self.world.chunks.get(&(x, z)) {
                    Some(c) => out.extend(c.blocks_where(is_portal)),
                    None => out.extend_from_slice(&self.save.saved_portal_blocks(
                        x,
                        z,
                        self.pack.assets.registry,
                        world_biomes!(self),
                        is_portal,
                    )),
                }
            }
        }
        out
    }
    /// `PortalForcer.findClosestPortalPosition` and the rectangle around it,
    /// for `plan` in the loaded dimension; builds nothing.
    pub(in crate::game) fn find_exit_portal(
        &mut self,
        plan: &Plan,
    ) -> Option<(BlockPos, FoundRectangle)> {
        let to_nether = plan.dim == Dim::Nether;
        let radius = if to_nether { 16 } else { 128 };
        let border = self.border_bounds();
        let portals = self.portal_blocks(plan.approximate, radius);
        let (min_section, max_section) = if to_nether { (0, 15) } else { (-4, 19) };
        let found = closest_portal(
            &portals,
            plan.approximate,
            to_nether,
            &border,
            min_section,
            max_section,
        )?;
        let state = self.world.block(found.x, found.y, found.z);
        let x_axis = self.res.behaviours().portal_axis(state).unwrap_or(true);
        let rectangle = largest_rectangle_around(
            found,
            if x_axis { Axis::X } else { Axis::Z },
            21,
            Axis::Y,
            21,
            |p| self.world.block(p.x, p.y, p.z) == state,
        );
        Some((found, rectangle))
    }
    /// The exit portal of `plan` in the loaded dimension: the closest one
    /// within reach, else one built by `PortalForcer.createPortal`.
    pub(super) fn nether_exit_portal(&mut self, plan: &Plan) -> Option<FoundRectangle> {
        let to_nether = plan.dim == Dim::Nether;
        let border = self.border_bounds();
        if let Some((found, rectangle)) = self.find_exit_portal(plan) {
            // NetherPortalBlock.getExitPortal: the ticket goes on the chunk
            // of the portal found.
            self.place_portal_ticket((found.x >> 4, found.z >> 4));
            return Some(rectangle);
        }
        let logical = if to_nether { 128 } else { 384 };
        let (approximate, x_axis) = (plan.approximate, plan.entry_x_axis);
        let created = self.with_level(|b, level, ticks| {
            b.create_portal(level, ticks, approximate, x_axis, &border, logical)
        });
        let _ = self.process_level_events();
        if let Some(exit) = created {
            let dim = if to_nether {
                "minecraft:the_nether"
            } else {
                "minecraft:overworld"
            };
            let corner = exit.min_corner;
            self.native_portal_create(dim, [corner.x, corner.y, corner.z]);
            // A portal built for the entity: the ticket goes on the chunk it
            // arrives in (`TeleportTransition.PLACE_PORTAL_TICKET`).
            let (at, _) = self.arrival(plan, exit);
            self.place_portal_ticket(Self::chunk_of(at));
        }
        created
    }
    /// `ChunkPos.containing`: the chunk a position is in.
    pub(super) fn chunk_of(at: Vec3) -> (i32, i32) {
        ((at.x.floor() as i32) >> 4, (at.z.floor() as i32) >> 4)
    }
    /// `Entity.placePortalTicket`: a `TicketType.PORTAL` ticket of radius 3
    /// on `chunk` in the level in place, which keeps the area running for 300
    /// ticks with no player in it (the entity that just came through ticks
    /// there, and goes back through the portal when its cooldown ends).
    /// Only a server holds levels with tickets of their own.
    pub(super) fn place_portal_ticket(&mut self, chunk: (i32, i32)) {
        if !self.options.execution_role.is_client() {
            self.tickets.add_portal(chunk);
        }
    }

    /// `NetherPortalBlock.createDimensionTransition`: the arrival position
    /// and the yaw turned by.
    pub(super) fn arrival(&self, plan: &Plan, exit: FoundRectangle) -> (Vec3, f32) {
        let b = self.res.behaviours();
        let corner = exit.min_corner;
        let exit_x_axis = b
            .portal_axis(self.world.block(corner.x, corner.y, corner.z))
            .unwrap_or(true);
        let (bottom, rotation) = exit_position(
            exit,
            exit_x_axis,
            plan.entry_x_axis,
            plan.offset,
            plan.width,
            plan.height,
        );
        let border = self.border_bounds();
        let at = collision_free_position(bottom, plan.width, plan.height, &border, |p| {
            b.collision_shape(self.world.block(p.x, p.y, p.z))
                .map(|s| s.moved(Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z))))
        });
        (at, rotation)
    }

    // ---- the player ----

    /// Whether the inventory key opens nothing: vanilla opens the screen,
    /// then `LocalPlayer.handlePortalTransitionEffect` closes it in the same
    /// tick while the player stands in a nether portal, so it is never drawn.
    /// The fading effect after leaving the portal closes nothing.
    pub(super) fn in_portal_transition(&self) -> bool {
        self.portals.client_was_inside
    }

    /// The same on a server's client: `Entity.canUsePortal` (alive, not a
    /// passenger), then only the client's `setAsInsidePortal`, which drives
    /// the confusion effect (`client_portal_effect`). On a server's client
    /// `portals.player` holds only the LocalPlayer's `portalCooldown`.
    pub(super) fn client_inside_nether_portal(&mut self) {
        if !self.player_can_use_portal() {
            return;
        }
        if self.portals.player.cooldown > 0 {
            self.portals.player.cooldown = PLAYER_DELAY;
        } else {
            self.portals.client_inside = true;
        }
    }
    /// Where to wait for the destination of `plan` to load, once its level
    /// is open: the portal its points of interest lead to, or the
    /// approximate exit.
    pub(super) fn arrival_centre(&self, plan: &Plan) -> Vec3 {
        let to_nether = plan.dim == Dim::Nether;
        let portals = self.portal_blocks(plan.approximate, if to_nether { 16 } else { 128 });
        let (min_section, max_section) = if to_nether { (0, 15) } else { (-4, 19) };
        let border = self.border_bounds();
        let found = closest_portal(
            &portals,
            plan.approximate,
            to_nether,
            &border,
            min_section,
            max_section,
        );
        let centre = found.unwrap_or(plan.approximate);
        Vec3::new(
            f64::from(centre.x) + 0.5,
            f64::from(centre.y),
            f64::from(centre.z) + 0.5,
        )
    }
    /// `LocalPlayer.handlePortalTransitionEffect`: whether the confusion
    /// effect grows this tick. Entering a portal closes the screen only when
    /// it does not allow it (`Screen.isAllowedInPortal`), and plays
    /// `block.portal.trigger` when the effect starts from nothing.
    pub(super) fn client_portal_effect(&mut self) -> bool {
        let inside = std::mem::take(&mut self.portals.client_inside);
        self.portals.client_was_inside = inside;
        if !inside {
            return false;
        }
        // No screen at all (`screen() == null`) is nothing to close.
        if self.screen != Screen::Playing && !self.screen.allowed_in_portal() {
            if matches!(self.screen, Screen::Creative | Screen::Inventory) {
                self.close_container();
                self.carried = None;
            }
            self.set_screen(Screen::Playing);
            self.capture(true);
        }
        if self.screen_fx.portal == 0.0 {
            let pitch = self.world.random.next_float() * 0.4 + 0.8;
            self.audio
                .play("minecraft:block.portal.trigger", None, 0.25, pitch);
        }
        true
    }

    // ---- other entities ----

    /// Positions before the entities move this tick, and `Entity.handlePortal`
    /// at the start of their tick: an item or mob that stayed in a portal
    /// goes through (transition time 0) and waits for its dimension.
    pub(super) fn handle_entity_portals(&mut self) {
        self.handle_portals(false, false);
    }
    /// `Entity.handlePortal` of the boats and minecarts, which `tick_boats`
    /// and `tick_carts` move: it runs before them, from the position they
    /// ended the last tick at, as the start of a vehicle's own tick does.
    pub(super) fn handle_vehicle_portals(&mut self) {
        self.handle_portals(true, false);
    }
    fn handle_portals(&mut self, vehicles: bool, after_move: bool) {
        if self.remote.is_some() {
            return;
        }
        self.load_portal_arrivals();
        let mut leaving: Vec<(u32, BlockPos)> = Vec::new();
        // Positions before the entities move, and (only for an entity in a
        // portal, or cooling down from one, which has a state here) whether it
        // can use one: asking every entity on every tick is not worth it. The
        // vehicles' pass opens the tick and the other kinds' pass follows, so
        // each walk takes only its own group.
        let mut before = std::mem::take(&mut self.portals.before);
        if vehicles {
            before.clear();
        }
        let mut ids: Vec<(u32, bool, i32)> = Vec::new();
        // Portal handling runs only in entity-ticking chunks; retain frozen
        // states while their entities are outside the simulation distance.
        let entity_ticking = self.entity_ticking_area();
        let frozen = self.mobs_below_off_level();
        let tracked = &self.portals.entities;
        let mut present = std::collections::HashSet::new();
        for_each_entity!(entity_lists_of!(self), entity => {
            let can_sweep = entity.sweeps_portals();
            let has_portal_state = tracked.contains_key(&entity.id());
            if has_portal_state {
                present.insert(entity.id());
            }
            // Entity.processPortalCooldown still runs for entities this
            // portal pass cannot send, such as an ender pearl.
            if entity.is_vehicle() == vehicles
                && entity.portal_after_move() == after_move
                && (can_sweep || has_portal_state)
            {
                let at = entity.position();
                if frozen.contains(&entity.id()) || !entity_ticking(at.x.floor() as i32, at.z.floor() as i32) {
                    continue;
                }
                if can_sweep && entity.portal_from().is_none() {
                    before.insert(entity.id(), entity.position());
                }
                if has_portal_state {
                    ids.push((
                        entity.id(),
                        can_sweep && entity.can_use_portal(self),
                        self.dimension_changing_delay(entity.id()),
                    ));
                }
            }
        });
        self.portals.before = before;
        self.portals.entities.retain(|id, _| present.contains(id));
        // SulfurCube.tickFuse runs before Entity.handlePortal. A cube removed
        // there must neither decrement its cooldown nor travel before its blast.
        let removed_before_portal: std::collections::HashSet<u32> = self
            .mobs
            .iter()
            .filter(|_| !vehicles)
            .filter(|m| horizon_client::mobs::sulfur_cube::removes_before_portal_tick(m))
            .map(|m| m.id)
            .collect();
        for (id, allowed, delay) in ids {
            if removed_before_portal.contains(&id) {
                continue;
            }
            let Some(state) = self.portals.entities.get_mut(&id) else {
                continue;
            };
            if let Some(entry) = state.handle(allowed, 0, delay) {
                leaving.push((id, entry));
            }
            if state.cooldown == 0 && state.process.is_none() {
                self.portals.entities.remove(&id);
            }
        }
        for (id, entry) in leaving {
            // `Entity.teleportCrossDimension` carries the passengers: a
            // vehicle server players ride takes them along.
            if self.lan.as_ref().is_some_and(|host| {
                host.hub.guests.iter().any(|guest| {
                    guest
                        .vehicle
                        .is_some_and(|mount| self.vehicle_tree_contains_mount(id, mount))
                })
            }) {
                let cooldown = self.portals.entities.get(&id).map_or(0, |s| s.cooldown);
                self.portals.carried_cooldown.insert(id, cooldown);
                if let Some(c) = self.carts.iter().find(|c| c.id == id) {
                    self.portals.carried_motion.insert(id, c.velocity);
                    self.portals.carried_at.insert(id, c.position);
                } else if let Some(b) = self.boats.iter().find(|b| b.id == id) {
                    self.portals.carried_at.insert(id, b.position);
                } else if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
                    self.portals.carried_at.insert(id, m.position);
                }
                self.portals.carrying.push((self.dim, id, entry));
            } else {
                self.send_through_portal(id, entry);
            }
        }
    }
    /// `Entity.load`: the saved `PortalCooldown` (an entity that just came
    /// through stands in its exit portal for 300 ticks without going back).
    pub(super) fn restore_portal_cooldown(&mut self, id: u32, tag: &Tag) {
        if let Some(cooldown) = tag
            .get("PortalCooldown")
            .and_then(horizon_client::nbt_path::int_value)
            && cooldown != 0
        {
            self.portals.entities.insert(
                id,
                PortalState {
                    cooldown,
                    process: None,
                },
            );
        }
    }
    /// `Entity.saveWithoutId` writes the live `PortalCooldown`: the countdown
    /// `Entity.processPortalCooldown` ticks down, not the value the entity was
    /// loaded with. Every entity that can stand in a portal takes it from the
    /// portal state, as the projectile writer does. An entity the portal
    /// state does not know is not on a cooldown, and writes 0.
    pub(super) fn save_portal_cooldown(&self, id: u32, tag: &mut Tag) {
        horizon_client::nether_portal::put_portal_cooldown(
            tag,
            horizon_client::nether_portal::portal_cooldown(&self.portals.entities, id),
        );
    }
    /// `Entity.saveWithoutId`: the modeled non-arrow projectile codecs
    /// share the live cooldown between command reads and chunk writes.
    pub(super) fn save_projectile_portal_cooldown(
        &self,
        projectile: &horizon_client::projectiles::Projectile,
        tag: &mut Tag,
    ) {
        if projectile.arrow.is_none()
            && let Tag::Compound(fields) = tag
        {
            let cooldown = self
                .portals
                .entities
                .get(&projectile.id)
                .map_or(0, |portal| portal.cooldown);
            fields.insert("PortalCooldown".into(), Tag::Int(cooldown));
        }
    }
    /// `Entity.getDimensionChangingDelay`, by class: boats and minecarts
    /// (`VehicleEntity`) always 10, projectiles (`Projectile`) 2; any other
    /// entity takes its first passenger's delay when that is a server player
    /// (`Player.getDimensionChangingDelay`), else 300.
    pub(super) fn dimension_changing_delay(&self, id: u32) -> i32 {
        if self.boats.iter().any(|b| b.id == id)
            || self.carts.iter().any(|c| c.id == id)
            || self.lan_acting_entity == Some(id)
            || self
                .lan
                .iter()
                .flat_map(|h| &h.hub.guests)
                .any(|g| g.entity_id as u32 == id)
        {
            VEHICLE_DELAY
        } else if self.creatures.projectiles.iter().any(|p| p.id == id)
            || self.tridents.iter().any(|t| t.id == id)
            || self.rockets.iter().any(|r| r.id == id)
        {
            PROJECTILE_DELAY
        } else if self
            .seated_passengers(id)
            .first()
            .is_some_and(|&(_, _, player)| player)
        {
            PLAYER_DELAY
        } else {
            ENTITY_DELAY
        }
    }
    /// `Entity.setAsInsidePortal` on an entity that is still on its portal
    /// cooldown, whatever the portal: the cooldown restarts and it stays.
    pub(super) fn restart_portal_cooldown(&mut self, id: u32) -> bool {
        if self
            .portals
            .entities
            .get(&id)
            .is_some_and(|state| state.cooldown > 0)
        {
            self.set_portal_cooldown(id);
            true
        } else {
            false
        }
    }
    /// `Entity.setPortalCooldown`: assign the passenger-dependent delay,
    /// leaving the existing portal process intact.
    pub(super) fn set_portal_cooldown(&mut self, id: u32) {
        let delay = self.dimension_changing_delay(id);
        self.portals.entities.entry(id).or_default().cooldown = delay;
    }
    /// `LivingEntity.canUsePortal` over a player's body and its transport
    /// (arismoko/horizon#943): `Entity.canUsePortal(false)` (alive, nobody's passenger) and
    /// not asleep (`isSleeping`). The one rule the client's portal entry,
    /// the player in place and the guest arm of the `LivingRef` read answer
    /// — a parity fix: the client's own entry used to skip the sleep half
    /// (vanilla `LivingEntity.canUsePortal` checks it).
    pub(super) fn can_use_portal_of(
        &self,
        p: &horizon_client::motion::PlayerMotion,
        riding: bool,
        sleeping: bool,
    ) -> bool {
        !p.health.dead_or_dying() && !riding && !sleeping
    }
    /// [`can_use_portal_of`](Self::can_use_portal_of) for the player of
    /// this game.
    pub(super) fn player_can_use_portal(&self) -> bool {
        self.player
            .as_ref()
            .is_some_and(|p| self.can_use_portal_of(p, self.riding.is_some(), p.sleeping))
    }
    /// The rest of `NetherPortalBlock.randomTick` after a won roll: with a
    /// player close enough for spawning, a zombified piglin spawns on the block
    /// under the portal column when it is a valid spawn, plays its ambient
    /// sound (`EntityType.spawn`) and waits out a portal cooldown.
    pub(super) fn portal_spawn(&mut self, pos: [i32; 3]) {
        // ChunkMap.anyPlayerCloseEnoughForSpawning: a non-spectator within 128
        // blocks of the chunk centre.
        let centre = [
            f64::from((pos[0] >> 4) * 16 + 8),
            f64::from((pos[2] >> 4) * 16 + 8),
        ];
        let close = self.level_players().iter().any(|p| {
            let (dx, dz) = (p.position.x - centre[0], p.position.z - centre[1]);
            !p.spectator && dx * dx + dz * dz < 16384.0
        });
        if !close {
            return;
        }
        let facts = self.res.facts();
        let mut y = pos[1];
        while &*facts.get(self.world.block(pos[0], y, pos[2])).name == "minecraft:nether_portal" {
            y -= 1;
        }
        // `BlockState.isValidSpawn(level, pos, ZOMBIFIED_PIGLIN)`: the same
        // per-state table the natural spawns read, so glowstone's 15 light and
        // bedrock's `isValidSpawn(never)` both refuse it.
        let w = self.mob_world_base(&[]);
        if !spawn_rules::is_valid_spawn(Species::ZombifiedPiglin, &w, pos[0], y, pos[2]) {
            return;
        }
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(y + 1),
            f64::from(pos[2]) + 0.5,
        ];
        if let Some(id) = self.spawn_mob(
            "minecraft:zombified_piglin",
            at,
            false,
            super::creatures::SpawnSite::Finalizing(
                horizon_client::mobs::spawn_rules::Reason::Structure,
            ),
        ) {
            // `EntityType.spawn` calls `Mob.playAmbientSound` once the mob is in
            // the level, before the cooldown below.
            let _ = self.mob_ambient(id);
            // setPortalCooldown: the new piglin does not travel at once.
            self.set_portal_cooldown(id);
        }
    }
    /// `canUsePortal(false)` of a mob: what the mob itself decides
    /// (`mobs::mob_can_use_portal`), riding nothing, not asleep, and not
    /// carrying the players this side does.
    pub(super) fn mob_portal_eligible(&self, m: &horizon_client::mobs::Mob) -> bool {
        horizon_client::mobs::mob_can_use_portal(m)
            && m.extra.mount.is_none()
            && self.riding != Some(m.id)
            && (!self.guest_rides(m.id) || self.carries_riders())
    }
    /// `Entity.canUsePortal(false)` for a primed TNT: it carries no mob.
    pub(super) fn tnt_can_use_portal(&self, id: u32) -> bool {
        // Id 0: not tracked yet, nothing to key its portal state by.
        id != 0 && !self.mobs.iter().any(|m| m.extra.mount == Some(id))
    }
    /// `canUsePortal(false)` of a boat or minecart: it is nobody's passenger;
    /// the mobs riding it, and on a server the players, cross with it.
    pub(super) fn vehicle_can_use_portal(&self, id: u32) -> bool {
        // The window's player is moved by its own trip, not by the vehicle's.
        if self.riding == Some(id) {
            return false;
        }
        // Mob riders always go along; server players only on a server.
        !self.ridden_by_guest(id) || self.carries_riders()
    }
    /// Whether a vehicle can take the server players riding it through a
    /// portal: only a server moves players between its levels. Elsewhere a
    /// ridden vehicle stays, rather than leave its rider behind.
    fn carries_riders(&self) -> bool {
        self.options.dedicated.is_some()
    }
    /// The vehicles that went through a portal in `dim` with server players
    /// on them.
    pub(super) fn take_carrying(&mut self, dim: Dim) -> Vec<(u32, BlockPos)> {
        let (here, other) = std::mem::take(&mut self.portals.carrying)
            .into_iter()
            .partition(|(d, ..)| *d == dim);
        self.portals.carrying = other;
        here.into_iter().map(|(_, id, entry)| (id, entry)).collect()
    }
    /// The plan of vehicle `id` through the portal at `entry`: from its own
    /// position and size (`getPortalDestination(level, vehicle)`). A boat or
    /// minecart, like a mob, lands in the middle of the exit portal's depth
    /// (`AbstractBoat` and `AbstractMinecart.getRelativePortalPosition`).
    pub(super) fn vehicle_plan(&self, id: u32, entry: BlockPos) -> Option<Plan> {
        let at = |own: Vec3| self.portals.carried_at.get(&id).copied().unwrap_or(own);
        if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
            return Some(self.entity_plan(entry, at(m.position), m.width(), m.height(), true));
        }
        if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            let (width, height) = (super::boats::WIDTH as f32, super::boats::HEIGHT as f32);
            return Some(self.entity_plan(entry, at(b.position), width, height, true));
        }
        let c = self.carts.iter().find(|c| c.id == id)?;
        let (width, height) = (
            super::minecarts::WIDTH as f32,
            super::minecarts::HEIGHT as f32,
        );
        Some(self.entity_plan(entry, at(c.position), width, height, true))
    }
    /// `NetherPortalBlock.entityInside` and `EndPortalBlock.entityInside`
    /// for the items, mobs, empty boats and minecarts, and arrows that moved
    /// this tick: the portal blocks their box swept through (an End portal's
    /// transition time is 0 too).
    pub(super) fn find_entity_portals(&mut self) {
        self.find_portals(None);
        // AbstractArrow, ThrowableProjectile and AbstractHurtingProjectile apply
        // block effects before calling Entity.tick, so their refreshed cooldown
        // is decremented (or their new portal handled) in this same tick.
        self.handle_portals(false, true);
    }
    /// The same for the vehicles server players ride, once `tick_guests` has
    /// moved them: a ridden vehicle moves with its rider, after the other
    /// entities were swept, so without this pass its portal is found a tick
    /// late, from where it has already moved on to.
    pub(super) fn find_ridden_vehicle_portals(&mut self, ridden: &[u32]) {
        if !ridden.is_empty() {
            self.find_portals(Some(ridden));
        }
    }
    fn find_portals(&mut self, only: Option<&[u32]>) {
        if self.remote.is_some() {
            return;
        }
        let ridden = if only.is_none() {
            self.guest_vehicles()
        } else {
            Vec::new()
        };
        // A trident or rocket in flight gets its id here, before the walk.
        if only.is_none() {
            for index in 0..self.tridents.len() {
                let t = &self.tridents[index];
                if !t.in_ground || t.previous != t.position {
                    super::entity_residency::assign_resident_id!(
                        self.tridents,
                        index,
                        t,
                        &mut self.next_entity,
                        None
                    );
                }
            }
            for index in 0..self.rockets.len() {
                let r = &self.rockets[index];
                if r.attached_to.is_none() && !r.attached {
                    super::entity_residency::assign_resident_id!(
                        self.rockets,
                        index,
                        r,
                        &mut self.next_entity,
                        None
                    );
                }
            }
        }
        let mut inside: Vec<(u32, BlockPos, i32)> = Vec::new();
        let mut portal_sections = std::mem::take(&mut self.portals.sections);
        // No loaded section has a portal state in its palette (nearly always): no
        // sweep below can meet a portal block, so the walk would find nothing.
        if !self.level_may_hold_portal(&mut portal_sections) {
            self.portals.sections = portal_sections;
            return;
        }
        // Only the entities that tick (in entity-ticking chunks) move, so only
        // they can have swept into a portal.
        let entity_ticking = self.entity_ticking_area();
        let frozen = self.mobs_below_off_level();
        // The first portal block each entity's box swept through since the
        // start of its tick (a stuck arrow that did not move enters none).
        for_each_entity!(entity_lists_of!(self), entity => {
            if !entity.sweeps_portals() {
                continue;
            }
            let (id, to) = (entity.id(), entity.position());
            if frozen.contains(&id) || !entity_ticking(to.x.floor() as i32, to.z.floor() as i32) {
                continue;
            }
            if !entity.portal_sweeps() || !entity.can_use_portal(self) {
                continue;
            }
            let swept_later = match only {
                Some(ids) => !ids.contains(&id),
                None => ridden.contains(&id),
            };
            if swept_later {
                continue;
            }
            let from = entity
                .portal_from()
                .unwrap_or_else(|| self.portals.before.get(&id).copied().unwrap_or(to));
            let Some(bounds) = entity.portal_bounds(to) else {
                continue;
            };
            if let Some(pos) = self.swept_portal_box(from, to, bounds, &mut portal_sections) {
                inside.push((id, pos, self.dimension_changing_delay(id)));
            }
        });
        self.portals.sections = portal_sections;
        for (id, pos, delay) in inside {
            self.portals
                .entities
                .entry(id)
                .or_default()
                .set_inside(pos, delay);
        }
    }
    /// Original mod-facing signature: a width-by-height box. The game's
    /// AABB path calls `swept_portal_box`; this wrapper keeps mods working.
    #[allow(dead_code)] // mod-facing signature; the game path uses _box
    fn swept_portal(
        &self,
        from: Vec3,
        to: Vec3,
        width: f32,
        height: f32,
        portal_sections: &mut horizon_client::portal_sections::PortalSections,
    ) -> Option<BlockPos> {
        let half = f64::from(width) / 2.0;
        let bounds = Aabb::new(
            Vec3::new(to.x - half, to.y, to.z - half),
            Vec3::new(to.x + half, to.y + f64::from(height), to.z + half),
        );
        self.swept_portal_box(from, to, bounds, portal_sections)
    }

    /// The first nether or End portal block an entity box swept through
    /// (`Entity.checkInsideBlocks`, whose first portal sets the entry).
    fn swept_portal_box(
        &self,
        from: Vec3,
        to: Vec3,
        bounds: Aabb,
        portal_sections: &mut horizon_client::portal_sections::PortalSections,
    ) -> Option<BlockPos> {
        let bounds = bounds.deflate(Vec3::new(
            f64::from(1.0e-5_f32),
            f64::from(1.0e-5_f32),
            f64::from(1.0e-5_f32),
        ));
        // Every block the sweep visits lies in the box covering both ends.
        let start = bounds.move_by(from.subtract(to));
        let reach = bounds.union_bounds(start).inflate(Vec3::new(1.0, 1.0, 1.0));
        if !self.may_hold_portal(reach, portal_sections) {
            return None;
        }
        let mut found = None;
        let _ = horizon_core::geometry::sweep_intersected_blocks(
            from,
            to,
            bounds,
            1_000_000,
            |p, _| {
                let state = self.world.block(p.x, p.y, p.z);
                if self.res.behaviours().portal_axis(state).is_some()
                    || (self.is_end_portal(state) && touches_end_portal_box(from, to, start, p))
                {
                    found = Some(p);
                    return false;
                }
                true
            },
        );
        found
    }
    /// Whether a section `reach` touches has a nether or End portal state in
    /// its palette: without one, no block there is a portal. Palette answers are
    /// kept per chunk across ticks (`portal_sections`); a box over many
    /// sections is not checked.
    fn may_hold_portal(
        &self,
        reach: horizon_core::geometry::Aabb,
        sections: &mut horizon_client::portal_sections::PortalSections,
    ) -> bool {
        use horizon_client::section_mask::SectionMask;
        let (min, max) = (reach.min_position(), reach.max_position());
        if !min.is_finite() || !max.is_finite() {
            return true;
        }
        let (min, max) = (BlockPos::containing(min), BlockPos::containing(max));
        let top = self.world.layout().sections() as i32 - 1;
        let (x0, x1) = (min.x >> 4, max.x >> 4);
        let (z0, z1) = (min.z >> 4, max.z >> 4);
        let (y0, y1) = (
            ((min.y - self.world.bottom_y()) >> 4).clamp(0, top),
            ((max.y - self.world.bottom_y()) >> 4).clamp(0, top),
        );
        let count = (i64::from(x1) - i64::from(x0) + 1)
            * (i64::from(y1) - i64::from(y0) + 1)
            * (i64::from(z1) - i64::from(z0) + 1);
        if count > 64 {
            return true;
        }
        let y_mask = SectionMask::below(y1 as usize + 1).from(y0 as usize);
        let chunks = &self.world.chunks;
        (x0..=x1).any(|cx| {
            (z0..=z1).any(|cz| {
                chunks.get(&(cx, cz)).is_some_and(|chunk| {
                    let portals = sections.get((cx, cz), chunk, |v| self.is_portal_state(v));
                    !(portals & y_mask).is_empty()
                })
            })
        })
    }
    /// [`Self::may_hold_portal`] for the whole level: whether any loaded section has
    /// a nether or End portal state in its palette.
    fn level_may_hold_portal(
        &self,
        sections: &mut horizon_client::portal_sections::PortalSections,
    ) -> bool {
        sections.any(&self.world.chunks, |v| self.is_portal_state(v))
    }
    /// A nether or End portal state, by its id in a palette.
    fn is_portal_state(&self, v: u32) -> bool {
        let state = horizon_world::registry::BlockStateId(v);
        self.res.behaviours().portal_axis(state).is_some() || self.is_end_portal(state)
    }
    /// `Entity.teleportCrossDimension` away from the loaded dimension: the
    /// entity is saved and removed, and waits for its dimension to load.
    fn remove_portal_vehicle_tree(&mut self, root: u32) {
        let tree = self.passenger_tree(root);
        self.mobs.retain(|m| !tree.contains(&m.id));
        self.persist.off_level.retain(|id, _| !tree.contains(id));
        self.boats.retain(|b| !tree.contains(&b.id));
        self.carts.retain(|c| !tree.contains(&c.id));
        self.take_passenger_entities(&tree, true);
    }
    /// Where an entity that changes level alone stands, and the width and
    /// height of its box: an item, a primed TNT, a trident, a firework, an
    /// experience orb, or an arrow or other projectile a chunk saves
    /// (`is_portal_projectile`). None for every other entity: a mob, a boat or
    /// a minecart crosses as a vehicle tree with its riders.
    pub(super) fn lone_traveller_box(&self, id: u32) -> Option<(Vec3, f32, f32)> {
        if let Some(e) = self.dropped.iter().find(|e| e.id == id) {
            return Some((e.position, 0.25, 0.25));
        }
        if let Some(t) = self.tnts.iter().find(|t| t.id == id) {
            return Some((t.position, 0.98, 0.98));
        }
        if let Some(t) = self.tridents.iter().find(|t| t.id == id) {
            return Some((t.position, 0.5, 0.5));
        }
        if let Some(r) = self.rockets.iter().find(|r| r.id == id) {
            return Some((r.position, 0.25, 0.25));
        }
        if let Some(o) = self.orbs.iter().find(|o| o.id == id) {
            return Some((o.position, 0.5, 0.5));
        }
        let entity = self.entity(id)?;
        let target = self.native_entity_target(entity)?;
        if matches!(
            target,
            super::commands::Target::Stand(_)
                | super::commands::Target::Frame(_)
                | super::commands::Target::Painting(_)
                | super::commands::Target::Falling(_)
                | super::commands::Target::Cushion(_)
                | super::commands::Target::Other(_)
                | super::commands::Target::Fangs(_)
                | super::commands::Target::Crystal(_)
                | super::commands::Target::Dragon(_)
                | super::commands::Target::Spawner(_)
                | super::commands::Target::Cloud(_)
        ) {
            let at = self.target_position(target)?;
            let [width, height] = self.entity_dimensions(entity)?;
            return Some((Vec3::new(at[0], at[1], at[2]), width as f32, height as f32));
        }
        let p = self
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == id && (is_arrow(p) || is_portal_projectile(p)))?;
        let (width, height) = projectile_box(p);
        Some((p.position, width, height))
    }
    /// `Entity.teleportCrossDimension` for an entity that changes level alone
    /// (`lone_traveller_box`): its saved compound, with its attachments, and
    /// the live state its copy takes over (`restoreFrom`), as it leaves the
    /// level in place (`removeAfterChangingDimensions`). None, with the entity
    /// left where it is, for any other entity or a projectile with no saved
    /// form.
    pub(super) fn take_lone_traveller(
        &mut self,
        id: u32,
    ) -> Option<(Tag, super::entity_initialization::TransferredEntityState)> {
        use super::commands::Target;
        let entity = self.entity(id)?;
        let target = self.native_entity_target(entity)?;
        if matches!(
            target,
            Target::Stand(_)
                | Target::Frame(_)
                | Target::Painting(_)
                | Target::Falling(_)
                | Target::Cushion(_)
                | Target::Other(_)
                | Target::Fangs(_)
                | Target::Crystal(_)
                | Target::Dragon(_)
                | Target::Spawner(_)
                | Target::Cloud(_)
        ) {
            let mut tag = self.command_target_nbt(target)?;
            super::persist::write_attachments(&mut tag, id, &self.entity_attachments);
            let state = self.take_transferred_entity_state(id);
            // Remove only the resident body: no death, drops, or discard effects.
            match target {
                Target::Stand(_) => self.armor_stands.retain(|e| e.id != id),
                Target::Frame(_) => self.item_frames.retain(|e| e.id != id),
                Target::Painting(_) => self.paintings.retain(|e| e.id != id),
                Target::Falling(_) => self.falling.retain(|e| e.id != id),
                Target::Cushion(_) => self.cushions.retain(|e| e.id != id),
                Target::Other(_) => self.others.retain(|e| e.id != id),
                Target::Fangs(_) => self.fangs.list.retain(|e| e.id != id),
                Target::Crystal(_) => self.crystals.retain(|e| e.id != id),
                Target::Dragon(_) => self.discard_resident_dragon(),
                Target::Spawner(_) => self.ominous_spawners.retain(|e| e.id != id),
                Target::Cloud(_) => self.breath_clouds.retain(|e| e.id != id),
                _ => unreachable!("lone traveller kind selected above"),
            }
            self.forget_entity(id);
            return Some((tag, state));
        }
        enum List {
            Item,
            Tnt,
            Trident,
            Rocket,
            Orb,
            Projectile,
        }
        let (mut tag, list, i) = if let Some(i) = self.dropped.iter().position(|e| e.id == id) {
            let uuid = self.uuid_for(id);
            let tag = horizon_client::entity_nbt::item_entity_to_nbt_with_items(
                &self.dropped[i],
                uuid,
                self.persist_raw(id),
                self.items(),
            );
            (tag, List::Item, i)
        } else if let Some(i) = self.tnts.iter().position(|t| t.id == id) {
            (self.tnt_nbt(i), List::Tnt, i)
        } else if let Some(i) = self.tridents.iter().position(|t| t.id == id) {
            let uuid = self.uuid_for(id);
            let tag =
                self.tridents[i].to_nbt(uuid, self.pack.assets.registry, self.persist_raw(id));
            (tag, List::Trident, i)
        } else if let Some(i) = self.rockets.iter().position(|r| r.id == id) {
            let uuid = self.uuid_for(id);
            let tag = self.rockets[i].to_nbt(uuid, self.level.player_uuid, self.persist_raw(id));
            (tag, List::Rocket, i)
        } else if let Some(i) = self.orbs.iter().position(|o| o.id == id) {
            let uuid = self.uuid_for(id);
            let tag = horizon_client::entity_nbt::orb_to_nbt(&self.orbs[i], uuid);
            (tag, List::Orb, i)
        } else {
            let i = self
                .creatures
                .projectiles
                .iter()
                .position(|p| p.id == id && (is_arrow(p) || is_portal_projectile(p)))?;
            let uuid = self.uuid_for(id);
            let p = &self.creatures.projectiles[i];
            let in_block = p
                .arrow
                .as_ref()
                .and_then(|a| a.last_state)
                .map(|s| self.state_to_nbt(s));
            let raw = self.persist_raw(id);
            let tag =
                horizon_client::entity_nbt::arrow_to_nbt(p, uuid, p.owner_uuid, in_block, raw)
                    .or_else(|| {
                        horizon_client::entity_nbt::fireball_to_nbt(p, uuid, p.owner_uuid, raw)
                    })
                    .or_else(|| {
                        horizon_client::entity_nbt::throwable_to_nbt(p, uuid, p.owner_uuid, raw)
                    })?;
            (tag, List::Projectile, i)
        };
        super::persist::write_attachments(&mut tag, id, &self.entity_attachments);
        let state = self.take_transferred_entity_state(id);
        match list {
            List::Item => {
                self.remove_dropped(i);
                self.detach_entity(id);
            }
            List::Tnt => {
                self.tnts.remove(i);
                self.forget_entity(id);
            }
            List::Trident => {
                self.tridents.remove(i);
                self.forget_entity(id);
            }
            List::Rocket => {
                self.rockets.remove(i);
                self.forget_entity(id);
            }
            List::Orb => {
                self.orbs.remove(i);
                self.forget_entity(id);
            }
            List::Projectile => {
                self.creatures.projectiles.remove(i);
                self.forget_entity(id);
            }
        }
        Some((tag, state))
    }

    fn send_through_portal(&mut self, id: u32, entry: BlockPos) {
        let arrivals = self.portals.arrivals.len();
        // `Entity.handlePortal`: setPortalCooldown before the teleport, which
        // restoreFrom copies to the arriving entity.
        let cooldown = self.dimension_changing_delay(id);
        if let Some((at, width, height)) = self.lone_traveller_box(id) {
            let living = self.armor_stands.iter().any(|stand| stand.id == id);
            let plan = self.entity_plan(entry, at, width, height, living);
            if !self.may_travel_through(id, &plan) {
                return;
            }
            let tnt_used_portal = self.tnts.iter().any(|t| t.id == id);
            let Some((tag, state)) = self.take_lone_traveller(id) else {
                return;
            };
            self.portals.arrivals.push(Arrival {
                tnt_used_portal,
                plan,
                tag,
                transferred: Some(TransferredArrivalState::Entity(state)),
            });
            self.portals.dirty = true;
        } else if let Some(i) = self.mobs.iter().position(|m| m.id == id) {
            let m = &self.mobs[i];
            let plan = self.entity_plan(entry, m.position, m.width(), m.height(), true);
            if !self.may_travel_through(id, &plan) {
                return;
            }
            let Some((mut tag, state)) = self.take_vehicle_tree_transfer(id) else {
                return;
            };
            // Entity.removeAfterChangingDimensions drops the leash.
            if let Tag::Compound(c) = &mut tag {
                c.remove(&"leash".into());
            }
            self.remove_portal_vehicle_tree(id);
            self.portals.arrivals.push(Arrival {
                tnt_used_portal: false,
                plan,
                tag,
                transferred: Some(TransferredArrivalState::VehicleTree(state)),
            });
            self.portals.dirty = true;
        } else if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            let b = &self.boats[i];
            let (width, height) = (super::boats::WIDTH as f32, super::boats::HEIGHT as f32);
            let plan = self.entity_plan(entry, b.position, width, height, true);
            if !self.may_travel_through(id, &plan) {
                return;
            }
            self.release_vehicle_container(id);
            let Some((tag, state)) = self.take_vehicle_tree_transfer(id) else {
                return;
            };
            self.remove_portal_vehicle_tree(id);
            self.portals.arrivals.push(Arrival {
                tnt_used_portal: false,
                plan,
                tag,
                transferred: Some(TransferredArrivalState::VehicleTree(state)),
            });
            self.portals.dirty = true;
        } else if let Some(i) = self.carts.iter().position(|c| c.id == id) {
            let c = &self.carts[i];
            let (width, height) = (
                super::minecarts::WIDTH as f32,
                super::minecarts::HEIGHT as f32,
            );
            let plan = self.entity_plan(entry, c.position, width, height, true);
            if !self.may_travel_through(id, &plan) {
                return;
            }
            self.release_vehicle_container(id);
            let Some((mut tag, state)) = self.take_vehicle_tree_transfer(id) else {
                return;
            };
            // `PositionMoveRotation.of(this)` takes the cart's `getKnownMovement`.
            let m = super::vehicle_teleport::cart_known_movement(
                horizon_client::entity_nbt::motion(&tag),
            );
            if let Tag::Compound(c) = &mut tag {
                c.insert("Motion".into(), doubles([m.x, m.y, m.z]));
            }
            self.remove_portal_vehicle_tree(id);
            self.portals.arrivals.push(Arrival {
                tnt_used_portal: false,
                plan,
                tag,
                transferred: Some(TransferredArrivalState::VehicleTree(state)),
            });
            self.portals.dirty = true;
        }
        if self.portals.arrivals.len() > arrivals {
            if let Some(Arrival {
                tag: Tag::Compound(c),
                ..
            }) = self.portals.arrivals.last_mut()
            {
                c.insert("PortalCooldown".into(), Tag::Int(cooldown));
            }
            self.native_entity_portal(self.dim.id(), id, [entry.x, entry.y, entry.z]);
        }
    }
    /// The chunks of the arrivals still waiting to land in `dim`, for the
    /// streaming `extra` areas that light them: vanilla's portal ticket
    /// (`Entity.teleport` adds `TicketType.PORTAL` at the destination) keeps
    /// the area loaded without a player.
    pub(super) fn arrival_chunks(&self, dim: Dim) -> Vec<(i32, i32)> {
        self.portals
            .arrivals
            .iter()
            .filter(|a| a.plan.dim == dim)
            .map(|a| (a.plan.approximate.x >> 4, a.plan.approximate.z >> 4))
            .collect()
    }
    /// `ServerLevel.addDuringTeleport`: the entities that arrive in the level in
    /// place join it now, before its entities tick, so a vehicle that crossed
    /// from an earlier level this server tick moves in its new level the same
    /// tick, and one that crossed from a later level stands there until the
    /// next. Only the portal arrivals join; the chunks' entities wait.
    pub(super) fn land_portal_arrivals(&mut self) {
        if self.remote.is_some() || self.portals.arrivals.is_empty() {
            return;
        }
        let _save_lease = horizon_client::join_seq::visibility::PassLease::enter();
        let waiting = std::mem::take(&mut self.world.loaded_entities);
        horizon_client::spans::time("resolve_arrivals", || self.resolve_arrivals());
        horizon_client::spans::time("spawn_arrivals", || self.spawn_loaded_entities());
        self.world.loaded_entities.extend(waiting);
    }
    /// Entities waiting for the loaded dimension whose exit area is loaded:
    /// their exit portal (found or built) or End destination, position,
    /// turned yaw and motion, and portal cooldown; they join the world with the chunk entities.
    pub(super) fn resolve_arrivals(&mut self) {
        if self.remote.is_some() {
            return;
        }
        let mut i = 0;
        while i < self.portals.arrivals.len() {
            let plan = self.portals.arrivals[i].plan;
            let (cx, cz) = (plan.approximate.x >> 4, plan.approximate.z >> 4);
            let ready = plan.dim == self.dim
                && (-1..=1).all(|dz| (-1..=1).all(|dx| self.streaming.is_lit(cx + dx, cz + dz)));
            if !ready {
                i += 1;
                continue;
            }
            let Arrival {
                plan,
                mut tag,
                transferred,
                tnt_used_portal,
            } = self.portals.arrivals.remove(i);
            self.portals.dirty = true;
            let motion = horizon_client::entity_nbt::motion(&tag);
            let [yaw, pitch] = horizon_client::entity_nbt::rotation(&tag);
            let was_yaw = yaw;
            let (at, motion, [yaw, pitch]) = if plan.end {
                // Relative.DELTA: the motion is kept.
                let at = self.end_portal_exit(&plan);
                // EndPortalBlock.getPortalDestination: PLACE_PORTAL_TICKET.
                self.place_portal_ticket(Self::chunk_of(at));
                let into_end = plan.dim == Dim::End;
                let turn =
                    end_portal_rotation(into_end, [yaw, pitch], self.level.world_spawn_angle);
                (at, motion, turn)
            } else {
                let exit =
                    horizon_client::spans::time("exit_portal", || self.nether_exit_portal(&plan));
                let Some(exit) = exit else {
                    continue;
                };
                let (at, rotation) = self.arrival(&plan, exit);
                (at, rotate_y(motion, -rotation), [yaw + rotation, pitch])
            };
            set_arrival_pose(&mut tag, at, motion, [yaw, pitch], yaw - was_yaw);
            preserve_arrival_cooldown(&mut tag);
            // ServerLevel.addDuringTeleport: a fresh add, not loaded from disk.
            if let Some(TransferredArrivalState::VehicleTree(state)) = transferred {
                let key = Self::chunk_of(at);
                if self.restore_vehicle_tree_state(tag, state).is_some() {
                    self.native_joins_flush(false, 0);
                    self.native_entities_load(self.dim.id(), [key.0, key.1], 1);
                }
                continue;
            }
            super::persist::mark_fresh(&mut tag);
            let immediate = matches!(
                horizon_client::entity_nbt::kind(&tag).as_deref(),
                Some("minecraft:item" | "minecraft:tnt")
            );
            let tag = if tnt_used_portal {
                PendingEntity::after_tnt_portal(tag)
            } else {
                tag.into()
            };
            let key = ((at.x.floor() as i32) >> 4, (at.z.floor() as i32) >> 4);
            if let Some(TransferredArrivalState::Entity(state)) = transferred {
                self.spawn_teleported_entity(key, tag, Some(state));
                self.native_entities_load(self.dim.id(), [key.0, key.1], 1);
                continue;
            }
            if immediate {
                self.spawn_teleported_entity(key, tag, None);
                self.native_entities_load(self.dim.id(), [key.0, key.1], 1);
                continue;
            }
            match self
                .world
                .loaded_entities
                .iter_mut()
                .find(|(k, _)| *k == key)
            {
                Some((_, tags)) => tags.push(tag),
                None => self.world.loaded_entities.push((key, vec![tag])),
            }
        }
    }

    /// `ItemEntity.teleport` / `PrimedTnt.teleport`: after the recreated server
    /// entity joins, merge its stack now or select the portal-safe blast calculator.
    pub(super) fn spawn_teleported_entity(
        &mut self,
        key: (i32, i32),
        pending: PendingEntity,
        transferred: Option<super::entity_initialization::TransferredEntityState>,
    ) {
        // The portal's runtime reason must reach admission before join callbacks;
        // saved NBT alone cannot request TNT's transient blast immunity.
        let (tag, reason) = pending.into_parts();
        let uuid = horizon_client::entity_nbt::uuid(&tag);
        let is_item = matches!(tag.get("id"), Some(Tag::String(kind))
            if kind.0.iter().copied().eq("minecraft:item".encode_utf16()));
        let is_tnt = matches!(tag.get("id"), Some(Tag::String(kind))
            if kind.0.iter().copied().eq("minecraft:tnt".encode_utf16()));
        self.spawn_loaded_entity_reason(key, tag, transferred, reason);
        if !is_item && !is_tnt {
            return;
        }
        // Construction and join callbacks can insert, cancel, or remove rows.
        // Resolve the surviving arrival by the resident UUID index after them.
        let Some(entity) = uuid.and_then(|uuid| {
            self.entity_by_uuid(mod_api::EntityUuid(
                horizon_client::lan::playerdata::ints_uuid(uuid),
            ))
        }) else {
            return;
        };
        let id = entity.id();
        if is_tnt {
            let location = self
                .resident_entities
                .table
                .borrow_mut()
                .server_admitted_identity(id, |at| self.locate_resident_owner(at))
                .map(|(at, _, _)| at);
            if let Some(i) = location.and_then(|at| self.tnts.locate(at.join)) {
                self.tnts[i].used_portal = true;
            }
        } else {
            let mut items = self.dropped.lift();
            let mut arrival = None;
            let mut removed: Vec<_> = items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    if item.id == id {
                        arrival = Some(i);
                    }
                    self.persist
                        .removals
                        .contains(&super::commands::Target::Item(item.id))
                })
                .collect();
            if let Some(item) = arrival {
                let mut cells = horizon_client::entities::ItemCells::new(&items);
                horizon_client::entities::merge_with_neighbours(
                    &mut items,
                    &mut removed,
                    &mut cells,
                    item,
                    false,
                    &|stack| self.stack_max_size(stack).max(1),
                );
            }
            let mut removed = removed.into_iter();
            // A logically removed row belongs to the native removal sweep, not
            // this merge's structural retirement.
            items.retain(|item| {
                !removed.next().unwrap()
                    || self
                        .persist
                        .removals
                        .contains(&super::commands::Target::Item(item.id))
            });
            self.put_back_dropped(items);
        }
    }

    /// `FallingBlockEntity.tick`: block effects mark its portal, then
    /// `Entity.handlePortal` transfers it before the landing branch runs.
    pub(in crate::game) fn falling_through_portal(
        &mut self,
        f: &mut super::FallingBlock,
    ) -> Option<Dim> {
        if self.remote.is_some() {
            return None;
        }
        let mut sections = std::mem::take(&mut self.portals.sections);
        let half = f64::from(0.98_f32) / 2.0;
        let bounds = Aabb::new(
            Vec3::new(f.position.x - half, f.position.y, f.position.z - half),
            Vec3::new(
                f.position.x + half,
                f.position.y + f64::from(0.98_f32),
                f.position.z + half,
            ),
        );
        let entry = self.swept_portal_box(f.previous, f.position, bounds, &mut sections);
        self.portals.sections = sections;
        let delay = self.dimension_changing_delay(f.id);
        if let Some(entry) = entry {
            f.portal.set_inside(entry, delay);
        }
        let entry = f.portal.handle(true, 0, delay)?;
        let plan = self.entity_plan(entry, f.position, 0.98, 0.98, false);
        // An untracked block (id 0) has no handle to ask about.
        let allowed = if f.id == 0 {
            self.may_enter(plan.dim)
        } else {
            self.with_entity_held_out(f.id, |game| game.may_travel_through(f.id, &plan))
        };
        if !allowed {
            return None;
        }
        let uuid = f.saved.as_ref().map_or_else(
            || {
                if f.id == 0 {
                    horizon_client::save::random_uuid()
                } else {
                    self.uuid_for(f.id)
                }
            },
            |saved| saved.0,
        );
        let modelled = horizon_client::entity_nbt::falling_block_to_nbt(
            self.state_to_nbt(f.state),
            f.position,
            f.velocity,
            f.age,
            f.drop_item,
            f.damage,
            f.fall_distance,
            f.on_ground,
            f.rotation,
            uuid,
        );
        let mut tag = super::persist::with_raw(modelled, f.saved.as_ref().map(|s| &s.1));
        if let Tag::Compound(fields) = &mut tag {
            fields.insert("PortalCooldown".into(), Tag::Int(f.portal.cooldown));
            if f.no_gravity {
                fields.insert("NoGravity".into(), Tag::Byte(1));
            } else {
                fields.remove(&"NoGravity".into());
            }
        }
        let transferred = if f.id != 0 {
            super::persist::write_attachments(&mut tag, f.id, &self.entity_attachments);
            let state = self.take_transferred_entity_state(f.id);
            self.forget_entity(f.id);
            Some(TransferredArrivalState::Entity(state))
        } else {
            None
        };
        self.portals.arrivals.push(Arrival {
            tnt_used_portal: false,
            plan,
            tag,
            transferred,
        });
        self.portals.dirty = true;
        Some(plan.dim)
    }

    // ---- persistence ----

    fn arrivals_path(&self) -> std::path::PathBuf {
        self.world_root
            .join("data")
            .join("horizon_portal_arrivals.dat")
    }
    /// The waiting entities of the open world (read once).
    pub(super) fn load_portal_arrivals(&mut self) {
        if self.portals.loaded || self.remote.is_some() {
            return;
        }
        self.portals.loaded = true;
        let Some(root) = horizon_client::save::read_dat(&self.arrivals_path()) else {
            return;
        };
        let Some(Tag::List(list)) = root.get("arrivals") else {
            return;
        };
        self.portals
            .arrivals
            .extend(list.iter().filter_map(arrival_from_tag));
    }
    /// Write the waiting entities when they changed.
    pub(super) fn save_portal_arrivals(&mut self) {
        if !self.portals.dirty || self.remote.is_some() {
            return;
        }
        let list = self.portals.arrivals.iter().map(arrival_tag).collect();
        let root = Tag::compound([("arrivals", Tag::List(list))]);
        if horizon_client::save::write_dat(&self.arrivals_path(), root).is_ok() {
            self.portals.dirty = false;
        }
    }
}

impl Game<'_> {}

#[cfg(test)]
mod tests {
    use super::super::server_travel::TravelCause;
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    };

    #[test]
    #[ignore = "daily"]
    fn portal_capture_preserves_rejected_objects_and_reverses_admission() {
        with_gate(|game, _, _| {
            game.options.execution_role =
                super::super::execution_role::ExecutionRole::IntegratedServer;
            game.options.game_dir = Some(
                std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
                    .join("portal-tree"),
            );
            game.build_registries();
            game.set_allow_commands(true);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            assert!(game.run_command(r#"summon minecraft:pig 0 64 0 {UUID:[I;0,0,0,175],Tags:["portal_root"],Passengers:[{id:"minecraft:pig",UUID:[I;0,0,0,175],Passengers:[{id:"minecraft:pig",UUID:[I;0,0,0,176]}]}]}"#));
            let root = game
                .mobs
                .iter()
                .find(|m| {
                    game.persist_raw(m.id).is_some_and(|t| {
                        horizon_client::entity_nbt::uuid(t) == Some([0, 0, 0, 175])
                    })
                })
                .unwrap()
                .id;
            let child = game.mob_passenger_ids(root)[0];
            let leaf = game.mob_passenger_ids(child)[0];
            assert!(!game.mobs.iter().any(|m| m.id == child));
            game.tickets.add_portal((0, 0));
            game.update_chunk_tickets();
            assert!(game.entity_ticking_area()(0, 0));
            game.set_portal_cooldown(leaf);
            let cooldown = game.portals.entities[&leaf].cooldown;
            game.handle_entity_portals();
            assert_eq!(game.portals.entities[&leaf].cooldown, cooldown);
            game.send_through_portal(root, BlockPos::new(0, 64, 0));
            assert!(game.mob_object(root).is_none());
            assert!(game.mob_object(child).is_none());
            assert!(game.mob_object(leaf).is_none());
            // Land the queued payload through the real arrival transform/adoption path.
            game.portals.arrivals[0].plan =
                Plan::end_portal(Dim::Overworld, BlockPos::new(0, 64, 0), 0.9, 0.9);
            assert_eq!(game.dim, Dim::Overworld);
            game.enter_level(Dim::End).unwrap();
            assert_eq!(game.dim, Dim::End);
            let target = game.portals.arrivals[0].plan.approximate;
            for z in (target.z >> 4) - 1..=(target.z >> 4) + 1 {
                for x in (target.x >> 4) - 1..=(target.x >> 4) + 1 {
                    game.streaming
                        .insert_remote(&mut game.world, horizon_client::chunk::Chunk::empty(x, z));
                }
            }
            game.resolve_arrivals();
            assert!(game.portals.arrivals.is_empty());
            let new_root = game
                .mob_objects()
                .find(|m| m.extra.mount.is_none())
                .unwrap()
                .id;
            let new_child = game.mob_passenger_ids(new_root)[0];
            let new_leaf = game.mob_passenger_ids(new_child)[0];
            assert!(!game.mobs.iter().any(|m| m.id == new_root));
            assert!(game.mobs.iter().any(|m| m.id == new_child));
            assert!(game.mobs_below_off_level().contains(&new_child));
            assert!(game.mobs_below_off_level().contains(&new_leaf));
            assert_eq!(game.portals.entities[&new_root].cooldown, ENTITY_DELAY);
            assert!(
                game.persist_raw(new_root)
                    .unwrap()
                    .get("horizon:fresh_add")
                    .is_none()
            );
        });
    }

    /// Cancels the travel of the entity whose id `spare` holds, and logs each
    /// (entity, dimension, cause) it is asked.
    struct Gate {
        spare: Arc<AtomicU32>,
        heard: Arc<Mutex<Vec<(u32, String, String)>>>,
        discard: bool,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Gate {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            e: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if self.spare.load(Ordering::Relaxed) == 1004
                && !e.mounting
                && e.entity == 102
                && let Some(sibling) = host.entity(103)
            {
                assert!(sibling.discard(host));
            }
            mod_api::Flow::Continue
        }

        fn entity_travel_to_dimension(
            &self,
            host: &mut dyn mod_api::ServerHost,
            e: &mut mod_api::EntityTravelToDimensionEvent,
        ) -> mod_api::Flow {
            self.heard.lock().unwrap().push((
                e.entity.id(),
                e.dimension.to_string(),
                e.cause.to_string(),
            ));
            let attack = self.spare.load(Ordering::Relaxed);
            if attack == 1001
                && e.entity.id() == 102
                && self
                    .heard
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|e| e.0 == 102)
                    .count()
                    == 1
            {
                let root = mod_api::EntityRef::new(e.entity.epoch(), 101);
                assert!(root.start_riding(host, e.entity, true));
            }
            if attack == 1002 && e.entity.id() == 102 {
                assert!(mod_api::EntityRef::new(e.entity.epoch(), 103).discard(host));
            }
            if self.discard {
                assert!(e.entity.discard(host));
                return mod_api::Flow::Continue;
            }
            if e.entity.id() == self.spare.load(Ordering::Relaxed) {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    /// Runs `test` with a `Gate` installed, on a server with guests 12 and 13.
    fn with_gate(test: impl FnOnce(&mut Game<'_>, &AtomicU32, &Mutex<Vec<(u32, String, String)>>)) {
        with_gate_mode(false, test);
    }

    fn with_gate_mode(
        discard: bool,
        test: impl FnOnce(&mut Game<'_>, &AtomicU32, &Mutex<Vec<(u32, String, String)>>),
    ) {
        let spare = Arc::new(AtomicU32::new(0));
        let heard = Arc::default();
        let mods = vec![mod_api::ModEntry::new(
            "gate",
            Box::new(Gate {
                spare: Arc::clone(&spare),
                heard: Arc::clone(&heard),
                discard,
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_guest_turn_tests::with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                test(game, &spare, &heard);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_follows_the_class_then_the_first_passenger() {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            game.boats.push(super::super::boats::Boat::new(
                101,
                Arc::from("oak"),
                Vec3::ZERO,
                0.0,
            ));
            game.carts.push(super::super::minecarts::Minecart::new(
                103,
                super::super::minecarts::CartKind::Rideable,
                Vec3::ZERO,
            ));
            let mob = |name: &str, id| {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                horizon_client::mobs::Mob::new(id, kind, Vec3::ZERO, 0.0)
            };
            let (cow, pig) = (mob("minecraft:cow", 102), mob("minecraft:pig", 104));
            game.mobs.push(cow);
            game.mobs.push(pig);
            assert_eq!(game.dimension_changing_delay(12), 10);
            let mut projectile = horizon_client::projectiles::Projectile::new(
                horizon_client::projectiles::ProjectileKind::Snowball,
                Vec3::ZERO,
                None,
            );
            projectile.id = 106;
            game.creatures.projectiles.push(projectile);
            assert_eq!(game.dimension_changing_delay(106), 2);
            let entry = BlockPos::new(0, 64, 0);
            // Boats and minecarts (`VehicleEntity`) are 10 whoever rides them;
            // a mob vehicle follows its first passenger. Riders: none, mob
            // only, player first, player then mob, then mob first.
            for (vehicle, by_class) in [(101, true), (103, true), (104, false)] {
                for (player, mob_order, by_rider) in [
                    (false, None, 300),
                    (false, Some(20), 300),
                    (true, None, 10),
                    (true, Some(20), 10),
                    (true, Some(-10), 300),
                ] {
                    let expected = if by_class { 10 } else { by_rider };
                    game.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                        g.vehicle = player.then_some(vehicle);
                        g.vehicle_order = 0;
                    });
                    game.mobs[0].extra.mount = mob_order.map(|_| vehicle);
                    game.mobs[0].extra.vehicle_order = mob_order.unwrap_or_default();
                    assert_eq!(
                        game.dimension_changing_delay(vehicle),
                        expected,
                        "vehicle {vehicle}, player {player}, mob {mob_order:?}"
                    );
                    portal_cooldown_keeps_the_process(game, vehicle, entry, expected);
                }
            }
        });
    }

    /// `Entity.setPortalCooldown` sets only the cooldown; the restart and
    /// the portal process use the same delay.
    fn portal_cooldown_keeps_the_process(
        game: &mut Game<'_>,
        id: u32,
        entry: BlockPos,
        expected: i32,
    ) {
        game.portals.entities.insert(
            id,
            PortalState {
                cooldown: 0,
                process: Some(horizon_client::nether_portal::Process {
                    entry,
                    time: 7,
                    inside: true,
                }),
            },
        );
        game.set_portal_cooldown(id);
        let state = game.portals.entities.get_mut(&id).unwrap();
        assert_eq!(state.cooldown, expected);
        let process = state.process.unwrap();
        assert_eq!(process.entry, entry);
        assert_eq!(process.time, 7);
        assert!(process.inside);
        state.cooldown = 1;
        assert!(game.restart_portal_cooldown(id));
        assert_eq!(game.portals.entities[&id].cooldown, expected);
        let delay = game.dimension_changing_delay(id);
        let state = game.portals.entities.get_mut(&id).unwrap();
        assert_eq!(state.handle(true, 0, delay), Some(entry));
        assert_eq!(state.cooldown, expected);
        assert_eq!(state.handle(true, 0, delay), None);
        assert_eq!(state.cooldown, expected - 1);
    }

    #[test]
    fn resolved_arrival_keeps_source_cooldown() {
        for cooldown in [2, 10, 300] {
            let mut tag = Tag::compound([("PortalCooldown", Tag::Int(cooldown))]);
            set_arrival_pose(&mut tag, Vec3::ZERO, Vec3::ZERO, [90.0, 0.0], 90.0);
            assert_eq!(tag.get("PortalCooldown"), Some(&Tag::Int(cooldown)));
            assert_eq!(horizon_client::entity_nbt::position(&tag), Some(Vec3::ZERO));
        }
    }

    /// `EntityTravelToDimensionEvent` for server players: both are asked with
    /// their own id, the target level and the cause; the spared one stays, the
    /// other may go.
    #[test]
    #[ignore = "daily"]
    fn a_mod_cancels_one_players_travel_and_allows_the_other() {
        with_gate(|game, spare, heard| {
            // Guests 12 and 13 are entities 12 and 13.
            spare.store(12, Ordering::Relaxed);
            assert!(!game.server_player_travel_allowed(12, Dim::Nether, TravelCause::NetherPortal));
            assert!(game.server_player_travel_allowed(13, Dim::End, TravelCause::EndPortal));
            assert_eq!(
                *heard.lock().unwrap(),
                [
                    (
                        12,
                        "minecraft:the_nether".to_owned(),
                        "nether_portal".to_owned()
                    ),
                    (13, "minecraft:the_end".to_owned(), "end_portal".to_owned()),
                ]
            );
        });
    }

    /// The same for two dropped items at a Nether portal: the spared one is
    /// still in the level and waits for no arrival, the other has left it for
    /// one.
    #[test]
    #[ignore = "daily"]
    fn a_mod_cancels_one_items_portal_trip_and_allows_the_other() {
        with_gate(|game, spare, heard| {
            let stone = || horizon_client::items::ItemStack::new("minecraft:stone", 1);
            game.spawn_item(stone(), [0.5, 64.0, 0.5], [0.0; 3], 0);
            game.spawn_item(stone(), [2.5, 64.0, 0.5], [0.0; 3], 0);
            let ids: Vec<u32> = game.dropped.iter().map(|d| d.id).collect();
            assert_eq!(ids.len(), 2);
            spare.store(ids[0], Ordering::Relaxed);
            for id in &ids {
                game.send_through_portal(*id, BlockPos::new(0, 64, 0));
            }
            let left: Vec<u32> = game.dropped.iter().map(|d| d.id).collect();
            assert_eq!(left, [ids[0]]);
            assert_eq!(game.portals.arrivals.len(), 1);
            assert_eq!(game.portals.arrivals[0].plan.dim, Dim::Nether);
            assert_eq!(
                game.portals.arrivals[0].tag.get("PortalCooldown"),
                Some(&Tag::Int(300))
            );
            let nether = || "minecraft:the_nether".to_owned();
            let cause = || "nether_portal".to_owned();
            assert_eq!(
                *heard.lock().unwrap(),
                [(ids[0], nether(), cause()), (ids[1], nether(), cause())]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn discarded_item_does_not_travel() {
        with_gate_mode(true, |game, _, heard| {
            game.spawn_item(
                horizon_client::items::ItemStack::new("minecraft:stone", 1),
                [0.5, 64.0, 0.5],
                [0.0; 3],
                0,
            );
            let id = game.dropped[0].id;
            game.send_through_portal(id, BlockPos::new(0, 64, 0));
            assert!(game.portals.arrivals.is_empty());
            assert_eq!(heard.lock().unwrap().len(), 1);
            assert!(game.level_entity(id as i32).is_none());
            assert_eq!(game.persist.removals.len(), 1);
            game.native_removals_flush();
            assert!(game.dropped.is_empty());
            assert!(game.persist.removals.is_empty());
            game.native_removals_flush();
            assert!(game.dropped.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn discarded_mob_and_tnt_do_not_travel() {
        with_gate_mode(true, |game, _, heard| {
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(horizon_client::mobs::Mob::new(
                101,
                kind,
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            ));
            game.spawn_primed_tnt(BlockPos::new(2, 64, 0), false, None, None);
            let tnt = game.tnts.last().unwrap().id;
            for id in [101, tnt] {
                game.send_through_portal(id, BlockPos::new(0, 64, 0));
                assert!(game.level_entity(id as i32).is_none());
            }
            assert!(game.portals.arrivals.is_empty());
            assert_eq!(heard.lock().unwrap().len(), 2);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn refused_mob_passenger_stays_behind() {
        with_gate(|game, spare, heard| {
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            for id in [101, 102, 103] {
                let mut mob = horizon_client::mobs::Mob::new(
                    id,
                    kind.clone(),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                if id != 101 {
                    mob.extra.mount = Some(101);
                }
                game.mobs.push(mob);
            }
            spare.store(102, Ordering::Relaxed);
            game.send_through_portal(101, BlockPos::new(0, 64, 0));
            assert_eq!(game.portals.arrivals.len(), 1);
            assert_eq!(game.mobs.iter().map(|m| m.id).collect::<Vec<_>>(), [102]);
            assert_eq!(game.mobs[0].extra.mount, None);
            assert_eq!(
                heard
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|e| e.0)
                    .collect::<Vec<_>>(),
                [101, 102, 103]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn passenger_callbacks_cannot_revisit_ancestors_or_discarded_siblings() {
        for attack in [1004, 1001, 1002] {
            with_gate(|game, spare, heard| {
                let kind = game.res.mob_types().get("minecraft:camel").unwrap().clone();
                for id in [101, 102, 103] {
                    let mut mob = horizon_client::mobs::Mob::new(
                        id,
                        kind.clone(),
                        Vec3::new(0.5, 64.0, 0.5),
                        0.0,
                    );
                    if id != 101 {
                        mob.extra.mount = Some(101);
                    }
                    game.mobs.push(mob);
                }
                spare.store(attack, Ordering::Relaxed);
                assert!(game.entity_may_travel(101, Dim::Nether, TravelCause::Teleport));
                let ids: Vec<_> = heard.lock().unwrap().iter().map(|e| e.0).collect();
                assert_eq!(
                    ids,
                    if attack == 1001 {
                        vec![101, 102, 103]
                    } else {
                        vec![101, 102]
                    }
                );
                assert!(
                    !(game.mobs[0].extra.mount == Some(102)
                        && game.mobs[1].extra.mount == Some(101))
                );
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn mixed_passengers_follow_boarding_not_connection_order() {
        with_gate(|game, _, heard| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:happy_ghast")
                .unwrap()
                .clone();
            for id in [101, 102] {
                let mut mob = horizon_client::mobs::Mob::new(
                    id,
                    kind.clone(),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                if id == 102 {
                    mob.extra.mount = Some(101);
                    mob.extra.vehicle_order = 20;
                }
                game.mobs.push(mob);
            }
            for (conn, order) in [(12, 30), (13, 10)] {
                game.lan.as_mut().unwrap().hub.update_guest(conn, |g| {
                    g.vehicle = Some(101);
                    g.vehicle_order = order;
                });
            }
            assert!(game.entity_may_travel(101, Dim::Nether, TravelCause::Teleport));
            assert_eq!(
                heard
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|e| e.0)
                    .collect::<Vec<_>>(),
                [101, 13, 102, 12]
            );
        });
    }

    #[test]
    fn portal_transfer_pending_arrival_does_not_save_transient_tnt_immunity() {
        let arrival = Arrival {
            plan: Plan::end_portal(Dim::End, BlockPos::new(0, 64, 0), 0.98, 0.98),
            tag: Tag::compound([("id", Tag::String("minecraft:tnt".into()))]),
            tnt_used_portal: true,
            transferred: None,
        };
        assert!(
            !arrival_from_tag(&arrival_tag(&arrival))
                .unwrap()
                .tnt_used_portal
        );
    }

    #[test]
    fn end_portals_lead_to_the_end_spawn_and_back_to_the_world_spawn() {
        let spawn = BlockPos::new(-7, 70, 12);
        for from in [Dim::Overworld, Dim::Nether] {
            let plan = Plan::end_portal(from, spawn, 0.9, 1.4);
            assert_eq!(plan.dim, Dim::End);
            assert_eq!(plan.approximate, BlockPos::new(100, 50, 0));
            assert!(plan.end);
        }
        let plan = Plan::end_portal(Dim::End, spawn, 0.25, 0.25);
        assert_eq!(plan.dim, Dim::Overworld);
        assert_eq!(plan.approximate, spawn);
    }

    #[test]
    fn the_end_portal_shape_takes_falling_items_and_standing_mobs() {
        let portal = BlockPos::new(0, 100, 3);
        let at = |y| Vec3::new(0.5, y, 3.5);
        let box_at = |pos: Vec3, width: f32, height: f32| {
            let half = f64::from(width) / 2.0;
            Aabb::new(
                Vec3::new(pos.x - half, pos.y, pos.z - half),
                Vec3::new(
                    pos.x + half,
                    pos.y + f64::from(height),
                    pos.z + half,
                ),
            )
        };
        // A dropped item falling through the slab, or in it.
        assert!(touches_end_portal_box(
            at(100.6),
            at(100.56),
            box_at(at(100.6), 0.25, 0.25),
            portal
        ));
        // One tick's fall across the whole slab, in it at neither end.
        assert!(touches_end_portal_box(
            at(101.0),
            at(99.9),
            box_at(at(101.0), 0.25, 0.25),
            portal
        ));
        // Resting below the slab (on the floor) or above it.
        assert!(!touches_end_portal_box(
            at(100.0),
            at(100.0),
            box_at(at(100.0), 0.25, 0.25),
            portal
        ));
        assert!(!touches_end_portal_box(
            at(100.8),
            at(100.8),
            box_at(at(100.8), 0.25, 0.25),
            portal
        ));
        // A cow standing in the portal block.
        assert!(touches_end_portal_box(
            at(100.0),
            at(100.0),
            box_at(at(100.0), 0.9, 1.4),
            portal
        ));
        // Beside it.
        let beside = Vec3::new(2.0, 100.0, 3.5);
        assert!(!touches_end_portal_box(
            beside,
            beside,
            box_at(beside, 0.9, 1.4),
            portal
        ));
    }

    #[test]
    fn end_portals_turn_entities_like_their_teleport_transition() {
        // Into the End: facing west, pitch kept.
        assert_eq!(
            end_portal_rotation(true, [-30.0, 20.0], [45.0, 10.0]),
            [90.0, 20.0]
        );
        // Out of it: turned by the world spawn's yaw and pitch, pitch clamped.
        assert_eq!(
            end_portal_rotation(false, [-30.0, 20.0], [45.0, 10.0]),
            [15.0, 30.0]
        );
        assert_eq!(
            end_portal_rotation(false, [0.0, 80.0], [0.0, 30.0]),
            [0.0, 90.0]
        );
    }

    #[test]
    fn legacy_arrivals_keep_explicit_positive_counters_without_type_migration() {
        for old in [None, Some(0), Some(-1), Some(5)] {
            let mut tag = Tag::compound([("id", Tag::String("minecraft:oak_boat".into()))]);
            if let (Some(old), Tag::Compound(fields)) = (old, &mut tag) {
                fields.insert("PortalCooldown".into(), Tag::Int(old));
            }
            preserve_arrival_cooldown(&mut tag);
            let arrival = Arrival {
                transferred: None,
                plan: Plan::end_portal(Dim::Overworld, BlockPos::new(0, 64, 0), 1.375, 0.5625),
                tag,
                tnt_used_portal: false,
            };
            let restored = arrival_from_tag(&arrival_tag(&arrival)).unwrap();
            let expected = old.filter(|n| *n > 0).unwrap_or(ENTITY_DELAY);
            assert_eq!(
                restored.tag.get("PortalCooldown"),
                Some(&Tag::Int(expected))
            );
        }
    }

    #[test]
    fn waiting_arrivals_keep_their_end_destination_when_saved() {
        let tag = Tag::compound([("id", Tag::String("minecraft:cow".into()))]);
        for (from, dim) in [(Dim::Overworld, Dim::End), (Dim::End, Dim::Overworld)] {
            let arrival = Arrival {
                tnt_used_portal: false,
                transferred: None,
                plan: Plan::end_portal(from, BlockPos::new(3, 64, -5), 0.9, 1.4),
                tag: tag.clone(),
            };
            let back = arrival_from_tag(&arrival_tag(&arrival)).unwrap();
            assert_eq!(back.plan.dim, dim);
            assert_eq!(back.plan.approximate, arrival.plan.approximate);
            assert!(back.plan.end);
            assert_eq!(back.tag, tag);
        }
        // Arrivals saved before End portals were Nether portal plans.
        let mut old = arrival_tag(&Arrival {
            tnt_used_portal: false,
            transferred: None,
            plan: Plan::end_portal(Dim::End, BlockPos::new(0, 64, 0), 0.25, 0.25),
            tag,
        });
        if let Tag::Compound(c) = &mut old {
            c.remove(&"end".into());
        }
        assert!(!arrival_from_tag(&old).unwrap().plan.end);
    }
}

#[cfg(test)]
#[path = "tests/portal_cooldown_tests.rs"]
mod cooldown_tests;

#[cfg(test)]
#[path = "nether_portal/arrival_effects_tests.rs"]
pub(in crate::game) mod arrival_effects_tests;

impl Game<'_> {
    /// PortalShape: an obsidian frame around 2..=21 by 3..=21 empty blocks.
    pub(super) fn portal_shape(
        &self,
        start: [i32; 3],
        x_axis: bool,
    ) -> Option<([i32; 3], i32, i32)> {
        let name = |p: [i32; 3]| {
            self.res
                .facts()
                .get(self.world.block(p[0], p[1], p[2]))
                .name
                .clone()
        };
        let empty = |p: [i32; 3]| {
            let n = name(p);
            matches!(
                &*n,
                "minecraft:air"
                    | "minecraft:cave_air"
                    | "minecraft:fire"
                    | "minecraft:soul_fire"
                    | "minecraft:nether_portal"
            )
        };
        let frame = |p: [i32; 3]| &*name(p) == "minecraft:obsidian";
        // findEmptyPortalShape requires numPortalBlocks == 0: a partly
        // filled frame is not a candidate for fire lighting.
        let is_portal = |p: [i32; 3]| &*name(p) == "minecraft:nether_portal";
        let d = if x_axis { [1, 0, 0] } else { [0, 0, 1] };
        let mut bottom = start;
        let mut n = 0;
        while empty([bottom[0], bottom[1] - 1, bottom[2]]) && n < 21 {
            bottom[1] -= 1;
            n += 1;
        }
        if !frame([bottom[0], bottom[1] - 1, bottom[2]]) {
            return None;
        }
        let mut left = bottom;
        n = 0;
        while empty([left[0] - d[0], left[1], left[2] - d[2]]) && n < 21 {
            left = [left[0] - d[0], left[1], left[2] - d[2]];
            n += 1;
        }
        if !frame([left[0] - d[0], left[1], left[2] - d[2]]) {
            return None;
        }
        let mut width = 0;
        while width <= 21 {
            let p = [left[0] + d[0] * width, left[1], left[2] + d[2] * width];
            if !empty(p) || !frame([p[0], p[1] - 1, p[2]]) {
                break;
            }
            width += 1;
        }
        if !(2..=21).contains(&width)
            || !frame([left[0] + d[0] * width, left[1], left[2] + d[2] * width])
        {
            return None;
        }
        let mut height = 0;
        let mut portal_blocks = 0;
        'rows: while height <= 21 {
            let y = left[1] + height;
            if !frame([left[0] - d[0], y, left[2] - d[2]])
                || !frame([left[0] + d[0] * width, y, left[2] + d[2] * width])
            {
                break;
            }
            for i in 0..width {
                let q = [left[0] + d[0] * i, y, left[2] + d[2] * i];
                if !empty(q) {
                    break 'rows;
                }
                if is_portal(q) {
                    portal_blocks += 1;
                }
            }
            height += 1;
        }
        let top_ok =
            (0..width).all(|i| frame([left[0] + d[0] * i, left[1] + height, left[2] + d[2] * i]));
        ((3..=21).contains(&height) && top_ok && portal_blocks == 0)
            .then_some((left, width, height))
    }
    /// BaseFireBlock.onPlace: lighting fire inside a frame creates a portal,
    /// only in the Overworld and the Nether (`inPortalDimension`).
    pub(super) fn try_light_portal(&mut self, at: [i32; 3]) -> Result<bool> {
        if !matches!(self.dim, Dim::Overworld | Dim::Nether) {
            return Ok(false);
        }
        for x_axis in [true, false] {
            if let Some((origin, width, height)) = self.portal_shape(at, x_axis) {
                let d = if x_axis { [1, 0, 0] } else { [0, 0, 1] };
                let registry = &self.pack.assets.registry;
                let Some(default) = registry.default_state("minecraft:nether_portal") else {
                    return Ok(false);
                };
                let mut portal = registry.state(default).cloned().ok_or("portal state")?;
                portal
                    .properties
                    .insert("axis".into(), if x_axis { "x" } else { "z" }.into());
                let id = registry.resolve(&portal).unwrap_or(default);
                // PortalShape.createPortalBlocks uses BlockPos.betweenClosed:
                // X varies fastest, then Y, then Z.
                let first = BlockPos::new(origin[0], origin[1], origin[2]);
                let last = BlockPos::new(
                    origin[0] + d[0] * (width - 1),
                    origin[1] + height - 1,
                    origin[2] + d[2] * (width - 1),
                );
                for pos in BlockPos::between_closed(first, last).expect("portal size is bounded") {
                    self.level_set_with_flags(
                        [pos.x, pos.y, pos.z],
                        id,
                        level_flags::UPDATE_CLIENTS | level_flags::UPDATE_KNOWN_SHAPE,
                    )?;
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
#[path = "tests/nether_portal_stand_tests.rs"]
mod stand_tests;
