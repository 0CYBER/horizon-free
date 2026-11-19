//! The hook an external shader pipeline fills (#14035, layer plan M6). A shader pack's runtime
//! (a platform crate, later) implements [`ExternalPipeline`]; the renderer calls it at fixed
//! points of a world frame and draws vanilla's way wherever it doesn't answer. With none set
//! (`Renderer::set_external_pipeline`), each point is one `Option` test.
//!
//! Workspace-internal: the trait names the engine's frame, its mesh format and its pass order,
//! has one implementer in this workspace, carries no stability promise and is never exposed to
//! mods. A mesher or pass change updates its implementer in the same PR. HDR output
//! (`output.rs`, `frame_policy`) is not an implementer: it stays the one output stage after the
//! frame, and a pipeline draws into the frame it gives.
//!
//! Shadows are an engine frame point, on request: a pipeline that wants a shadow pass answers
//! [`ShadowView`], the engine culls the sections from it ([`ShadowCull`]) and hands back the
//! terrain seen from the light. Vanilla's path never asks, so it never computes shadow
//! visibility.

use glam::{Vec3, Vec4};
use horizon_shader_api::stages::GROUP;

use super::pack_timing::PassTiming;
// The engine types its points name, reachable where the trait is.
pub use super::SectionLists;
pub use super::rt::scene::{RayScene, Traced, TracedEntities};
pub use super::weather::Weather;
use super::{Cull, EffectiveOutput, Frame, Frustum, SectionKey, Sides, facing_runs, frustum};
use crate::fast_hash::FxHashMap;

/// What a world frame gives an external pipeline at its points: the frame's own values, and
/// the display's from the renderer's one output snapshot (`EffectiveOutput`: whether HDR is on,
/// paper white, peak), never worked out again, so each split-screen window keeps its own.
pub struct FrameState<'a> {
    pub frame: &'a Frame<'a>,
    pub output: &'a EffectiveOutput,
}

