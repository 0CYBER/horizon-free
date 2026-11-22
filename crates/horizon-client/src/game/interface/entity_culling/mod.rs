//! Entity mesh occlusion and view-distance checks.
use super::{Game, Vertex, entity::RenderEntityBounds, particle_culling::section_scene};
use horizon_client::gfx::native_entity::EntityKind;
use horizon_client::graphics::EntityCullingMode;
use horizon_client_gameplay::entity_occlusion::OcclusionHost;
pub(super) use horizon_client_gameplay::entity_occlusion::{
    EntityCullDecision, EntityOcclusion, cull_box, default_range, entity_view_scale,
};
use horizon_core::geometry::Aabb;

/// The orb quad is asymmetric about its translated centre: its farthest corner
/// is sqrt(0.15² + 0.225²) blocks away. A sphere covers every billboard rotation
/// and pickup-animation position; include rounding of camera-relative vertices.
fn orb_billboard_radius(origin: glam::Vec3) -> f32 {
    0.3 + origin.abs().max_element() * f32::EPSILON * 4.0
}

pub(super) fn orb_in_view(view: &horizon_client::gfx::JoinedFrusta, origin: glam::Vec3) -> bool {
    view.intersects_sphere(origin, orb_billboard_radius(origin))
}

/// Who sees an entity this frame: the camera, and the ray scene's entity layer, while it is on
/// and the entity is within its reach (`Renderer::ray_entity_reach`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Sight {
    pub camera: bool,
    pub rays: bool,
}

/// The entity occlusion culling a frame runs: `setting`, or none while a mod's world view draws
/// the level this frame. The culler proves an entity hidden from the player's camera only; a world
/// view looks from elsewhere and would lose entities it sees (a player behind a hill on a map).
fn culling_with_views(setting: EntityCullingMode, world_views: bool) -> EntityCullingMode {
    if world_views {
        EntityCullingMode::Off
    } else {
        setting
    }
}

