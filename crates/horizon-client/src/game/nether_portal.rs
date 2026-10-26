//! Nether portals for the player and the other entities:
//! `Entity.setAsInsidePortal` / `handlePortal` with `PortalProcessor`,
//! `NetherPortalBlock.getPortalTransitionTime` and `getPortalDestination`
//! (`PortalForcer` through `horizon_world::level::portal_travel`), and
//! `LocalPlayer.handlePortalTransitionEffect`.
//!
//! Only one dimension is loaded at a time. The player's destination is found
//! once the other dimension has loaded around it. An item or mob that goes
//! through waits as an `Arrival` (its saved compound and where it stood in its
//! portal) until the player is in that dimension and its exit is loaded.
use super::{Dim, Game, Screen};
use horizon_client::Result;
use horizon_client::nether_portal::PortalState;
use horizon_core::{
    geometry::{Axis, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::{
    level::portal_travel::{
        BorderBounds, FoundRectangle, closest_portal, collision_free_position, exit_position,
        largest_rectangle_around, relative_position,
    },
    nbt::Tag,
};
use std::collections::HashMap;

/// `Player.getDimensionChangingDelay`.
pub(super) const PLAYER_DELAY: i32 = 10;
/// `Entity.getDimensionChangingDelay`.
const ENTITY_DELAY: i32 = 300;

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
}

/// An entity on its way to a dimension that isn't loaded.
#[derive(Clone, Debug)]
struct Arrival {
    plan: Plan,
    tag: Tag,
}

/// The nether portal state of the world (`Game::portals`).
#[derive(Default)]
pub(super) struct Portals {
    /// The player's.
    pub(super) player: PortalState,
    /// The player's travel, while the destination loads.
    pub(super) travel: Option<Plan>,
    /// `LocalPlayer.portalProcess.isInsidePortalThisTick`.
    client_inside: bool,
    /// Items and mobs, by entity id.
    entities: HashMap<u32, PortalState>,
    /// Positions before this tick's moves, by entity id.
    before: HashMap<u32, Vec3>,
    arrivals: Vec<Arrival>,
    loaded: bool,
    dirty: bool,
    /// Per-chunk portal palette masks, kept across ticks (portal_sections).
    pub(super) sections: horizon_client::portal_sections::PortalSections,
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
fn doubles(values: [f64; 3]) -> Tag {
    Tag::List(values.iter().map(|v| Tag::Double(v.to_bits())).collect())
}

impl Game<'_> {
    /// The world border's extent.
    fn border_bounds(&self) -> BorderBounds {
        let [min_x, min_z, max_x, max_z] = self.border.bounds();
        BorderBounds {
            min_x,
            min_z,
            max_x,
            max_z,
        }
    }
    /// `ServerLevel.isAllowedToEnterPortal`.
    pub(super) fn may_enter(&self, target: Dim) -> bool {
        target != Dim::Nether || self.ui.rules.bool("allow_entering_nether_using_portals")
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
        // DimensionType.getTeleportationScale: 8 out of the Nether, 1/8 into it.
        let scale = if self.dim == Dim::Nether {
            8.0
        } else {
            1.0 / 8.0
        };
        Plan {
            dim: target,
            approximate: self.border_bounds().clamp(at.x * scale, at.y, at.z * scale),
            entry_x_axis,
            offset,
            width,
            height,
        }
    }
    /// The nether portal blocks of the chunks within `radius` of `center`
    /// (the points of interest `PortalForcer` searches): loaded chunks as
    /// they are, the others as saved; chunks never generated have none.
    fn portal_blocks(&self, center: BlockPos, radius: i32) -> Vec<BlockPos> {
        let b = self.res.behaviours();
        let chunk_radius = radius.div_euclid(16) + 1;
        let (cx, cz) = (center.x >> 4, center.z >> 4);
        let mut out = Vec::new();
        for z in cz - chunk_radius..=cz + chunk_radius {
            for x in cx - chunk_radius..=cx + chunk_radius {
                let saved;
                let chunk = match self.world.chunks.get(&(x, z)) {
                    Some(c) => &**c,
                    None => match self
                        .save
                        .load(x, z, self.pack.assets.registry, self.res.biomes())
                    {
                        Some(c) => {
                            saved = c;
                            &saved
                        }
                        None => continue,
                    },
                };
                for (s, section) in chunk.sections.iter().enumerate() {
                    if !section.blocks.maybe_has(|v| {
                        b.portal_axis(horizon_world::registry::BlockStateId(v))
                            .is_some()
                    }) {
                        continue;
                    }
                    let base = horizon_client::chunk::MIN_Y + 16 * s as i32;
                    for y in base..base + 16 {
                        for lz in 0..16 {
                            for lx in 0..16 {
                                let (wx, wz) = (x * 16 + lx, z * 16 + lz);
                                if b.portal_axis(chunk.block(wx, y, wz)).is_some() {
                                    out.push(BlockPos::new(wx, y, wz));
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }
    /// The exit portal of `plan` in the loaded dimension: the closest one
    /// within reach, else one built by `PortalForcer.createPortal`.
    pub(super) fn nether_exit_portal(&mut self, plan: &Plan) -> Option<FoundRectangle> {
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
        );
        if let Some(found) = found {
            let state = self.world.block(found.x, found.y, found.z);
            let x_axis = self.res.behaviours().portal_axis(state).unwrap_or(true);
            return Some(largest_rectangle_around(
                found,
                if x_axis { Axis::X } else { Axis::Z },
                21,
                Axis::Y,
                21,
                |p| self.world.block(p.x, p.y, p.z) == state,
            ));
        }
        let logical = if to_nether { 128 } else { 384 };
        let (approximate, x_axis) = (plan.approximate, plan.entry_x_axis);
        let created = self.with_level(|b, level, ticks| {
            b.create_portal(level, ticks, approximate, x_axis, &border, logical)
        });
        let _ = self.process_level_events();
        created
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

    /// `NetherPortalBlock.entityInside` for the player: `canUsePortal`, then
    /// `setAsInsidePortal` (and the client's own, which drives the effect).
    pub(super) fn player_inside_nether_portal(&mut self, pos: BlockPos) {
        let alive = self
            .player
            .as_ref()
            .is_some_and(|p| !p.health.dead_or_dying());
        if !alive || self.riding.is_some() || self.sleeping.is_some() {
            return;
        }
        self.portals.player.set_inside(pos, PLAYER_DELAY);
        self.portals.client_inside = true;
    }
    /// The same on a server's client: `Entity.canUsePortal` (alive, not a
    /// passenger), then only the client's `setAsInsidePortal`, which drives
    /// the confusion effect (`client_portal_effect`).
    pub(super) fn client_inside_nether_portal(&mut self) {
        let alive = self
            .player
            .as_ref()
            .is_some_and(|p| !p.health.dead_or_dying());
        if alive && self.riding.is_none() {
            self.portals.client_inside = true;
        }
    }
    /// `ServerPlayer.handlePortal` (not while changing dimension: there is no
    /// player then). Players wait `players_nether_portal_default_delay`
    /// ticks, or the creative delay with invulnerable abilities.
    pub(super) fn tick_nether_portal(&mut self) -> Result<()> {
        // A server's portals are its own.
        if self.remote.is_some() {
            return Ok(());
        }
        let Some(p) = self.player.as_ref() else {
            return Ok(());
        };
        if self.travelling.is_some() {
            return Ok(());
        }
        let allowed = !p.health.dead_or_dying() && self.riding.is_none() && self.sleeping.is_none();
        let rule = if p.invulnerable {
            "players_nether_portal_creative_delay"
        } else {
            "players_nether_portal_default_delay"
        };
        let transition = self.ui.rules.int(rule).max(0);
        let Some(entry) = self
            .portals
            .player
            .handle(allowed, transition, PLAYER_DELAY)
        else {
            return Ok(());
        };
        self.player_through_nether_portal(entry)
    }
    /// `Entity.handlePortal` once the player's portal process at `entry`
    /// finished: `NetherPortalBlock.getPortalDestination`, then
    /// `teleportToPortalDestination` (`isAllowedToEnterPortal`).
    pub(super) fn player_through_nether_portal(&mut self, entry: BlockPos) -> Result<()> {
        let Some(p) = self.player.as_ref() else {
            return Ok(());
        };
        let plan = self.portal_plan(
            entry,
            p.position,
            p.dimensions.width,
            p.dimensions.height,
            true,
        );
        if !self.may_enter(plan.dim) {
            return Ok(());
        }
        self.portals.travel = Some(plan);
        self.end_portal_travel = false;
        self.enter_dimension(plan.dim)?;
        let centre = self.arrival_centre(&plan);
        if let Some((_, target)) = self.travelling.as_mut() {
            *target = centre;
        }
        Ok(())
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
    /// The player's arrival once the destination has loaded around the
    /// approximate exit: the waiting entities first, then the exit portal
    /// (found or built), the position in it, the turned yaw and motion, and
    /// `TeleportTransition.PLAY_PORTAL_SOUND` (level event 1032).
    pub(super) fn arrive_through_nether_portal(
        &mut self,
        mut player: horizon_client::motion::PlayerMotion,
        target: Vec3,
    ) -> Result<()> {
        self.resolve_arrivals();
        let plan = self.portals.travel.take();
        let Some(plan) = plan else {
            // A command teleport can change dimensions without going through
            // a portal. It has no portal placement, rotation, or sound effect.
            player.position = target;
            player.velocity = Vec3::ZERO;
            player.fall_distance = 0.0;
            self.previous_position = target;
            self.player = Some(player);
            self.pad.player_replaced();
            self.sway = Default::default();
            self.apply_equipment();
            return Ok(());
        };
        let exit = self.nether_exit_portal(&plan);
        let (position, rotation) = match exit {
            Some(exit) => self.arrival(&plan, exit),
            None => (target, 0.0),
        };
        player.position = position;
        // Relative.ROTATION and Relative.DELTA: the yaw turns, the motion
        // turns with it.
        player.yaw += rotation;
        player.head_yaw += rotation;
        self.yaw += rotation;
        player.velocity = rotate_y(player.velocity, -rotation);
        self.previous_position = player.position;
        self.player = Some(player);
        self.pad.player_replaced();
        self.sway = Default::default();
        self.apply_equipment();
        let pitch = self.world.random.next_float() * 0.4 + 0.8;
        self.audio
            .play("minecraft:block.portal.travel", None, 0.25, pitch);
        Ok(())
    }
    /// `LocalPlayer.handlePortalTransitionEffect`: whether the confusion
    /// effect grows this tick. Entering a portal closes screens other than
    /// the chat and death screens and plays `block.portal.trigger` when the
    /// effect starts from nothing.
    pub(super) fn client_portal_effect(&mut self) -> bool {
        if !std::mem::take(&mut self.portals.client_inside) {
            return false;
        }
        match self.screen {
            Screen::Playing | Screen::Chat | Screen::Dead => {}
            Screen::Inventory | Screen::Creative => {
                self.close_container();
                self.carried = None;
                self.screen = Screen::Playing;
                self.capture(true);
            }
            _ => {
                self.screen = Screen::Playing;
                self.capture(true);
            }
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
        if self.remote.is_some() {
            return;
        }
        self.load_portal_arrivals();
        let mut leaving: Vec<(u32, BlockPos)> = Vec::new();
        self.portals.before.clear();
        for e in &self.dropped {
            self.portals.before.insert(e.id, e.position);
        }
        for m in &self.mobs {
            self.portals.before.insert(m.id, m.position);
        }
        let ids: Vec<(u32, bool)> = self
            .dropped
            .iter()
            .map(|e| (e.id, true))
            .chain(self.mobs.iter().map(|m| (m.id, self.mob_can_use_portal(m))))
            .collect();
        let present: std::collections::HashSet<u32> = ids.iter().map(|(id, _)| *id).collect();
        self.portals.entities.retain(|id, _| present.contains(id));
        for (id, allowed) in ids {
            let Some(state) = self.portals.entities.get_mut(&id) else {
                continue;
            };
            if let Some(entry) = state.handle(allowed, 0, ENTITY_DELAY) {
                leaving.push((id, entry));
            }
            if state.cooldown == 0 && state.process.is_none() {
                self.portals.entities.remove(&id);
            }
        }
        for (id, entry) in leaving {
            self.send_through_portal(id, entry);
        }
    }
    /// `Entity.load`: the saved `PortalCooldown` (an entity that just came
    /// through stands in its exit portal for 300 ticks without going back).
    pub(super) fn restore_portal_cooldown(&mut self, id: u32, tag: &Tag) {
        if let Some(Tag::Int(cooldown)) = tag.get("PortalCooldown")
            && *cooldown > 0
        {
            self.portals.entities.insert(
                id,
                PortalState {
                    cooldown: *cooldown,
                    process: None,
                },
            );
        }
    }
    /// `canUsePortal(false)` of a mob: alive, riding nothing, not asleep,
    /// not carrying the player, and not a boss that never travels.
    fn mob_can_use_portal(&self, m: &horizon_client::mobs::Mob) -> bool {
        use horizon_client::mobs::Species;
        m.alive()
            && m.extra.mount.is_none()
            && self.riding != Some(m.id)
            && m.species() != Species::Wither
    }
    /// `NetherPortalBlock.entityInside` for the items and mobs that moved this
    /// tick: the portal blocks their box swept through.
    pub(super) fn find_entity_portals(&mut self) {
        if self.remote.is_some() {
            return;
        }
        let mut inside: Vec<(u32, BlockPos)> = Vec::new();
        let items: Vec<(u32, Vec3, f32, f32)> = self
            .dropped
            .iter()
            .map(|e| (e.id, e.position, 0.25, 0.25))
            .collect();
        let mobs: Vec<(u32, Vec3, f32, f32)> = self
            .mobs
            .iter()
            .filter(|m| self.mob_can_use_portal(m))
            .map(|m| (m.id, m.position, m.width(), m.height()))
            .collect();
        let mut portal_sections = std::mem::take(&mut self.portals.sections);
        for (id, to, width, height) in items.into_iter().chain(mobs) {
            let from = self.portals.before.get(&id).copied().unwrap_or(to);
            if let Some(pos) = self.swept_portal(from, to, width, height, &mut portal_sections) {
                inside.push((id, pos));
            }
        }
        self.portals.sections = portal_sections;
        for (id, pos) in inside {
            self.portals
                .entities
                .entry(id)
                .or_default()
                .set_inside(pos, ENTITY_DELAY);
        }
    }
    /// The first nether portal block an entity box swept through
    /// (`Entity.checkInsideBlocks`, whose first portal sets the entry).
    fn swept_portal(
        &self,
        from: Vec3,
        to: Vec3,
        width: f32,
        height: f32,
        portal_sections: &mut horizon_client::portal_sections::PortalSections,
    ) -> Option<BlockPos> {
        let half = f64::from(width) / 2.0;
        let bounds = horizon_core::geometry::Aabb::new(
            Vec3::new(to.x - half, to.y, to.z - half),
            Vec3::new(to.x + half, to.y + f64::from(height), to.z + half),
        )
        .deflate(Vec3::new(
            f64::from(1.0e-5_f32),
            f64::from(1.0e-5_f32),
            f64::from(1.0e-5_f32),
        ));
        // Every block the sweep visits lies in the box covering both ends.
        let reach = bounds
            .union_bounds(bounds.move_by(from.subtract(to)))
            .inflate(Vec3::new(1.0, 1.0, 1.0));
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
                if self
                    .res
                    .behaviours()
                    .portal_axis(self.world.block(p.x, p.y, p.z))
                    .is_some()
                {
                    found = Some(p);
                    return false;
                }
                true
            },
        );
        found
    }
    /// Whether a section `reach` touches has a nether portal state in its
    /// palette: without one, no block there is a portal. Palette answers are
    /// kept per chunk across ticks (`portal_sections`); a box over many
    /// sections is not checked.
    fn may_hold_portal(
        &self,
        reach: horizon_core::geometry::Aabb,
        sections: &mut horizon_client::portal_sections::PortalSections,
    ) -> bool {
        use horizon_client::chunk::{MIN_Y, SECTIONS};
        let (min, max) = (reach.min_position(), reach.max_position());
        if !min.is_finite() || !max.is_finite() {
            return true;
        }
        let (min, max) = (BlockPos::containing(min), BlockPos::containing(max));
        let top = SECTIONS as i32 - 1;
        let (x0, x1) = (min.x >> 4, max.x >> 4);
        let (z0, z1) = (min.z >> 4, max.z >> 4);
        let (y0, y1) = (
            ((min.y - MIN_Y) >> 4).clamp(0, top),
            ((max.y - MIN_Y) >> 4).clamp(0, top),
        );
        let count = (i64::from(x1) - i64::from(x0) + 1)
            * (i64::from(y1) - i64::from(y0) + 1)
            * (i64::from(z1) - i64::from(z0) + 1);
        if count > 64 {
            return true;
        }
        let mut y_mask = 0u32;
        for sy in y0..=y1 {
            y_mask |= 1 << sy;
        }
        let behaviours = self.res.behaviours();
        let chunks = &self.world.chunks;
        (x0..=x1).any(|cx| {
            (z0..=z1).any(|cz| {
                chunks.get(&(cx, cz)).is_some_and(|chunk| {
                    sections.get((cx, cz), chunk, |v| {
                        behaviours
                            .portal_axis(horizon_world::registry::BlockStateId(v))
                            .is_some()
                    }) & y_mask
                        != 0
                })
            })
        })
    }
    /// `Entity.teleportCrossDimension` away from the loaded dimension: the
    /// entity is saved and removed, and waits for its dimension to load.
    fn send_through_portal(&mut self, id: u32, entry: BlockPos) {
        if let Some(i) = self.dropped.iter().position(|e| e.id == id) {
            let e = &self.dropped[i];
            let plan = self.portal_plan(entry, e.position, 0.25, 0.25, false);
            if !self.may_enter(plan.dim) {
                return;
            }
            let uuid = self.uuid_for(id);
            let tag = horizon_client::entity_nbt::item_entity_to_nbt(
                &self.dropped[i],
                uuid,
                self.persist_raw(id),
            );
            self.dropped.remove(i);
            self.forget_entity(id);
            self.portals.arrivals.push(Arrival { plan, tag });
            self.portals.dirty = true;
        } else if let Some(i) = self.mobs.iter().position(|m| m.id == id) {
            let m = &self.mobs[i];
            let plan = self.portal_plan(entry, m.position, m.width(), m.height(), true);
            if !self.may_enter(plan.dim) {
                return;
            }
            let mut tag = self.mob_tree_nbt(i);
            // Entity.removeAfterChangingDimensions drops the leash.
            if let Tag::Compound(c) = &mut tag {
                c.remove(&"leash".into());
            }
            let mut gone = vec![id];
            let mut k = 0;
            while k < gone.len() {
                let v = gone[k];
                gone.extend(
                    self.mobs
                        .iter()
                        .filter(|m| m.extra.mount == Some(v))
                        .map(|m| m.id),
                );
                k += 1;
            }
            self.mobs.retain(|m| !gone.contains(&m.id));
            for g in gone {
                self.forget_entity(g);
            }
            self.portals.arrivals.push(Arrival { plan, tag });
            self.portals.dirty = true;
        }
    }
    /// Entities waiting for the loaded dimension whose exit area is loaded:
    /// their exit portal (found or built), position, turned yaw and motion,
    /// and portal cooldown; they join the world with the chunk entities.
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
            let Arrival { plan, mut tag } = self.portals.arrivals.remove(i);
            self.portals.dirty = true;
            let Some(exit) = self.nether_exit_portal(&plan) else {
                continue;
            };
            let (at, rotation) = self.arrival(&plan, exit);
            let motion = rotate_y(horizon_client::entity_nbt::motion(&tag), -rotation);
            let [yaw, pitch] = horizon_client::entity_nbt::rotation(&tag);
            if let Tag::Compound(c) = &mut tag {
                c.insert("Pos".into(), doubles([at.x, at.y, at.z]));
                c.insert("Motion".into(), doubles([motion.x, motion.y, motion.z]));
                c.insert(
                    "Rotation".into(),
                    Tag::List(vec![
                        Tag::Float((yaw + rotation).to_bits()),
                        Tag::Float(pitch.to_bits()),
                    ]),
                );
                // setPortalCooldown before the teleport, copied by restoreFrom.
                c.insert("PortalCooldown".into(), Tag::Int(ENTITY_DELAY));
            }
            let key = ((at.x.floor() as i32) >> 4, (at.z.floor() as i32) >> 4);
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
        for a in list {
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
            let dim = match horizon_client::save::text(a.get("dimension")).as_deref() {
                Some("minecraft:the_nether") => Dim::Nether,
                Some(_) => Dim::Overworld,
                None => continue,
            };
            let (Some(x), Some(y), Some(z), Some(tag)) =
                (int("x"), int("y"), int("z"), a.get("entity"))
            else {
                continue;
            };
            self.portals.arrivals.push(Arrival {
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
                },
                tag: tag.clone(),
            });
        }
    }
    /// Write the waiting entities when they changed.
    pub(super) fn save_portal_arrivals(&mut self) {
        if !self.portals.dirty || self.remote.is_some() {
            return;
        }
        let list = self
            .portals
            .arrivals
            .iter()
            .map(|a| {
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
                    ("entity", a.tag.clone()),
                ])
            })
            .collect();
        let root = Tag::compound([("arrivals", Tag::List(list))]);
        if horizon_client::save::write_dat(&self.arrivals_path(), root).is_ok() {
            self.portals.dirty = false;
        }
    }
}
