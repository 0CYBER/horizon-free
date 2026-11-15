// game-module-visibility: pub
//! Mod block entities drawn on the client: `native_block_entity_submit`
//! (vanilla's `BlockEntityRenderer.submit`) gets a [`BlockEntityDraw`], whose
//! pose stack (`PoseStack`) places what the helpers here add to the entity
//! batch, as vanilla's block entity renderers draw.
pub use mod_api::game::CubeFace;

use super::{Dim, Game};
use glam::{DVec3, Mat4, Quat, Vec3};
use horizon_assets::{
    bake::{bake_flat_face, default_uv},
    item_model::DisplayContext,
    model::{Direction, Material},
};
use horizon_client::{
    block_entity_render::{TEXT_OVER_OUTLINE, diffuse, diffuse_lights, vertex_light},
    containers::{BlockEntity, ModRevision},
    fast_hash::FxHashMap,
    gfx::Cull,
    hud::Rect,
    items::ItemStack,
    mesher::Vertex,
    mobs::model_layers,
    text::Text,
};
use horizon_world::registry::BlockStateId;
#[cfg(test)]
use mod_api::prelude::*;
use mod_api::{Identifier, ModelError, ModelPart, ModelRef};

/// Atlas-resolved standalone geometry, indexed by model slot and resource generation.
pub(super) struct StandaloneGeometry {
    generation: u64,
    slots: Vec<Option<Vec<super::minecarts::PreparedModelFace>>>,
    translucent: Vec<bool>,
    bounds: Vec<[[f32; 3]; 2]>,
}

/// Whether a resolved slot can take instances, and the local box used to cull copies.
pub(super) struct InstanceSlot {
    pub translucent: bool,
    pub bounds: [[f32; 3]; 2],
}

fn face_bounds(faces: &[super::minecarts::PreparedModelFace]) -> [[f32; 3]; 2] {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for face in faces {
        for pos in face.positions() {
            for axis in 0..3 {
                min[axis] = min[axis].min(pos[axis]);
                max[axis] = max[axis].max(pos[axis]);
            }
        }
    }
    if min[0].is_finite() {
        [min, max]
    } else {
        [[0.0; 3], [0.0; 3]]
    }
}

impl StandaloneGeometry {
    pub(super) fn new(pack: &super::PackResources, gui: &super::UiResources) -> Self {
        let catalog = &pack.render.model_catalog;
        let mut slots = Vec::new();
        let mut translucent = Vec::new();
        let mut bounds = Vec::new();
        for slot in catalog.standalone_slots() {
            let index = slot as usize;
            if slots.len() <= index {
                slots.resize_with(index + 1, || None);
                translucent.resize(index + 1, false);
                bounds.resize(index + 1, [[0.0; 3], [0.0; 3]]);
            }

            let model = catalog.model_by_slot(slot).expect("standalone slot exists");
            let prepared: Vec<_> = model
                .faces
                .iter()
                .filter_map(|face| {
                    let sprite = pack
                        .layer_names
                        .get(&face.layer)
                        .and_then(|name| gui.atlas.sprites.get(name))?;
                    Some(super::minecarts::PreparedModelFace::new(
                        &face.quad, *sprite,
                    ))
                })
                .collect();
            translucent[index] = model.faces.iter().any(|face| face.translucent);
            bounds[index] = face_bounds(&prepared);
            slots[index] = Some(prepared);
        }
        Self {
            generation: pack.render.generation,
            slots,
            translucent,
            bounds,
        }
    }

    pub(super) fn instance_slot(&self, slot: u32) -> Option<InstanceSlot> {
        let index = slot as usize;
        self.slots.get(index)?.is_some().then_some(InstanceSlot {
            translucent: self.translucent[index],
            bounds: self.bounds[index],
        })
    }

    pub(super) fn faces(&self, slot: u32) -> Option<&[super::minecarts::PreparedModelFace]> {
        self.slots.get(slot as usize)?.as_deref()
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// Vertices and per-slot index ranges for the generation's GPU buffer.
    pub(super) fn prepared_upload(
        &self,
    ) -> (
        Vec<horizon_client::gfx::PreparedModelVertex>,
        Vec<Option<(u32, u32)>>,
    ) {
        let mut vertices = Vec::new();
        let mut ranges = Vec::new();
        for slot in &self.slots {
            let Some(faces) = slot else {
                ranges.push(None);
                continue;
            };
            let index_start = vertices.len() as u32 / 4 * 6;
            for face in faces {
                for corner in 0..4 {
                    vertices.push(face.gpu_vertex(corner));
                }
            }
            let index_count = vertices.len() as u32 / 4 * 6 - index_start;
            ranges.push((index_count > 0).then_some((index_start, index_count)));
        }
        (vertices, ranges)
    }
}

/// `BlockEntityRenderer.getViewDistance`'s default, in blocks.
const VIEW_DISTANCE: f64 = 64.0;

/// Each mod block entity's view distance (`BlockEntityRenderer.getViewDistance`), by block
/// state, as the parts answered `native_block_entity_view_distance` once for each mod block;
/// states with none keep [`VIEW_DISTANCE`]. `furthest` is how far block entities are gathered.
pub(super) struct ViewDistances {
    by_state: FxHashMap<u32, f64>,
    furthest: f64,
}

impl ViewDistances {
    /// The distances of the block states `states` (id, block): `ask` is asked once for each
    /// mod block, never for vanilla's, and an answer that isn't finite and above 0 keeps
    /// [`VIEW_DISTANCE`].
    fn of_states<'a>(
        states: impl Iterator<Item = (u32, &'a str)>,
        mut ask: impl FnMut(&str) -> f64,
    ) -> Self {
        let mut by_block: FxHashMap<&str, f64> = FxHashMap::default();
        let mut by_state = FxHashMap::default();
        for (state, block) in states {
            if block.starts_with("minecraft:") {
                continue;
            }
            let distance = *by_block.entry(block).or_insert_with(|| {
                let distance = ask(block);
                if distance.is_finite() && distance > 0.0 {
                    distance
                } else {
                    VIEW_DISTANCE
                }
            });
            if distance != VIEW_DISTANCE {
                by_state.insert(state, distance);
            }
        }
        let furthest = by_state.values().copied().fold(VIEW_DISTANCE, f64::max);
        Self { by_state, furthest }
    }
    /// How far the block entity of `state` draws.
    fn of(&self, state: u32) -> f64 {
        self.by_state.get(&state).copied().unwrap_or(VIEW_DISTANCE)
    }
}
/// A mod block entity's bounds until its mod says otherwise: the block with
/// the block of margin Horizon culls vanilla's block entities with.
const DEFAULT_BOUNDS: [f32; 6] = [-1.0, -1.0, -1.0, 2.0, 2.0, 2.0];

/// Whether `bounds` (min then max, in blocks from `origin`, the block's corner
/// relative to the camera) meet the frustum; an infinite bound always does
/// (NeoForge's `INFINITE_EXTENT_AABB`), as does a frame without a frustum.
fn in_frustum(frustum: Option<&impl Cull>, origin: Vec3, bounds: [f32; 6]) -> bool {
    if bounds.iter().any(|b| !b.is_finite()) {
        return true;
    }
    let min = origin + Vec3::new(bounds[0], bounds[1], bounds[2]);
    let max = origin + Vec3::new(bounds[3], bounds[4], bounds[5]);
    frustum.is_none_or(|f| f.intersects(min, max))
}

/// One mod block entity's frame, as `submit` gets it: the render state's
/// partial tick, game time and light, and the pose stack the helpers draw
/// under (`native_draw_quad`, `native_draw_cube`, `native_draw_block`).
// Native mods read and drive it; nothing in Horizon does.
pub struct BlockEntityDraw {
    /// The frame's partial tick, 0 to 1 (`BlockEntityRenderState`).
    partial: f32,
    /// The level's game time, for animations.
    game_time: i64,
    /// The client's animation clock, in ticks with the partial tick (`spin`):
    /// the ticks its level ran (`TextureManager.tick`'s count), never set back.
    animation: f64,
    /// The light the helpers draw with: the block's own
    /// (`LevelRenderer.getLightCoords`), 16 times sky light in bits 0..8 and
    /// 16 times block light in bits 8..16; set it to light a part otherwise.
    light: u32,
    /// The block drawn, for `native_draw_item`'s owner.
    block: [i32; 3],
    poses: Vec<Mat4>,
    lights: (Vec3, Vec3),
    vertices: Vec<Vertex>,
    kept: bool,
    /// Where each turning part starts in `vertices` (`spin`), and where the
    /// parts standing still start again (`still`, none).
    spins: Vec<(usize, Option<Spin>)>,
    /// World block entities record a prepared copy instead of CPU vertices.
    /// Shader packs, GUI scenes and mobs leave this clear, so the model stays
    /// in the vertex stream those paths and their spans already cover.
    prepared: bool,
    records: Vec<PreparedRecord>,
}

struct PreparedRecord {
    at: usize,
    slot: u32,
    generation: u64,
    pose: Mat4,
    tint: u32,
    light: u32,
}

/// Instance tint in the same bytes the CPU vertex colour uses.
pub(super) fn prepared_tint(rgb: u32) -> [f32; 4] {
    horizon_client::hud_layout::multiply(
        u32::MAX,
        horizon_client::mobs::render::abgr(rgb & 0xFF_FFFF),
    )
    .to_le_bytes()
    .map(|byte| byte as f32 / 255.0)
}

/// An Iris pack (`pack_ids`) or one of Horizon's own packs owns the CPU entity stream.
pub(super) fn prepared_route(
    iris_pack: bool,
    native_pack: bool,
    uploaded: Option<u64>,
    generation: u64,
) -> bool {
    !iris_pack && !native_pack && uploaded == Some(generation)
}

pub(super) fn prepared_instance(
    pose: Mat4,
    origin: DVec3,
    camera: DVec3,
    tint: u32,
    light: u32,
) -> horizon_client::gfx::PreparedModelInstance {
    horizon_client::gfx::PreparedModelInstance::new(
        pose,
        origin,
        camera,
        prepared_tint(tint),
        light,
    )
}

/// A part of a block entity's frame that turns by itself (`BlockEntityDraw::spin`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Spin {
    axis: Vec3,
    pivot: Vec3,
    degrees_per_tick: f32,
    phase: f32,
}

impl Spin {
    /// The turn at animation time `time` (ticks with the partial tick), about
    /// the pivot, in the block's coordinates. Counted in f64: a long session
    /// has more ticks than an f32 holds exactly.
    fn turn(&self, time: f64) -> Mat4 {
        let t = time;
        let degrees =
            (t * f64::from(self.degrees_per_tick) + f64::from(self.phase)).rem_euclid(360.0);
        Mat4::from_translation(self.pivot)
            * Mat4::from_quat(Quat::from_axis_angle(
                self.axis,
                (degrees as f32).to_radians(),
            ))
            * Mat4::from_translation(-self.pivot)
    }
}

/// `vertices`, drawn relative to the block, moved to the camera by `origin`
/// and each part of `spins` (starts relative to `vertices`) turned as it is
/// at animation time `time`, onto `out`.
pub(super) fn place_turning(
    vertices: &[Vertex],
    spins: &[(usize, Option<Spin>)],
    time: f64,
    origin: Vec3,
    out: &mut Vec<Vertex>,
) {
    let mut turn: Option<Mat4> = None;
    let mut next = spins.iter().peekable();
    for (i, v) in vertices.iter().enumerate() {
        while let Some((start, spin)) = next.next_if(|(start, _)| *start <= i) {
            let _ = start;
            turn = spin.map(|s| s.turn(time));
        }
        let mut v = *v;
        if let Some(m) = turn {
            v.pos = m.transform_point3(Vec3::from_array(v.pos)).to_array();
            if v.normal[3] != 0 {
                let n = Vec3::new(
                    f32::from(v.normal[0]),
                    f32::from(v.normal[1]),
                    f32::from(v.normal[2]),
                );
                let n = m.transform_vector3(n);
                v.normal = [
                    n.x.round().clamp(-127.0, 127.0) as i8,
                    n.y.round().clamp(-127.0, 127.0) as i8,
                    n.z.round().clamp(-127.0, 127.0) as i8,
                    v.normal[3],
                ];
            }
        }
        v.pos = (Vec3::from_array(v.pos) + origin).to_array();
        out.push(v);
    }
}

#[allow(dead_code)]
impl BlockEntityDraw {
    /// A frame drawn from `pose` (relative to the camera), lit by `light` and
    /// `lights`, adding to `vertices`; `block` is where `native_draw_item`'s
    /// owner stands. A mob's frame (`native_render_living_pre`) is made this way.
    pub(super) fn posed(
        partial: f32,
        game_time: i64,
        light: u32,
        block: [i32; 3],
        pose: Mat4,
        lights: (Vec3, Vec3),
        vertices: Vec<Vertex>,
    ) -> Self {
        Self {
            partial,
            game_time,
            animation: 0.0,
            light,
            block,
            poses: vec![pose],
            lights,
            vertices,
            kept: false,
            spins: Vec::new(),
            prepared: false,
            records: Vec::new(),
        }
    }

    /// This frame on the client's animation clock (`Game::animation_time`),
    /// which turns its `spin` parts and `animation_time` reads.
    pub(super) fn animated(mut self, time: f64) -> Self {
        self.animation = time;
        self
    }
    /// The frame's time on the animation clock (`native_draw_animation_time`).
    pub(super) fn animation(&self) -> f64 {
        self.animation
    }

    fn spinning(&self) -> bool {
        self.spins.last().is_some_and(|(_, spin)| spin.is_some())
    }

    /// Record a prepared copy at the current vertex, unless this draw is CPU-only or spinning.
    fn defer_model(&mut self, slot: u32, generation: u64, tint: u32) -> bool {
        if !self.prepared || self.spinning() {
            return false;
        }
        self.records.push(PreparedRecord {
            at: self.vertices.len(),
            slot,
            generation,
            pose: self.pose(),
            tint,
            light: self.light,
        });
        true
    }

