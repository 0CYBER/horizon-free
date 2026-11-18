//! Entity mesh occlusion: the culler state kept between frames and the decisions it makes.
use crate::{
    occlusion_culling::{AsyncOcclusionCuller, QuerySource, SyncOccluder, opaque},
    section_scene::SectionScene,
};
use horizon_client::{
    gfx::Renderer, graphics::EntityCullingMode, mesher::Vertex, mobs::Species,
    voxel_occlusion::Culler as OcclusionCuller,
};
use horizon_core::geometry::{Aabb, Vec3};

const MAX_ENTITY_SECTION_COVERAGE: i64 = 50;

/// The inclusive range of sections an entity's padded render bounds can touch,
/// in the layout whose section 0 starts at `bottom_y`. `None` when the bounds
/// are not finite, hold block coordinates outside `i32`, or span more sections
/// than `MAX_ENTITY_SECTION_COVERAGE`: those go to the per-entity path.
fn entity_section_span(bounds: Aabb, bottom_y: i32) -> Option<([i32; 3], [i32; 3])> {
    // A half-block border keeps models near section edges checked against
    // every section their geometry could touch.
    let lo = bounds.min_position();
    let hi = bounds.max_position();
    let min_block = [lo.x - 0.5, lo.y - 0.5, lo.z - 0.5];
    let max_block = [hi.x + 0.5, hi.y + 0.5, hi.z + 0.5];
    if min_block
        .into_iter()
        .chain(max_block)
        .any(|v| !v.is_finite() || v < f64::from(i32::MIN) || v > f64::from(i32::MAX))
    {
        return None;
    }
    let min_block = min_block.map(|v| v.floor() as i32);
    let max_block = max_block.map(|v| v.floor() as i32);
    let min_y = min_block[1].checked_sub(bottom_y)?;
    let max_y = max_block[1].checked_sub(bottom_y)?;
    let min = [min_block[0] >> 4, min_y >> 4, min_block[2] >> 4];
    let max = [max_block[0] >> 4, max_y >> 4, max_block[2] >> 4];
    let sections = (0..3).try_fold(1_i64, |count, axis| {
        count.checked_mul(i64::from(max[axis]) - i64::from(min[axis]) + 1)
    });
    // Very large render envelopes (for example, beams) are cheaper and safer to
    // leave to the existing per-entity path.
    sections
        .is_some_and(|count| count <= MAX_ENTITY_SECTION_COVERAGE)
        .then_some((min, max))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EntityCullDecision {
    Bypass,
    Hidden,
    CheckVertices,
    Pending,
    /// The camera doesn't draw the entity, out of its view or hidden, and the ray scene's entity
    /// layer reaches it: its model goes to the layer alone (`Game::ray_only`).
    Rays,
}

impl EntityCullDecision {
    pub const fn hides_entity(self) -> bool {
        matches!(self, Self::Hidden)
    }

    pub const fn checks_vertices(self) -> bool {
        matches!(self, Self::CheckVertices)
    }

    pub const fn rays_only(self) -> bool {
        matches!(self, Self::Rays)
    }
}

/// The entity culler's state for this frame and what it keeps between frames.
#[derive(Default)]
pub struct EntityOcclusion {
    /// Whether the renderer's section-occlusion graph is active for this view.
    section_visibility_valid: bool,
    /// The camera section in the current world layout.
    section_camera: Option<(i32, i32, i32)>,
    sync: Option<OcclusionCuller>,
    asynchronous: Option<Box<AsyncOcclusionCuller>>,
    async_failed: bool,
    /// `Renderer::pack_shadow_casters` for this frame's entities: (keep what the culler hides,
    /// build the player's model in first person); (false, false) without a shader pack.
    pub shadow_casters: (bool, bool),
}

fn sync_occluder<'a>(scene: SectionScene<'a>) -> SyncOccluder<'a> {
    SyncOccluder::new(
        scene.world,
        scene.facts,
        scene.renderer,
        scene.streaming,
        scene.pending_uploads,
    )
}

impl EntityOcclusion {
    pub fn asynchronous(&self) -> Option<&AsyncOcclusionCuller> {
        self.asynchronous.as_deref()
    }