impl Game<'_> {
    /// `settings.horizon.entity_culling` for this frame (`culling_with_views`).
    pub(super) fn entity_culling_mode(&self) -> EntityCullingMode {
        let views = self.view_frustum.is_some_and(|f| f.has_views());
        culling_with_views(self.settings.horizon.entity_culling, views)
    }

    pub(super) fn prepare_entity_culling(&mut self, camera: glam::DVec3) {
        // The ray scene's entity layer reads the stream's vertices: GPU-posed models have none.
        let reach = self.renderer.as_ref().filter(|_| !self.skinning());
        self.native_shaders.ray_reach = reach.and_then(|r| r.ray_entity_reach(camera));
        self.native_shaders.ray_only.get_mut().clear();
        self.native_shaders.ray_runs.get_mut().clear();
        // The mods' render hooks: whether any listens this frame, and no entity canceled yet.
        super::native_mob_draw::begin_entity_frame();
        self.occlusion
            .prepare_shadow_casters(self.renderer.as_ref(), self.pack_ids.is_some());
    }

    pub(super) fn cull_entity_vertices(&self, vertices: &mut Vec<Vertex>, start: usize) -> bool {
        let mode = self.entity_culling_mode();
        let hidden = self.occlusion_hidden(Some(vertices), start, None);
        if hidden
            && !(self.occlusion.shadow_casters.0 && self.hide_spans_from(start, vertices.len()))
        {
            self.occlusion
                .record_vertices_culled(mode, vertices.len().saturating_sub(start));
            self.drop_culled(vertices, start);
        }
        hidden
    }

    /// Drops a culled model, vertices `start..`: to the ray scene's entity layer instead, when
    /// `culled_for_rays` keeps it and the layer reaches it.
    fn drop_culled(&self, vertices: &mut Vec<Vertex>, start: usize) {
        if self.native_shaders.ray_keeps.get()
            && let Some(reach) = &self.native_shaders.ray_reach
        {
            let corners = vertices[start..].iter().map(|v| glam::Vec3::from(v.pos));
            let (lo, hi) = corners.fold((glam::Vec3::MAX, glam::Vec3::MIN), |(lo, hi), p| {
                (lo.min(p), hi.max(p))
            });
            if reach.intersects(lo, hi) {
                return self.ray_only(vertices, start);
            }
        }
        vertices.truncate(start);
    }

    /// Runs `cull`, which culls one model of the entity layer's kinds: occlusion moves it to the
    /// layer instead of dropping it, while the layer is on (`drop_culled`).
    pub(super) fn culled_for_rays<R>(&self, cull: impl FnOnce() -> R) -> R {
        let keeps = &self.native_shaders.ray_keeps;
        keeps.set(self.native_shaders.ray_reach.is_some());
        let culled = cull();
        keeps.set(false);
        culled
    }

    /// Moves vertices `start..` of `out`, one model the camera doesn't draw, to the ray scene's
    /// entity layer alone, as an entity's run: what `lit_entity` names of the camera's.
    pub(super) fn ray_only(&self, out: &mut Vec<Vertex>, start: usize) {
        let mut ray = self.native_shaders.ray_only.borrow_mut();
        let first = ray.len();
        ray.extend(out.drain(start..));
        let mut runs = self.native_shaders.ray_runs.borrow_mut();
        runs.push(first..ray.len(), EntityKind::Entity);
    }

    /// The cull decision of an entity `sight` sees, `decide` while the camera does. One the
    /// camera doesn't draw, out of its view or hidden, goes to the entity layer within its reach.
    pub(super) fn sighted_decision(
        &self,
        sight: Sight,
        decide: impl FnOnce() -> EntityCullDecision,
    ) -> EntityCullDecision {
        if !sight.camera {
            return EntityCullDecision::Rays;
        }
        match decide() {
            EntityCullDecision::Hidden if sight.rays => EntityCullDecision::Rays,
            decision => decision,
        }
    }

    /// `finish_entity_occlusion` for a model of the entity layer's kinds (`lit_entity`): one it
    /// takes alone moves there, and so does one occlusion hides within its reach.
    pub(super) fn finish_lit_entity<T: RenderEntityBounds>(
        &self,
        decision: EntityCullDecision,
        entity: &T,
        vertices: &mut Vec<Vertex>,
        start: usize,
    ) {
        if decision.rays_only() {
            return self.ray_only(vertices, start);
        }
        self.culled_for_rays(|| self.finish_entity_occlusion(decision, entity, vertices, start));
    }

    pub(super) fn cull_entity_camera_bounds(&self, lo: glam::Vec3, hi: glam::Vec3) -> bool {
        if self.occlusion_hidden_camera_bounds(lo, hi) {
            self.occlusion
                .record_vertices_culled(self.entity_culling_mode(), 2);
            return true;
        }
        false
    }

    fn occlusion_hidden_camera_bounds(&self, lo: glam::Vec3, hi: glam::Vec3) -> bool {
        let mode = self.entity_culling_mode();
        self.occlusion
            .hidden_camera_bounds(section_scene!(self), self, mode, lo, hi)
    }

    fn entity_hidden_sync_camera_bounds(&self, lo: glam::Vec3, hi: glam::Vec3) -> bool {
        self.occlusion
            .hidden_sync_camera_bounds(section_scene!(self), lo, hi)
    }

    /// Pre-model visibility hook for dynamic block-entity geometry.
    pub(super) fn block_entity_occluded(&self, bounds: Aabb) -> bool {
        self.occlusion_hidden(None, 0, Some(bounds))
    }

    /// Cull from the shared entity AABB before generating its model vertices.
    pub(super) fn entity_cull_decision<T: RenderEntityBounds>(
        &self,
        entity: &T,
        alpha: f64,
    ) -> EntityCullDecision {
        self.entity_cull_decision_in_bounds(entity, entity.render_bounds(alpha))
    }

    /// Cull from a renderer-computed envelope when the rendered transform has
    /// extra offsets or model geometry that the entity type alone cannot see.
    pub(super) fn entity_cull_decision_in_bounds<T: RenderEntityBounds>(
        &self,
        entity: &T,
        render_bounds: Aabb,
    ) -> EntityCullDecision {
        let mode = self.entity_culling_mode();
        if self.occlusion.bypasses(mode) || entity.render_glowing(self) {
            return EntityCullDecision::Bypass;
        }
        self.occlusion.decide(
            section_scene!(self),
            self,
            mode,
            entity.render_id(),
            render_bounds,
        )
    }

    /// Refine an AABB-visible result with the exact bounds of the rendered mesh.
    pub(super) fn finish_entity_occlusion<T: RenderEntityBounds>(
        &self,
        decision: EntityCullDecision,
        entity: &T,
        vertices: &mut Vec<Vertex>,
        start: usize,
    ) {
        if decision.checks_vertices()
            && !self.occlusion.shadow_casters.0
            && !entity.render_glowing(self)
        {
            self.cull_entity_vertices(vertices, start);
        }
    }

    /// Reject entities whose padded render bounds lie entirely in sections the
    /// terrain visibility graph cannot reach. Unknown graph cells fail open.
    fn terrain_sections_occlude(&self, bounds: Aabb) -> bool {
        self.occlusion
            .terrain_sections_occlude(section_scene!(self), bounds)
    }

    fn occlusion_hidden(
        &self,
        vertices: Option<&[Vertex]>,
        start: usize,
        bounds: Option<Aabb>,
    ) -> bool {
        let mode = self.entity_culling_mode();
        self.occlusion
            .hidden(section_scene!(self), self, mode, vertices, start, bounds)
    }

    fn entity_hidden_sync(
        &self,
        vertices: Option<&[Vertex]>,
        start: usize,
        bounds: Option<Aabb>,
    ) -> bool {
        self.occlusion
            .hidden_sync(section_scene!(self), vertices, start, bounds)
    }
}

