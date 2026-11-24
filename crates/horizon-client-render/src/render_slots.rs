//! Engine-owned frame data and load-time render dispatch slots.
use crate::render_geometry::{EntitySpan, Vertex};
pub use crate::render_views::{
    WorldView, WorldViewDrawn, WorldViewRefusal, WorldViewRefusalReason, WorldViews,
};
use glam::{DVec3, Mat4, Vec3};
use std::sync::OnceLock;

pub mod celestial_sky;
pub struct WorldDrawn<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    pub size: [u32; 2],
    pub depth: &'a wgpu::TextureView,
    pub depth_size: [u32; 2],
    pub view_proj: Mat4,
    pub jittered: Mat4,
    pub camera: DVec3,
    pub entities: &'a [Vertex],
    pub spans: &'a [EntitySpan],
    pub motion: &'a [(u32, u64)],
    pub atlas: Option<&'a wgpu::Texture>,
    pub quad_indices: &'a wgpu::Buffer,
    pub block_sprites: BlockSpriteArrays<'a>,
    pub look: WorldLook,
}
/// The sky, fog, light and water the frame was drawn under: the environment values the world's
/// shaders (and a shader pack's uniforms) are built from, for a mod pass that should match them.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldLook {
    /// The sky's and the fog's colours, 0 to 1 as vanilla gives them (sRGB-encoded, not linear),
    /// alpha 1 (`Globals::fog_color` for
    /// `fog_color`: water's or lava's fog when the camera is in it).
    pub sky_color: [f32; 4],
    pub fog_color: [f32; 4],
    /// The fog's distances in blocks: the environment's start and end, the render distance's
    /// start and end (`Globals::fog`, what terrain.wgsl's `apply_fog` mixes).
    pub fog: [f32; 4],
    /// Unit world directions toward the sun, the moon, and the body that lights the frame (the
    /// sun by day, the moon by night), turned by the shader pack's `sunPathRotation` when one is
    /// on.
    pub sun_direction: Vec3,
    pub moon_direction: Vec3,
    pub light_direction: Vec3,
    /// The sun's place in the day, as packs' `sunAngle`: 0 at sunrise, 0.25 at noon, 0.5 at
    /// sunset.
    pub sun_angle: f32,
    /// How much sky light the frame has (0 to 1) and its colour; the ambient light's colour and
    /// the block light's tint.
    pub sky_light_factor: f32,
    pub sky_light_color: [f32; 3],
    pub ambient_light_color: [f32; 3],
    pub block_light_tint: [f32; 3],
    /// The sunrise or sunset glow (alpha 0 when none) and how bright the stars are.
    pub sunrise_color: [f32; 4],
    pub star_brightness: f32,
    /// The camera is under water, and water's fog colour.
    pub underwater: bool,
    pub water_fog_color: [f32; 3],
    /// Night vision's strength, 0 to 1.
    pub night_vision: f32,
    /// A shader pack drew the frame: its own sky, fog, lighting and tonemap are in `color`, and
    /// the values above are what its uniforms were built from.
    pub shader_pack: bool,
}
pub struct BlockSpriteArrays<'a> {
    pub views: &'a [wgpu::TextureView],
    pub split: u32,
    pub size: u32,
    pub nearest: &'a wgpu::Sampler,
    pub filtered: &'a wgpu::Sampler,
}
pub struct HandDrawn<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    pub size: [u32; 2],
    pub hand: &'a [Vertex],
    pub view_proj: Mat4,
    pub camera: DVec3,
    pub atlas: Option<&'a wgpu::Texture>,
    pub quad_indices: &'a wgpu::Buffer,
}
#[derive(Default)]
pub struct RenderNeeds {
    pub entity_spans: bool,
    pub world_depth: bool,
}
/// Installed once by the rendering provider before startup. Dispatch never allocates.
pub struct Slots {
    pub world_views: fn(&mut WorldViews),
    pub world_views_hooked: fn() -> bool,
    pub world_view_drawn: for<'a> fn(&mut WorldViewDrawn<'a>),
    pub world_drawn: for<'a> fn(&mut WorldDrawn<'a>),
    pub hand_drawn: for<'a> fn(&mut HandDrawn<'a>),
    pub render_needs: fn(&mut RenderNeeds),
    pub world_drawn_hooked: fn() -> bool,
    pub hand_drawn_hooked: fn() -> bool,
}
static SLOTS: OnceLock<Slots> = OnceLock::new();
pub fn install(slots: Slots) {
    let _ = SLOTS.set(slots);
}
pub fn world_drawn(world: &mut WorldDrawn<'_>) {
    if let Some(s) = SLOTS.get() {
        (s.world_drawn)(world);
    }
}
pub fn hand_drawn(hand: &mut HandDrawn<'_>) {
    if let Some(s) = SLOTS.get() {
        (s.hand_drawn)(hand);
    }
}
pub fn needs() -> RenderNeeds {
    let mut n = RenderNeeds::default();
    if let Some(s) = SLOTS.get() {
        (s.render_needs)(&mut n);
    }
    n
}
pub fn world_drawn_hooked() -> bool {
    SLOTS.get().is_some_and(|s| (s.world_drawn_hooked)())
}
pub fn hand_drawn_hooked() -> bool {
    SLOTS.get().is_some_and(|s| (s.hand_drawn_hooked)())
}

/// Gather the provider's requested views, each taken or refused as it is pushed.
pub fn world_views(views: &mut WorldViews) {
    if let Some(s) = SLOTS.get() {
        (s.world_views)(views);
    }
}
pub fn world_views_hooked() -> bool {
    SLOTS.get().is_some_and(|s| (s.world_views_hooked)())
}
pub fn world_view_drawn(view: &mut WorldViewDrawn<'_>) {
    if let Some(s) = SLOTS.get() {
        (s.world_view_drawn)(view);
    }
}