    pub fn prepare_shadow_casters(&mut self, renderer: Option<&Renderer>, has_pack: bool) {
        // Spans tell the pack's passes which entities are hidden; without them, nothing changes.
        self.shadow_casters = match (renderer, has_pack) {
            (Some(r), true) => r.pack_shadow_casters(),
            // A native pack's shadow pass has no spans to hide entities with: it only needs
            // the first-person model.
            (Some(r), false) => (false, r.native_shadow()),
            _ => (false, false),
        };
    }

    /// Culling off, or a shader pack keeping what the culler hides: every entity is drawn.
    pub fn bypasses(&self, mode: EntityCullingMode) -> bool {
        mode == EntityCullingMode::Off || self.shadow_casters.0
    }

    /// Counts `count` entity vertices dropped, after `hidden` said so.
    pub fn record_vertices_culled(&self, mode: EntityCullingMode, count: usize) {
        if mode == EntityCullingMode::Asynchronous
            && let Some(culler) = &self.asynchronous
        {
            culler.record_entity_vertices_culled(count);
        }
    }

    pub fn cull_camera_bounds(
        &self,
        scene: SectionScene<'_>,
        mode: EntityCullingMode,
        lo: glam::Vec3,
        hi: glam::Vec3,
    ) -> bool {
        if self.hidden_camera_bounds(scene, mode, lo, hi) {
            if mode == EntityCullingMode::Asynchronous
                && let Some(culler) = &self.asynchronous
            {
                culler.record_entity_vertices_culled(2);
            }
            return true;
        }
        false
    }

    fn hidden_camera_bounds(
        &self,
        scene: SectionScene<'_>,
        mode: EntityCullingMode,
        lo: glam::Vec3,
        hi: glam::Vec3,
    ) -> bool {
        match mode {
            EntityCullingMode::Off => false,
            EntityCullingMode::Asynchronous => {
                if let Some(culler) = &self.asynchronous {
                    if !culler.camera_is_stationary() {
                        return false;
                    }
                    let Some(query) = culler.query_camera_bounds(lo, hi) else {
                        return false;
                    };
                    match culler.query_result(&query, QuerySource::EntityMesh) {
                        Some((false, _)) => false,
                        Some((true, Some(proof)))
                            if culler.proof_is_current(
                                scene.world,
                                scene.renderer,
                                scene.streaming,
                                scene.pending_uploads,
                                proof,
                            ) =>
                        {
                            true
                        }
                        Some(_) | None => {
                            culler.queue(query);
                            false
                        }
                    }
                } else {
                    self.hidden_sync_camera_bounds(scene, lo, hi)
                }
            }
            EntityCullingMode::Synchronous => self.hidden_sync_camera_bounds(scene, lo, hi),
        }
    }

    fn hidden_sync_camera_bounds(
        &self,
        scene: SectionScene<'_>,
        lo: glam::Vec3,
        hi: glam::Vec3,
    ) -> bool {
        self.sync.as_ref().is_some_and(|culler| {
            let mut occluder = sync_occluder(scene);
            culler.hidden_camera_bounds(lo.to_array(), hi.to_array(), |pos| occluder.ready(pos))
        })
    }