impl OcclusionHost for Game<'_> {
    fn entity_hidden_sync(
        &self,
        vertices: Option<&[Vertex]>,
        start: usize,
        bounds: Option<Aabb>,
    ) -> bool {
        Game::entity_hidden_sync(self, vertices, start, bounds)
    }
    fn entity_hidden_sync_camera_bounds(&self, lo: glam::Vec3, hi: glam::Vec3) -> bool {
        Game::entity_hidden_sync_camera_bounds(self, lo, hi)
    }
    fn terrain_sections_occlude(&self, bounds: Aabb) -> bool {
        Game::terrain_sections_occlude(self, bounds)
    }
}

impl Game<'_> {
    /// Entity.setViewScale (LevelExtractor.extractVisibleEntities): the entity
    /// distance option times the render distance's boost — 1 at 8 chunks, 2.5
    /// at 20 or more. The render distance is the client setting, capped by
    /// the server's view distance when connected
    /// (Options.getEffectiveRenderDistance).
    pub(super) fn view_scale(&self) -> f64 {
        let distance = self.effective_render_distance();
        entity_view_scale(distance, self.settings.entity_distance)
    }
}

impl Game<'_> {
    /// EntityRenderer.shouldRender: the entity within `range` blocks of the
    /// camera, measured from its current `position`, and its culling box
    /// (`lo`..`hi`, interpolated, in world space) grown by 0.5 in the view;
    /// and whether the ray scene's entity layer reaches that box (`sight`).
    /// `None` when neither sees it.
    pub(super) fn entity_sight(
        &self,
        camera: glam::DVec3,
        position: glam::DVec3,
        range: f64,
        lo: glam::DVec3,
        hi: glam::DVec3,
    ) -> Option<Sight> {
        if (position - camera).length_squared() >= range * range {
            return None;
        }
        self.sight(
            (lo - camera - glam::DVec3::splat(0.5)).as_vec3(),
            (hi - camera + glam::DVec3::splat(0.5)).as_vec3(),
        )
    }

    /// Who sees the camera-relative box `lo..hi`: the view's frustum, and the entity layer's
    /// reach while it is on. `None` when neither does.
    pub(super) fn sight(&self, lo: glam::Vec3, hi: glam::Vec3) -> Option<Sight> {
        let camera = self
            .view_frustum
            .as_ref()
            .is_none_or(|v| v.intersects(lo, hi));
        let reach = self.native_shaders.ray_reach.as_ref();
        let rays = reach.is_some_and(|r| r.intersects(lo, hi));
        (camera || rays).then_some(Sight { camera, rays })
    }
}