/// An external shader pipeline, called by the renderer at fixed points of a world frame (not
/// title frames, unless a point says so), in the order below, and as it is installed, replaced
/// and reset. Every method has a default that leaves vanilla's way in place.
pub trait ExternalPipeline: Send + std::any::Any {
    /// What it does in the engine's place, asked once, when it is installed. None: it draws
    /// nothing of the world, and the renderer runs as without a shader pack. Some: it also
    /// presents the world, in `run_post`, which then runs in place of the engine's finish.
    fn capabilities(&self) -> Option<Capabilities> {
        None
    }
    /// It runs from now on in place of `old` (none: nothing ran), dropped after: a rebuild of
    /// the same pipeline may take what `old` kept from frame to frame. What it keeps is made
    /// here, at `size`, the size its frames draw at. Refused when there is no room for that:
    /// then it never runs, and `old` either keeps running or, when it can't, stops too.
    fn take_over(
        &mut self,
        _device: &wgpu::Device,
        _old: Option<&mut dyn ExternalPipeline>,
        _size: [u32; 2],
    ) -> Result<(), Refused> {
        Ok(())
    }
    /// It stops with nothing in its place: what it kept from frame to frame goes now.
    fn retire(&mut self, _device: &wgpu::Device) {}
    /// Every section is gone (a new world, a new dimension).
    fn world_cleared(&mut self) {}
    /// Forget what was kept from earlier frames: the next is drawn as a fresh pipeline's first.
    fn reset_history(&mut self) {}
    /// The window's drawable size changed.
    fn resize(&mut self, _size: [u32; 2]) {}
    /// Times its passes, or stops, as the renderer's settings say, before the world passes
    /// (title frames too).
    fn set_up_timing(&mut self, _setup: TimingSetup<'_>) {}
    /// The frame it timed was submitted.
    fn timing_submitted(&mut self, _queue: &wgpu::Queue) {}
    /// The timer of its passes, where it times them.
    fn timer(&mut self) -> Option<&mut Option<PassTiming>> {
        None
    }
    fn timing(&self) -> Option<&PassTiming> {
        None
    }
    /// A world frame starts, before culling.
    fn begin_frame(&mut self, _state: &FrameState<'_>) {}
    /// The shadow pass this frame wants; `None` draws none.
    fn shadow_request(&self, _state: &FrameState<'_>) -> Option<ShadowView> {
        None
    }
    /// After the visibility walk (title frames too, with no sections).
    fn prepare_displacement(
        &mut self,
        _prepare: DisplacementPrepare<'_, &mut dyn Iterator<Item = Near<'_>>>,
    ) {
    }
    /// This frame's inputs. Returns whether its sky draws in a pass of its own.
    fn start_frame(&mut self, _start: FrameStart<'_>) -> bool {
        false
    }
    /// Its shadow passes, from the lists of what the light sees.
    fn draw_shadow(&mut self, _draws: ShadowDraws<'_>) {}
    /// Its passes before the world, after the shadow map.
    fn before_world(
        &mut self,
        _device: &wgpu::Device,
        _encoder: &mut wgpu::CommandEncoder,
        _scene: Option<&Traced<'_>>,
    ) {
    }
    /// Its sky in a pass of its own, where `start_frame` said so.
    fn draw_sky_pass(&mut self, _sky: SkyPass<'_>) {}
    /// After the water mirror (title frames too).
    fn prepare_world_targets(&mut self, _world: WorldTargets<'_>) {}
    /// Joins this frame's ray `scene` from `camera` to what its terrain reads.
    fn bind_scene(
        &mut self,
        _device: &wgpu::Device,
        _scene: Option<&RayScene>,
        _camera: glam::DVec3,
    ) {
    }
    /// The layers its terrain draws with this frame, in place of the engine's.
    fn terrain_layers(&self) -> Option<TerrainLayers<'_>> {
        None
    }
    /// The layers its entities draw with this frame, in place of the engine's.
    fn entity_layers(&self) -> Option<EntityLayers<'_>> {
        None
    }
    /// Its sky in the world pass. False when it has none, for the engine's own.
    fn draw_sky(&self, _pass: &mut wgpu::RenderPass<'_>) -> bool {
        false
    }
    /// Its grid of the solid terrain near the camera, drawn by `layers`' displaced layer, in a
    /// terrain pass with the engine's group 0 bound.
    fn draw_displaced(&self, _pass: &mut wgpu::RenderPass<'_>, _layers: TerrainLayers<'_>) {}
    /// Where the world pass stops before translucent terrain, for layers that read it.
    fn copy_opaque(&self, _copy: OpaqueCopy<'_>) {}
    /// Its rain and snow. False when it has none, for the engine's own.
    fn draw_weather(
        &self,
        _pass: &mut wgpu::RenderPass<'_>,
        _weather: &Weather,
        _entities: &wgpu::BindGroup,
    ) -> bool {
        false
    }
    /// Its post passes in place of the engine's finish of the world: a pipeline whose
    /// `capabilities` are Some writes the frame to `post.screen` here, since the renderer then
    /// doesn't. The default writes nothing, for pipelines whose `capabilities` are None.
    fn run_post(&mut self, _post: WorldPost<'_>) {}
}

/// Why a pipeline couldn't take over (`ExternalPipeline::take_over`), and what that left running.
#[derive(Debug, PartialEq, Eq)]
pub struct Refused {
    /// Why, for the player.
    pub why: String,
    /// The pipeline that ran keeps running. False: it can't either, and none runs.
    pub old_runs: bool,
}

/// What a shader pack's executor does in the engine's place, fixed from when it is installed
/// until it is replaced (for now only Horizon's own packs, `NativePack::capabilities`). The
/// renderer reads these where it decides, never asking the executor each frame; with none
/// running it has none, and draws vanilla's way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// It draws a shadow map from the engine's shadow lists ([`ShadowView`]).
    pub shadow: bool,
    /// It reads the water mirror, which needs the frame's `water_plane`.
    pub reflection: bool,
    /// It takes the near solid quads of the visibility walk's sections for a displaced grid.
    pub displacement: bool,
    /// Its terrain draws with layers of its own.
    pub material: bool,
    /// Its sky leaves out vanilla's clouds, sun, moon or stars.
    pub hides_clouds: bool,
    pub hides_sun: bool,
    pub hides_moon: bool,
    pub hides_stars: bool,
    /// Its post ends in a tone curve of its own, in place of the engine's.
    pub tone_curve: bool,
    /// It traces rays through a scene the engine keeps.
    pub ray_scene: bool,
    /// Its entities draw with pipelines of its own: the game names the runs of them it lights.
    pub entities: bool,
    /// Its rays read the scene's entity layer, which the engine then builds.
    pub ray_entities: bool,
    /// Its rays read the terrain's quad looks in the ray scene, so sections keep them.
    pub ray_looks: bool,
    /// Its terrain pipelines bind the arenas' quad looks (`looks::wanted`): sections carry them.
    pub looks: bool,
}

/// What the renderer hands the pack's post stage (`ExternalPipeline::run_post`).
pub struct WorldPost<'a> {
    pub device: &'a wgpu::Device,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The world's scaled target (colour, depth), of `size`.
    pub world: (&'a wgpu::TextureView, &'a wgpu::TextureView),
    /// The window's frame, which the last pass draws.
    pub screen: &'a wgpu::TextureView,
    pub size: [u32; 2],
    /// The ray scene and the camera, when the game traces rays.
    pub scene: Option<(&'a RayScene, glam::DVec3)>,
}

/// The renderer's pass timing settings (`ExternalPipeline::set_up_timing`).
pub struct TimingSetup<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    /// `set_pass_timing`, `set_pass_timing_gap` and `set_keep_timed_frames`.
    pub on: bool,
    pub gap: std::time::Duration,
    pub keep_frames: bool,
}

/// What the renderer hands the pack before its world passes (`ExternalPipeline::start_frame`).
pub struct FrameStart<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub frame: &'a Frame<'a>,
    /// The world's size this frame: the scaled target's, else the window's.
    pub size: [u32; 2],
    /// This frame's output, whose display values a 0.2 pack reads.
    pub output: &'a EffectiveOutput,
    /// The light sources the section meshes list, and their revision (`BlockLight::update`).
    pub lights: &'a FxHashMap<SectionKey, Vec<([i32; 3], u32)>>,
    pub light_revision: u64,
    /// The sky is open this frame.
    pub sky_visible: bool,
    /// The ray scene, when the game traces rays (`ExternalPipeline::start_frame`).
    pub scene: Option<&'a RayScene>,
    /// This frame's lightmap, RGBA rows of sky level by block level (`Renderer::lightmap_pixels`).
    pub lightmap: &'a [u8; 16 * 16 * 4],
}

/// What the renderer hands the pack after the water mirror (`ExternalPipeline::prepare_world_targets`).
pub struct WorldTargets<'a> {
    pub device: &'a wgpu::Device,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The water mirror's colour, when one was drawn this frame.
    pub reflection: Option<wgpu::TextureView>,
    pub title: bool,
    /// The world's depth, and its colour where it can be copied (the scaled target, not the
    /// window's own).
    pub depth: &'a wgpu::Texture,
    pub color: Option<&'a wgpu::Texture>,
}

/// What the renderer hands the shadow passes each frame.
pub struct ShadowDraws<'a> {
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub timing: Option<&'a mut PassTiming>,
    /// What the light sees, when the frame asked for a shadow view.
    pub lists: Option<ShadowLists<'a>>,
}