    fn bake_recorded(&mut self, slots: &[Option<Vec<super::minecarts::PreparedModelFace>>]) {
        while let Some(record) = self.records.pop() {
            let Some(faces) = slots.get(record.slot as usize).and_then(Option::as_deref) else {
                continue;
            };
            let mut extra = Vec::new();
            super::minecarts::emit_model_faces(
                faces.iter().copied().map(|face| (face, record.light)),
                record.pose,
                true,
                |index| (index >= 0).then_some(record.tint & 0xFF_FFFF),
                &mut extra,
            );
            super::minecarts::scale_tint_alpha(&mut extra, faces, record.tint);
            let count = extra.len();
            for (at, _) in &mut self.spins {
                if *at >= record.at {
                    *at += count;
                }
            }
            self.vertices.splice(record.at..record.at, extra);
        }
    }

    fn push_instances(
        &mut self,
        origin: DVec3,
        camera: DVec3,
        out: &mut Vec<horizon_client::gfx::PreparedModelDraw>,
    ) {
        out.extend(
            self.records
                .drain(..)
                .map(|record| horizon_client::gfx::PreparedModelDraw {
                    cpu_vertices: record.at as u32,
                    generation: record.generation,
                    slot: record.slot,
                    instance: {
                        let mut instance = prepared_instance(
                            record.pose,
                            origin,
                            camera,
                            record.tint,
                            record.light,
                        );
                        instance.tint[3] = (record.tint >> 24) as f32 / 255.0;
                        instance
                    },
                }),
        );
    }
    /// The vertices drawn, with those it was made with first.
    pub(super) fn into_vertices(mut self) -> Vec<Vertex> {
        super::draw_boundary::truncate(&mut self.vertices, super::draw_boundary::MAX_DRAW_VERTICES);
        self.vertices
    }
    /// The vertices drawn and where its turning parts start in them (`spin`).
    pub(super) fn into_turning(mut self) -> (Vec<Vertex>, Vec<(usize, Option<Spin>)>) {
        super::draw_boundary::truncate(&mut self.vertices, super::draw_boundary::MAX_DRAW_VERTICES);
        self.spins.retain(|(at, _)| *at <= self.vertices.len());
        (self.vertices, self.spins)
    }
    /// `PoseStack.pushPose`: `pop` comes back to the pose as it is now.
    fn push(&mut self) {
        let pose = self.pose();
        self.poses.push(pose);
    }
    /// `PoseStack.popPose`; the block's own pose is never popped.
    fn pop(&mut self) {
        if self.poses.len() > 1 {
            self.poses.pop();
        }
    }
    /// Moves what is drawn next by `by`, in blocks.
    fn translate(&mut self, by: [f32; 3]) {
        self.apply(Mat4::from_translation(Vec3::from_array(by)));
    }
    /// Turns what is drawn next by `degrees` about `axis` (counterclockwise
    /// seen from the axis' positive end, as `Axis.rotationDegrees`) through
    /// `pivot`, in blocks from the pose's origin: `[0.5; 3]` is the block's centre.
    fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, pivot: [f32; 3]) {
        let axis = Vec3::from_array(axis).normalize_or(Vec3::Y);
        let pivot = Vec3::from_array(pivot);
        self.apply(
            Mat4::from_translation(pivot)
                * Mat4::from_quat(Quat::from_axis_angle(axis, degrees.to_radians()))
                * Mat4::from_translation(-pivot),
        );
    }
    /// Scales what is drawn next by `by` about the pose's origin.
    fn scale(&mut self, by: [f32; 3]) {
        self.apply(Mat4::from_scale(Vec3::from_array(by)));
    }
    /// Keeps what this call drew: the block is drawn the same again, with
    /// neither `native_block_entity_submit` nor
    /// `native_block_entity_render_bounding_box` called, until its state,
    /// its block entity (what the server sent), its light or the resource
    /// packs change. Call it only when what you drew reads none of
    /// `partial`, `game_time`, the blocks around it or anything else of
    /// yours, and moves nothing by itself but through `spin`: an idle machine,
    /// or a shaft turning at a steady speed, not one whose speed changes from
    /// frame to frame. Kept geometry stands still: an enchanted item's glint, and an
    /// item drawn with `native_draw_item` whose model reads the time (a clock,
    /// a compass), stop where they were.
    fn keep(&mut self) {
        self.kept = true;
    }
    /// What is drawn next turns by itself: `degrees_per_tick` degrees each
    /// tick of this client's animation clock (`animation`; a shaft at `rpm`
    /// turns `rpm * 0.3`, as Create's do), about `axis` through `pivot`, both
    /// in blocks from the block's corner whatever the pose, counterclockwise
    /// seen from the axis' positive end. The angle is `animation *
    /// degrees_per_tick + phase`. The clock is the client's own
    /// (`Game::animation_time`), not the game time, which a server's time
    /// updates set back: so a part turns smoothly. Draw the part at rest; the
    /// game turns it each frame. With `keep` it goes on turning with no call
    /// of `native_block_entity_submit` until what it was drawn with changes.
    /// With a new speed, pass `old_angle - animation * new_speed` as the
    /// phase, or it jumps. Quads drawn while it is active are lit where they
    /// turn to (`quad`). Until `still` or the next `spin`. A block entity's
    /// frame only: a mob's (`native_render_living_pre`) draws it at rest. API
    /// @API:spinning_geometry@.
    pub fn spin(&mut self, axis: [f32; 3], pivot: [f32; 3], degrees_per_tick: f32, phase: f32) {
        let spin = Spin {
            axis: Vec3::from_array(axis).normalize_or(Vec3::Y),
            pivot: Vec3::from_array(pivot),
            degrees_per_tick: if degrees_per_tick.is_finite() {
                degrees_per_tick
            } else {
                0.0
            },
            phase: if phase.is_finite() { phase } else { 0.0 },
        };
        self.spins.push((self.vertices.len(), Some(spin)));
    }
    /// What is drawn next stands still again, after `spin`.
    /// API @API:spinning_geometry@.
    pub fn still(&mut self) {
        self.spins.push((self.vertices.len(), None));
    }
    fn apply(&mut self, m: Mat4) {
        if let Some(pose) = self.poses.last_mut() {
            *pose *= m;
        }
    }
    pub(super) fn pose(&self) -> Mat4 {
        self.poses.last().copied().unwrap_or(Mat4::IDENTITY)
    }
    pub(super) fn vertices_mut(&mut self) -> &mut Vec<Vertex> {
        &mut self.vertices
    }
    /// The light the helpers draw with (`native_draw_light`).
    pub(super) fn light(&self) -> u32 {
        self.light
    }
    /// A quad of `corners` (blocks, under the pose) with unit UVs, shaded as
    /// vanilla's block entity models are (`Lighting`'s two lights).
    fn quad(&mut self, corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], sprite: Rect, tint: u32) {
        let pose = self.pose();
        let at = corners.map(|c| pose.transform_point3(Vec3::from_array(c)));
        if !super::draw_boundary::quad_allowed(
            at.map(|p| p.to_array()),
            uv,
            self.vertices.len(),
            super::draw_boundary::MAX_DRAW_VERTICES,
        ) {
            return;
        }
        let normal = (at[1] - at[0]).cross(at[2] - at[0]).normalize_or_zero();
        let normal = if normal == Vec3::ZERO {
            (at[2] - at[1]).cross(at[3] - at[1]).normalize_or(Vec3::Y)
        } else {
            normal
        };
        // A turning part is lit where it turns to (`place_turning` turns the
        // normal, the shader lights it, as `draw_block`'s faces are lit);
        // a part standing still keeps its light in its colour.
        let (color, normal) = if self.spinning() {
            let n = (normal * 127.0).round();
            (shaded(tint, 1.0), [n.x as i8, n.y as i8, n.z as i8, 127])
        } else {
            (shaded(tint, diffuse(normal, self.lights)), [0; 4])
        };
        for (p, uv) in at.into_iter().zip(uv) {
            self.vertices.push(Vertex {
                pos: p.to_array(),
                uv: [
                    sprite.u0 + (sprite.u1 - sprite.u0) * uv[0],
                    sprite.v0 + (sprite.v1 - sprite.v0) * uv[1],
                ],
                layer: 0,
                color,
                light: self.light,
                normal,
                glint_uv: [0.0, 0.0],
            });
        }
    }
}

/// 0xRRGGBB `tint` times `brightness`, as an opaque vertex colour.
fn shaded(tint: u32, brightness: f32) -> u32 {
    let c = |shift: u32| (((tint >> shift) & 255) as f32 * brightness) as u32;
    0xFF00_0000 | c(0) << 16 | c(8) << 8 | c(16)
}

/// Game ticks kept geometry stays after it was last drawn, as signs' text.
const KEPT_FOR: i64 = 100;

/// What kept geometry was drawn with besides its block entity: drawn again
/// when any of it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeptKey {
    dim: Dim,
    state: u32,
    light: u32,
    /// `Font::epoch` when it was drawn: a cache reset moves glyphs in the
    /// atlas, so text a mod kept (`native_draw_text`) is drawn again, as a
    /// sign's kept text is.
    glyphs: u64,
}

struct KeptEntry {
    key: KeptKey,
    /// Of the block entity it was drawn from: equal, nothing changed it.
    revision: ModRevision,
    bounds: [f32; 6],
    /// Game time it was last drawn.
    used: i64,
    /// Relative to the block's corner.
    vertices: Vec<Vertex>,
    /// Its turning parts (`BlockEntityDraw::spin`), from the start of `vertices`.
    spins: Vec<(usize, Option<Spin>)>,
}

/// Geometry mods kept (`BlockEntityDraw::keep`), by position. As
/// `SignTexts` keeps a sign's text, it is kept relative to the block and
/// moved to the camera each frame.
#[derive(Default)]
pub(super) struct KeptGeometry {
    entries: FxHashMap<[i32; 3], KeptEntry>,
    /// Game time of the last sweep of what is out of view.
    swept: i64,
}

impl KeptGeometry {
    /// Drops what was not drawn for `KEPT_FOR` ticks, once every `KEPT_FOR`.
    fn sweep(&mut self, now: i64) {
        if (now - self.swept).abs() >= KEPT_FOR {
            self.swept = now;
            self.entries
                .retain(|_, e| (now - KEPT_FOR..=now).contains(&e.used));
        }
    }
    /// What is kept at `pos` when it was drawn with `key` and its block
    /// entity at `revision`, marked drawn at `now`.
    fn get(
        &mut self,
        pos: [i32; 3],
        key: KeptKey,
        revision: ModRevision,
        now: i64,
    ) -> Option<&KeptEntry> {
        let entry = self
            .entries
            .get_mut(&pos)
            .filter(|e| e.key == key && e.revision == revision)?;
        entry.used = now;
        Some(entry)
    }
    fn keep(
        &mut self,
        pos: [i32; 3],
        key: KeptKey,
        revision: ModRevision,
        bounds: [f32; 6],
        vertices: &[Vertex],
        spins: Vec<(usize, Option<Spin>)>,
        now: i64,
    ) {
        let mut kept = self
            .entries
            .remove(&pos)
            .map_or_else(Vec::new, |e| e.vertices);
        kept.clear();
        kept.extend_from_slice(vertices);
        self.entries.insert(
            pos,
            KeptEntry {
                key,
                revision,
                bounds,
                used: now,
                vertices: kept,
                spins,
            },
        );
    }
    fn forget(&mut self, pos: [i32; 3]) {
        self.entries.remove(&pos);
    }
    /// Forgets everything: the atlas's sprites moved.
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }
}