    /// `entity_cull_decision_in_bounds` once the bypass and glowing checks let it through.
    pub fn decide(
        &self,
        scene: SectionScene<'_>,
        mode: EntityCullingMode,
        render_id: u32,
        render_bounds: Aabb,
    ) -> EntityCullDecision {
        // `render_bounds` is already the conservative model and animation
        // envelope. Only cover f32 vertex rounding here; another half-block
        // would make coarse walls falsely visible and force mesh generation.
        let margin = Vec3::new(1.0e-4, 1.0e-4, 1.0e-4);
        let bounds = render_bounds.inflate(margin);
        if self.terrain_sections_occlude(scene, bounds) {
            if let Some(culler) = &self.asynchronous {
                culler.record_entity_culled();
            }
            return EntityCullDecision::Hidden;
        }
        match mode {
            EntityCullingMode::Off => EntityCullDecision::Bypass,
            EntityCullingMode::Synchronous => {
                if self.hidden_sync(scene, None, 0, Some(bounds)) {
                    EntityCullDecision::Hidden
                } else {
                    EntityCullDecision::CheckVertices
                }
            }
            EntityCullingMode::Asynchronous => {
                let Some(culler) = &self.asynchronous else {
                    return if self.hidden_sync(scene, None, 0, Some(bounds)) {
                        EntityCullDecision::Hidden
                    } else {
                        EntityCullDecision::CheckVertices
                    };
                };
                if !culler.camera_is_stationary() {
                    return EntityCullDecision::CheckVertices;
                }
                let Some(query) = culler.query_entity_bounds(bounds) else {
                    return EntityCullDecision::CheckVertices;
                };
                culler.sample_entity_aabb(render_id, &query);
                match culler.query_result(&query, QuerySource::EntityAabb) {
                    Some((false, _)) => EntityCullDecision::CheckVertices,
                    Some((true, Some(proof)))
                        if culler.proof_is_current(
                            scene.world,
                            scene.renderer,
                            scene.streaming,
                            scene.pending_uploads,
                            proof,
                        ) =>
                    {
                        culler.record_entity_culled();
                        EntityCullDecision::Hidden
                    }
                    Some(_) | None => {
                        culler.queue(query);
                        EntityCullDecision::Pending
                    }
                }
            }
        }
    }

    /// Reject entities whose padded render bounds lie entirely in sections the
    /// terrain visibility graph cannot reach. Unknown graph cells fail open.
    fn terrain_sections_occlude(&self, scene: SectionScene<'_>, bounds: Aabb) -> bool {
        if !self.section_visibility_valid || (self.sync.is_none() && self.asynchronous.is_none()) {
            return false;
        }
        let Some(renderer) = scene.renderer else {
            return false;
        };
        let Some(camera_section) = self.section_camera else {
            return false;
        };
        let Some((min, max)) = entity_section_span(bounds, scene.world.bottom_y()) else {
            return false;
        };

        for x in min[0]..=max[0] {
            for y in min[1]..=max[1] {
                for z in min[2]..=max[2] {
                    let Some(visible) =
                        renderer.section_occlusion_visible((x, y, z), camera_section)
                    else {
                        return false;
                    };
                    if visible {
                        return false;
                    }
                }
            }
        }
        true
    }

    pub fn hidden(
        &self,
        scene: SectionScene<'_>,
        mode: EntityCullingMode,
        vertices: Option<&[Vertex]>,
        start: usize,
        bounds: Option<Aabb>,
    ) -> bool {
        match mode {
            EntityCullingMode::Off => false,
            EntityCullingMode::Synchronous => self.hidden_sync(scene, vertices, start, bounds),
            EntityCullingMode::Asynchronous => {
                if let Some(culler) = &self.asynchronous {
                    if !culler.camera_is_stationary() {
                        return false;
                    }
                    let Some(query) = (match bounds {
                        Some(bounds) => culler.query_entity_bounds(bounds),
                        None => vertices.and_then(|vertices| culler.query(vertices, start)),
                    }) else {
                        return false;
                    };
                    match culler.query_result(&query, QuerySource::EntityMesh) {
                        Some((false, _)) => false,
                        Some((true, Some(proof)))
                            if culler.proof_is_current(
                                scene.world,
                                scene.renderer,
                                scene.streaming,
                                scene.pending_uploads,
                                proof,
                            ) =>
                        {
                            true
                        }
                        Some(_) | None => {
                            culler.queue(query);
                            false
                        }
                    }
                } else {
                    self.hidden_sync(scene, vertices, start, bounds)
                }
            }
        }
    }

    fn hidden_sync(
        &self,
        scene: SectionScene<'_>,
        vertices: Option<&[Vertex]>,
        start: usize,
        bounds: Option<Aabb>,
    ) -> bool {
        self.sync.as_ref().is_some_and(|culler| {
            let mut occluder = sync_occluder(scene);
            match bounds {
                Some(bounds) => culler.hidden_bounds(bounds, |pos| occluder.ready(pos)),
                None => vertices.is_some_and(|vertices| {
                    culler.hidden(vertices, start, |pos| occluder.ready(pos))
                }),
            }
        })
    }

