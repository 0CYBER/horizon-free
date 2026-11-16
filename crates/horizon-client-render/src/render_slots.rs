//! Engine-owned frame data and load-time render dispatch slots.
use crate::render_geometry::{EntitySpan, Vertex};
pub use crate::render_views::{
    WorldView, WorldViewDrawn, WorldViewRefusal, WorldViewRefusalReason, WorldViews,
};
use glam::{DVec3, Mat4};
use std::sync::OnceLock;
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