/// An entity's position as a `DVec3`.
pub(super) fn at(p: horizon_core::geometry::Vec3) -> glam::DVec3 {
    glam::DVec3::new(p.x, p.y, p.z)
}

impl Game<'_> {
    /// `entity_sight`'s camera for an entity whose box stands on its position,
    /// `width` wide and `height` tall, interpolated by `alpha` and grown by
    /// `grow` (a renderer's getBoundingBoxForCulling).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn standing_entity_in_view(
        &self,
        camera: glam::DVec3,
        previous: glam::DVec3,
        position: glam::DVec3,
        alpha: f64,
        width: f64,
        height: f64,
        range: f64,
        grow: f64,
    ) -> bool {
        let sight = self.standing_entity_sight(
            camera, previous, position, alpha, width, height, range, grow,
        );
        sight.is_some_and(|s| s.camera)
    }

    /// `standing_entity_in_view`, and whether the ray scene's entity layer reaches the entity
    /// (`entity_sight`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn standing_entity_sight(
        &self,
        camera: glam::DVec3,
        previous: glam::DVec3,
        position: glam::DVec3,
        alpha: f64,
        width: f64,
        height: f64,
        range: f64,
        grow: f64,
    ) -> Option<Sight> {
        let p = previous.lerp(position, alpha);
        let half = glam::DVec3::new(width / 2.0 + grow, grow, width / 2.0 + grow);
        let lo = p - half;
        let hi = p + glam::DVec3::new(half.x, height + grow, half.z);
        self.entity_sight(camera, position, range, lo, hi)
    }
}

impl Game<'_> {
    pub(super) fn prepare_occlusion_culling(&mut self, camera: glam::DVec3) {
        let mode = self.entity_culling_mode();
        let requested = mode != EntityCullingMode::Off;
        let eligible = requested
            // Packs can replace opaque models, so their terrain is not a safe occluder.
            && self.model_packs_are_vanilla()
            && self.player.is_some()
            && !self.spectator();
        let cave_culling = self.settings.horizon.cave_culling;
        self.occlusion
            .prepare(section_scene!(self), mode, camera, eligible, cave_culling);
    }

    pub(super) fn finish_async_occlusion_culling(&mut self) {
        self.occlusion.finish_async(section_scene!(self));
    }
}

#[cfg(test)]
mod precheck_envelope_tests {
    use super::super::entity::RenderEntityBounds;
    use horizon_client::mobs::{Mob, MobData, MobTypes, Species, render};
    use horizon_core::geometry::Vec3;
    use std::{path::Path, sync::Arc};