    /// `eligible`: culling is on, the model packs are vanilla, and a non-spectator player looks.
    pub fn prepare(
        &mut self,
        scene: SectionScene<'_>,
        mode: EntityCullingMode,
        camera: glam::DVec3,
        eligible: bool,
        cave_culling: bool,
    ) {
        self.section_camera = None;
        self.section_visibility_valid = false;
        if !eligible {
            self.sync = None;
            self.asynchronous = None;
            self.async_failed = false;
            return;
        }
        let at = camera.to_array().map(|v| v.floor() as i32);
        self.section_camera = at[1]
            .checked_sub(scene.world.bottom_y())
            .map(|y| (at[0] >> 4, y >> 4, at[2] >> 4));
        let camera_facts = scene.facts.get(scene.world.block(at[0], at[1], at[2]));
        if opaque(camera_facts) {
            self.sync = None;
            self.asynchronous = None;
            self.async_failed = false;
            return;
        }
        self.section_visibility_valid =
            cave_culling && !camera_facts.solid_render && self.section_camera.is_some();

        let wants_async = mode == EntityCullingMode::Asynchronous;
        let wants_sync = mode == EntityCullingMode::Synchronous;
        if wants_async {
            if self.asynchronous.is_none() && !self.async_failed {
                match AsyncOcclusionCuller::new() {
                    Ok(culler) => self.asynchronous = Some(Box::new(culler)),
                    Err(error) => {
                        log::warn!("Horizon: cannot start world-occlusion-culling worker: {error}");
                        self.async_failed = true;
                    }
                }
            }
            if let Some(culler) = &mut self.asynchronous {
                culler.begin_frame(camera, scene.world);
            }
        } else {
            self.asynchronous = None;
            self.async_failed = false;
        }

        let async_unavailable = wants_async && self.asynchronous.is_none();
        self.sync = (wants_sync || async_unavailable)
            .then(|| OcclusionCuller::new(camera.to_array(), 8_192));
    }

    pub fn finish_async(&mut self, scene: SectionScene<'_>) {
        if let Some(culler) = &mut self.asynchronous {
            culler.finish_frame(scene.world, scene.facts);
        }
    }
}

/// Entity.setViewScale: `Mth.clamp(renderDistance / 8, 1, 2.5) *
/// entityDistanceScaling`. Every entity's draw distance is scaled by it, so
/// a small mob is drawn nearer than a big one and a longer render distance
/// draws both further away.
pub fn entity_view_scale(render_distance: i32, entity_distance: f32) -> f64 {
    (f64::from(render_distance) / 8.0).clamp(1.0, 2.5) * f64::from(entity_distance)
}