/// The renderer's lists of what the light sees, and what they draw from.
pub struct ShadowLists<'a> {
    /// The world's group 0.
    pub group: &'a wgpu::BindGroup,
    pub sections: SectionLists<'a>,
    /// The frame draws its terrain flat (`sections.flat`'s shadow calls).
    pub flat_frame: bool,
    pub instances: &'a wgpu::Buffer,
    pub indirect: &'a wgpu::Buffer,
    /// The light's first draw, its solid draws and its translucent draws after them.
    pub draws: [u32; 3],
    /// The entities' group 0 and vertices: `entity_quads` from the start, then
    /// `shadow_only_quads` before the GPU-posed outlines at `outline_base`.
    pub entity_group: &'a wgpu::BindGroup,
    pub entity_vertices: &'a wgpu::Buffer,
    pub entity_quads: u32,
    pub shadow_only_quads: u32,
    pub outline_base: u32,
}

/// What the renderer hands the pack for its sky's pass of its own (`ExternalPipeline::draw_sky_pass`).
pub struct SkyPass<'a> {
    pub device: &'a wgpu::Device,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The world's colour and depth, and the colour's format.
    pub target: super::timing::Target<'a>,
    pub depth: &'a wgpu::TextureView,
    /// The world's clear colour.
    pub clear: wgpu::Color,
    /// The terrain's group 0.
    pub globals: &'a wgpu::BindGroup,
}