impl Game<'_> {
    /// The mod block entities within view distance of `camera` whose block's
    /// template has none, which the mesher's block entity lists (made from
    /// the template) leave out: where they are and their block's state, for
    /// `mod_block_entity_geometry`. From the per-chunk list of
    /// `BlockEntities::mods_near`, not a scan of every block entity.
    /// The client's animation clock, in ticks: the ticks its level has run
    /// (`texture_ticks`, `TextureManager.tick`'s count) and the level's partial
    /// tick (`Clock::fraction`, held while frozen), never the camera's, which
    /// runs on in a frozen level and would turn parts back as it wraps.
    /// Pausing and `/tick freeze` hold it and `SetTime` never sets it back, so a
    /// `spin` part turns smoothly whatever the server's clock does; each client
    /// counts from its own start.
    pub(super) fn animation_time(&self) -> f64 {
        self.texture_ticks as f64 + self.clock.fraction()
    }
    pub(super) fn untemplated_mod_block_entities(&self, camera: DVec3) -> Vec<([i32; 3], u32)> {
        self.world
            .block_entities
            .mods_near(camera.x, camera.z, self.view_distances().furthest)
            .map(|p| ([p.0, p.1, p.2], self.world.block(p.0, p.1, p.2).0))
            .filter(|&(_, state)| !self.res.facts().get(BlockStateId(state)).block_entity)
            .collect()
    }
    /// The mod block entities' view distances: the parts are asked once for each mod block
    /// (`native_block_entity_view_distance`) the first time they're needed, and kept.
    fn view_distances(&self) -> &ViewDistances {
        self.mod_view_distances.get_or_init(|| {
            let registry = self.pack.assets.registry;
            let states = (0..registry.len() as u32).filter_map(|i| {
                registry
                    .state(BlockStateId(i))
                    .map(|state| (i, state.name.as_str()))
            });
            ViewDistances::of_states(states, |block| {
                let mut distance = VIEW_DISTANCE;
                self.native_block_entity_view_distance(block, &mut distance);
                distance
            })
        })
    }
    /// `native_block_entity_submit` for each of `blocks` (in drawn sections,
    /// drawn by no vanilla renderer) whose block entity is a mod's, closer than
    /// `BlockEntityRenderer.getViewDistance` to the camera (`shouldRender`)
    /// and with its bounds (`native_block_entity_render_bounding_box`) in the
    /// frustum. Entity occlusion culling is for ordinary entities; block
    /// entities use these renderer-provided bounds instead.
    pub(super) fn mod_block_entity_geometry<'a>(
        &self,
        blocks: impl Iterator<Item = &'a ([i32; 3], u32)>,
        camera: DVec3,
        partial: f32,
        out: &mut Vec<Vertex>,
    ) {
        let mut draw = BlockEntityDraw {
            partial,
            game_time: self.world.game_time,
            animation: self.animation_time(),
            light: 0,
            block: [0; 3],
            poses: Vec::new(),
            lights: diffuse_lights(self.nether_lighting()),
            vertices: std::mem::take(out),
            kept: false,
            spins: Vec::new(),
            prepared: prepared_route(
                self.pack_ids.is_some(),
                !self.native_shaders.picked.is_empty(),
                self.renderer
                    .as_ref()
                    .and_then(|renderer| renderer.prepared_generation()),
                self.standalone_geometry.generation(),
            ),
            records: Vec::new(),
        };
        let now = self.world.game_time;
        let mut kept = self.kept_block_entities.borrow_mut();
        kept.sweep(now);
        let distances = self.view_distances();
        for &(pos, state) in blocks {
            let corner = DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
            // Vec3.closerThan: strictly closer.
            let distance = distances.of(state);
            if (corner + 0.5).distance_squared(camera) >= distance * distance {
                continue;
            }
            let Some(BlockEntity::Mod(entity)) =
                self.world.block_entities.get(&(pos[0], pos[1], pos[2]))
            else {
                continue;
            };
            let origin = (corner - camera).as_vec3();
            let light = vertex_light(self.world.light(pos[0], pos[1], pos[2]));
            let key = KeptKey {
                dim: self.dim,
                state,
                light,
                glyphs: self.gui.atlas.font.epoch(),
            };
            if let Some(entry) = kept.get(pos, key, entity.revision(), now) {
                if in_frustum(self.view_frustum.as_ref(), origin, entry.bounds) {
                    let start = draw.vertices.len();
                    self.mark_block_entity(start, state);
                    if entry.spins.is_empty() {
                        draw.vertices.extend(entry.vertices.iter().map(|v| Vertex {
                            pos: (Vec3::from_array(v.pos) + origin).to_array(),
                            ..*v
                        }));
                    } else {
                        place_turning(
                            &entry.vertices,
                            &entry.spins,
                            draw.animation,
                            origin,
                            &mut draw.vertices,
                        );
                    }
                }
                continue;
            }
            let id = BlockStateId(state);
            let block = &*self.res.facts().get(id).name;
            let mut bounds = DEFAULT_BOUNDS;
            self.api_block_entity_render_bounding_box(pos, block, id, entity, &mut bounds);
            if !in_frustum(self.view_frustum.as_ref(), origin, bounds) {
                continue;
            }
            let start = draw.vertices.len();
            self.mark_block_entity(start, state);
            // Drawn relative to the block, then moved to the camera, so what
            // is kept can be moved again.
            draw.block = pos;
            draw.poses.clear();
            draw.poses.push(Mat4::IDENTITY);
            draw.light = light;
            draw.kept = false;
            draw.spins.clear();
            draw.records.clear();
            self.api_block_entity_submit(pos, block, id, entity, &mut draw);
            self.finish_prepared_draw(&mut draw, corner, camera);
            // The turning parts, from the start of this block's vertices.
            let spins: Vec<(usize, Option<Spin>)> = draw
                .spins
                .drain(..)
                .map(|(at, spin)| (at.saturating_sub(start), spin))
                .collect();
            if draw.kept {
                kept.keep(
                    pos,
                    key,
                    entity.revision(),
                    bounds,
                    &draw.vertices[start..],
                    spins.clone(),
                    now,
                );
            } else {
                kept.forget(pos);
            }
            if spins.is_empty() {
                for v in &mut draw.vertices[start..] {
                    v.pos = (Vec3::from_array(v.pos) + origin).to_array();
                }
            } else {
                let drawn = draw.vertices.split_off(start);
                place_turning(&drawn, &spins, draw.animation, origin, &mut draw.vertices);
            }
        }
        *out = draw.vertices;
    }

    pub(super) fn prepared_copy_room(&self, already: u64, incoming: usize) -> Option<usize> {
        let room = self
            .renderer
            .as_ref()
            .map(|renderer| renderer.prepared_instance_room(already, incoming));
        #[cfg(test)]
        let room = room.or_else(|| horizon_client::gfx::capped_prepared_room(already, incoming));
        room
    }

    /// Keep, spin, or no renderer bakes every copy into the vertex span.
    /// Otherwise copies that fit the instance buffer are staged, and the rest are
    /// baked at their own cut so they stay after the instanced prefix.
    fn finish_prepared_draw(&self, draw: &mut BlockEntityDraw, origin: DVec3, camera: DVec3) {
        if draw.records.is_empty() {
            return;
        }
        if draw.kept || !draw.spins.is_empty() {
            draw.bake_recorded(&self.standalone_geometry.slots);
            return;
        }
        let already = self.prepared_model_draws.borrow().len() as u64;
        let incoming = draw.records.len();
        let Some(room) = self.prepared_copy_room(already, incoming) else {
            draw.bake_recorded(&self.standalone_geometry.slots);
            return;
        };
        if room == 0 {
            draw.bake_recorded(&self.standalone_geometry.slots);
            return;
        }
        let overflow = (room < draw.records.len()).then(|| draw.records.split_off(room));
        draw.push_instances(origin, camera, &mut self.prepared_model_draws.borrow_mut());
        if let Some(overflow) = overflow {
            draw.records = overflow;
            draw.bake_recorded(&self.standalone_geometry.slots);
        }
    }
}

/// Helpers for drawing mod block entities, kept as stable as the hook points: the
/// methods of `BlockEntityDrawContext`.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// `PoseStack.pushPose`: `pop` comes back to the pose as it is now.
    pub(super) fn native_draw_push(&self, draw: &mut BlockEntityDraw) {
        draw.push();
    }
    /// `PoseStack.popPose`; the block's own pose is never popped.
    pub(super) fn native_draw_pop(&self, draw: &mut BlockEntityDraw) {
        draw.pop();
    }
    /// Moves what is drawn next by `by`, in blocks.
    pub(super) fn native_draw_translate(&self, draw: &mut BlockEntityDraw, by: [f32; 3]) {
        draw.translate(by);
    }
    /// Turns what is drawn next by `degrees` about `axis` (counterclockwise
    /// seen from the axis' positive end, as `Axis.rotationDegrees`) through
    /// `pivot`, in blocks from the pose's origin: `[0.5; 3]` is the block's centre.
    pub(super) fn native_draw_rotate_about(
        &self,
        draw: &mut BlockEntityDraw,
        axis: [f32; 3],
        degrees: f32,
        pivot: [f32; 3],
    ) {
        draw.rotate_about(axis, degrees, pivot);
    }
    /// Scales what is drawn next by `by` about the pose's origin.
    pub(super) fn native_draw_scale(&self, draw: &mut BlockEntityDraw, by: [f32; 3]) {
        draw.scale(by);
    }
    /// What is drawn next turns by itself on the animation clock (`BlockEntityDraw::spin`).
    pub(super) fn native_draw_spin(
        &self,
        draw: &mut BlockEntityDraw,
        axis: [f32; 3],
        pivot: [f32; 3],
        degrees_per_tick: f32,
        phase: f32,
    ) {
        draw.spin(axis, pivot, degrees_per_tick, phase);
    }
    /// What is drawn next stands still again, after `spin`.
    pub(super) fn native_draw_still(&self, draw: &mut BlockEntityDraw) {
        draw.still();
    }
    /// Keeps what this call drew, with neither `native_block_entity_submit` nor
    /// `native_block_entity_render_bounding_box` called until its state, block entity,
    /// light or resource packs change. Parts marked with `spin` keep turning on the
    /// animation clock; other geometry stays as submitted. Do not bake frame-dependent
    /// values into kept vertices: item glint and time-dependent item models stop
    /// where they were, and neighboring blocks do not invalidate the cache.
    pub(super) fn native_draw_keep(&self, draw: &mut BlockEntityDraw) {
        draw.keep();
    }
    /// The frame's partial tick, 0 to 1 (`BlockEntityRenderState`).
    pub(super) fn native_draw_partial(&self, draw: &BlockEntityDraw) -> f32 {
        draw.partial
    }
    /// The level's game time, for animations.
    pub(super) fn native_draw_game_time(&self, draw: &BlockEntityDraw) -> i64 {
        draw.game_time
    }
    /// The frame on this client's animation clock, in ticks with the partial tick
    /// (`Game::animation_time`): what turns `spin` parts.
    pub(super) fn native_draw_animation_time(&self, draw: &BlockEntityDraw) -> f64 {
        draw.animation
    }
    /// The light the helpers draw with: the block's own
    /// (`LevelRenderer.getLightCoords`), 16 times sky light in bits 0..8 and
    /// 16 times block light in bits 8..16.
    pub(super) fn native_draw_light(&self, draw: &BlockEntityDraw) -> u32 {
        draw.light
    }
    /// Lights what is drawn next with `light` (as `native_draw_light` reads it).
    pub(super) fn native_draw_set_light(&self, draw: &mut BlockEntityDraw, light: u32) {
        draw.light = light;
    }
    /// Draws a quad with `corners` in blocks under `draw`'s pose, in a model
    /// face's order (top left, bottom left, bottom right, top right as seen
    /// from its front), textured with the `uv` rectangle (u0, v0, u1, v1 in
    /// texels of 16 per side) of `sprite` (`mymod:entity/screen`,
    /// `mymod:block/casing`, `block/stone`) times the 0xRRGGBB `tint`
    /// (0xFFFFFF for none). False for an unknown sprite.
    pub(super) fn native_draw_quad(
        &self,
        draw: &mut BlockEntityDraw,
        sprite: &str,
        corners: [[f32; 3]; 4],
        uv: [f32; 4],
        tint: u32,
    ) -> bool {
        let Some(rect) = self.gui.atlas.sprites.get(sprite).copied() else {
            return false;
        };
        let [u0, v0, u1, v1] = uv.map(|t| t / 16.0);
        draw.quad(
            corners,
            [[u0, v0], [u0, v1], [u1, v1], [u1, v0]],
            rect,
            tint,
        );
        true
    }

    /// Draws `text` under `draw`'s pose as vanilla's `Font.drawInBatch` draws a
    /// component in the world (a sign's line, a text display's): in the pose's
    /// x-y plane, one unit a font pixel, y down, its left end at `at`, facing
    /// +z. Scale the pose first as a sign does (`draw.scale([1.0 / 64.0,
    /// -1.0 / 64.0, 1.0 / 64.0])` for its letters, 8 pixels high). It is drawn
    /// with its styles, through the path name tags and text displays use
    /// (`HudBatch::text_styled_world`): each span's colour (the rest in the
    /// 0xRRGGBB `color`), bold, italic, underline, strikethrough, obfuscated,
    /// its font, inline objects and `§` codes, right-to-left text in visual
    /// order. `glowing` text is full bright (a glow ink sac's), the rest lit by
    /// `draw.light`. With `outline`, a 0xRRGGBB colour (`Some(0)` is black),
    /// each glyph is ringed with it as glowing sign text is
    /// (`Font.drawInBatch8xOutline`). Returns the width drawn, in font pixels
    /// (`Font.width`): centre a line by starting it at minus half its width.
    /// Obfuscated glyphs change only when the block is drawn again, so kept
    /// text (`keep`) stands still. API @API:draw_text@.
    pub(super) fn native_draw_text(
        &self,
        draw: &mut BlockEntityDraw,
        text: &Text,
        at: [f32; 2],
        color: u32,
        glowing: bool,
        outline: Option<u32>,
    ) -> f32 {
        let opaque =
            |c: u32| horizon_client::hud::rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, 255);
        let atlas = &self.gui.atlas;
        let mut batch = horizon_client::hud::HudBatch::scratch([1, 1]);
        let (width, front) = match outline {
            Some(ring) => batch.text_styled_outline_world(
                atlas,
                text,
                at[0],
                at[1],
                opaque(color),
                opaque(ring),
            ),
            None => (
                batch.text_styled_world(atlas, text, at[0], at[1], opaque(color), false),
                0,
            ),
        };
        // Glyphs new to the atlas upload with this frame (`Game::text_patches`).
        self.text_patches.borrow_mut().append(&mut batch.patches);
        let light = if glowing { 240 | 240 << 8 } else { draw.light };
        let pose = draw.pose();
        for (i, quad) in batch.vertices.as_chunks::<6>().0.iter().enumerate() {
            // The text sits a hair in front of its outline, as a sign's does
            // (`TEXT_OVER_OUTLINE`): one mesh, so no polygon offset.
            let lift = if outline.is_some() && i * 6 >= front {
                TEXT_OVER_OUTLINE
            } else {
                0.0
            };
            let corners = horizon_client::text_display::CORNERS.map(|c| quad[c]);
            let at = corners.map(|v| {
                pose.transform_point3(Vec3::new(v.pos[0], v.pos[1], v.depth + lift))
                    .to_array()
            });
            if !super::draw_boundary::quad_allowed(
                at,
                corners.map(|v| v.uv),
                draw.vertices.len(),
                super::draw_boundary::MAX_DRAW_VERTICES,
            ) {
                continue;
            }
            for (pos, v) in at.into_iter().zip(corners) {
                draw.vertices.push(Vertex {
                    pos,
                    uv: v.uv,
                    layer: 0,
                    color: v.color,
                    light,
                    normal: [0; 4],
                    glint_uv: [0.0, 0.0],
                });
            }
        }
        width.ceil()
    }
    /// The width of `text` in font pixels, as `draw_text` draws it, styles
    /// included (`Font.width`), to lay a line out before drawing it.
    pub(super) fn native_draw_text_width(&self, _draw: &BlockEntityDraw, text: &Text) -> f32 {
        self.gui.atlas.font.width_text(text)
    }

    /// Draws a box from `from` to `to` (texels of 16 per block, as a block
    /// model's element) under `draw`'s pose, with the faces given, by
    /// `Direction` (down, up, north, south, west, east), baked as a model
    /// element's are (`FaceBakery`), times the 0xRRGGBB `tint`. A turned box
    /// is drawn under a turned pose (`BlockEntityDraw::rotate_about`). False
    /// when a face's sprite is unknown; that face is left out.
    pub(super) fn native_draw_cube(
        &self,
        draw: &mut BlockEntityDraw,
        from: [f32; 3],
        to: [f32; 3],
        faces: [Option<CubeFace<'_>>; 6],
        tint: u32,
    ) -> bool {
        let mut all = true;
        for (direction, face) in Direction::ALL.into_iter().zip(faces) {
            let Some(face) = face else { continue };
            let Some(rect) = self.gui.atlas.sprites.get(face.sprite).copied() else {
                all = false;
                continue;
            };
            let uv = face.uv.unwrap_or_else(|| default_uv(from, to, direction));
            let material = Material {
                sprite: String::new(),
                force_translucent: false,
            };
            if let Ok(quad) = bake_flat_face(from, to, uv, direction, material, -1) {
                draw.quad(quad.positions, quad.uv, rect, tint);
            }
        }
        all
    }

    /// Draws block `block`'s model (`mymod:cog`, any block with a model) in
    /// the state with `properties` (`[("axis", "x")]`, the rest default)
    /// under `draw`'s pose, as vanilla's `submitBlockModel` draws a block in
    /// an entity's place: a block's baked model, with its default tints. A
    /// moving part that is no block is a standalone model
    /// (`native_draw_model`). False for an unknown block or property.
    pub(super) fn native_draw_block(
        &self,
        draw: &mut BlockEntityDraw,
        block: &str,
        properties: &[(&str, &str)],
    ) -> bool {
        let registry = &self.pack.assets.registry;
        let Some(mut state) = registry
            .default_state(block)
            .and_then(|id| registry.state(id))
            .cloned()
        else {
            return false;
        };
        for (key, value) in properties {
            match state.properties.get_mut(*key) {
                Some(v) => *v = (*value).to_owned(),
                None => return false,
            }
        }
        let Some(id) = registry.id(&state) else {
            return false;
        };
        let pose = draw.pose();
        self.block_model_vertices(id, 42, pose, draw.light, true, &mut draw.vertices);
        true
    }

    /// Resolve a registered standalone model once in `client_setup` and again in
    /// `client_resources_reloaded`, not each frame. Resource-pack overrides and other
    /// namespaces use the normal shared model store; no resource is read or baked here.
    pub(super) fn native_model(&self, id: &Identifier) -> Result<ModelRef, ModelError> {
        self.pack.render.model(id)
    }

    /// Draws the resolved standalone model under `draw`'s pose: unturned, from
    /// the block's lower north-west corner, lit by `draw.light`. Faces with a tint
    /// index are multiplied by `tint` (`0xAARRGGBB`, the same as `WorldQuad::color`;
    /// `0xFFFFFFFF` for none), alpha included. Faces with no tint index stay opaque.
    /// A stale generation or absent slot returns an error without appending geometry.
    pub(super) fn native_draw_model(
        &self,
        draw: &mut BlockEntityDraw,
        model: ModelRef,
        tint: u32,
    ) -> Result<(), ModelError> {
        use mod_api::__private::game::Fields;
        let fields = model.fields();
        if fields.generation != self.standalone_geometry.generation() {
            return Err(ModelError::StaleModel);
        }
        let faces = self
            .standalone_geometry
            .slots
            .get(fields.slot as usize)
            .and_then(Option::as_ref)
            .ok_or(ModelError::UnknownModel)?;
        if draw.defer_model(fields.slot, fields.generation, tint) {
            return Ok(());
        }
        let light = draw.light;
        let start = draw.vertices.len();
        super::minecarts::emit_model_faces(
            faces.iter().copied().map(|face| (face, light)),
            draw.pose(),
            true,
            |index| (index >= 0).then_some(tint & 0xFF_FFFF),
            &mut draw.vertices,
        );
        super::minecarts::scale_tint_alpha(&mut draw.vertices[start..], faces, tint);
        Ok(())
    }

    /// Draws model part `part` and the parts under it under `draw`'s pose, as
    /// vanilla's `ModelPart.render` does: each part moved to its place (in
    /// texels, 16 to a block), turned about z, then y, then x, and scaled,
    /// its faces textured from `sprite` (`mymod:entity/golem`, a content
    /// mod's `assets/mymod/textures/entity/golem.png`, or vanilla's
    /// `entity/zombie/zombie`) times the 0xRRGGBB `tint`. A part that isn't
    /// `visible` draws nothing, one that skips drawing (`skip_draw`) only its
    /// children. A mob's model is built in the model's own frame (+y down,
    /// its front towards -z, its feet 24 texels down): in `native_render_living_pre`,
    /// `draw.scale([1.0, -1.0, -1.0])` then `draw.translate([0.0, -1.501,
    /// 0.0])` first, as `LivingEntityRenderer` does. False for an unknown
    /// sprite. API @API:model_layers@.
    pub(super) fn native_draw_model_part(
        &self,
        draw: &mut BlockEntityDraw,
        part: &ModelPart,
        sprite: &str,
        tint: u32,
    ) -> bool {
        let Some(rect) = self.gui.atlas.sprites.get(sprite).copied() else {
            return false;
        };
        draw_model_part(draw, part, rect, tint);
        true
    }

    /// Entity model layer `layer` baked into its `ModelPart` tree, each part
    /// at its pose, as `EntityModelSet.bakeLayer` bakes it: one a native mod
    /// registered (`native_register_layer_definitions`), or vanilla's, by the name
    /// `entity-models.json` gives it (`minecraft:zombie#main`,
    /// `minecraft:player#main`). The tree is the caller's own copy: bake it
    /// once, as a renderer does, and set its parts' poses each frame. None
    /// for a layer neither has. A vanilla layer no mob has (a player's, a
    /// chest's) is read from the resources each call, so slow. API
    /// @API:model_layers@.
    pub(super) fn bake_layer(&self, layer: &str) -> Option<ModelPart> {
        if let Some(definition) = model_layers::registered(layer) {
            return Some(definition.bake_root());
        }
        let key = layer.strip_prefix("minecraft:")?;
        if let Some(part) = self.res.mob_types().values().find_map(|t| t.layer(key)) {
            return Some(model_layers::from_part(part));
        }
        let models = horizon_client::mobs::EntityModelDocument::load(&self.vanilla_root()).ok()?;
        models
            .model(layer)
            .map(|part| model_layers::from_part(&part))
    }

    /// Draws `stack` under `draw`'s pose as an item is drawn in the world
    /// (`ItemModelResolver.updateForTopItem` and `ItemStackRenderState.submit`), centred on the
    /// pose's origin, in its model's `display` transform: `Fixed` as a
    /// campfire's or an item frame's, `Ground` as a dropped item's. `seed`
    /// picks among random models (a campfire passes its slot plus its
    /// position's hash). `owner_yaw` makes the block entity the item's
    /// owner (`ItemOwner`), at the block's centre and turned that many
    /// degrees (`getVisualRotationYInDegrees`), as a shelf is: a compass on
    /// it points to its target from there. None draws it with no owner, as
    /// a campfire does. False for an empty stack or one with no model. The
    /// client's copy of a block entity has no item slots (`entity.items` is
    /// empty there): send the stacks you draw in
    /// `native_block_entity_update_tag` and read them from `entity.nbt`.
    pub(super) fn native_draw_item(
        &self,
        draw: &mut BlockEntityDraw,
        stack: &ItemStack,
        display: DisplayContext,
        owner_yaw: Option<f32>,
        seed: i32,
    ) -> bool {
        // ShelfBlockEntity.position(): Vec3.atCenterOf(getBlockPos()).
        let owner = owner_yaw
            .map(|yaw| super::item_draw::owner_at(draw.block.map(|c| f64::from(c) + 0.5), yaw, ""));
        let item = self.resolve_item(stack, display, owner.as_ref(), seed);
        if item.is_empty() {
            return false;
        }
        let pose = draw.pose();
        self.submit_item_world(&item, pose, draw.light, &mut draw.vertices);
        true
    }

    /// Property `key` of block state `state` (a machine's `facing`, from the
    /// `state` `native_block_entity_submit` gets), if it has one: no lookup in
    /// the world, so cheap each frame.
    pub(super) fn block_state_property(&self, state: BlockStateId, key: &str) -> Option<&str> {
        self.pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .map(String::as_str)
    }
    /// The block `state` is a state of: its name, as `GameLevel`'s helper of
    /// the same name gives it for the hooks that get a state in the world.
    pub(super) fn native_state_block(&self, state: BlockStateId) -> Option<&str> {
        self.pack.assets.registry.state(state).map(|s| &*s.name)
    }
}