/// Entity.shouldRenderAtSqrDistance's default range: 64 times the bounding
/// box's mean edge, scaled by the view scale.
pub fn default_range(size: glam::DVec3, view_scale: f64) -> f64 {
    let edge = (size.x + size.y + size.z) / 3.0;
    let edge = if edge.is_nan() { 1.0 } else { edge };
    edge * 64.0 * view_scale
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CullGrow {
    side: f64,
    all: f64,
    down: f64,
}

impl CullGrow {
    fn of(species: Species, height: f64, dragon_head: bool) -> Self {
        let mut grow = Self::default();
        if dragon_head {
            grow.all += 0.5;
        }
        match species {
            Species::Illusioner => grow.side += 3.0,
            Species::Sniffer => grow.all += f64::from(0.6_f32), // inflate(0.6F)
            Species::HappyGhast => grow.down += height / 2.0,
            _ => {}
        }
        grow
    }
}

fn grown_box(
    pos: glam::DVec3,
    width: f64,
    height: f64,
    grow: CullGrow,
) -> (glam::DVec3, glam::DVec3) {
    let side = width / 2.0 + grow.side + grow.all;
    (
        pos - glam::DVec3::new(side, grow.all + grow.down, side),
        pos + glam::DVec3::new(side, height + grow.all, side),
    )
}

/// `LivingEntityRenderer.getBoundingBoxForCulling` for the mob's interpolated
/// position, including renderer-specific growth for mirrors, riders and heads.
pub fn cull_box(mob: &horizon_client::mobs::Mob, alpha: f64) -> (glam::DVec3, glam::DVec3) {
    let (width, height, _) = mob.dims();
    let grow = CullGrow::of(
        mob.species(),
        height,
        mob.extra.head_id() == Some("minecraft:dragon_head"),
    );
    let pos = glam::DVec3::new(
        mob.previous.x + (mob.position.x - mob.previous.x) * alpha,
        mob.previous.y + (mob.position.y - mob.previous.y) * alpha,
        mob.previous.z + (mob.position.z - mob.previous.z) * alpha,
    );
    grown_box(pos, width, height, grow)
}

#[cfg(test)]
mod vanilla_entity_tests {
    use super::*;

    /// `Entity.setViewScale`: the entity distance option times the render
    /// distance's boost — 1 at eight chunks, 1.5 at twelve, 2.5 at twenty.
    #[test]
    fn the_view_scale_follows_the_render_distance() {
        assert_eq!(entity_view_scale(4, 1.0), 1.0);
        assert_eq!(entity_view_scale(8, 1.0), 1.0);
        assert_eq!(entity_view_scale(12, 1.0), 1.5);
        assert_eq!(entity_view_scale(20, 1.0), 2.5);
        assert_eq!(entity_view_scale(32, 1.0), 2.5);
        // The entity distance option scales it down.
        assert_eq!(entity_view_scale(12, 0.5), 0.75);
    }

    /// `Entity.shouldRenderAtSqrDistance`: a chicken's box (0.4 by 0.7 by
    /// 0.4) is drawn up to 48 blocks away at the vanilla defaults, not the
    /// 32 blocks a missing view scale leaves (arismoko/horizon#2273).
    #[test]
    fn a_chicken_is_drawn_forty_eight_blocks_away() {
        let range = default_range(glam::DVec3::new(0.4, 0.7, 0.4), entity_view_scale(12, 1.0));
        assert_eq!(range, 48.0);
    }

    /// `Entity.shouldRenderAtSqrDistance` for a dropped item (`ItemEntity`
    /// keeps the default and its box is 0.25 on each side): 24 blocks at
    /// render distance 12, 32 at 16, 40 from 20 on; never 64.
    #[test]
    fn a_dropped_item_is_drawn_sixteen_blocks_times_the_view_scale() {
        let item = glam::DVec3::splat(0.25);
        assert_eq!(default_range(item, entity_view_scale(12, 1.0)), 24.0);
        assert_eq!(default_range(item, entity_view_scale(16, 1.0)), 32.0);
        assert_eq!(default_range(item, entity_view_scale(32, 1.0)), 40.0);
    }

    /// `getBoundingBoxForCulling`: only four living renderers grow the box.
    /// An illusioner asks for three blocks either side (room for its mirror
    /// images, which are drawn outside its own box), a sniffer for 0.6 all
    /// round, a happy ghast for half its height below (its riders hang under
    /// it) and anyone wearing a dragon head for 0.5 all round.
    #[test]
    fn only_the_four_living_renderers_grow_the_culling_box() {
        assert_eq!(
            CullGrow::of(Species::Villager, 1.95, false),
            CullGrow::default()
        );
        let illusioner = CullGrow::of(Species::Illusioner, 1.95, false);
        assert_eq!(
            (illusioner.side, illusioner.all, illusioner.down),
            (3.0, 0.0, 0.0)
        );
        let sniffer = CullGrow::of(Species::Sniffer, 1.75, false);
        assert!((sniffer.all - 0.6).abs() < 1e-7 && sniffer.side == 0.0);
        let ghast = CullGrow::of(Species::HappyGhast, 4.0, false);
        assert_eq!((ghast.down, ghast.all), (2.0, 0.0));
        // A dragon head grows every axis, on any living entity.
        let headed = CullGrow::of(Species::Villager, 1.95, true);
        assert_eq!((headed.all, headed.side, headed.down), (0.5, 0.0, 0.0));
        // The growths add up: an illusioner in a dragon head gets both.
        let both = CullGrow::of(Species::Illusioner, 1.95, true);
        assert_eq!((both.all, both.side), (0.5, 3.0));
    }

    /// The box the frustum test sees: an illusioner's own box is 0.6 wide, so
    /// its culling box reaches 3.3 blocks either side of its position and
    /// 0.5 below its feet, where a villager's reaches 0.8.
    #[test]
    fn an_illusioner_is_culled_three_blocks_wider_than_its_box() {
        let pos = glam::DVec3::new(10.0, 64.0, -4.0);
        let (lo, hi) = grown_box(
            pos,
            0.6,
            1.95,
            CullGrow::of(Species::Illusioner, 1.95, false),
        );
        assert_eq!(lo, glam::DVec3::new(10.0 - 3.3, 64.0, -4.0 - 3.3));
        assert_eq!(hi, glam::DVec3::new(10.0 + 3.3, 64.0 + 1.95, -4.0 + 3.3));
        let (lo, hi) = grown_box(pos, 0.6, 1.95, CullGrow::of(Species::Villager, 1.95, false));
        assert!((lo.x - (10.0 - 0.3)).abs() < 1e-9);
        assert!((hi.y - (64.0 + 1.95)).abs() < 1e-9);
        // A happy ghast's box drops by half its height below its feet.
        let (lo, hi) = grown_box(pos, 4.0, 4.0, CullGrow::of(Species::HappyGhast, 4.0, false));
        assert_eq!(lo.y, 62.0);
        assert_eq!(hi.y, 68.0);
    }

    /// `Entity.shouldRenderAtSqrDistance` on a broken box: the size falls
    /// back to one block.
    #[test]
    fn a_broken_bounding_box_falls_back_to_one_block() {
        let range = default_range(
            glam::DVec3::new(f64::NAN, 0.7, 0.4),
            entity_view_scale(12, 1.0),
        );
        assert_eq!(range, 96.0);
    }

    /// The sections an entity's padded bounds can touch: the half-block border
    /// a model near a section edge reaches into, the layout `bottom_y` sets,
    /// and the boxes that get no answer and stay on the per-entity path.
    #[test]
    fn entity_section_spans_cover_the_border_and_fail_open() {
        let span = |lo: [f64; 3], hi: [f64; 3], bottom_y: i32| {
            entity_section_span(
                Aabb::new(
                    Vec3::new(lo[0], lo[1], lo[2]),
                    Vec3::new(hi[0], hi[1], hi[2]),
                ),
                bottom_y,
            )
        };

        // A chicken well inside one section stays in it.
        assert_eq!(
            span([100.2, -63.4, 7.1], [100.6, -62.9, 7.5], -64),
            Some(([6, 0, 0], [6, 0, 0]))
        );
        // A box 0.8 wide inside the next section does not spill out of it.
        assert_eq!(
            span([16.6, -60.0, 100.6], [17.4, -59.4, 101.4], -64),
            Some(([1, 0, 6], [1, 0, 6]))
        );
        // On a section border the model reaches the section either side.
        assert_eq!(
            span([16.0, -60.0, 100.0], [16.0, -59.0, 100.0], -64),
            Some(([0, 0, 6], [1, 0, 6]))
        );
        // `bottom_y` places section 0: the same box in the Nether.
        assert_eq!(
            span([100.2, -63.4, 7.1], [100.6, -62.9, 7.5], 0),
            Some(([6, -4, 0], [6, -4, 0]))
        );
        // Not finite, beyond `i32`, or more sections than the coverage cap:
        // no answer.
        assert_eq!(span([f64::NAN, 0.0, 0.0], [f64::NAN, 1.0, 1.0], -64), None);
        assert_eq!(span([0.0; 3], [f64::INFINITY; 3], -64), None);
        assert_eq!(
            span([0.0; 3], [f64::from(i32::MAX) + 1.0, 1.0, 1.0], -64),
            None
        );
        // A beam across the world spans far more than the cap.
        assert_eq!(span([0.0, -63.0, 0.0], [1.0e6, -62.0, 1.0], -64), None);
    }
}
#[cfg(test)]
#[path = "entity_occlusion_tests.rs"]
mod tests;