/// The pack's terrain layers, which the renderer's terrain passes draw in place of its own
/// (`ExternalPipeline::terrain_layers`).
#[derive(Clone, Copy)]
pub struct TerrainLayers<'a> {
    /// The solid layer (with the cutout test) and the translucent one, for the instanced draws.
    pub solid: &'a wgpu::RenderPipeline,
    pub translucent: &'a wgpu::RenderPipeline,
    /// The solid and translucent layers for the flat draws, where those run.
    pub flat: Option<&'a (wgpu::RenderPipeline, wgpu::RenderPipeline)>,
    /// The layer of its grid of the solid terrain near the camera, where it draws one
    /// (`ExternalPipeline::draw_displaced`).
    pub displaced: Option<&'a wgpu::RenderPipeline>,
    /// The solid layers for 4x MSAA (instanced, then flat draws), when the pack's material got them.
    pub msaa: Option<&'a (wgpu::RenderPipeline, Option<wgpu::RenderPipeline>)>,
    /// The solid layers' second target, which they write beside the world's colour.
    pub surface: Option<&'a wgpu::TextureView>,
    /// The layers read the world before translucent terrain: the renderer stops there for
    /// `ExternalPipeline::copy_opaque`.
    pub reads_opaque: bool,
    /// What the layers read beside the terrain's groups (`bind`).
    pub frame: &'a wgpu::BindGroup,
}

impl TerrainLayers<'_> {
    /// Binds what the layers read beside the terrain's groups, in each pass that draws them.
    pub fn bind(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(GROUP, self.frame, &[]);
    }
}

/// What the renderer hands the pack where its world pass stops before translucent terrain
/// (`ExternalPipeline::copy_opaque`).
pub struct OpaqueCopy<'a> {
    pub device: &'a wgpu::Device,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The world's depth, and its colour where it can be copied (the scaled target, not the
    /// window's own).
    pub depth: &'a wgpu::Texture,
    pub color: Option<&'a wgpu::TextureView>,
}

/// What the renderer hands `[displacement]` once a frame, after the visibility walk
/// (`ExternalPipeline::prepare_displacement`).
pub struct DisplacementPrepare<'a, I> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The visible sections with solid quads, nearest first.
    pub sections: I,
    pub view_proj: glam::Mat4,
}

/// The pipelines a pack's entity stage draws the entity stream's ordinary quads and the
/// retained block entities with, and what they read beside the entities' groups
/// (`ExternalPipeline::entity_layers`).
#[derive(Clone, Copy)]
pub struct EntityLayers<'a> {
    pub entities: &'a wgpu::RenderPipeline,
    pub retained: &'a wgpu::RenderPipeline,
    /// Their group 3: the pack's frame, with this frame's version of the kept resources the
    /// stage reads where it reads any.
    pub frame: &'a wgpu::BindGroup,
}

/// A section the grid may draw from this frame: its arena page, the base vertex of its solid
/// draws (`SectionGpu::solid_base`: in the compact layout past `PACKED_BASE`), its solid quads
/// and its camera-relative origin, and its page's quad looks while a pack reads them
/// (`Arena::looks`).
pub struct Near<'a> {
    pub buffer: &'a wgpu::Buffer,
    pub looks: Option<&'a wgpu::Buffer>,
    pub base: u32,
    pub quads: u32,
    pub origin: [f32; 3],
}

/// Sets the pipeline of the entity stream's ordinary quads, or of the `retained` block
/// entities: the `stage`'s with the pack's frame at its group, else Horizon's `own`.
pub(super) fn set_entity_pipeline(
    pass: &mut wgpu::RenderPass<'_>,
    stage: Option<EntityLayers<'_>>,
    own: &wgpu::RenderPipeline,
    retained: bool,
) {
    let Some(stage) = stage else {
        return pass.set_pipeline(own);
    };
    pass.set_pipeline(if retained {
        stage.retained
    } else {
        stage.entities
    });
    pass.set_bind_group(GROUP, stage.frame, &[]);
}

/// What the shadow camera sees of the terrain, for the game's draw lists: its view and
/// projection in wgpu's clip space, the direction toward the light, and how far from the camera
/// terrain is drawn (`shadowDistanceRenderMul`).
pub struct ShadowView {
    pub cull: glam::Mat4,
    pub light: glam::Vec3,
    pub distance: Option<f32>,
    /// Draw everything, every face, within this many blocks (`shadow.culling=false|reversed`):
    /// packs that voxelize the world in their shadow pass need all of it.
    pub unculled: Option<f32>,
    /// Every face of the drawn sections, not only those that can face the light. Iris 1.11.7's
    /// shadow map holds the faces turned away from the light too (#9033): under a hill at low sun
    /// a cave's ceiling, which faces away, is often all that shadows the cave.
    pub every_face: bool,
    /// Only the sections whose shadow can fall into the camera's view: those the camera's
    /// frustum, stretched toward the light, takes in (`shadow_casters`). Our probes' Iris 1.11.7
    /// maps fit keeping only those with `shadow.culling` unset (#8464): under a noon sun, only
    /// the terrain above the ground the camera sees; with a low sun ahead, none behind the camera.
    pub view_casters: bool,
    /// Whether the projection's near and far planes in `cull` drop sections too. Our probes'
    /// Iris 1.11.7 maps hold the terrain at every depth we put it along the light (from 535 blocks
    /// beyond the shadow camera to about 120 before it), whatever `shadowNearPlane` and
    /// `shadowFarPlane` the pack sets, or none (#10403): only its sides cull.
    pub depth_planes: bool,
}