/// `ModelPart.render`: `part`'s faces under its own pose
/// (`translateAndRotate`), then its children under theirs.
fn draw_model_part(draw: &mut BlockEntityDraw, part: &ModelPart, sprite: Rect, tint: u32) {
    if !part.visible || (part.is_empty() && part.children().next().is_none()) {
        return;
    }
    draw.push();
    draw.apply(part_transform(part));
    if !part.skip_draw {
        for polygon in &part.polygons {
            let corners = polygon.vertices.map(|v| v.pos.map(|c| c / 16.0));
            let uv = polygon.vertices.map(|v| [v.u, v.v]);
            draw.quad(corners, uv, sprite, tint);
        }
    }
    for (_, child) in part.children() {
        draw_model_part(draw, child, sprite, tint);
    }
    draw.pop();
}

/// `ModelPart.translateAndRotate`: to the part's place in blocks, turned
/// about z, then y, then x (`rotationZYX`), then scaled.
fn part_transform(part: &ModelPart) -> Mat4 {
    let mut m = Mat4::from_translation(Vec3::new(part.x, part.y, part.z) / 16.0);
    if part.x_rot != 0.0 || part.y_rot != 0.0 || part.z_rot != 0.0 {
        m *= Mat4::from_quat(Quat::from_euler(
            glam::EulerRot::ZYX,
            part.z_rot,
            part.y_rot,
            part.x_rot,
        ));
    }
    if part.x_scale != 1.0 || part.y_scale != 1.0 || part.z_scale != 1.0 {
        m *= Mat4::from_scale(Vec3::new(part.x_scale, part.y_scale, part.z_scale));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::containers::{ModBlockEntity, ModBlockEntityFields};

    fn draw() -> BlockEntityDraw {
        BlockEntityDraw {
            partial: 0.0,
            game_time: 0,
            animation: 0.0,
            light: 0xF0F0,
            block: [0; 3],
            poses: vec![Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0))],
            lights: diffuse_lights(false),
            vertices: Vec::new(),
            kept: false,
            spins: Vec::new(),
            prepared: false,
            records: Vec::new(),
        }
    }

    fn sprite() -> Rect {
        Rect {
            u0: 0.5,
            v0: 0.25,
            u1: 0.75,
            v1: 0.5,
            w: 16,
            h: 16,
        }
    }

    /// A quad standing still has its light baked into its colour, as before; one drawn under
    /// `spin` keeps its plain tint and its outward normal (`w = 127`, the shader lights it), and
    /// once turned its normal points where the face does: up turned a quarter about +x faces +z.
    #[test]
    fn a_turning_quad_is_lit_where_it_turns_to() {
        let mut draw = draw();
        let top = [
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
        ];
        let tint = 0x80_40_20;
        draw.quad(top, [[0.0; 2]; 4], sprite(), tint);
        draw.spin([1.0, 0.0, 0.0], [0.5; 3], 90.0, 0.0);
        draw.quad(top, [[0.0; 2]; 4], sprite(), tint);
        let (still, turning) = draw.vertices.split_at(4);
        let lit = shaded(tint, diffuse(Vec3::Y, draw.lights));
        assert!(still.iter().all(|v| v.normal == [0; 4] && v.color == lit));
        assert!(
            turning
                .iter()
                .all(|v| v.normal == [0, 127, 0, 127] && v.color == shaded(tint, 1.0))
        );
        let mut out = Vec::new();
        place_turning(&draw.vertices, &draw.spins, 1.0, Vec3::ZERO, &mut out);
        assert!(out[..4].iter().all(|v| v.normal == [0; 4]));
        assert!(out[4..].iter().all(|v| v.normal == [0, 0, 127, 127]));
    }

    #[test]
    fn non_finite_quad_submission_leaves_geometry_unchanged() {
        let mut surface = draw();
        let corners = [
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let uv = [[0.0; 2]; 4];
        surface.quad(corners, uv, sprite(), 0xFFFFFF);
        assert_eq!(surface.vertices.len(), 4);
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut bad_corners = corners;
            bad_corners[1][2] = bad;
            surface.quad(bad_corners, uv, sprite(), 0xFFFFFF);
            let mut bad_uv = uv;
            bad_uv[2][1] = bad;
            surface.quad(corners, bad_uv, sprite(), 0xFFFFFF);
        }
        assert_eq!(surface.into_vertices().len(), 4);
    }

    #[test]
    fn prepared_faces_transform_and_preserve_tint_and_emission() {
        use super::super::minecarts::{PreparedModelFace, emit_model_faces};
        let mut quad = bake_flat_face(
            [0.0; 3],
            [16.0; 3],
            [0.0, 0.0, 16.0, 16.0],
            Direction::Up,
            Material {
                sprite: String::new(),
                force_translucent: false,
            },
            0,
        )
        .unwrap();
        quad.positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        quad.uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        quad.light_emission = 12;
        let tinted = PreparedModelFace::new(&quad, sprite());
        quad.tint_index = -1;
        let plain = PreparedModelFace::new(&quad, sprite());
        let pose = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 3.0, 4.0),
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(10.0, 20.0, 30.0),
        );
        let mut out = Vec::new();
        emit_model_faces(
            [(tinted, 0x2030), (plain, 0x2030)],
            pose,
            true,
            |index| (index >= 0).then_some(0x123456),
            &mut out,
        );
        let positions = [
            [10.0, 20.0, 30.0],
            [10.0, 22.0, 30.0],
            [7.0, 22.0, 30.0],
            [7.0, 20.0, 30.0],
        ];
        let uvs = [[0.5, 0.25], [0.75, 0.25], [0.75, 0.5], [0.5, 0.5]];
        for (i, v) in out.iter().enumerate() {
            for axis in 0..3 {
                assert!((v.pos[axis] - positions[i % 4][axis]).abs() < 0.00001);
            }
            assert_eq!(v.uv, uvs[i % 4]);
            assert_eq!(v.normal, [-127, 0, 0, 127]);
            assert_eq!(v.light, 0xC030);
            assert_eq!(v.color, if i < 4 { 0xFF563412 } else { u32::MAX });
        }
        assert_eq!(out.len(), 8);
    }

    fn marker(pos: f32) -> Vertex {
        Vertex {
            pos: [pos, 0.0, 0.0],
            uv: [0.0, 0.0],
            layer: 0,
            color: 0,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0, 0.0],
        }
    }

    #[test]
    fn prepared_route_stays_cpu_for_either_shader_pack() {
        assert!(super::prepared_route(false, false, Some(4), 4));
        assert!(!super::prepared_route(true, false, Some(4), 4), "iris pack");
        assert!(
            !super::prepared_route(false, true, Some(4), 4),
            "horizon pack"
        );
        assert!(!super::prepared_route(false, false, None, 4));
        assert!(!super::prepared_route(false, false, Some(3), 4));
    }

    #[test]
    fn prepared_models_stay_ordered_and_cpu_inside_the_block_span() {
        use super::super::minecarts::{PreparedModelFace, emit_model_faces};
        let mut quad = bake_flat_face(
            [0.0; 3],
            [16.0; 3],
            [0.0, 0.0, 16.0, 16.0],
            Direction::South,
            Material {
                sprite: String::new(),
                force_translucent: false,
            },
            0,
        )
        .unwrap();
        quad.positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        quad.uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let face = PreparedModelFace::new(&quad, sprite());
        let slots = vec![Some(vec![face])];
        let mut gpu = draw();
        gpu.prepared = true;
        gpu.light = 0xF030;
        gpu.vertices.extend((0..4).map(|i| marker(i as f32)));
        let span = gpu.vertices.len();
        assert!(gpu.defer_model(0, 3, 0xFF80_C040));
        gpu.vertices.extend((0..4).map(|i| marker(10.0 + i as f32)));
        let origin = DVec3::new(8_000_000.5, 70.0, -4_000_000.25);
        let camera = DVec3::new(8_000_000.0, 70.2, -4_000_000.0);
        let mut instances = Vec::new();
        gpu.push_instances(origin, camera, &mut instances);
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].cpu_vertices, span as u32);
        assert_eq!(gpu.vertices.len(), 8, "a prepared copy is not in the span");
        assert!((0..4).all(|i| gpu.vertices[i].pos[0] < 4.0));
        assert!((4..8).all(|i| gpu.vertices[i].pos[0] >= 10.0));

        let mut cpu = draw();
        cpu.prepared = false;
        assert!(
            !cpu.defer_model(0, 3, 0xFF80_C040),
            "packs, gui and mobs stay on the cpu"
        );
        cpu.prepared = true;
        cpu.vertices.extend((0..4).map(|i| marker(i as f32)));
        assert!(cpu.defer_model(0, 3, 0xFF80_C040));
        let pose = cpu.records[0].pose;
        cpu.spin([0.0, 1.0, 0.0], [0.5; 3], 12.0, 0.0);
        cpu.bake_recorded(&slots);
        let mut expected = Vec::new();
        emit_model_faces(
            [(face, cpu.light)],
            pose,
            true,
            |index| (index >= 0).then_some(0xFF80_C040),
            &mut expected,
        );
        assert_eq!(cpu.vertices.len(), 4 + expected.len());
        assert_eq!(cpu.spins[0].0, 4 + expected.len());
        for (got, want) in cpu.vertices[4..].iter().zip(&expected) {
            assert_eq!(got.pos, want.pos);
            assert_eq!(got.uv, want.uv);
            assert_eq!(got.color, want.color);
            assert_eq!(got.light, want.light);
            assert_eq!(got.normal, want.normal);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn prepared_model_entity_pixels() {
        use mod_api::__private::game::Fields;
        let _fixture =
            horizon_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let model = game
                .render_assets()
                .model(&Identifier::parse("minecraft:block/grass_block").unwrap())
                .unwrap();
            let prepared = game.standalone_geometry.slots[model.fields().slot as usize]
                .as_ref()
                .unwrap();
            let face_index = prepared
                .iter()
                .position(|face| face.facing == Direction::South && face.tint_index >= 0)
                .unwrap();
            let mut surface = draw();
            surface.poses = vec![Mat4::from_rotation_z(0.25)];
            surface.light = 0x7030;
            game.native_draw_model(&mut surface, model, 0xFF80_C040)
                .unwrap();
            let vertices = &surface.vertices[face_index * 4..face_index * 4 + 4];
            assert!(vertices.iter().all(|v| v.light == 0x7030));
            let atlas = &game.gui.atlas;
            let mut pixels = vec![255; (atlas.width * atlas.height * 4) as usize];
            // Prepared indices exclude baked faces without an atlas sprite.
            let source = game.pack.render.resolve_model(model).unwrap();
            let sprite = source
                .faces
                .iter()
                .filter_map(|face| {
                    game.pack
                        .layer_names
                        .get(&face.layer)
                        .and_then(|name| atlas.sprites.get(name))
                        .copied()
                })
                .nth(face_index)
                .unwrap();
            let x0 = (sprite.u0 * atlas.width as f32).round() as u32;
            let x1 = (sprite.u1 * atlas.width as f32).round() as u32;
            let y0 = (sprite.v0 * atlas.height as f32).round() as u32;
            let y1 = (sprite.v1 * atlas.height as f32).round() as u32;
            for y in y0..y1 {
                for x in x0..(x0 + x1) / 2 {
                    pixels[((y * atlas.width + x) * 4 + 3) as usize] = 0;
                }
            }
            let view = Mat4::from_scale_rotation_translation(
                Vec3::new(1.4, 1.4, 0.2),
                Quat::IDENTITY,
                Vec3::new(-0.55, -0.85, 0.5),
            );
            let rendered = horizon_client::gfx::entity_pixel_test::render(
                vertices,
                [atlas.width, atlas.height],
                &pixels,
                view,
                false,
            )
            .expect("software Vulkan entity pixels");
            let coverage = rendered
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
                .count();
            let pixel = |x: usize, y: usize| &rendered[(y * 64 + x) * 4..(y * 64 + x + 1) * 4];
            assert_eq!(pixel(32, 32), [48, 107, 47, 255]);
            assert_eq!(pixel(0, 0), [0, 0, 0, 255]);
            assert!((950..1050).contains(&coverage), "coverage was {coverage}");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_full_instance_buffer_bakes_the_tail_after_the_cut() {
        use super::super::minecarts::PreparedModelFace;
        use horizon_client::gfx::cap_prepared_instance_slots;
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                cap_prepared_instance_slots(u64::MAX);
            }
        }
        let _reset = Reset;
        let mut quad = bake_flat_face(
            [0.0; 3],
            [0.0; 3],
            [0.0; 4],
            Direction::South,
            Material {
                sprite: String::new(),
                force_translucent: false,
            },
            0,
        )
        .unwrap();
        quad.positions = [[0.0; 3]; 4];
        let face = PreparedModelFace::new(&quad, sprite());
        // kind 0 is an ordinary quad at x; kind 1 is a prepared copy (x, tint, light).
        let scene = [
            (0, 1.0, 0, 0),                 // A
            (1, 10.0, 0xFF01_0203, 0x1110), // B
            (1, 20.0, 0xFF04_0506, 0x2220), // C, same cut as B
            (0, 2.0, 0, 0),                 // D
            (1, 30.0, 0xFF07_0809, 0x3330), // E
            (0, 3.0, 0, 0),                 // F
        ];
        let split = [
            (0, 1.0, 0, 0),
            (1, 10.0, 0xFF01_0203, 0x1110),
            (1, 20.0, 0xFF04_0506, 0x2220),
            (1, 40.0, 0xFF0A_0B0C, 0x4440),
            (0, 2.0, 0, 0),
        ];
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.standalone_geometry.slots = vec![Some(vec![face])];
            let play = |cap, steps: &[_]| {
                cap_prepared_instance_slots(cap);
                let mut draw = draw();
                draw.prepared = true;
                for (kind, x, tint, light) in steps {
                    if *kind == 0 {
                        draw.vertices.extend(std::iter::repeat_n(marker(*x), 4));
                    } else {
                        draw.light = *light;
                        draw.poses = vec![Mat4::from_translation(Vec3::new(*x, 0.0, 0.0))];
                        assert!(draw.defer_model(0, 1, *tint));
                    }
                }
                game.prepared_model_draws.borrow_mut().clear();
                game.finish_prepared_draw(&mut draw, DVec3::ZERO, DVec3::ZERO);
                assert!(draw.records.is_empty());
                draw
            };
            let xs = |d: &BlockEntityDraw| {
                d.vertices
                    .iter()
                    .step_by(4)
                    .map(|v| v.pos[0])
                    .collect::<Vec<_>>()
            };
            let paint = |d: &BlockEntityDraw, i: usize, color: u32, light: u32| {
                assert_eq!((d.vertices[i].color, d.vertices[i].light), (color, light));
            };
            let cuts = |game: &Game| {
                game.prepared_model_draws
                    .borrow()
                    .iter()
                    .map(|c| c.cpu_vertices)
                    .collect::<Vec<_>>()
            };
            let same = |got: Vec<f32>, want: &[f32]| assert_eq!(got.as_slice(), want);
            let staged = |game: &Game, want: &[u32]| assert_eq!(cuts(game).as_slice(), want);
            let draw = play(1, &scene);
            same(xs(&draw), &[1.0, 20.0, 2.0, 30.0, 3.0]);
            paint(&draw, 4, 0xFF06_0504, 0x2220);
            paint(&draw, 12, 0xFF09_0807, 0x3330);
            staged(game, &[4]);
            assert_eq!(
                game.prepared_model_draws.borrow()[0]
                    .instance
                    .matrix()
                    .w_axis
                    .x,
                10.0
            );
            same(xs(&play(0, &scene)), &[1.0, 10.0, 20.0, 2.0, 30.0, 3.0]);
            assert!(cuts(game).is_empty());
            same(xs(&play(3, &scene)), &[1.0, 2.0, 3.0]);
            staged(game, &[4, 4, 8]);
            let over = play(2, &scene);
            same(xs(&over), &[1.0, 2.0, 30.0, 3.0]);
            paint(&over, 8, 0xFF09_0807, 0x3330);
            staged(game, &[4, 4]);
            let tail = play(1, &split);
            same(xs(&tail), &[1.0, 20.0, 40.0, 2.0]);
            paint(&tail, 4, 0xFF06_0504, 0x2220);
            paint(&tail, 8, 0xFF0C_0B0A, 0x4440);
            staged(game, &[4]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn prepared_instance_matches_cpu_pixels() {
        use glam::DVec3;
        use horizon_client::gfx::fail_next_prepared_upload;
        use mod_api::__private::game::Fields;
        let _fixture =
            horizon_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let model = game
                .render_assets()
                .model(&Identifier::parse("minecraft:block/grass_block").unwrap())
                .unwrap();
            let slot = model.fields().slot;
            let local = Mat4::from_scale_rotation_translation(
                Vec3::new(1.2, 0.5, 1.7),
                Quat::from_rotation_z(0.25),
                Vec3::ZERO,
            );
            let origin = DVec3::new(8_000_000.5, 70.0, -4_000_000.25);
            let camera = DVec3::new(8_000_000.0, 70.2, -4_000_000.0);
            let pose = horizon_client::gfx::camera_matrix(local, origin, camera);
            let mut surface = draw();
            surface.poses = vec![pose];
            surface.light = 0xF030;
            game.native_draw_model(&mut surface, model, 0xFF80_C040)
                .unwrap();
            let instance = super::prepared_instance(local, origin, camera, 0xFF80_C040, 0xF030);
            assert_eq!(instance.matrix(), pose);
            let (vertices, ranges) = game.standalone_geometry.prepared_upload();
            let atlas = &game.gui.atlas;
            let mut pixels = atlas.pixels().to_vec();
            let face_index = game.standalone_geometry.slots[slot as usize]
                .as_ref()
                .unwrap()
                .iter()
                .position(|face| face.facing == Direction::South && face.tint_index >= 0)
                .unwrap();
            let sprite = game
                .pack
                .render
                .resolve_model(model)
                .unwrap()
                .faces
                .iter()
                .filter_map(|face| {
                    game.pack
                        .layer_names
                        .get(&face.layer)
                        .and_then(|name| atlas.sprites.get(name))
                        .copied()
                })
                .nth(face_index)
                .unwrap();
            let x0 = (sprite.u0 * atlas.width as f32).round() as u32;
            let x1 = (sprite.u1 * atlas.width as f32).round() as u32;
            let y0 = (sprite.v0 * atlas.height as f32).round() as u32;
            let y1 = (sprite.v1 * atlas.height as f32).round() as u32;
            for y in y0..y1 {
                for x in x0..(x0 + x1) / 2 {
                    pixels[((y * atlas.width + x) * 4 + 3) as usize] = 0;
                }
            }
            let view = Mat4::from_scale_rotation_translation(
                Vec3::new(1.4, 1.4, 0.2),
                Quat::IDENTITY,
                Vec3::new(-0.55, -0.85, 0.5),
            );
            let cpu = horizon_client::gfx::entity_pixel_test::render(
                &surface.vertices,
                [atlas.width, atlas.height],
                &pixels,
                view,
                true,
            )
            .expect("software Vulkan entity pixels");
            let uploaded = game.standalone_geometry.generation();
            let instance_pixels = |ranges, requested| {
                horizon_client::gfx::entity_pixel_test::render_instance(
                    horizon_client::gfx::entity_pixel_test::PreparedInstanceDraw {
                        vertices: &vertices,
                        ranges,
                        uploaded,
                        requested,
                        slot,
                        instances: std::slice::from_ref(&instance),
                        per_copy: false,
                        hdr: true,
                        atlas_size: [atlas.width, atlas.height],
                        atlas: &pixels,
                        view_proj: view,
                    },
                )
            };
            let gpu = instance_pixels(ranges, uploaded).expect("software Vulkan prepared instance");
            let mut worst = 0u8;
            for (cpu, gpu) in cpu.iter().zip(gpu.iter()) {
                worst = worst.max(cpu.abs_diff(*gpu));
            }
            assert!(worst <= 2, "cpu and instance pixels differed by {worst}");
            // The HDR mark uses bias 0 on both routes, so it does not land on the
            // colour pass's biased depth. Coverage is the colour, not a mid alpha.
            assert!(
                cpu.as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
            );
            fail_next_prepared_upload();
            assert!(
                instance_pixels(game.standalone_geometry.prepared_upload().1, uploaded).is_err(),
                "a forced upload failure must not draw"
            );
            assert!(
                instance_pixels(
                    game.standalone_geometry.prepared_upload().1,
                    uploaded.wrapping_add(1),
                )
                .is_err(),
                "a stale generation must not draw"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn resolved_model_draws_and_resource_replacement_invalidates_it() {
        use super::super::host::draw::GameBlockEntityDrawContext;
        use mod_api::BlockEntityDrawContext;

        let _fixture =
            horizon_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let id = Identifier::parse("minecraft:block/grass_block").unwrap();
            let model = game.render_assets().model(&id).unwrap();
            for phase in [
                mod_api::__private::ModPhase::Setup,
                mod_api::__private::ModPhase::Reloading,
            ] {
                let _scope = mod_api::__private::PhaseOwner::scope(phase);
                assert_eq!(game.render_assets().model(&id), Ok(model));
            }
            assert_eq!(
                game.render_assets()
                    .model(&Identifier::parse("mymod:block/absent").unwrap()),
                Err(ModelError::UnknownModel)
            );
            let mut surface = draw();
            surface.poses.push(Mat4::from_scale_rotation_translation(
                Vec3::new(2.0, 0.5, 1.5),
                Quat::from_rotation_y(0.7) * Quat::from_rotation_x(0.3),
                Vec3::new(8.0, -4.0, 3.0),
            ));
            surface.light = 0x7030;
            // Compare the consumer with the existing shared emitter, under the same pose,
            // tint, atlas and light: handle resolution must not change any vertex bytes.
            let mut expected = Vec::new();
            game.baked_model_vertices(
                [game
                    .pack
                    .render
                    .model_catalog
                    .standalone("minecraft:block/grass_block")
                    .unwrap()],
                surface.pose(),
                |_| surface.light,
                true,
                |index| (index >= 0).then_some(0xFF12_3456),
                &mut expected,
            );
            assert!(!expected.is_empty());
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            assert_eq!(drawn.model(model, 0xFF12_3456), Ok(()));
            let bytes = |vertices: &[Vertex]| bytemuck::cast_slice::<_, u8>(vertices).to_vec();
            assert_eq!(bytes(&surface.vertices), bytes(&expected));
            assert!(surface.vertices.iter().all(|v| v.light == 0x7030));
            // An explicitly emissive tinted face and its untinted twin exercise both
            // branches independently of the vanilla grass model's current contents.
            let source = game.pack.render.resolve_model(model).unwrap();
            let face = &source.faces[0];
            let sprite = game
                .pack
                .layer_names
                .get(&face.layer)
                .and_then(|name| game.gui.atlas.sprites.get(name))
                .copied()
                .unwrap();
            let mut tinted = face.quad.clone();
            tinted.tint_index = 0;
            tinted.light_emission = 12;
            let mut plain = tinted.clone();
            plain.tint_index = -1;
            let mut emitted = Vec::new();
            super::super::minecarts::emit_model_faces(
                [
                    (
                        super::super::minecarts::PreparedModelFace::new(&tinted, sprite),
                        0x2030,
                    ),
                    (
                        super::super::minecarts::PreparedModelFace::new(&plain, sprite),
                        0x2030,
                    ),
                ],
                Mat4::IDENTITY,
                true,
                |index| (index >= 0).then_some(0x123456),
                &mut emitted,
            );
            assert_eq!(emitted.len(), 8);
            assert!(emitted.iter().all(|v| v.light == 0xC030));
            assert!(emitted[..4].iter().all(|v| v.color == 0xFF563412));
            assert!(emitted[4..].iter().all(|v| v.color == u32::MAX));

            // Same content and emitter on each side; this times CPU work only.
            let mut scratch = Vec::with_capacity(expected.len());
            let rounds = 20_000;
            let start = std::time::Instant::now();
            for _ in 0..rounds {
                scratch.clear();
                game.baked_model_vertices(
                    [source],
                    surface.pose(),
                    |_| surface.light,
                    true,
                    |index| (index >= 0).then_some(0x123456),
                    &mut scratch,
                );
                std::hint::black_box(&scratch);
            }
            let before_time = start.elapsed();
            let start = std::time::Instant::now();
            for _ in 0..rounds {
                surface.vertices.clear();
                game.native_draw_model(&mut surface, model, 0xFF12_3456)
                    .unwrap();
                std::hint::black_box(&surface.vertices);
            }
            eprintln!(
                "CPU model {rounds} draws: lookup {before_time:?}, prepared {:?}",
                start.elapsed()
            );
            let before = bytes(&surface.vertices);
            let invalid =
                mod_api::__private::built::model_ref(game.pack.render.generation, u32::MAX);
            assert_eq!(
                game.native_draw_model(&mut surface, invalid, 0xFFFF_FFFF),
                Err(ModelError::UnknownModel)
            );
            assert_eq!(bytes(&surface.vertices), before);

            // A failed resource build never publishes a replacement, so the handle remains valid.
            let missing = game.options.resources.join("absent-model-test-root");
            let listeners = game.register_client_reload_listeners();
            assert!(
                super::super::packs::stage_pack(
                    &missing,
                    &[],
                    &[],
                    &game.mods,
                    listeners,
                    &[],
                    &[],
                    &game.pack.assets,
                    &game.res,
                    &game.pack.context,
                    false,
                    false,
                    &super::super::Progress::new(),
                )
                .is_err()
            );
            assert_eq!(
                game.native_draw_model(&mut surface, model, 0xFF12_3456),
                Ok(())
            );
            let before = bytes(&surface.vertices);
            // Identical texture bytes still produce a new model generation.
            let old_generation = game.pack.render.generation;
            let listeners = game.register_client_reload_listeners();
            let staged = super::super::packs::stage_pack(
                &game.options.resources,
                &[],
                &[],
                &game.mods,
                listeners,
                &[],
                &[],
                &game.pack.assets,
                &game.res,
                &game.pack.context,
                false,
                false,
                &super::super::Progress::new(),
            )
            .unwrap();
            game.apply_staged_pack(staged).unwrap();
            assert_ne!(game.pack.render.generation, old_generation);
            assert_eq!(
                game.native_draw_model(&mut surface, model, 0xFF12_3456),
                Err(ModelError::StaleModel)
            );
            assert_eq!(bytes(&surface.vertices), before);
            let fresh = game.render_assets().model(&id).unwrap();
            assert_ne!(fresh, model);
            assert_eq!(
                game.native_draw_model(&mut surface, fresh, 0xFF12_3456),
                Ok(())
            );
        });
    }

    fn rows(m: Mat4) -> [f32; 12] {
        let c = m.to_cols_array();
        [
            c[0], c[4], c[8], c[12], c[1], c[5], c[9], c[13], c[2], c[6], c[10], c[14],
        ]
    }

    #[test]
    #[ignore = "daily"]
    fn model_tint_alpha_agrees_on_keep_deferred_and_batch() {
        use super::super::host::draw::{GameBlockEntityDrawContext, GameWorldDrawContext};
        use horizon_client::gfx::entity_pixel_test::{self, PreparedInstanceDraw};
        use mod_api::{
            BlockEntityDrawContext, Identifier, LightCoords, ModelInstance, Overlay,
            WorldDrawContext,
        };
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                horizon_client::gfx::cap_prepared_instance_slots(u64::MAX);
                horizon_client::gfx::set_test_prepared_generation(None);
            }
        }
        let _fixture =
            horizon_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _restore = Restore;
            let id = Identifier::parse("minecraft:block/grass_block").unwrap();
            let model = game.render_assets().model(&id).unwrap();
            let generation = mod_api::__private::game::Fields::fields(&model).generation;
            horizon_client::gfx::set_test_prepared_generation(Some(generation));
            horizon_client::gfx::cap_prepared_instance_slots(8);
            let local = Mat4::from_scale_rotation_translation(
                Vec3::new(0.85, 1.15, 0.7),
                Quat::from_rotation_y(0.6),
                Vec3::new(0.12, -0.05, 0.08),
            );
            let origin = DVec3::new(8_000_000.5, 70.0, -4_000_000.25);
            let camera = DVec3::new(8_000_000.0, 70.2, -4_000_000.0);
            let tint = 0x40FF_3040;
            let mut spun = draw();
            spun.prepared = true;
            spun.light = 0xF0F0;
            spun.poses = vec![local];
            spun.spin([0.0, 1.0, 0.0], [0.5; 3], 0.0, 0.0);
            {
                let mut ctx = GameBlockEntityDrawContext {
                    game,
                    surface: &mut spun,
                };
                ctx.model(model, tint).unwrap();
            }
            let mut kept = draw();
            kept.prepared = true;
            kept.light = 0xF0F0;
            kept.poses = vec![local];
            {
                let mut ctx = GameBlockEntityDrawContext {
                    game,
                    surface: &mut kept,
                };
                ctx.model(model, tint).unwrap();
                ctx.keep();
            }
            game.finish_prepared_draw(&mut kept, origin, camera);
            assert_eq!(spun.vertices.len(), kept.vertices.len());
            for (spun, kept) in spun.vertices.iter().zip(kept.vertices.iter()) {
                assert_eq!(spun.color, kept.color, "spin and keep tint alpha differ");
            }
            let slot = mod_api::__private::game::Fields::fields(&model).slot;
            let faces = game.standalone_geometry.faces(slot).unwrap();
            let (quads, _) = kept.vertices.as_chunks::<4>();
            assert!(
                quads
                    .iter()
                    .zip(faces)
                    .any(|(quad, face)| face.tint_index < 0 && quad[0].color >> 24 == 255),
                "an untinted face stays opaque"
            );
            assert!(
                quads
                    .iter()
                    .zip(faces)
                    .any(|(quad, face)| face.tint_index >= 0 && quad[0].color >> 24 == 0x40),
                "a tinted face takes the tint alpha"
            );
            let shift = (origin - camera).as_vec3();
            for vertex in &mut kept.vertices {
                vertex.pos = (Vec3::from(vertex.pos) + shift).to_array();
            }
            let mut deferred = draw();
            deferred.prepared = true;
            deferred.light = 0xF0F0;
            deferred.poses = vec![local];
            {
                let mut ctx = GameBlockEntityDrawContext {
                    game,
                    surface: &mut deferred,
                };
                ctx.model(model, tint).unwrap();
            }
            game.prepared_model_draws.borrow_mut().clear();
            game.finish_prepared_draw(&mut deferred, origin, camera);
            let deferred_instance = game.prepared_model_draws.borrow()[0].instance;
            game.prepared_model_draws.borrow_mut().clear();
            let mut batch = crate::world_geometry::WorldGeometry::default();
            batch.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
            let copy = ModelInstance::new(
                origin.to_array(),
                rows(local),
                tint,
                LightCoords::FULL_BRIGHT,
                Overlay::NONE,
            );
            {
                let mut ctx = GameWorldDrawContext {
                    game,
                    surface: &mut batch,
                };
                ctx.draw_model_instances(model, &[copy]).unwrap();
            }
            let mut entities = Vec::new();
            game.commit_model_instances(&mut batch, camera, &mut entities);
            let batch_instance = game.prepared_model_draws.borrow()[0].instance;
            let alpha = 0x40 as f32 / 255.0;
            assert!((deferred_instance.tint[3] - alpha).abs() < 1e-6);
            assert!((batch_instance.tint[3] - alpha).abs() < 1e-6);
            let atlas = &game.gui.atlas;
            let pixels = atlas.pixels();
            let uploaded = game.standalone_geometry.generation();
            let (vertices, ranges) = game.standalone_geometry.prepared_upload();
            let view = Mat4::from_scale_rotation_translation(
                Vec3::new(1.4, 1.4, 0.2),
                Quat::IDENTITY,
                Vec3::new(-0.55, -0.85, 0.5),
            );
            let cpu = entity_pixel_test::render(
                &kept.vertices,
                [atlas.width, atlas.height],
                &pixels,
                view,
                true,
            )
            .expect("keep pixels");
            let gpu = |instance: &horizon_client::gfx::PreparedModelInstance| {
                entity_pixel_test::render_instance(PreparedInstanceDraw {
                    vertices: &vertices,
                    ranges: ranges.clone(),
                    uploaded,
                    requested: uploaded,
                    slot,
                    instances: std::slice::from_ref(instance),
                    per_copy: false,
                    hdr: true,
                    atlas_size: [atlas.width, atlas.height],
                    atlas: &pixels,
                    view_proj: view,
                })
                .expect("instance pixels")
            };
            let deferred_pixels = gpu(&deferred_instance);
            let batch_pixels = gpu(&batch_instance);
            let worst = |a: &[u8], b: &[u8]| {
                a.iter()
                    .zip(b)
                    .fold(0u8, |worst, (p, q)| worst.max(p.abs_diff(*q)))
            };
            assert_eq!(worst(&cpu, &deferred_pixels), 0, "keep and deferred differ");
            assert_eq!(worst(&cpu, &batch_pixels), 0, "keep and batch differ");
        });
    }

    /// Under a `shaders/` pack, a batch copy and the per-copy `model()` route draw the same
    /// vertices in a block-entity run, so our probe pack (one flat colour per program) paints
    /// both with `gbuffers_block`. Without a pack the batch marks no run and bakes the same.
    #[test]
    #[ignore = "daily"]
    fn a_pack_shades_batch_and_per_copy_models_with_one_program() {
        use super::super::host::draw::{GameBlockEntityDrawContext, GameWorldDrawContext};
        use horizon_client::gfx::entity_pixel_test::{self, PACK_COLOURS};
        use mod_api::{BlockEntityDrawContext, LightCoords, ModelInstance, Overlay};
        use mod_api::{Identifier, WorldDrawContext};
        let _fixture =
            horizon_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let id = Identifier::parse("minecraft:block/grass_block").unwrap();
            let model = game.render_assets().model(&id).unwrap();
            let local = Mat4::from_scale_rotation_translation(
                Vec3::new(0.85, 1.15, 0.7),
                Quat::from_rotation_y(0.6),
                Vec3::new(0.12, -0.05, 0.08),
            );
            let corner = DVec3::new(8_000_000.0, 70.0, -4_000_001.0);
            let camera = DVec3::new(8_000_000.3, 70.4, -4_000_003.0);
            let tint = 0xFF80_C040;
            let light = LightCoords::new(3, 15).unwrap();
            let take_spans = |game: &Game<'_>| std::mem::take(&mut *game.entity_spans.borrow_mut());
            let batch = |game: &mut Game<'_>| {
                let mut draw = crate::world_geometry::WorldGeometry::default();
                draw.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
                let copy =
                    ModelInstance::new(corner.to_array(), rows(local), tint, light, Overlay::NONE);
                GameWorldDrawContext {
                    game,
                    surface: &mut draw,
                }
                .draw_model_instances(model, &[copy])
                .unwrap();
                let mut entities = Vec::new();
                game.commit_model_instances(&mut draw, camera, &mut entities);
                (entities, take_spans(game))
            };
            take_spans(game);
            let (plain, plain_spans) = batch(game);
            assert!(
                !plain.is_empty() && plain_spans.is_empty(),
                "no pack, no run"
            );

            game.pack_ids = Some(Default::default());
            // What `mod_block_entity_geometry` does around a mod's `model()` call.
            let mut per_copy = draw();
            per_copy.poses = vec![local];
            per_copy.light = 0x30F0;
            game.mark_block_entity(0, 1);
            GameBlockEntityDrawContext {
                game,
                surface: &mut per_copy,
            }
            .model(model, tint)
            .unwrap();
            game.finish_prepared_draw(&mut per_copy, corner, camera);
            let shift = (corner - camera).as_vec3();
            for vertex in &mut per_copy.vertices {
                vertex.pos = (Vec3::from(vertex.pos) + shift).to_array();
            }
            let per_copy_spans = take_spans(game);
            let (copies, copy_spans) = batch(game);
            // Independent run-boundary expectations for equal and alternating overlays.
            for (overlays, expected) in [
                (vec![], vec![]),
                (vec![Overlay::NONE; 3], vec![0]),
                (
                    vec![Overlay::NONE, Overlay::new(0, true).unwrap(), Overlay::NONE],
                    vec![0, 1, 2],
                ),
            ] {
                let mut surface = crate::world_geometry::WorldGeometry::default();
                surface.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
                let instances: Vec<_> = overlays
                    .into_iter()
                    .map(|overlay| {
                        ModelInstance::new(corner.to_array(), rows(local), tint, light, overlay)
                    })
                    .collect();
                GameWorldDrawContext {
                    game,
                    surface: &mut surface,
                }
                .draw_model_instances(model, &instances)
                .unwrap();
                let mut vertices = Vec::new();
                game.commit_model_instances(&mut surface, camera, &mut vertices);
                let spans = take_spans(game);
                assert_eq!(
                    spans.iter().map(|s| s.start as usize).collect::<Vec<_>>(),
                    expected
                        .into_iter()
                        .map(|i| i * copies.len())
                        .collect::<Vec<_>>()
                );
                assert!(spans.iter().all(
                    |s| s.id == 0 && s.stage == horizon_shaders::render_stage("BLOCK_ENTITIES")
                ));
            }
            game.pack_ids = None;
            let bytes = |v: &[Vertex]| bytemuck::cast_slice::<Vertex, u8>(v).to_vec();
            assert_eq!(bytes(&copies), bytes(&per_copy.vertices), "route vertices");
            assert_eq!(bytes(&copies), bytes(&plain), "the pack changed the bake");
            let pixels = |vertices: &[Vertex], spans: &[_]| {
                entity_pixel_test::render_pack(vertices, spans).expect("software Vulkan pack")
            };
            let reference = pixels(&per_copy.vertices, &per_copy_spans);
            let painted: Vec<_> = reference.iter().filter(|p| p[..3] != [0; 3]).collect();
            assert!(
                painted.len() > 20,
                "the probe pack drew {} pixels",
                painted.len()
            );
            assert!(
                painted.iter().all(|p| **p == PACK_COLOURS[1]),
                "gbuffers_block"
            );
            let batch = pixels(&copies, &copy_spans);
            let differing = batch.iter().zip(&reference).filter(|(a, b)| a != b).count();
            assert_eq!(differing, 0, "batch pixels differ from per-copy ones");
        });
    }

    /// Each mod block is asked once, whatever its number of states; vanilla blocks never are;
    /// an answer that isn't finite and above 0 keeps the default; the gather reaches the
    /// furthest.
    #[test]
    fn view_distances_are_asked_once_per_mod_block() {
        let states = [
            (1, "minecraft:stone"),
            (2, "mymod:beam"),
            (3, "mymod:beam"),
            (4, "mymod:screw"),
            (5, "mymod:gear"),
        ];
        let mut asked = Vec::new();
        let distances = ViewDistances::of_states(states.into_iter(), |block| {
            asked.push(block.to_owned());
            match block {
                "mymod:beam" => 256.0,
                "mymod:screw" => f64::NAN,
                _ => VIEW_DISTANCE,
            }
        });
        assert_eq!(asked, ["mymod:beam", "mymod:screw", "mymod:gear"]);
        assert_eq!(distances.of(2), 256.0);
        assert_eq!(distances.of(3), 256.0);
        assert_eq!(distances.of(4), VIEW_DISTANCE);
        assert_eq!(distances.of(5), VIEW_DISTANCE);
        assert_eq!(distances.of(1), VIEW_DISTANCE);
        assert_eq!(distances.furthest, 256.0);
        let none = ViewDistances::of_states(std::iter::empty(), |_| unreachable!());
        assert_eq!(none.furthest, VIEW_DISTANCE);
    }

    /// `BlockEntityDrawContext::text` lays the font's glyphs out under the pose, a quad per
    /// glyph from left to right, in the frame's light and the opaque colour it was given, with
    /// the text's styles: bold widens it and doubles its glyphs, and an outline (black too)
    /// rings each glyph with eight copies behind the text.
    #[test]
    #[ignore = "daily"]
    fn text_lays_glyphs_out_under_the_pose_in_the_frames_light() {
        use super::super::host::draw::GameBlockEntityDrawContext;
        use mod_api::BlockEntityDrawContext;

        super::super::native_entity_refs::tests::with_game(|game| {
            // The server fixture only measures fonts; this test needs drawable glyphs.
            game.gui.atlas = horizon_client::hud::HudAtlas::build(
                &horizon_assets::resource_source::ResourceSource::directory(
                    game.options.resources.to_path_buf(),
                ),
            )
            .unwrap();
            let plain = mod_api::Text::literal("AÁ");
            let mut style = mod_api::text::TextStyle::default();
            style.bold = Some(true);
            let bold = mod_api::Text::literal("A").with_style(style);
            let mut surface = draw();
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            assert_eq!(drawn.text_width(&mod_api::Text::literal("A")), Ok(6.0));
            assert_eq!(drawn.text_width(&plain), Ok(12.0));
            // Font.width: bold adds a pixel a glyph.
            assert_eq!(drawn.text_width(&bold), Ok(7.0));
            assert_eq!(
                drawn.text(&plain, [0.0, 0.0], 0xFF0000, false, None),
                Ok(12.0)
            );
            assert_eq!(surface.vertices.len(), 8, "a quad per glyph");
            // Vanilla's bitmap providers: A is 8x8 with ascent 7; Á is 9x12 with ascent 10.
            // GlyphBitmap places the top at 7 - ascent; BakedSheetGlyph adds the pen's position.
            let mut at: Vec<[f32; 3]> = surface.vertices.iter().map(|v| v.pos).collect();
            at.sort_by(|a, b| a.partial_cmp(b).unwrap());
            assert_eq!(
                at,
                [
                    [10.0, 0.0, 0.0],
                    [10.0, 8.0, 0.0],
                    [16.0, -3.0, 0.0],
                    [16.0, 9.0, 0.0],
                    [18.0, 0.0, 0.0],
                    [18.0, 8.0, 0.0],
                    [25.0, -3.0, 0.0],
                    [25.0, 9.0, 0.0],
                ]
            );
            // Red, as a vertex colour stores it (ABGR), opaque.
            assert!(surface.vertices.iter().all(|v| v.color == 0xFF00_00FF));
            assert!(surface.vertices.iter().all(|v| v.light == 0xF0F0));

            // Bold: each glyph again a pixel to the right (Font's bold offset).
            surface.vertices.clear();
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            assert_eq!(drawn.text(&bold, [0.0, 0.0], 0xFFFFFF, true, None), Ok(7.0));
            assert_eq!(surface.vertices.len(), 8, "the glyph twice");
            assert!(surface.vertices.iter().all(|v| v.light == 240 | 240 << 8));

            // A black outline: eight rings behind, then the text a hair in front of them.
            surface.vertices.clear();
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            drawn
                .text(
                    &mod_api::Text::literal("A"),
                    [0.0, 0.0],
                    0xFFFFFF,
                    true,
                    Some(0),
                )
                .unwrap();
            assert_eq!(surface.vertices.len(), 9 * 4);
            let (rings, front) = surface.vertices.split_at(8 * 4);
            assert!(
                rings
                    .iter()
                    .all(|v| v.color == 0xFF00_0000 && v.pos[2] == 0.0)
            );
            assert!(
                front
                    .iter()
                    .all(|v| v.color == u32::MAX && v.pos[2] == TEXT_OVER_OUTLINE)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn fractional_text_advance_rounds_up_for_drawing_and_measurement() {
        use super::super::host::draw::GameBlockEntityDrawContext;
        use horizon_assets::resource_source::{Resource, ResourceSource};
        use mod_api::BlockEntityDrawContext;
        super::super::native_entity_refs::tests::with_game(|game| {
            let source = ResourceSource::indexed(std::collections::BTreeMap::from([(
                "assets/minecraft/font/default.json".into(),
                Resource::bytes(
                    br#"{"providers":[{"type":"space","advances":{"X":2.25}}]}"#.to_vec(),
                ),
            )]));
            game.gui.atlas.font = horizon_client::font::Font::metrics(&source);
            let mut surface = draw();
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            let text = mod_api::Text::literal("XX");
            assert_eq!(drawn.text_width(&text), Ok(5.0));
            assert_eq!(drawn.text(&text, [0.0; 2], 0xFFFFFF, false, None), Ok(5.0));
            assert_eq!(
                drawn.text(&text, [0.0; 2], 0xFFFFFF, true, Some(0)),
                Ok(5.0)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn animation_clock_survives_pause_freeze_resume_and_time_updates() {
        use horizon_protocol::packets::play;
        use std::time::Duration;
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.clock = horizon_client::clock::FixedStep::client();
            game.texture_ticks = 0;
            game.clock.advance(Duration::from_millis(75), 50.0);
            game.tick_frame_visuals(true);
            assert_eq!(game.animation_time(), 1.5);
            game.clock.end_frame(true);
            game.clock.advance(Duration::from_millis(50), 50.0);
            assert_eq!(game.animation_time(), 1.5);
            game.clock.end_frame(false);
            game.clock.set_frozen(true);
            // Freeze requests become the frame's running decision at the
            // tick boundary, before end_frame snapshots it for interpolation.
            assert!(!game.clock.world_runs());
            game.clock.end_frame(false);
            assert_eq!(game.animation_time(), 2.0);
            game.clock.advance(Duration::from_millis(25), 50.0);
            let running = game.clock.world_runs();
            game.tick_frame_visuals(running);
            assert_eq!(game.texture_ticks, 1);
            assert_eq!(game.animation_time(), 2.0);
            assert_eq!(game.clock.camera_fraction(), 0.0);
            game.handle_packet(play::Clientbound::SetTime(play::SetTime {
                game_time: 0,
                clocks: vec![],
            }))
            .unwrap();
            assert_eq!(game.animation_time(), 2.0);
            game.clock.set_frozen(false);
            assert!(game.clock.world_runs());
            game.clock.end_frame(false);
            game.clock.advance(Duration::from_millis(75), 50.0);
            let running = game.clock.world_runs();
            game.tick_frame_visuals(running);
            assert_eq!(game.animation_time(), 2.5);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn atlas_reset_discards_queued_text_uploads() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.text_patches
                .borrow_mut()
                .push(horizon_client::font::Patch {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    rgba: vec![255; 4],
                });
            game.forget_atlas_slots();
            assert!(game.text_patches.borrow().is_empty());
        });
    }

    /// A face a mod builds with `CubeFace::new` and `with_uv` draws in its sprite through
    /// `BlockEntityDrawContext::cube`, its UVs cut from the sprite; an unknown sprite's face
    /// is left out and the call says so.
    #[test]
    #[ignore = "daily"]
    fn a_mod_built_cube_face_draws_in_its_sprite() {
        use super::super::host::draw::GameBlockEntityDrawContext;
        use mod_api::BlockEntityDrawContext;

        super::super::native_entity_refs::tests::with_game(|game| {
            // A server-side game holds only the font, so the sprite is given its place.
            game.gui
                .atlas
                .sprites
                .insert("mymod:block/casing".into(), sprite());
            let casing = sprite();
            let mut surface = draw();
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            let mut faces = [None; 6];
            // Up, the casing's top-left quarter.
            faces[1] =
                Some(mod_api::CubeFace::new("mymod:block/casing").with_uv([0.0, 0.0, 8.0, 8.0]));
            assert!(drawn.cube([0.0; 3], [16.0; 3], faces, 0xFFFFFF));
            assert_eq!(surface.vertices.len(), 4, "one face");
            let (mid_u, mid_v) = ((casing.u0 + casing.u1) / 2.0, (casing.v0 + casing.v1) / 2.0);
            for v in &surface.vertices {
                assert_eq!(v.pos[1], 1.0, "a 16-texel box's top, in blocks");
                assert!((casing.u0..=mid_u).contains(&v.uv[0]), "{:?}", v.uv);
                assert!((casing.v0..=mid_v).contains(&v.uv[1]), "{:?}", v.uv);
            }

            let mut surface = draw();
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            faces[0] = Some(mod_api::CubeFace::new("mymod:block/missing"));
            assert!(!drawn.cube([0.0; 3], [16.0; 3], faces, 0xFFFFFF));
            assert_eq!(surface.vertices.len(), 4, "the casing face alone");
        });
    }

    #[test]
    fn the_pose_stack_turns_about_its_pivot_and_pops_back() {
        let mut d = draw();
        d.push();
        d.rotate_about([0.0, 1.0, 0.0], 90.0, [0.5, 0.5, 0.5]);
        // (1, 0, 0.5) turns a quarter about the block's centre to (0.5, 0, 0).
        let p = d.pose().transform_point3(Vec3::new(1.0, 0.0, 0.5));
        assert!(p.abs_diff_eq(Vec3::new(10.5, 0.0, 0.0), 1e-5), "{p}");
        d.pop();
        d.pop();
        // Popping past the block's own pose keeps it.
        let p = d.pose().transform_point3(Vec3::ZERO);
        assert_eq!(p, Vec3::new(10.0, 0.0, 0.0));
        d.translate([0.0, 2.0, 0.0]);
        d.scale([2.0, 2.0, 2.0]);
        let p = d.pose().transform_point3(Vec3::ONE);
        assert_eq!(p, Vec3::new(12.0, 4.0, 2.0));
    }

    /// An arm a block east of the frame's origin, turned a quarter about z,
    /// with a hand a block below its pivot: `translateAndRotate` turns each
    /// part about its own pivot, then moves it, and a child goes with its
    /// parent. Expected corners are worked out by hand from JOML's
    /// `rotationZYX` (a right-handed quarter turn takes +x to +y).
    #[test]
    fn a_model_part_draws_turned_about_its_pivot_and_carries_its_children() {
        use horizon_core::geometry::Direction as Face;
        use mod_api::{CubeListBuilder, LayerDefinition, MeshDefinition, PartPose};
        let mut mesh = MeshDefinition::new();
        let root = mesh.root();
        let cube = |face| CubeListBuilder::create().add_box_faces([0.0; 3], [16.0; 3], &[face]);
        let arm = mesh.add_or_replace_child(
            root,
            "arm",
            cube(Face::South),
            PartPose::offset_and_rotation(16.0, 0.0, 0.0, 0.0, 0.0, std::f32::consts::FRAC_PI_2),
        );
        mesh.add_or_replace_child(
            arm,
            "hand",
            cube(Face::North),
            PartPose::offset(0.0, 16.0, 0.0),
        );
        let mut model = LayerDefinition::create(mesh, 64, 64).bake_root();
        let drawn = |model: &ModelPart| {
            let mut d = draw();
            draw_model_part(&mut d, model, sprite(), 0xFF_FFFF);
            d.into_vertices()
        };
        let near = |v: &Vertex, at: [f32; 3]| {
            assert!(
                Vec3::from(v.pos).abs_diff_eq(Vec3::from(at), 1e-5),
                "{:?} not {at:?}",
                v.pos
            );
        };

        let all = drawn(&model);
        assert_eq!(all.len(), 8);
        // The arm's south face, (0, 0, 1) (1, 0, 1) (1, 1, 1) (0, 1, 1) in
        // its frame, turned and moved a block east, then the frame's 10.
        for (v, at) in all.iter().zip([
            [11.0, 0.0, 1.0],
            [11.0, 1.0, 1.0],
            [10.0, 1.0, 1.0],
            [10.0, 0.0, 1.0],
        ]) {
            near(v, at);
        }
        // Its net's last strip, 48 to 64 across and 16 to 32 down, in the sprite.
        assert_eq!(all[0].uv, [0.75, 0.3125]);
        // The hand's north face starts at its (1, 0, 0), the arm's (1, 1, 0).
        near(&all[4], [10.0, 1.0, 0.0]);
        near(&all[5], [10.0, 0.0, 0.0]);

        let arm = model.get_child_mut("arm").unwrap();
        arm.skip_draw = true;
        let hand_only = drawn(&model);
        assert_eq!(hand_only.len(), 4);
        near(&hand_only[0], [10.0, 1.0, 0.0]);
        model.get_child_mut("arm").unwrap().visible = false;
        assert!(drawn(&model).is_empty());
    }

    /// A mod registers a layer (`register_layer_definitions`) at start: the game bakes it by its
    /// id and draws it, before and after a resource reload, beside vanilla's layers. The cube's
    /// south face, (0, 0, 1) to (1, 1, 1) in its part's frame, sits a block east of the part's
    /// origin and then the frame's 10.
    #[test]
    #[ignore = "daily"]
    fn a_registered_layer_bakes_and_draws_before_and_after_a_reload() {
        use horizon_core::geometry::Direction as Face;
        use mod_api::{CubeListBuilder, LayerDefinition, MeshDefinition, PartPose};
        struct Golem;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Golem {
            fn register_layer_definitions(
                &self,
                _: &mod_api::RegistrationContext,
                event: &mut mod_api::RegisterLayerDefinitionsEvent<'_>,
            ) {
                let mut mesh = MeshDefinition::new();
                let root = mesh.root();
                mesh.add_or_replace_child(
                    root,
                    "body",
                    CubeListBuilder::create().add_box_faces([0.0; 3], [16.0; 3], &[Face::South]),
                    PartPose::offset(16.0, 0.0, 0.0),
                );
                event.layers.push((
                    "golem_mod:golem#main".into(),
                    LayerDefinition::create(mesh, 64, 64),
                ));
            }
        }
        let drawn = |game: &Game<'_>| {
            let part = game
                .bake_layer("golem_mod:golem#main")
                .expect("the mod's layer bakes");
            let mut d = draw();
            assert!(game.native_draw_model_part(&mut d, &part, "entity/zombie/zombie", 0xFF_FFFF));
            d.into_vertices().iter().map(|v| v.pos).collect::<Vec<_>>()
        };
        let face = [
            [11.0, 0.0, 1.0],
            [12.0, 0.0, 1.0],
            [12.0, 1.0, 1.0],
            [11.0, 1.0, 1.0],
        ];
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("golem_mod", Box::new(Golem))].into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                crate::game::mods::register_layer_definitions(
                    crate::game::execution_role::ExecutionRole::Client,
                    std::path::Path::new("."),
                );
                assert_eq!(drawn(game), face);
                assert!(game.bake_layer("golem_mod:other#main").is_none());
                assert!(game.bake_layer("minecraft:zombie#main").is_some());
                game.spawn_pack_reload(crate::game::packs::ReloadKind::Selection)
                    .unwrap();
                let started = std::time::Instant::now();
                while game.pack_reload.is_some() {
                    assert!(
                        started.elapsed().as_secs() < 300,
                        "the reload never finished"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    game.drive_pack_reload();
                }
                assert_eq!(drawn(game), face, "the same layer after the reload");
            });
        });
    }

    #[test]
    fn a_quad_lands_in_its_sprite_lit_and_shaded() {
        let mut d = draw();
        // An upward face: full brightness under vanilla's level lights is
        // below 1 (two lights), so the colour is shaded but opaque.
        d.quad(
            [
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
                [1.0, 1.0, 0.0],
            ],
            [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            sprite(),
            0xFFFFFF,
        );
        assert_eq!(d.vertices.len(), 4);
        let v = d.vertices[2];
        assert_eq!(v.pos, [11.0, 1.0, 1.0]);
        assert_eq!(v.uv, [0.75, 0.5]);
        assert_eq!(v.light, 0xF0F0);
        let up = diffuse(Vec3::Y, diffuse_lights(false));
        assert_eq!(v.color, shaded(0xFFFFFF, up));
        assert_eq!(v.color >> 24, 0xFF);
    }

    /// A frustum that sees nothing.
    struct Blind;
    impl Cull for Blind {
        fn intersects(&self, _: Vec3, _: Vec3) -> bool {
            false
        }
        fn contains(&self, _: Vec3, _: Vec3) -> bool {
            false
        }
    }

    #[test]
    fn infinite_bounds_and_frames_without_a_frustum_are_never_culled() {
        let origin = Vec3::new(1.0e6, 0.0, 0.0);
        let infinite = [f32::NEG_INFINITY, 0.0, 0.0, f32::INFINITY, 1.0, 1.0];
        assert!(!in_frustum(Some(&Blind), origin, DEFAULT_BOUNDS));
        assert!(in_frustum(Some(&Blind), origin, infinite));
        assert!(in_frustum(None::<&Blind>, origin, DEFAULT_BOUNDS));
    }

    #[test]
    fn a_tint_keeps_its_channels_in_vertex_order() {
        // The vertex colour is RGBA8 read little-endian: red in the low byte.
        assert_eq!(shaded(0x10_20_30, 1.0), 0xFF_30_20_10);
    }

    fn machine() -> ModRevision {
        ModBlockEntity::new("mymod:machine".into(), Default::default()).revision()
    }

    fn vertex(x: f32) -> Vertex {
        Vertex {
            pos: [x, 0.0, 0.0],
            ..bytemuck::Zeroable::zeroed()
        }
    }

    const KEY: KeptKey = KeptKey {
        dim: Dim::Overworld,
        state: 7,
        light: 0xF0F0,
        glyphs: 0,
    };

    /// A part drawn after `spin` turns about its pivot by the animation clock
    /// (90 degrees a tick about +y takes +x to -z, by the right
    /// hand), its normal with it; what is drawn after `still` and before the
    /// spin stays; all of it is moved by the origin.
    #[test]
    fn a_spinning_part_turns_with_the_animation_clock_and_the_rest_stays() {
        let mut draw = draw();
        let at = |x: f32, y: f32, z: f32, normal: [i8; 4]| Vertex {
            pos: [x, y, z],
            normal,
            ..bytemuck::Zeroable::zeroed()
        };
        draw.vertices.push(at(1.0, 0.5, 0.5, [0; 4]));
        draw.spin([0.0, 1.0, 0.0], [0.5, 0.5, 0.5], 90.0, 0.0);
        draw.vertices.push(at(1.0, 0.5, 0.5, [127, 0, 0, 127]));
        draw.still();
        draw.vertices.push(at(0.0, 0.0, 0.0, [0; 4]));
        let origin = Vec3::new(10.0, 0.0, 0.0);
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4);
        // A tick and a half: 135 degrees.
        let mut out = Vec::new();
        place_turning(&draw.vertices, &draw.spins, 1.5, origin, &mut out);
        assert!(close(out[0].pos, [11.0, 0.5, 0.5]), "{:?}", out[0].pos);
        let h = 0.5 * std::f32::consts::FRAC_1_SQRT_2;
        assert!(
            close(out[1].pos, [10.5 - h, 0.5, 0.5 - h]),
            "{:?}",
            out[1].pos
        );
        assert!(close(out[2].pos, [10.0, 0.0, 0.0]), "{:?}", out[2].pos);
        // One tick: a quarter turn, +x to -z, the normal along.
        out.clear();
        place_turning(&draw.vertices, &draw.spins, 1.0, origin, &mut out);
        assert!(close(out[1].pos, [10.5, 0.5, 0.0]), "{:?}", out[1].pos);
        assert_eq!(out[1].normal, [0, 0, -127, 127]);
        // Four ticks: all the way round, and a long session's late clock is no
        // less exact.
        out.clear();
        place_turning(
            &draw.vertices,
            &draw.spins,
            4_000_000_000.0,
            origin,
            &mut out,
        );
        assert!(close(out[1].pos, [11.0, 0.5, 0.5]), "{:?}", out[1].pos);
    }

    #[test]
    fn kept_geometry_is_drawn_again_until_what_it_was_drawn_with_changes() {
        let mut kept = KeptGeometry::default();
        let pos = [1, 2, 3];
        let mut entity = ModBlockEntity::new(
            "mymod:machine".into(),
            ModBlockEntityFields {
                data: vec![0],
                ..Default::default()
            },
        );
        let drawn = entity.revision();
        kept.keep(
            pos,
            KEY,
            drawn,
            DEFAULT_BOUNDS,
            &[vertex(0.5)],
            Vec::new(),
            0,
        );
        let entry = kept.get(pos, KEY, entity.revision(), 1).expect("kept");
        assert_eq!(entry.vertices.len(), 1);
        assert_eq!(entry.vertices[0].pos, [0.5, 0.0, 0.0]);
        assert_eq!(entry.bounds, DEFAULT_BOUNDS);
        // New data from the server, another state, light or dimension, or
        // another position: none of them is the kept geometry.
        entity.data[0] = 1;
        assert!(kept.get(pos, KEY, entity.revision(), 1).is_none());
        for changed in [
            KeptKey { state: 8, ..KEY },
            KeptKey {
                light: 0x00F0,
                ..KEY
            },
            KeptKey {
                dim: Dim::Nether,
                ..KEY
            },
            KeptKey { glyphs: 1, ..KEY },
        ] {
            assert!(kept.get(pos, changed, drawn, 1).is_none());
        }
        assert!(kept.get([1, 2, 4], KEY, drawn, 1).is_none());
        // Another block entity in its place, as the old one was: not it.
        let replaced = ModBlockEntity::new(
            "mymod:machine".into(),
            ModBlockEntityFields {
                data: vec![0],
                ..Default::default()
            },
        );
        assert!(kept.get(pos, KEY, replaced.revision(), 1).is_none());
        // Drawn again and kept with the new data, it replaces the old.
        kept.keep(
            pos,
            KEY,
            entity.revision(),
            DEFAULT_BOUNDS,
            &[vertex(0.25)],
            Vec::new(),
            2,
        );
        assert!(kept.get(pos, KEY, drawn, 2).is_none());
        let entry = kept
            .get(pos, KEY, entity.revision(), 2)
            .expect("kept again");
        assert_eq!(entry.vertices[0].pos, [0.25, 0.0, 0.0]);
        // Drawn without keeping: forgotten.
        kept.forget(pos);
        assert!(kept.get(pos, KEY, entity.revision(), 3).is_none());
    }

    #[test]
    fn kept_geometry_out_of_view_for_its_ticks_is_swept() {
        let mut kept = KeptGeometry::default();
        let (seen, gone) = ([0, 0, 0], [5, 0, 0]);
        let m = machine();
        kept.keep(seen, KEY, m, DEFAULT_BOUNDS, &[vertex(0.0)], Vec::new(), 0);
        kept.keep(gone, KEY, m, DEFAULT_BOUNDS, &[vertex(0.0)], Vec::new(), 0);
        kept.get(gone, KEY, m, 50);
        // A sweep at 100 keeps both: each was drawn in the last 100 ticks.
        kept.sweep(100);
        assert!(kept.entries.contains_key(&gone));
        assert!(kept.entries.contains_key(&seen));
        kept.get(seen, KEY, m, 150);
        // At 199 no sweep is due yet; at 200 the one last drawn at 50 goes.
        kept.sweep(199);
        assert!(kept.entries.contains_key(&gone));
        kept.sweep(200);
        assert!(!kept.entries.contains_key(&gone));
        assert!(kept.get(seen, KEY, m, 200).is_some());
    }
}