    #[test]
    fn vanilla_models_and_animations_stay_inside_the_precheck_envelope() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            eprintln!("no prepared resources: skipped model coverage");
            return;
        }
        let types = MobTypes::load(&root).unwrap();
        let data = MobData {
            resolved: Default::default(),
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: MobData::load_animations(&root),
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
            mod_spawns: Default::default(),
        };
        let camera = glam::DVec3::new(3.0, 62.0, -4.0);
        let mut checked = 0;
        for kind in types.types.values() {
            for sample in 0..12 {
                let mut mob = Mob::new(1, Arc::new(kind.clone()), Vec3::new(1.5, 64.0, 3.25), 37.0);
                mob.previous = Vec3::new(1.25, 64.25, 3.0);
                mob.age = sample * 37;
                mob.limb_speed = 0.8;
                mob.limb_position = sample as f32 * 1.7;
                mob.head_pitch = if sample % 2 == 0 { 85.0 } else { -85.0 };
                mob.extra.age = if sample >= 6 { -100 } else { 0 };
                mob.death_time = if sample % 3 == 0 { 10 } else { 0 };
                mob.extra.flag = sample % 2 == 0;
                mob.extra.size = match mob.species() {
                    Species::Slime
                    | Species::MagmaCube
                    | Species::SulfurCube
                    | Species::Phantom => {
                        if sample % 2 == 0 {
                            1
                        } else {
                            4
                        }
                    }
                    _ => mob.extra.size,
                };
                if sample % 3 == 1 {
                    mob.custom_name = Some("Dinnerbone".into());
                }
                for partial in [0.0, 0.4, 1.0] {
                    let bounds = mob.render_bounds(f64::from(partial));
                    let lo = bounds.min_position();
                    let hi = bounds.max_position();
                    let lo = [lo.x, lo.y, lo.z];
                    let hi = [hi.x, hi.y, hi.z];
                    let mut vertices = Vec::new();
                    render::render_with(
                        &mob,
                        partial,
                        camera,
                        &|_| Some([0.0, 0.0, 1.0, 1.0]),
                        0xF0F0,
                        &data,
                        &Default::default(),
                        &[],
                        &mut vertices,
                    );
                    for vertex in &vertices {
                        for axis in 0..3 {
                            let world = f64::from(vertex.pos[axis]) + camera[axis];
                            assert!(
                                world >= lo[axis] - 1.0e-4 && world <= hi[axis] + 1.0e-4,
                                "{} sample {sample} partial {partial}: {world} outside {lo:?}..{hi:?}",
                                kind.id,
                            );
                        }
                    }
                    checked += usize::from(!vertices.is_empty());
                }
            }
        }
        assert!(
            checked > 1_000,
            "insufficient real model coverage: {checked}"
        );
    }
}

#[cfg(test)]
mod orb_tests {
    use super::orb_billboard_radius;

    #[test]
    fn sphere_contains_the_asymmetric_billboard_including_vertex_rounding() {
        use glam::Vec3;
        use horizon_client::entities::{ORB_QUAD, ORB_QUAD_SCALE};

        for origin in [
            Vec3::ZERO,
            Vec3::new(-47.25, 12.5, 81.0),
            Vec3::new(30_000_000.0, -30_000_000.0, 256.0),
        ] {
            let radius = orb_billboard_radius(origin);
            for yaw in (0..360).step_by(15) {
                for pitch in [-90.0_f32, -89.99, -45.0, 0.0, 45.0, 89.99, 90.0] {
                    let (yaw, pitch) = ((yaw as f32).to_radians(), pitch.to_radians());
                    let forward = Vec3::new(
                        -yaw.sin() * pitch.cos(),
                        -pitch.sin(),
                        yaw.cos() * pitch.cos(),
                    );
                    let right = forward.cross(Vec3::Y).normalize_or_zero();
                    let up = right.cross(forward).normalize_or_zero();
                    for corner in ORB_QUAD {
                        let vertex = origin
                            + right * (corner[0] * ORB_QUAD_SCALE)
                            + up * (corner[1] * ORB_QUAD_SCALE);
                        assert!(
                            vertex.distance(origin) <= radius,
                            "{origin:?}, yaw {yaw}, pitch {pitch}: {vertex:?} outside {radius}"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "ray_only_tests.rs"]
mod ray_only_tests;

#[cfg(test)]
mod view_tests {
    use super::culling_with_views;
    use horizon_client::graphics::EntityCullingMode;

    #[test]
    fn a_world_view_turns_entity_occlusion_off_for_its_frame() {
        for setting in EntityCullingMode::ALL {
            assert_eq!(culling_with_views(setting, false), setting);
            assert_eq!(culling_with_views(setting, true), EntityCullingMode::Off);
        }
    }
}