/// A shader pack's shadow camera's box, and with `casters` only what can shade the camera's view
/// (`shadow_casters`): a box must meet all of them.
#[derive(Clone, Copy, Debug)]
pub struct ShadowCull {
    pub light: Frustum,
    pub casters: Option<[Frustum; 2]>,
    /// `shadowDistance × shadowDistanceRenderMul`: only what lies within this many blocks of the
    /// camera along each axis, a cube around it. Our probe's Iris 1.11.7 maps keep the sections
    /// whose box, grown by `SHADOW_CASTER_MARGIN` as for the casters, meets that cube (#8464):
    /// out to its corners, and a section 1 block past its side but not one 1.5 blocks past.
    pub reach: Option<f32>,
}
impl ShadowCull {
    /// Whether the camera-relative box `min..max`, grown by the margin, meets (`whole`: lies
    /// within) `reach`'s cube.
    fn within_reach(&self, min: Vec3, max: Vec3, whole: bool) -> bool {
        let (min, max) = (min - SHADOW_CASTER_MARGIN, max + SHADOW_CASTER_MARGIN);
        self.reach.is_none_or(|d| {
            if whole {
                min.cmpge(Vec3::splat(-d)).all() && max.cmple(Vec3::splat(d)).all()
            } else {
                max.cmpge(Vec3::splat(-d)).all() && min.cmple(Vec3::splat(d)).all()
            }
        })
    }
}
/// How far past a section's 16 blocks the shadow pass looks for what can shade the view: our
/// probes' Iris 1.11.7 maps keep the sections whose box, grown by this much on every side, meets
/// `shadow_casters`' shape. Over 76 scenes at fields of view 30 to 110, Iris keeps every section
/// that needs up to 1.1224 blocks and drops every one that needs 1.1252 or more (#8464): one
/// growth, whatever the field of view, of 9/8 (a fit, not Iris's code). Without it the edges of
/// the wedge lose a column of terrain.
const SHADOW_CASTER_MARGIN: f32 = 1.125;
impl Cull for ShadowCull {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        let (grown_min, grown_max) = (min - SHADOW_CASTER_MARGIN, max + SHADOW_CASTER_MARGIN);
        self.within_reach(min, max, false)
            && self.light.intersects(min, max)
            && self
                .casters
                .is_none_or(|c| c.iter().all(|f| f.intersects(grown_min, grown_max)))
    }
    fn contains(&self, min: Vec3, max: Vec3) -> bool {
        let (grown_min, grown_max) = (min - SHADOW_CASTER_MARGIN, max + SHADOW_CASTER_MARGIN);
        self.within_reach(min, max, true)
            && self.light.contains(min, max)
            && self
                .casters
                .is_none_or(|c| c.iter().all(|f| f.contains(grown_min, grown_max)))
    }
    /// Our probe's Iris 1.11.7 maps fit testing the casters' whole sections (a guess that matched).
    fn tight(&self) -> bool {
        self.casters.is_none()
    }
}
/// The shadow camera's box that the shadow pass draws terrain in: the sides of `view.cull`, and
/// its near and far planes only with `depth_planes`.
pub(super) fn shadow_box(view: &ShadowView) -> Frustum {
    let mut planes = frustum(view.cull);
    if !view.depth_planes {
        // `frustum`'s faces at depth 0 and 1 take in everything.
        planes[4] = Vec4::W;
        planes[5] = Vec4::W;
    }
    Frustum { planes }
}

/// The runs of a section's solid quads the shadow pass draws: every face within the unculled
/// distance, or everywhere for `every_face`; else the quads that can face the light.
pub(super) fn shadow_runs(
    sides: &Sides,
    view: &ShadowView,
    origin: [f32; 3],
    draw: impl FnMut(u32, u32),
) {
    let unculled = view.unculled.is_some_and(|d| {
        let o = Vec3::from(origin) + 8.0;
        o.x.abs().max(o.z.abs()) <= d + 8.0
    });
    let toward = (!unculled && !view.every_face).then_some(view.light * 1.0e5);
    facing_runs(sides, toward, origin, draw);
}
