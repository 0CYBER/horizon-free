//! GPU renderer for streamed terrain sections, sky, block outline and HUD.
//! Section origins are supplied relative to the camera each frame so large
//! world coordinates keep full precision.
use crate::{
    Result,
    environment::Sky,
    graphics::GraphicsApi,
    hud::{HudBatch, HudRenderer},
    mesher::{SectionMesh, TerrainVertex, Vertex},
    sprites::SpriteAnimation,
};
use glam::{DVec3, Mat4, Vec3, Vec4};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Instant};
use wgpu::util::DeviceExt;
use winit::window::Window;

// RenderPipelines.CRUMBLING: dst * src + src * dst, alpha replaced.
const CRUMBLING_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Dst,
        dst_factor: wgpu::BlendFactor::Src,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent::REPLACE,
};
/// `TerrainVertex` for `vs_terrain`: position, UV, layer, light, colour.
const TERRAIN_ATTRIBUTES: [wgpu::VertexAttribute; 5] =
    wgpu::vertex_attr_array![0=>Float32x3,1=>Uint16x2,2=>Uint16,4=>Uint8x2,3=>Unorm8x4];

/// entity.wgsl after the OverlayTexture constants it reads.
fn entity_shader() -> String {
    crate::overlay::wgsl_constants() + include_str!("entity.wgsl")
}

mod entity_batch;
mod weather;
use entity_batch::EntityBatch;
pub use entity_batch::WATER_MASK_OVERLAY;

/// Window.getAppropriateLineWidth, in framebuffer pixels.
pub fn outline_width(width: u32) -> f32 {
    2.5f32.max(width as f32 / 1920.0 * 2.5)
}

/// ScreenSize, stroke width and ProjectionType.PERSPECTIVE layering.
pub fn outline_params(width: u32, height: u32) -> [f32; 4] {
    [
        width as f32,
        height.max(1) as f32,
        outline_width(width),
        1.0 - 1.0 / 4096.0,
    ]
}

/// Camera-relative segment endpoints; no shape expansion in vanilla 26.3.
pub fn outline_vertices(edges: &[[f64; 6]], camera: glam::DVec3) -> Vec<[f32; 6]> {
    edges
        .iter()
        .map(|e| std::array::from_fn(|i| (e[i] - camera.to_array()[i % 3]) as f32))
        .collect()
}

mod animate;
mod arena;
mod cull;
mod end_sky;
mod flat;
mod java_stages;
pub mod lightning;
mod lod;
mod occlusion;
mod pack_stages;
mod page_runs;
pub mod panorama;
mod resident;
mod stars;
mod timing;
pub mod translucency;
pub(crate) mod upload;
mod world_post;

/// A column whose every section has its mesh (`Renderer::lod_detail_columns`).
const COMPLETE_COLUMN: u32 = (1 << (crate::chunk::HEIGHT / 16)) - 1;
/// A section's instance: its camera-relative origin and, in `w`, the step of
/// its fixed-point UVs (`terrain.wgsl`'s `terrain_uv`).
/// The world pass again after occlusion culling's test: it loads both
/// targets (phase 2 draws over phase 1, reading the test's results).
fn world_pass_after_culling<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    pack: bool,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Horizon world, after culling"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: if pack {
                    wgpu::StoreOp::Store
                } else {
                    wgpu::StoreOp::Discard
                },
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    })
}
fn section_instance(origin: [f32; 3]) -> [f32; 4] {
    [
        origin[0],
        origin[1],
        origin[2],
        1.0 / crate::mesher::UV_STEPS,
    ]
}
/// A new value for `Renderer::block_entity_revision`.
fn next_block_entity_revision() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    fog_color: [f32; 4],
    fog: [f32; 4],
    sky_color: [f32; 4],
    misc: [f32; 4],
    inverse_view: [[f32; 4]; 4],
    sunrise: [f32; 4],
    sun: [f32; 4],
    /// x: sky fog end, y: cloud fog end, z: dark disc (camera below the horizon), w: unused.
    fog_ends: [f32; 4],
    /// Atlas rects (u0, v0, width, height) of end_sky.png and end_portal.png.
    end_portal_sprites: [[f32; 4]; 2],
    /// x: vanilla's `GameTime` uniform; y: nether cardinal lighting; z: how many
    /// `dynamic_lights` are set; w unused.
    time: [f32; 4],
    /// Atlas rect of a destroy stage (entity.wgsl takes only its size: every stage's is the same,
    /// and each crack carries its own stage's origin).
    crumbling_sprite: [f32; 4],
    /// Framebuffer size, selection stroke width, perspective layering scale.
    outline_params: [f32; 4],
    /// Horizon Extras' dynamic lights: camera-relative position and level (`dynamic_light`).
    dynamic_lights: [[f32; 4]; crate::dynamic_light::MAX],
}
pub type SectionKey = (i32, i32, i32);
struct SectionGpu {
    /// Arena page and first vertex in it: solid quads, then translucent ones.
    page: u32,
    start: u64,
    solid: u32,
    translucent: u32,
    /// The solid quads by facing: back-facing groups are skipped.
    faces: crate::mesher::FaceGroups,
    sort: translucency::SortState,
    sorted_indices: Option<(u32, u64)>,
    /// Its id for flat draws (`flat::Flat::add`), when they are on.
    flat_id: Option<u32>,
}
/// Line segments the hit outline can hold.
const MAX_OUTLINE_EDGES: u64 = 256;
/// Staging buffer size for section meshes: a frame of uploads while
/// exploring fits in a few.
const SECTION_UPLOAD_CHUNK: u64 = 4 << 20;
/// Section copies are submitted early past this many staged bytes, which
/// bounds the staging memory frames that are not drawn can hold.
const SECTION_COPY_LIMIT: u64 = 64 << 20;
pub struct Frame<'a> {
    pub camera: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_degrees: f32,
    /// The `GameRenderer.bobHurt`/`bobView` pose multiplied into the
    /// projection (`crate::camera::bob_matrix`).
    pub bob: Mat4,
    pub sky: Sky,
    /// Render distance in blocks (fog end).
    pub distance: f32,
    /// Full-detail terrain distance in blocks; distant LODs bypass the cave graph.
    pub detail_distance: f32,
    /// `Camera.depthFar`: the far plane (`crate::camera::depth_far`).
    pub depth_far: f32,
    pub underwater: bool,
    /// Skip sections the camera cannot see through open faces (vanilla's
    /// occlusion graph; off in spectator-like views inside blocks).
    pub cave_culling: bool,
    /// Title screen: only the dedicated panorama cube map and the HUD.
    pub title: bool,
    /// Dimensions without a skybox (the Nether) clear to the fog colour.
    pub skybox: bool,
    /// The End: clear to black and draw the resident sky box behind the world.
    pub end_sky: bool,
    /// End sky atlas bounds (u0, v0, u1, v1); absent when the sprite is missing.
    pub end_sky_sprite: Option<[f32; 4]>,
    /// Environmental fog start/end (Nether 10..96), if any.
    pub environment_fog: Option<(f32, f32)>,
    pub water_fog: [f32; 3],
    /// `FogRenderer.setupFog` result; replaces the underwater/environment fields when set.
    pub fog: Option<crate::fog::FogData>,
    /// Camera eye below the horizon height and not under water (`shouldRenderDarkDisc`).
    pub dark_disc: bool,
    /// The hit block outline as line segments (from, to) in world space.
    pub outline: Vec<[f64; 6]>,
    /// `BlockOutlineRenderState.isTranslucent`: draw after translucent terrain.
    pub outline_after_terrain: bool,
    /// `lines` with a colour and pixel width (`gizmos::camera_relative`):
    /// the fishing line and debug gizmos, depth tested, then gizmos drawn
    /// over everything.
    pub debug_lines: &'a [[f32; 8]],
    pub debug_lines_on_top: &'a [[f32; 8]],
    /// `DebugCrosshairRenderer`'s axes, camera relative like `debug_lines`:
    /// drawn after the hand at its projection, without the bob or fog.
    pub debug_crosshair: &'a [[f32; 8]],
    pub hud: &'a HudBatch,
    /// CardinalLighting.Type.NETHER, otherwise DEFAULT.
    pub nether_lighting: bool,
    /// Camera-relative entity geometry textured from the shared atlas.
    pub entities: &'a [Vertex],
    pub weather: &'a [crate::weather::Vertex],
    pub weather_time: crate::weather::Time,
    /// Horizon Extras' dynamic lights, nearest first (`dynamic_light::select`); empty when off.
    pub dynamic_lights: &'a [[f32; 4]],
    /// Cloud mesh inputs; geometry and GPU storage are retained by the renderer.
    pub clouds: Option<crate::clouds::Input<'a>>,
    /// Retained lightning bolts, drawn before the other additive entities.
    pub lightning: lightning::Input<'a>,
    /// Additive translucent geometry after lightning (dragon rays).
    pub additive: &'a [Vertex],
    /// Name tags over everything, without depth (`TEXT_SEE_THROUGH`).
    pub see_through: &'a [Vertex],
    /// Camera-relative sun and moon, or the animated End flash (additive,
    /// behind everything and after the resident End sky box).
    pub celestial: &'a [Vertex],
    /// The stars, drawn from their static buffer after the sun and moon:
    /// the sky's rotation (camera-relative world from star space) and the
    /// star brightness.
    pub stars: Option<(Mat4, f32)>,
    /// First-person hand and held item, drawn after clearing depth.
    pub hand: &'a [Vertex],
    /// `CameraRenderState.hudFov`: the hand's own field of view
    /// (`GameRenderer.render3dHud`).
    pub hand_fov_degrees: f32,
    /// Full-screen camera effects drawn over the world and hand, under the HUD.
    pub screen: &'a [crate::screen_effects::ScreenQuad],
    /// The world border wall, when near enough to be drawn.
    pub border: Option<BorderDraw>,
    pub game_time: f32,
    /// `GlobalSettingsUniform` GameTime: (game time % 24000 + partial tick) / 24000.
    pub shader_game_time: f32,
    /// Atlas rects (u0, v0, width, height) of end_sky.png and end_portal.png
    /// (`rendertype_end_portal`'s Sampler0 and Sampler1).
    pub end_portal_sprites: [[f32; 4]; 2],
    /// A destroy-stage sprite: projected UVs repeat inside a rect of this size, from the
    /// origin each crack vertex carries (`breaking::stage_light`).
    pub crumbling_sprite: [f32; 4],
    pub ticks: u64,
    /// The darkness effect on the lightmap: the brightness it takes off the
    /// gamma option and the pulsing scale subtracted from the light.
    pub darkness: (f32, f32),
    /// Night vision (or conduit water vision) intensity on the lightmap.
    pub night_vision: f32,
    /// Torch flicker added to the block light factor of 1.4.
    pub block_flicker: f32,
    /// `Screen.extractBlurredBackground`: blur the frame by this radius
    /// (`menuBackgroundBlurriness`) before the screen is drawn over it.
    pub menu_blur: Option<f32>,
    /// What a shader pack's `Frame` uniform reads beyond the rest of the frame.
    pub shader: ShaderInputs,
}
/// A shader pack's view of the game this frame (docs/shader-packs.md, `Frame`).
#[derive(Clone, Copy, Debug, Default)]
pub struct ShaderInputs {
    /// `Level.getRainLevel` and `getThunderLevel` at the partial tick.
    pub rain: f32,
    pub thunder: f32,
    /// The camera's fog type: 0 air, 1 water, 2 lava, 3 powder snow.
    pub submerged: u8,
    /// 0 overworld, -1 the Nether, 1 the End.
    pub dimension: i8,
    pub has_sky_light: bool,
    /// `DimensionType.moonPhase`, 0..=7.
    pub moon_phase: u8,
    /// The time of day as a fraction (0 sunrise, 0.25 noon) and the partial tick.
    pub day_fraction: f32,
    pub partial_tick: f32,
    /// The block and sky light (0..15) of the block holding the camera
    /// entity's eyes; no sky light without a sky.
    pub eye_light: [u8; 2],
}
/// One frame of the world border wall (`WorldBorderRenderer.render`).
#[derive(Clone, Debug)]
pub struct BorderDraw {
    /// The cached wall buffer; uploaded only when `rebuilt`.
    pub vertices: [crate::border_render::BorderVertex; 16],
    pub rebuilt: bool,
    pub transforms: crate::border_render::BorderTransforms,
    /// Walls to draw (indices of four-vertex quads), nearest first.
    pub sides: Vec<usize>,
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BorderUniform {
    color: [f32; 4],
    offset: [f32; 4],
}
pub struct Renderer {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    /// The window's swapchain; `None` when rendering offscreen (`--offscreen`).
    surface: Option<wgpu::Surface<'static>>,
    /// The offscreen colour target, sized like `config`.
    offscreen: Option<wgpu::Texture>,
    /// Offscreen submissions still on the GPU: at most two frames run ahead.
    in_flight: std::collections::VecDeque<wgpu::SubmissionIndex>,
    /// Which steps of the first frame on this device still go to the log
    /// (see `startup_step`).
    first_frame: FirstFrame,
    gpu_timer: Option<timing::GpuTimer>,
    /// GPU frame times (ms) read back since the caller last drained them.
    pub gpu_times: Vec<f32>,
    /// The last lightmap written, RGBA rows of sky level by block level
    /// (F3+4 shows it).
    pub lightmap_pixels: [u8; 16 * 16 * 4],
    /// Time the last `render` spent blocked on the GPU or the display
    /// (acquiring, presenting, offscreen pacing), excluded from CPU time.
    pub last_wait: std::time::Duration,
    /// CPU time of the last `render`, waits excluded.
    pub last_encode: std::time::Duration,
    window: Arc<Window>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    solid: wgpu::RenderPipeline,
    translucent: wgpu::RenderPipeline,
    /// Flat section draws (`flat`), where multi-draw is emulated.
    flat: Option<flat::Flat>,
    /// `flat` draws this frame's sections (off: the per-section draws).
    flat_on: bool,
    /// Occlusion culling of solid section draws, where the GPU's instance
    /// counts reach a multi-draw's entries (Vulkan, Metal).
    cull: Option<cull::Cull>,
    cull_on: bool,
    sky: wgpu::RenderPipeline,
    dark_disc: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    /// Gizmo lines (`RenderTypes.linesTranslucentNoDepthWrite`) and the
    /// always-on-top ones.
    debug_lines: wgpu::RenderPipeline,
    debug_lines_on_top: wgpu::RenderPipeline,
    debug_line_buffer: wgpu::Buffer,
    debug_line_capacity: u64,
    /// The terrain resources with the debug crosshair's projection
    /// (`crosshair_globals`).
    crosshair_group: wgpu::BindGroup,
    crosshair_globals: wgpu::Buffer,
    shadow_pipeline: wgpu::RenderPipeline,
    shadow_entity_pipeline: wgpu::RenderPipeline,
    shadow_globals: wgpu::Buffer,
    shadow_group: wgpu::BindGroup,
    shadow_entity_group: wgpu::BindGroup,
    entity_pipeline: wgpu::RenderPipeline,
    weather: weather::Weather,
    crumbling_pipeline: wgpu::RenderPipeline,
    /// `RenderPipelines.WATER_MASK`: depth writes only, no colour.
    water_mask_pipeline: wgpu::RenderPipeline,
    cloud_pipeline: wgpu::RenderPipeline,
    cloud_mesh: crate::clouds::Mesh,
    cloud_buffer: wgpu::Buffer,
    cloud_capacity: u64,
    cloud_uniform: wgpu::Buffer,
    cloud_group: wgpu::BindGroup,
    additive_pipeline: wgpu::RenderPipeline,
    lightning_pipeline: wgpu::RenderPipeline,
    lightning: lightning::Renderer,
    see_through_pipeline: wgpu::RenderPipeline,
    celestial_pipeline: wgpu::RenderPipeline,
    celestial_buffer: wgpu::Buffer,
    end_sky: end_sky::EndSky,
    stars: stars::Stars,
    panorama: panorama::Panorama,
    /// `RenderPipelines.WORLD_BORDER` and its buffers.
    border_pipeline: wgpu::RenderPipeline,
    border_group: wgpu::BindGroup,
    border_uniform: wgpu::Buffer,
    border_vertices: wgpu::Buffer,
    /// Screen effects: alpha-blended, then vignette (`dst * (1 - src)`)
    /// pipelines, each for every sampling (index `blend * 4 + sampling`).
    screen_pipelines: [wgpu::RenderPipeline; 8],
    screen_buffer: wgpu::Buffer,
    screen_capacity: u64,
    entity_group: wgpu::BindGroup,
    /// The entity resources with the hand's projection (`hand_globals`).
    hand_group: wgpu::BindGroup,
    hand_globals: wgpu::Buffer,
    entity_buffer: wgpu::Buffer,
    entity_capacity: u64,
    entity_batch: EntityBatch,
    group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    lightmap: wgpu::Texture,
    /// The block sprites, [`SPRITE_SPLIT`] layers per array (the first is
    /// `sprites` in terrain.wgsl, the others `sprites_1`...).
    sprite_textures: Vec<wgpu::Texture>,
    /// A 1-layer array bound where a pack needs fewer arrays.
    sprite_unused: wgpu::TextureView,
    mip_levels: u32,
    /// The mipmap levels sampled (the Mipmap Levels option), at most `mip_levels - 1`.
    mipmaps: u32,
    terrain_layout: wgpu::BindGroupLayout,
    sprite_sampler: wgpu::Sampler,
    lightmap_view: wgpu::TextureView,
    lightmap_sampler: wgpu::Sampler,
    animations: animate::SpriteAnimator,
    uploads: upload::Uploads,
    /// Section meshes are staged through their own belt into
    /// `section_copies`, which the next frame records its commands after:
    /// `Queue::write_buffer` would make a staging buffer per write.
    section_uploads: upload::Uploads,
    section_copies: Option<wgpu::CommandEncoder>,
    /// Bytes staged in `section_copies`.
    section_copy_bytes: u64,
    /// Whether `lightmap` holds `lightmap_pixels` yet.
    lightmap_uploaded: bool,
    sprite_minimum_size: u32,
    indices: wgpu::Buffer,
    index_quads: u32,
    instances: wgpu::Buffer,
    instance_capacity: u64,
    outline: wgpu::Buffer,
    depth: wgpu::TextureView,
    sections: resident::Resident<SectionGpu>,
    lods: HashMap<crate::lod::TileKey, lod::Tile>,
    /// The chunk columns the resident tiles leave out (`Lod::take_cutout`):
    /// their sections are drawn even past the render distance, so terrain
    /// leaving it never opens a hole before the tiles fill it in.
    lod_cutout: crate::fast_hash::FxHashSet<(i32, i32)>,
    /// Removed sections of `lod_cutout` columns, released once tiles cover them.
    lod_retiring: crate::fast_hash::FxHashSet<SectionKey>,
    /// Finished tile meshes `upload_lods` has yet to upload.
    lod_pending: Option<lod::Pending>,
    detail_columns: HashMap<(i32, i32), u32>,
    /// Bumped whenever a column becomes or stops being complete.
    detail_revision: u64,
    arena: arena::Arena,
    sorted_indices: arena::Arena,
    translucent_sorter: translucency::Sorter,
    translucency_cursor: usize,
    /// `DrawIndexedIndirectArgs` for this frame's solid then translucent draws.
    indirect: wgpu::Buffer,
    indirect_capacity: u64,
    /// Indirect draws may set `first_instance` (the section origin).
    pub multi_draw: bool,
    /// Per-frame scratch, kept to avoid allocating every frame.
    visible: Vec<resident::Visible>,
    visible_lods: Vec<lod::Visible>,
    translucent_draws: Vec<(u64, usize, [u32; 5], (u32, Option<u32>))>,
    origins: Vec<[f32; 4]>,
    /// `DrawIndexedIndirectArgs` words: index count, instance count, first
    /// index, base vertex, first instance.
    draws: Vec<[u32; 5]>,
    /// Vertex arena page and optional sorted index arena page of each draw.
    draw_pages: Vec<(u32, Option<u32>)>,
    /// Solid draws left to occlusion culling's phase 2, while the list is
    /// built, and their pages.
    phase2_draws: Vec<[u32; 5]>,
    phase2_pages: Vec<(u32, Option<u32>)>,
    /// Each section's solid draws, to group them by arena page.
    solid_runs: page_runs::PageRuns,
    phase2_runs: page_runs::PageRuns,
    /// Block-entity blocks per meshed section (world position, state id).
    block_entities: HashMap<SectionKey, Vec<([i32; 3], u32)>>,
    /// Changes whenever `block_entities` does (unique across renderers).
    block_entity_revision: u64,
    /// VisGraph face connectivity per meshed section (cave culling).
    occlusion: occlusion::Occlusion,
    /// Resolution scale and FXAA (Horizon settings).
    world_post: world_post::Post,
    /// The selected shader pack's stages (Horizon setting; none by default).
    pack: Option<Pack>,
    pub hud: HudRenderer,
    /// Vanilla's menu background blur, made on first use.
    menu_blur: Option<crate::menu_blur::MenuBlur>,
    pub info: wgpu::AdapterInfo,
    pub suspended: bool,
    pub drawn_sections: usize,
    /// Last frame's section draws (face ranges and translucent sections),
    /// and the draw calls the backend issued for them.
    pub section_draws: usize,
    pub section_draw_calls: usize,
    pub uploaded_bytes: u64,
    started: Instant,
    /// Write the next presented frame to this PNG path (control socket).
    pub screenshot: Option<PathBuf>,
    /// Outcome of the last screenshot: frame size, or the error.
    pub screenshot_result: Option<std::result::Result<[u32; 2], String>>,
    /// Brightness option (lightmap gamma), 0..1.
    pub gamma: f32,
    /// Optional full-light lightmap (Horizon Extras).
    pub fullbright: bool,
    /// Advance animated block textures (Horizon setting; vanilla always does).
    pub animated_textures: bool,
    present_modes: Vec<wgpu::PresentMode>,
}
/// Every layer's mip levels, layer major (`first` is the first layer's
/// index into `settings`), made by `MipmapGenerator`.
/// Block sprite layers per texture array: the smallest
/// `max_texture_array_layers` of the GPUs Horizon runs on. Mods add sprites
/// past it into more arrays (terrain.wgsl's `sprite`).
pub(crate) const SPRITE_SPLIT: u32 = 2048;
/// [`SPRITE_SPLIT`], or a smaller split from `HORIZON_SPRITE_SPLIT` (dev
/// only: vanilla's sprites drawn from several arrays must look the same).
fn sprite_split() -> u32 {
    std::env::var("HORIZON_SPRITE_SPLIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| (1..=SPRITE_SPLIT).contains(&n))
        .unwrap_or(SPRITE_SPLIT)
}
/// Sprite arrays terrain.wgsl binds (`sprites`, `sprites_1`...).
const SPRITE_ARRAYS: u32 = 8;
/// The views of terrain.wgsl's sprite arrays, the unused ones a 1-layer
/// stand-in.
fn sprite_views(
    textures: &[wgpu::Texture],
    unused: &wgpu::TextureView,
    mips: Option<u32>,
) -> Vec<wgpu::TextureView> {
    (0..SPRITE_ARRAYS as usize)
        .map(|i| match textures.get(i) {
            Some(t) => t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                mip_level_count: mips,
                ..Default::default()
            }),
            None => unused.clone(),
        })
        .collect()
}
/// terrain.wgsl's group 0.
#[allow(clippy::too_many_arguments)]
fn terrain_group(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    sprites: &[wgpu::TextureView],
    sprite_sampler: &wgpu::Sampler,
    lightmap: &wgpu::TextureView,
    lightmap_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let view = |binding, view| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    };
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            view(1, &sprites[0]),
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sprite_sampler),
            },
            view(3, lightmap),
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(lightmap_sampler),
            },
            view(5, &sprites[1]),
            view(6, &sprites[2]),
            view(7, &sprites[3]),
            view(8, &sprites[4]),
            view(9, &sprites[5]),
            view(10, &sprites[6]),
            view(11, &sprites[7]),
        ],
    })
}
/// CPU texture uploads prepared by the resource worker. Consumed by renderer
/// construction so the temporary mip chains are released after GPU upload.
pub struct PreparedTextures {
    sprites: Vec<(Vec<u8>, u32)>,
    forcefield: (Vec<u8>, u32),
}
impl PreparedTextures {
    pub fn new(
        pixels: &[u8],
        size: u32,
        layers: u32,
        settings: &[crate::mipmap::Settings],
        forcefield: (&[u8], u32),
    ) -> Self {
        Self::with_split(pixels, size, layers, settings, forcefield, sprite_split())
    }

    fn with_split(
        pixels: &[u8],
        size: u32,
        layers: u32,
        settings: &[crate::mipmap::Settings],
        forcefield: (&[u8], u32),
        split: u32,
    ) -> Self {
        let layer_bytes = (size * size * 4) as usize;
        Self {
            sprites: (0..layers.div_ceil(split).max(1))
                .map(|array| {
                    let first = array * split;
                    let count = layers.saturating_sub(first).clamp(1, split);
                    let start = (first as usize * layer_bytes).min(pixels.len());
                    mip_chain(&pixels[start..], size, settings, first, count)
                })
                .collect(),
            forcefield: mip_chain(forcefield.0, forcefield.1, &[], 0, 1),
        }
    }
}

fn mip_chain(
    pixels: &[u8],
    size: u32,
    settings: &[crate::mipmap::Settings],
    first: u32,
    layers: u32,
) -> (Vec<u8>, u32) {
    let levels = size.trailing_zeros() + 1;
    let layer_bytes = (size * size * 4) as usize;
    // Exactly the chain's size: cached animation frames count their capacity.
    let chain_bytes: usize = (0..levels).map(|l| layer_bytes >> (2 * l)).sum();
    let mut out = Vec::with_capacity(chain_bytes * layers as usize);
    for layer in 0..layers as usize {
        let base = crate::mipmap::Image {
            width: size,
            height: size,
            pixels: pixels[layer * layer_bytes..(layer + 1) * layer_bytes]
                .as_chunks::<4>()
                .0
                .to_vec(),
        };
        let s = settings
            .get(first as usize + layer)
            .copied()
            .unwrap_or_default();
        let has_transparent = s.has_transparent || base.pixels.iter().any(|p| p[3] == 0);
        for image in crate::mipmap::generate(
            base,
            levels - 1,
            crate::mipmap::Settings {
                has_transparent,
                ..s
            },
        ) {
            out.extend(image.pixels.iter().flatten());
        }
    }
    debug_assert_eq!(out.len(), chain_bytes * layers as usize);
    (out, levels)
}
/// The wgpu instance options, without wgpu's validation of indirect draws
/// in release builds. That validation (meant for untrusted WebGPU content)
/// copies every indirect draw through a compute pass before the render pass
/// and costs about a tenth of `render`'s CPU time on Metal. The only indirect
/// draws are the section draws `render` writes from resident sections, in
/// bounds by construction: index counts within `indices`, vertices within
/// the section's arena range, first instances below the origins written that
/// frame. Debug builds keep validating them; `WGPU_VALIDATION_INDIRECT_CALL=1`
/// turns it back on. DirectX 12 keeps it too: there wgpu-core's validation
/// pass is also what passes each draw's first instance to shaders reading
/// `instance_index`, and DirectX 12 is the fallback, where working matters
/// more than CPU time.
fn instance_descriptor(backends: wgpu::Backends) -> wgpu::InstanceDescriptor {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends.with_env();
    let dx12 = cfg!(windows) && descriptor.backends.contains(wgpu::Backends::DX12);
    if !cfg!(debug_assertions) && !dx12 {
        descriptor
            .flags
            .remove(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL);
    }
    descriptor.with_env()
}
/// The backends to open a device on, each tried in turn until one gives a
/// device. `api` is the player's choice (`GraphicsApi::with_env` applied);
/// `wgpu_backend`: whether `WGPU_BACKEND` is set, which picks the backends
/// itself. Only Windows has two: Auto tries Vulkan, then DirectX 12 when
/// Vulkan has no adapter (no Vulkan driver) or no device. A native crash in
/// the driver can't be caught in the game; the launcher switches the setting
/// to DirectX 12 after one at the renderer's start (#746).
fn backend_order(api: GraphicsApi, windows: bool, wgpu_backend: bool) -> Vec<wgpu::Backends> {
    if !windows || wgpu_backend {
        return vec![wgpu::Backends::default()];
    }
    match api {
        GraphicsApi::Auto => vec![wgpu::Backends::VULKAN, wgpu::Backends::DX12],
        GraphicsApi::Vulkan => vec![wgpu::Backends::VULKAN],
        GraphicsApi::Dx12 => vec![wgpu::Backends::DX12],
    }
}
fn backends_name(backends: wgpu::Backends) -> &'static str {
    match backends {
        wgpu::Backends::VULKAN => "Vulkan",
        wgpu::Backends::DX12 => "DirectX 12",
        _ => "graphics",
    }
}
/// A device and what it came from.
struct Opened {
    instance: wgpu::Instance,
    surface: Option<wgpu::Surface<'static>>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}
/// Logs the adapter and records it for telemetry and crash reports.
fn describe_adapter(adapter: &wgpu::Adapter) {
    let info = adapter.get_info();
    log::info!(
        "Horizon renderer: {} ({:?}, {:?})",
        info.name,
        info.backend,
        info.device_type
    );
    crate::telemetry::set_gpu(crate::telemetry::Gpu {
        name: info.name.clone(),
        vendor: horizon_telemetry::system::gpu_vendor(&info.name, info.vendor).to_owned(),
        kind: crate::telemetry::gpu_kind(&format!("{:?}", info.device_type)).to_owned(),
        driver: format!("{} {}", info.driver, info.driver_info)
            .trim()
            .to_owned(),
        backend: format!("{:?}", info.backend),
    });
    crate::crash::set_detail(
        "Graphics Adapter",
        format!(
            "{} ({:?}, {:?}), driver {} {}",
            info.name, info.backend, info.device_type, info.driver, info.driver_info
        ),
    );
}
/// Whether an adapter's texture arrays hold the block textures Horizon puts
/// in one array.
fn check_layers(limits: &wgpu::Limits, layers: u32) -> Result<()> {
    let needed = layers.min(sprite_split());
    if limits.max_texture_array_layers < needed {
        return Err(format!(
            "GPU supports {} texture layers; {needed} required",
            limits.max_texture_array_layers
        )
        .into());
    }
    Ok(())
}
/// The swapchain format: a non-sRGB target, because vanilla blends and tints in gamma
/// space, and 8-bit, because wgpu gives `Rgba16Float` the linear `ExtendedSrgb` color
/// space: the compositor would read these gamma-encoded values as linear light and wash
/// every colour out.
fn surface_format(
    formats: &[wgpu::TextureFormat],
    default: wgpu::TextureFormat,
) -> wgpu::TextureFormat {
    [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ]
    .into_iter()
    .find(|f| formats.contains(f))
    .or_else(|| formats.iter().copied().find(|f| !f.is_srgb()))
    .unwrap_or(default)
}
/// Opens a device on the first of `backend_order`'s backends that gives one.
/// Offscreen rendering needs no surface, so it can use a GPU the window's
/// display cannot present from (a headless compositor).
async fn open_device(
    window: &Arc<Window>,
    offscreen: bool,
    api: GraphicsApi,
    layers: u32,
) -> Result<Opened> {
    let order = backend_order(
        api.with_env(),
        cfg!(windows),
        std::env::var_os("WGPU_BACKEND").is_some(),
    );
    let mut failures = Vec::new();
    for (attempt, backends) in order.iter().copied().enumerate() {
        let name = backends_name(backends);
        if attempt > 0 {
            log::warn!("Horizon renderer: trying {name} ({})", failures.join("; "));
        }
        let instance = wgpu::Instance::new(instance_descriptor(backends));
        let surface = if offscreen {
            None
        } else {
            Some(instance.create_surface(window.clone())?)
        };
        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: surface.as_ref(),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
        {
            Ok(adapter) => adapter,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        describe_adapter(&adapter);
        let info = adapter.get_info();
        let limits = adapter.limits();
        if let Err(e) = check_layers(&limits, layers) {
            failures.push(format!("{name}: {e}"));
            continue;
        }
        // Mappable primary buffers: section pages that no longer fit in
        // video memory go to system memory.
        let features = timing::GpuTimer::features(&adapter)
            | cull::Cull::features(&adapter)
            | (adapter.features()
                & (wgpu::Features::INDIRECT_FIRST_INSTANCE
                    | wgpu::Features::MAPPABLE_PRIMARY_BUFFERS));
        let driver = format!("{} {}", info.driver, info.driver_info);
        startup_step(format_args!(
            "requesting the device (driver {}) with features [{features}] and the adapter's \
             limits (buffers up to {} bytes, storage bindings up to {} bytes, {} texture layers)",
            match driver.trim() {
                "" => "unnamed",
                driver => driver,
            },
            limits.max_buffer_size,
            limits.max_storage_buffer_binding_size,
            limits.max_texture_array_layers,
        ));
        match adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Horizon device"),
                required_features: features,
                required_limits: limits,
                ..Default::default()
            })
            .await
        {
            Ok((device, queue)) => {
                return Ok(Opened {
                    instance,
                    surface,
                    adapter,
                    device,
                    queue,
                });
            }
            Err(e) => failures.push(format!("{name} device: {e}")),
        }
    }
    Err(format!("{NO_GPU_DRIVER} ({})", failures.join("; ")).into())
}
/// What to tell a player whose system offers no usable graphics adapter.
/// Vulkan is loaded at run time (the game links no graphics library), so a
/// missing driver shows here rather than as a missing-DLL dialog.
#[cfg(target_os = "windows")]
const NO_GPU_DRIVER: &str = "No graphics driver with Vulkan or DirectX 12 found: Horizon needs a GPU \
     driver with one of them. Install the latest driver for your NVIDIA, AMD or Intel GPU and \
     start the game again";
#[cfg(target_os = "macos")]
const NO_GPU_DRIVER: &str = "No Metal graphics device found";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const NO_GPU_DRIVER: &str = "No Vulkan graphics driver found: Horizon needs the Vulkan loader \
     (libvulkan.so.1) and a Vulkan driver for your GPU (Mesa's, or NVIDIA's)";
/// Logs a step of the renderer's start, for a native crash in the graphics
/// driver: it leaves no panic or crash report, so the log's last line is the
/// only sign of where the game stopped. The log file is written unbuffered;
/// the flush is for loggers that buffer.
fn startup_step(step: std::fmt::Arguments) {
    log::info!("Horizon renderer: {step}");
    log::logger().flush();
}
/// How far the first frame on a new device got, for `startup_step`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FirstFrame {
    Acquire,
    Submit,
    Drawn,
}
fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}
impl Renderer {
    /// Sections with GPU buffers, for soak tests watching residency.
    pub fn resident_sections(&self) -> usize {
        self.sections.len()
    }
    /// Quads of the far terrain tiles with GPU buffers.
    pub fn lod_quads(&self) -> u64 {
        self.lods
            .values()
            .map(|tile| u64::from(tile.mesh.solid + tile.mesh.translucent))
            .sum()
    }
    /// Bytes of the section vertex arena's buffers, and of the vertices
    /// resident sections hold in them.
    pub fn section_vertex_bytes(&self) -> [u64; 2] {
        let stride = std::mem::size_of::<TerrainVertex>() as u64;
        let vertices: u64 = self
            .sections
            .values()
            .chain(self.lods.values().map(|tile| &tile.mesh))
            .map(|s| u64::from(s.solid + s.translucent) * 4)
            .sum();
        [self.arena.bytes(), vertices * stride]
    }
}
impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        offscreen: bool,
        sprite_pixels: &[u8],
        sprite_size: u32,
        sprite_minimum_size: u32,
        layers: u32,
        sprite_mips: &[crate::mipmap::Settings],
        animations: &[SpriteAnimation],
        hud_atlas: (&[u8], u32, u32),
        forcefield: (&[u8], u32),
        panorama_faces: &panorama::Faces,
        prepared: Option<PreparedTextures>,
        previous: Option<&Self>,
        api: GraphicsApi,
    ) -> Result<Self> {
        let split = sprite_split();
        if layers > split * SPRITE_ARRAYS {
            return Err(format!(
                "{layers} block textures; Horizon draws at most {}",
                split * SPRITE_ARRAYS
            )
            .into());
        }
        let Opened {
            instance,
            surface,
            adapter,
            device,
            queue,
        } = match previous {
            // Reload resources on the same device; replacing live Vulkan
            // devices can invalidate the driver's swapchain state on the same
            // window.
            Some(renderer) => {
                describe_adapter(&renderer.adapter);
                check_layers(&renderer.adapter.limits(), layers)?;
                Opened {
                    surface: if offscreen {
                        None
                    } else {
                        Some(renderer.instance.create_surface(window.clone())?)
                    },
                    instance: renderer.instance.clone(),
                    adapter: renderer.adapter.clone(),
                    device: renderer.device.clone(),
                    queue: renderer.queue.clone(),
                }
            }
            None => open_device(&window, offscreen, api, layers).await?,
        };
        let info = adapter.get_info();
        crate::graphics::suggest_reduced_far_detail(crate::graphics::lod_capped(&info));
        let size = window.inner_size();
        let (config, present_modes) = if let Some(surface) = &surface {
            let mut config = surface
                .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                .ok_or("surface has no supported configuration")?;
            let caps = surface.get_capabilities(&adapter);
            config.format = surface_format(&caps.formats, config.format);
            config.view_formats = vec![config.format.remove_srgb_suffix()];
            // Screenshots copy the presented frame back from the swapchain.
            if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
                config.usage |= wgpu::TextureUsages::COPY_SRC;
            }
            config.present_mode = previous
                .map(|renderer| renderer.config.present_mode)
                .filter(|mode| caps.present_modes.contains(mode))
                .unwrap_or_else(|| {
                    if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
                        wgpu::PresentMode::Mailbox
                    } else {
                        wgpu::PresentMode::Fifo
                    }
                });
            if previous.is_none() {
                startup_step(format_args!(
                    "device ready; configuring the surface ({}x{} {:?}, {:?})",
                    config.width, config.height, config.format, config.present_mode
                ));
            }
            surface.configure(&device, &config);
            (config, caps.present_modes.clone())
        } else {
            let format = wgpu::TextureFormat::Bgra8Unorm;
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                format,
                width: size.width.max(1),
                height: size.height.max(1),
                present_mode: wgpu::PresentMode::Immediate,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![format],
                color_space: wgpu::SurfaceColorSpace::default(),
            };
            (config, Vec::new())
        };
        let offscreen = surface
            .is_none()
            .then(|| Self::offscreen_target(&device, &config));
        if previous.is_none() {
            startup_step(format_args!("creating the pipelines"));
        }
        let gpu_timer = timing::GpuTimer::new(&device, &queue);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon terrain"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain.wgsl").into()),
        });
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let sprite_array = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        };
        let sprite_arrays = layers.div_ceil(split).max(1);
        if sprite_arrays > 1 {
            log::info!("Horizon: {layers} block textures in {sprite_arrays} arrays");
        }
        let sprite_constants = [
            ("SPRITE_ARRAYS", f64::from(sprite_arrays)),
            ("SPRITE_SPLIT", f64::from(split)),
        ];
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon terrain bindings"),
            entries: &[
                entry(
                    0,
                    vf,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                ),
                entry(
                    1 + 1,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    3,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    4,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(5, vf, sprite_array),
                entry(6, vf, sprite_array),
                entry(7, vf, sprite_array),
                entry(8, vf, sprite_array),
                entry(9, vf, sprite_array),
                entry(10, vf, sprite_array),
                entry(11, vf, sprite_array),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon terrain layout"),
            bind_group_layouts: &[Some(&bindings)],
            ..Default::default()
        });
        let instance_attributes = wgpu::vertex_attr_array![5=>Float32x4];
        let terrain_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &TERRAIN_ATTRIBUTES,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 16,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attributes,
            }),
        ];
        let line_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3];
        let line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 24,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &line_attributes,
        })];
        let alpha_blend = wgpu::BlendState::ALPHA_BLENDING;
        let debug_line_attributes =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Unorm8x4,3=>Float32];
        let debug_line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 32,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &debug_line_attributes,
        })];
        let pipeline_in = |layout: &wgpu::PipelineLayout,
                           label: &str,
                           vertex: &str,
                           fragment: &str,
                           buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                           topology: wgpu::PrimitiveTopology,
                           cull: Option<wgpu::Face>,
                           depth_write: bool,
                           depth_test: bool,
                           blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(if depth_test {
                        wgpu::CompareFunction::GreaterEqual
                    } else {
                        wgpu::CompareFunction::Always
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &sprite_constants,
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format.remove_srgb_suffix(),
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = |label: &str,
                        vertex: &str,
                        fragment: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        topology: wgpu::PrimitiveTopology,
                        cull: Option<wgpu::Face>,
                        depth_write: bool,
                        depth_test: bool,
                        blend: Option<wgpu::BlendState>| {
            pipeline_in(
                &layout,
                label,
                vertex,
                fragment,
                buffers,
                topology,
                cull,
                depth_write,
                depth_test,
                blend,
            )
        };
        let tri = wgpu::PrimitiveTopology::TriangleList;
        let solid = pipeline(
            "Terrain solid",
            "vs_terrain",
            "fs_solid",
            &terrain_buffers,
            tri,
            Some(wgpu::Face::Back),
            true,
            true,
            None,
        );
        let translucent = pipeline(
            "Terrain translucent",
            "vs_terrain",
            "fs_translucent",
            &terrain_buffers,
            tri,
            Some(wgpu::Face::Back),
            true,
            true,
            Some(alpha_blend),
        );
        let sky = pipeline(
            "Sky",
            "vs_sky",
            "fs_sky",
            &[],
            tri,
            None,
            false,
            false,
            None,
        );
        let dark_disc = pipeline(
            "Dark disc",
            "vs_sky",
            "fs_dark_disc",
            &[],
            tri,
            None,
            false,
            false,
            Some(alpha_blend),
        );
        let lines = pipeline(
            "Block outline",
            "vs_line",
            "fs_line",
            &line_buffers,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
            true,
            true,
            Some(alpha_blend),
        );
        let debug_lines = pipeline(
            "Debug lines",
            "vs_debug_line",
            "fs_debug_line",
            &debug_line_buffers,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
            false,
            true,
            Some(alpha_blend),
        );
        let debug_lines_on_top = pipeline(
            "Debug lines on top",
            "vs_debug_line",
            "fs_debug_line",
            &debug_line_buffers,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
            false,
            false,
            Some(alpha_blend),
        );
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hand_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("First-person hand globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let crosshair_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Debug crosshair globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack shadow globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Terrain shadow"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_terrain"),
                compilation_options: Default::default(),
                buffers: &terrain_buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: tri,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let prepared = prepared.unwrap_or_else(|| {
            PreparedTextures::new(sprite_pixels, sprite_size, layers, sprite_mips, forcefield)
        });
        let mut mip_levels = 1;
        let sprite_textures: Vec<wgpu::Texture> = prepared
            .sprites
            .into_iter()
            .enumerate()
            .map(|(array, (mips, levels))| {
                let first = array as u32 * split;
                let count = layers.saturating_sub(first).clamp(1, split);
                mip_levels = levels;
                device.create_texture_with_data(
                    &queue,
                    &wgpu::TextureDescriptor {
                        label: Some("Block sprites"),
                        size: wgpu::Extent3d {
                            width: sprite_size,
                            height: sprite_size,
                            depth_or_array_layers: count,
                        },
                        mip_level_count: levels,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_DST
                            | wgpu::TextureUsages::STORAGE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    &mips,
                )
            })
            .collect();
        let animations = animate::SpriteAnimator::new(
            &device,
            animations,
            sprite_size,
            sprite_mips,
            &sprite_textures,
            split,
        );
        let sprite_unused = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Block sprites (unused)"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        let sprite_views = sprite_views(&sprite_textures, &sprite_unused, None);
        let sprite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Block sprites"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let lightmap = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Lightmap"),
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let lightmap_view = lightmap.create_view(&Default::default());
        let lightmap_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Lightmap"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let group = terrain_group(
            &device,
            "Horizon terrain resources",
            &bindings,
            &globals,
            &sprite_views,
            &sprite_sampler,
            &lightmap_view,
            &lightmap_sampler,
        );
        let crosshair_group = terrain_group(
            &device,
            "Debug crosshair resources",
            &bindings,
            &crosshair_globals,
            &sprite_views,
            &sprite_sampler,
            &lightmap_view,
            &lightmap_sampler,
        );
        let shadow_group = terrain_group(
            &device,
            "Shader pack shadow terrain resources",
            &bindings,
            &shadow_globals,
            &sprite_views,
            &sprite_sampler,
            &lightmap_view,
            &lightmap_sampler,
        );
        let index_quads = 1 << 16;
        let indices = Self::quad_indices(&device, index_quads);
        let instance_capacity = 4096;
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Section origins"),
            size: instance_capacity * 16,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let outline = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Outline"),
            size: 24 * MAX_OUTLINE_EDGES,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let debug_line_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Debug lines"),
            size: 32 * 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (atlas_pixels, atlas_width, atlas_height) = hud_atlas;
        let limit = device.limits().max_texture_dimension_2d;
        if atlas_width.max(atlas_height) > limit {
            return Err(format!(
                "the shared atlas is {atlas_width}x{atlas_height}; this GPU allows {limit}x{limit}"
            )
            .into());
        }
        let atlas = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("Shared atlas"),
                size: wgpu::Extent3d {
                    width: atlas_width,
                    height: atlas_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            atlas_pixels,
        );
        let atlas_view = atlas.create_view(&Default::default());
        let mut hud = HudRenderer::new(&device, config.format.remove_srgb_suffix(), &atlas_view)?;
        hud.atlas_texture = Some(atlas.clone());
        let entity_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon entities"),
            source: wgpu::ShaderSource::Wgsl(entity_shader().into()),
        });
        let entity_bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Entity bindings"),
            entries: &[
                entry(
                    0,
                    vf,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    2,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    3,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    4,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let weather = weather::Weather::new(
            &device,
            config.format.remove_srgb_suffix(),
            &entity_bindings,
            &entity_shader,
        );
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Atlas"),
            ..Default::default()
        });
        let entity_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Entity resources"),
            layout: &entity_bindings,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&lightmap_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&lightmap_sampler),
                },
            ],
        });
        let hand_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("First-person hand resources"),
            layout: &entity_bindings,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: hand_globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&lightmap_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&lightmap_sampler),
                },
            ],
        });
        let shadow_entity_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader pack shadow entity resources"),
            layout: &entity_bindings,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: shadow_globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&lightmap_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&lightmap_sampler),
                },
            ],
        });
        let entity_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Entity layout"),
            bind_group_layouts: &[Some(&entity_bindings)],
            ..Default::default()
        });
        let cloud_bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cloud bindings"),
            entries: &[entry(
                0,
                wgpu::ShaderStages::VERTEX,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            )],
        });
        let cloud_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cloud transforms and colors"),
            size: std::mem::size_of::<crate::clouds::Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cloud_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Cloud resources"),
            layout: &cloud_bindings,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cloud_uniform.as_entire_binding(),
            }],
        });
        let cloud_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cloud layout"),
            bind_group_layouts: &[Some(&entity_bindings), Some(&cloud_bindings)],
            ..Default::default()
        });
        let cloud_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cloud vertices"),
            size: 4 * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entity_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4];
        let entity_vertex = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &entity_attributes,
        })];
        #[allow(clippy::too_many_arguments)]
        let entity_pipeline_with = |label,
                                    cull_mode,
                                    depth_write,
                                    compare,
                                    bias,
                                    blend,
                                    clouds,
                                    fragment: &str,
                                    write_mask| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(if clouds {
                    &cloud_layout
                } else {
                    &entity_layout
                }),
                vertex: wgpu::VertexState {
                    module: &entity_shader,
                    entry_point: Some(if clouds { "vs_cloud" } else { "vs_entity" }),
                    compilation_options: Default::default(),
                    buffers: &entity_vertex,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias,
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &entity_shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format.remove_srgb_suffix(),
                        blend: Some(blend),
                        write_mask,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let entity_pipeline = |label, cull_mode, depth_write, compare, bias, blend, clouds| {
            entity_pipeline_with(
                label,
                cull_mode,
                depth_write,
                compare,
                bias,
                blend,
                clouds,
                "fs_entity",
                wgpu::ColorWrites::ALL,
            )
        };
        let shadow_entity_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Entity shadow"),
                layout: Some(&entity_layout),
                vertex: wgpu::VertexState {
                    module: &entity_shader,
                    entry_point: Some("vs_entity"),
                    compilation_options: Default::default(),
                    buffers: &entity_vertex,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            });
        // RenderPipelines.WATER_MASK: the boat's water patch writes depth only.
        let water_mask_pipeline = entity_pipeline_with(
            "Water mask",
            None,
            true,
            wgpu::CompareFunction::GreaterEqual,
            Default::default(),
            alpha_blend,
            false,
            "fs_water_mask",
            wgpu::ColorWrites::empty(),
        );
        let crumbling_pipeline = entity_pipeline(
            "Breaking cracks",
            Some(wgpu::Face::Back),
            false,
            wgpu::CompareFunction::GreaterEqual,
            wgpu::DepthBiasState {
                constant: 10,
                slope_scale: 1.0,
                clamp: 0.0,
            },
            CRUMBLING_BLEND,
            false,
        );
        let cloud_pipeline = entity_pipeline(
            "Clouds",
            Some(wgpu::Face::Back),
            true,
            wgpu::CompareFunction::GreaterEqual,
            Default::default(),
            alpha_blend,
            true,
        );
        // RenderPipelines.TEXT_SEE_THROUGH: translucent, no depth test or writes.
        let see_through_pipeline = entity_pipeline(
            "See-through text",
            None,
            false,
            wgpu::CompareFunction::Always,
            Default::default(),
            alpha_blend,
            false,
        );
        let entity_pipeline = entity_pipeline(
            "Entities",
            None,
            true,
            wgpu::CompareFunction::GreaterEqual,
            Default::default(),
            alpha_blend,
            false,
        );
        let lightning_attributes = wgpu::vertex_attr_array![6=>Float32x3];
        let lightning_vertex = [
            entity_vertex[0].clone(),
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<[f32; 3]>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &lightning_attributes,
            }),
        ];
        // BlendFunction.LIGHTNING: src alpha, one; no depth writes.
        let additive_descriptor = wgpu::RenderPipelineDescriptor {
            label: Some("Additive entities"),
            layout: Some(&entity_layout),
            vertex: wgpu::VertexState {
                module: &entity_shader,
                entry_point: Some("vs_entity"),
                compilation_options: Default::default(),
                buffers: &entity_vertex,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &entity_shader,
                entry_point: Some("fs_celestial"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format.remove_srgb_suffix(),
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        };
        let additive_pipeline = device.create_render_pipeline(&additive_descriptor);
        let lightning_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Lightning"),
            vertex: wgpu::VertexState {
                entry_point: Some("vs_lightning"),
                buffers: &lightning_vertex,
                ..additive_descriptor.vertex.clone()
            },
            ..additive_descriptor
        });
        let celestial_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Celestial bodies"),
            layout: Some(&entity_layout),
            vertex: wgpu::VertexState {
                module: &entity_shader,
                entry_point: Some("vs_entity"),
                compilation_options: Default::default(),
                buffers: &entity_vertex,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &entity_shader,
                entry_point: Some("fs_celestial"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format.remove_srgb_suffix(),
                    // RenderPipelines.CELESTIAL: additive (src alpha, one).
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // WorldBorderRenderer: misc/forcefield.png as a MipmappedTexture
        // (repeat, linear minification with mipmaps, nearest magnification).
        let forcefield_size = forcefield.1;
        let (forcefield_mips, forcefield_levels) = prepared.forcefield;
        let forcefield_texture = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("Forcefield"),
                size: wgpu::Extent3d {
                    width: forcefield_size,
                    height: forcefield_size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: forcefield_levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &forcefield_mips,
        );
        let forcefield_view = forcefield_texture.create_view(&Default::default());
        let forcefield_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Forcefield"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            min_filter: wgpu::FilterMode::Linear,
            mag_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let border_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("World border"),
            size: std::mem::size_of::<BorderUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let border_vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("World border vertices"),
            size: 16 * std::mem::size_of::<crate::border_render::BorderVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let border_bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("World border bindings"),
            entries: &[
                entry(
                    0,
                    vf,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    2,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    3,
                    vf,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let border_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("World border resources"),
            layout: &border_bindings,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&forcefield_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&forcefield_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: border_uniform.as_entire_binding(),
                },
            ],
        });
        let border_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("World border layout"),
            bind_group_layouts: &[Some(&border_bindings)],
            ..Default::default()
        });
        let border_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon world border"),
            source: wgpu::ShaderSource::Wgsl(include_str!("border.wgsl").into()),
        });
        let border_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2];
        // RenderPipelines.WORLD_BORDER: BlendFunction.OVERLAY, no culling,
        // depth writes with a polygon offset of 3 towards the camera.
        let border_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("World border"),
            layout: Some(&border_layout),
            vertex: wgpu::VertexState {
                module: &border_shader,
                entry_point: Some("vs_border"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<crate::border_render::BorderVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &border_attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 3,
                    slope_scale: 3.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &border_shader,
                entry_point: Some("fs_border"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format.remove_srgb_suffix(),
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::Zero,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let screen_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon screen effects"),
            source: wgpu::ShaderSource::Wgsl(include_str!("screen.wgsl").into()),
        });
        let screen_attributes =
            wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Float32x4,3=>Float32x4];
        let screen_vertex = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::screen_effects::ScreenVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &screen_attributes,
        })];
        let vignette_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::OneMinusSrc,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        // One per blend and `.png.mcmeta` sampling (screen_effects::SAMPLE_*):
        // the shader is specialised, so a pixel pays for none of the others.
        let screen_pipelines: [wgpu::RenderPipeline; 8] = std::array::from_fn(|i| {
            let (entry, blend) = if i & 4 == 0 {
                ("fs_screen", alpha_blend)
            } else {
                ("fs_vignette", vignette_blend)
            };
            let on = |bit: u32| f64::from(u8::from(i as u32 & bit != 0));
            let constants = [
                ("BLUR", on(crate::screen_effects::SAMPLE_BLUR)),
                ("CLAMP", on(crate::screen_effects::SAMPLE_CLAMP)),
            ];
            let compilation_options = wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Screen effects"),
                layout: Some(&entity_layout),
                vertex: wgpu::VertexState {
                    module: &screen_shader,
                    entry_point: Some("vs_screen"),
                    compilation_options: compilation_options.clone(),
                    buffers: &screen_vertex,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &screen_shader,
                    entry_point: Some(entry),
                    compilation_options,
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format.remove_srgb_suffix(),
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let screen_capacity = 64;
        let screen_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Screen effect vertices"),
            size: screen_capacity
                * std::mem::size_of::<crate::screen_effects::ScreenVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let celestial_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Celestial vertices"),
            size: 2048 * 4 * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entity_capacity = 1 << 14;
        let entity_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Entity vertices"),
            size: entity_capacity * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let arena = arena::Arena::new(
            &device,
            std::mem::size_of::<TerrainVertex>() as u64,
            1 << 20,
        );
        // Sections as one index list per frame where multi-draw is emulated.
        let flat = flat::wanted(&adapter, &device).then(|| {
            let buffers = [Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &TERRAIN_ATTRIBUTES,
            })];
            flat::Flat::new(
                &device,
                &bindings,
                arena.page_vertices(),
                |layout, label, fragment, blend| {
                    pipeline_in(
                        layout,
                        label,
                        "vs_terrain_flat",
                        fragment,
                        &buffers,
                        tri,
                        Some(wgpu::Face::Back),
                        true,
                        true,
                        blend,
                    )
                },
            )
        });
        let sorted_indices = arena::Arena::indices(&device, 1 << 16, flat.is_some());
        let indirect = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Section draws"),
            size: 1024 * 20,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let multi_draw = device
            .features()
            .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE);
        // HORIZON_GPU_CULL=0 turns occlusion culling off, `all` culls every
        // frame however few its draws and little it hides, `shader` tests in
        // the shader as on Metal (A/B checks).
        let cull_env = std::env::var("HORIZON_GPU_CULL");
        let cull = cull::Cull::supported(&adapter, &device)
            .filter(|_| cull_env.as_deref() != Ok("0"))
            .map(|depth_test| match cull_env.as_deref() {
                Ok("shader") => cull::DepthTest::InShader,
                _ => depth_test,
            })
            .map(|depth_test| {
                let mut cull =
                    cull::Cull::new(&device, config.format.remove_srgb_suffix(), depth_test);
                cull.always = cull_env.as_deref() == Ok("all");
                if !cull.always && adapter.get_info().backend == wgpu::Backend::Metal {
                    cull.min_draws = cull::TILED_MIN_DRAWS;
                }
                cull
            });
        let depth = Self::depth(&device, &config, cull.as_ref());
        let world_post = world_post::Post::new(&device, config.format.remove_srgb_suffix());
        let uploads = upload::Uploads::new(&device);
        let section_uploads = upload::Uploads::with_chunk(&device, SECTION_UPLOAD_CHUNK);
        let stars = stars::Stars::new(&device, config.format.remove_srgb_suffix());
        let panorama = panorama::Panorama::new(
            &device,
            &queue,
            config.format.remove_srgb_suffix(),
            panorama_faces,
        );
        if previous.is_none() {
            startup_step(format_args!(
                "pipelines ready (multi-draw {}, occlusion culling {}, GPU timer {})",
                on_off(multi_draw),
                on_off(cull.is_some()),
                on_off(gpu_timer.is_some()),
            ));
        }
        Ok(Self {
            instance,
            adapter,
            surface,
            offscreen,
            in_flight: std::collections::VecDeque::new(),
            first_frame: if previous.is_none() {
                FirstFrame::Acquire
            } else {
                FirstFrame::Drawn
            },
            gpu_timer,
            gpu_times: Vec::new(),
            lightmap_pixels: [0; 16 * 16 * 4],
            last_wait: std::time::Duration::ZERO,
            last_encode: std::time::Duration::ZERO,
            window,
            device,
            queue,
            config,
            solid,
            translucent,
            flat_on: flat.is_some(),
            cull_on: cull.is_some(),
            cull,
            flat,
            sky,
            dark_disc,
            lines,
            debug_lines,
            debug_lines_on_top,
            debug_line_buffer,
            debug_line_capacity: 4096,
            crosshair_group,
            crosshair_globals,
            shadow_pipeline,
            shadow_entity_pipeline,
            shadow_globals,
            shadow_group,
            shadow_entity_group,
            entity_pipeline,
            weather,
            crumbling_pipeline,
            water_mask_pipeline,
            cloud_pipeline,
            cloud_mesh: Default::default(),
            cloud_buffer,
            cloud_capacity: 4,
            cloud_uniform,
            cloud_group,
            additive_pipeline,
            lightning_pipeline,
            lightning: lightning::Renderer::new(),
            see_through_pipeline,
            celestial_pipeline,
            celestial_buffer,
            end_sky: end_sky::EndSky::default(),
            stars,
            panorama,
            border_pipeline,
            border_group,
            border_uniform,
            border_vertices,
            screen_pipelines,
            screen_buffer,
            screen_capacity,
            entity_group,
            hand_group,
            hand_globals,
            entity_buffer,
            entity_capacity,
            entity_batch: EntityBatch::default(),
            group,
            globals,
            lightmap,
            sprite_textures,
            sprite_unused,
            mip_levels,
            mipmaps: mip_levels - 1,
            terrain_layout: bindings,
            sprite_sampler,
            lightmap_view,
            lightmap_sampler,
            animations,
            uploads,
            section_uploads,
            section_copies: None,
            section_copy_bytes: 0,
            lightmap_uploaded: false,
            sprite_minimum_size,
            indices,
            index_quads,
            instances,
            instance_capacity,
            outline,
            depth,
            sections: resident::Resident::default(),
            lods: HashMap::new(),
            lod_cutout: Default::default(),
            lod_retiring: Default::default(),
            lod_pending: None,
            detail_columns: HashMap::new(),
            detail_revision: 0,
            arena,
            sorted_indices,
            translucent_sorter: translucency::Sorter::default(),
            translucency_cursor: 0,
            indirect,
            indirect_capacity: 1024,
            multi_draw,
            visible: Vec::new(),
            visible_lods: Vec::new(),
            translucent_draws: Vec::new(),
            origins: Vec::new(),
            draws: Vec::new(),
            draw_pages: Vec::new(),
            phase2_draws: Vec::new(),
            phase2_pages: Vec::new(),
            solid_runs: page_runs::PageRuns::default(),
            phase2_runs: page_runs::PageRuns::default(),
            block_entities: HashMap::new(),
            block_entity_revision: next_block_entity_revision(),
            occlusion: occlusion::Occlusion::new(crate::chunk::MIN_Y, crate::chunk::HEIGHT / 16),
            world_post,
            pack: None,
            hud,
            menu_blur: None,
            info,
            suspended: size.width == 0 || size.height == 0,
            drawn_sections: 0,
            section_draws: 0,
            section_draw_calls: 0,
            uploaded_bytes: 0,
            started: Instant::now(),
            screenshot: None,
            screenshot_result: None,
            gamma: 0.5,
            fullbright: false,
            animated_textures: true,
            present_modes,
        })
    }
    fn quad_indices(device: &wgpu::Device, quads: u32) -> wgpu::Buffer {
        let indices: Vec<u32> = (0..quads)
            .flat_map(|q| {
                let b = q * 4;
                [b, b + 1, b + 2, b, b + 2, b + 3]
            })
            .collect();
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Quad indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        })
    }
    fn offscreen_target(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Offscreen frame"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: config.usage,
            view_formats: &config.view_formats,
        })
    }
    /// Release the window surface before another renderer uses the same window.
    pub fn release_surface(&mut self) {
        self.surface = None;
    }
    /// Reattach a window after surface loss or a failed renderer replacement.
    pub fn recreate_surface(&mut self) -> Result<()> {
        if self.offscreen.is_some() {
            return Ok(());
        }
        // Wayland allows only one tearing-control object per window surface.
        self.release_surface();
        let surface = self.instance.create_surface(self.window.clone())?;
        surface.configure(&self.device, &self.config);
        self.surface = Some(surface);
        Ok(())
    }
    /// The world's depth, which occlusion culling's test may read.
    fn depth(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        cull: Option<&cull::Cull>,
    ) -> wgpu::TextureView {
        let read = cull.is_some_and(|c| c.depth_test() == cull::DepthTest::InShader);
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Horizon depth"),
                size: wgpu::Extent3d {
                    width: config.width,
                    height: config.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: if read {
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
                } else {
                    wgpu::TextureUsages::RENDER_ATTACHMENT
                },
                view_formats: &[],
            })
            .create_view(&Default::default())
    }
    /// VSync on presents with Fifo, capped to the display's refresh rate in a
    /// window and in fullscreen alike; Horizon's present-mode setting applies
    /// only with VSync off (`window::chosen_present_mode`).
    pub fn set_present_mode(&mut self, vsync: bool, mode: crate::graphics::PresentMode) {
        use crate::graphics::PresentMode as P;
        use crate::window::{PresentMode, chosen_present_mode};
        let supported: Vec<PresentMode> = self
            .present_modes
            .iter()
            .filter_map(|m| match m {
                wgpu::PresentMode::Fifo => Some(PresentMode::Fifo),
                wgpu::PresentMode::Mailbox => Some(PresentMode::Mailbox),
                wgpu::PresentMode::Immediate => Some(PresentMode::Immediate),
                _ => None,
            })
            .collect();
        let setting = match mode {
            P::Auto => None,
            P::Fifo => Some(PresentMode::Fifo),
            P::Mailbox => Some(PresentMode::Mailbox),
            P::Immediate => Some(PresentMode::Immediate),
        };
        let mode = match chosen_present_mode(&supported, vsync, setting) {
            Some(PresentMode::Mailbox) => wgpu::PresentMode::Mailbox,
            Some(PresentMode::Immediate) => wgpu::PresentMode::Immediate,
            _ => wgpu::PresentMode::Fifo,
        };
        if self.config.present_mode != mode
            && let Some(surface) = &self.surface
        {
            self.config.present_mode = mode;
            surface.configure(&self.device, &self.config);
        }
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        self.suspended = width == 0 || height == 0;
        if self.suspended {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.config);
        } else {
            self.offscreen = Some(Self::offscreen_target(&self.device, &self.config));
        }
        self.depth = Self::depth(&self.device, &self.config, self.cull.as_ref());
    }
    /// Whether sections are drawn as one index list per frame (`flat`).
    pub fn flat_draws(&self) -> bool {
        self.flat_on
    }
    /// Switches between flat and per-section draws, for A/B checks.
    pub fn set_flat_draws(&mut self, on: bool) -> Result<()> {
        if on && self.flat.is_none() {
            return Err("flat section draws are off on this device".into());
        }
        self.flat_on = on;
        Ok(())
    }
    /// Whether solid sections are occlusion culled (`cull`).
    pub fn section_cull(&self) -> bool {
        self.cull_on
    }
    /// Switches occlusion culling, for A/B checks.
    pub fn set_section_cull(&mut self, on: bool) -> Result<()> {
        if on && self.cull.is_none() {
            return Err("occlusion culling is off on this device".into());
        }
        self.cull_on = on;
        Ok(())
    }
    /// Of the boxes occlusion culling's last test read back drew, how many
    /// it did not see and how many it drew, and the share of solid indices
    /// it hid.
    pub fn culled_sections(&self) -> Option<([usize; 2], f32)> {
        self.cull
            .as_ref()
            .filter(|_| self.cull_on)
            .map(|c| c.hidden)
    }
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }
    pub fn has_section(&self, key: SectionKey) -> bool {
        self.sections.contains(key)
    }
    /// Bring the cave culling graph up to date for `camera` without waiting:
    /// a search this starts runs on its worker while the frame is prepared,
    /// and `render` takes it if it has finished.
    pub fn start_occlusion(&mut self, camera: DVec3, distance: f32) {
        self.occlusion.update(camera, (distance / 16.0) as i32 + 2);
    }
    pub fn upload(&mut self, key: SectionKey, mesh: &SectionMesh) {
        self.occlusion.set(key, mesh.visibility);
        let changed = if mesh.block_entities.is_empty() {
            self.block_entities.remove(&key).is_some()
        } else {
            let old = self.block_entities.insert(key, mesh.block_entities.clone());
            old.as_deref() != Some(&mesh.block_entities[..])
        };
        if changed {
            self.block_entity_revision = next_block_entity_revision();
        }
        if !self.lod_retiring.is_empty() {
            self.lod_retiring.remove(&key);
        }
        self.release_section(key);
        let section = self.upload_mesh(mesh);
        self.set_detail(
            key,
            section.is_some() || (mesh.solid.is_empty() && mesh.translucent.is_empty()),
        );
        if let Some(section) = section {
            self.sections.insert(key, section);
        }
    }
    fn upload_mesh(&mut self, mesh: &SectionMesh) -> Option<SectionGpu> {
        let solid = (mesh.solid.len() / 4) as u32;
        let translucent = (mesh.translucent.len() / 4) as u32;
        if solid + translucent == 0 {
            return None;
        }
        let max = solid.max(translucent);
        if max > self.index_quads {
            self.index_quads = max.next_power_of_two();
            self.indices = Self::quad_indices(&self.device, self.index_quads);
        }
        let vertices = (mesh.solid.len() + mesh.translucent.len()) as u64;
        let encoder = self.section_copies.get_or_insert_with(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Section uploads"),
                })
        });
        // With video and system memory both full, the section is not drawn.
        let (page, start) = self.arena.alloc(&self.device, encoder, vertices)?;
        let sorted_indices = if translucent > 0 {
            let indices =
                self.sorted_indices
                    .alloc(&self.device, encoder, u64::from(translucent) * 6);
            if indices.is_none() {
                self.arena.release(page, start, vertices);
                return None;
            }
            indices
        } else {
            None
        };
        let flat_id = self.flat.as_mut().map(|f| {
            f.add(
                &self.device,
                encoder,
                &mut self.section_uploads,
                page,
                start,
                vertices,
            )
        });
        let stride = std::mem::size_of::<TerrainVertex>() as u64;
        for (offset, part) in [(0, &mesh.solid), (mesh.solid.len(), &mesh.translucent)] {
            if !part.is_empty() {
                let bytes: &[u8] = bytemuck::cast_slice(part);
                self.uploaded_bytes += bytes.len() as u64;
                self.section_copy_bytes += bytes.len() as u64;
                self.section_uploads.buffer(
                    encoder,
                    self.arena.buffer(page),
                    (start + offset as u64) * stride,
                    bytes,
                );
            }
        }
        if self.section_copy_bytes >= SECTION_COPY_LIMIT
            && let Some(copies) = self.take_section_copies()
        {
            self.queue.submit([copies]);
        }
        Some(SectionGpu {
            page,
            start,
            solid,
            translucent,
            faces: mesh.faces,
            sort: translucency::SortState::new(
                mesh.translucent
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|q| translucency::quad_center(q[0].pos, q[2].pos))
                    .collect(),
            ),
            sorted_indices,
            flat_id,
        })
    }
    /// The section copies recorded since the last submission, closed.
    fn take_section_copies(&mut self) -> Option<wgpu::CommandBuffer> {
        let encoder = self.section_copies.take()?;
        self.section_copy_bytes = 0;
        self.section_uploads.finish(&encoder);
        Some(encoder.finish())
    }
    /// Draws `count` section draws from `first` with the arena and section
    /// origins bound: one indirect multi-draw, or a loop without it.
    /// Consecutive draws in one arena page share a vertex buffer binding,
    /// keeping the list's order (translucent sections go back to front).
    /// `indirect` holds the list's draws from `base`.
    fn draw_sections(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indirect: &wgpu::Buffer,
        base: u32,
        first: u32,
        count: u32,
    ) {
        let (mut i, end) = (first as usize, (first + count) as usize);
        while i < end {
            let (page, index_page) = self.draw_pages[i];
            let run = self.draw_pages[i..end]
                .iter()
                .take_while(|&&p| p == (page, index_page))
                .count();
            pass.set_vertex_buffer(0, self.arena.buffer(page).slice(..));
            let indices = index_page.map_or(&self.indices, |p| self.sorted_indices.buffer(p));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            if self.multi_draw {
                let offset = (i - base as usize) as u64 * 20;
                pass.multi_draw_indexed_indirect(indirect, offset, run as u32);
            } else {
                for d in &self.draws[i..i + run] {
                    pass.draw_indexed(d[2]..d[2] + d[0], d[3] as i32, d[4]..d[4] + 1);
                }
            }
            i += run;
        }
    }
    fn release_section(&mut self, key: SectionKey) {
        if let Some(s) = self.sections.remove(key) {
            self.release_mesh(s);
        }
    }
    fn release_mesh(&mut self, s: SectionGpu) {
        if let (Some(flat), Some(id)) = (&mut self.flat, s.flat_id) {
            flat.remove(id);
        }
        if let Some((page, start)) = s.sorted_indices {
            self.sorted_indices
                .release(page, start, u64::from(s.translucent) * 6);
        }
        self.arena
            .release(s.page, s.start, u64::from(s.solid + s.translucent) * 4);
    }
    /// Block-entity blocks of every meshed section (see `SectionMesh::block_entities`).
    pub fn block_entities(&self) -> impl Iterator<Item = &([i32; 3], u32)> {
        self.block_entities.values().flatten()
    }
    /// A value that changes whenever `block_entities` gives something else.
    pub fn block_entity_revision(&self) -> u64 {
        self.block_entity_revision
    }
    pub fn clear_sections(&mut self) {
        self.block_entities.clear();
        self.block_entity_revision = next_block_entity_revision();
        self.sections.clear();
        self.lods.clear();
        self.lod_cutout.clear();
        self.lod_retiring.clear();
        self.lod_pending = None;
        self.detail_columns.clear();
        self.detail_revision += 1;
        self.arena.clear();
        if let Some(flat) = &mut self.flat {
            flat.clear();
        }
        self.sorted_indices.clear();
        self.translucency_cursor = 0;
        self.occlusion.clear();
    }
    pub fn lod_detail_columns(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.detail_columns
            .iter()
            .filter_map(move |(&column, &mask)| (mask == COMPLETE_COLUMN).then_some(column))
    }
    /// Changes whenever `lod_detail_columns` does.
    pub fn lod_detail_revision(&self) -> u64 {
        self.detail_revision
    }
    /// Whether the section at `key` has its mesh, for `lod_detail_columns`.
    fn set_detail(&mut self, key: SectionKey, meshed: bool) {
        let column = (key.0, key.2);
        let mask = self.detail_columns.get(&column).copied().unwrap_or(0);
        let next = if meshed {
            mask | 1 << key.1
        } else {
            mask & !(1 << key.1)
        };
        if (mask == COMPLETE_COLUMN) != (next == COMPLETE_COLUMN) {
            self.detail_revision += 1;
        }
        if next == 0 {
            self.detail_columns.remove(&column);
        } else {
            self.detail_columns.insert(column, next);
        }
    }
    pub fn remove(&mut self, key: SectionKey) {
        self.set_detail(key, false);
        if self.block_entities.remove(&key).is_some() {
            self.block_entity_revision = next_block_entity_revision();
        }
        self.occlusion.remove(key);
        if !self.lod_cutout.is_empty() && self.lod_cutout.contains(&(key.0, key.2)) {
            // Distant tiles still leave its column out: drawn until they don't.
            self.lod_retiring.insert(key);
        } else {
            self.release_section(key);
        }
    }
    /// Run `pack`'s stages over the world from now on (`None`: no pack, the
    /// vanilla frame). On error the previous pack stays.
    pub fn set_shader_pack(&mut self, pack: Option<&crate::shader_pack::ShaderPack>) -> Result<()> {
        let format = self.config.format.remove_srgb_suffix();
        self.pack = match pack {
            Some(pack) => Some(match &pack.java {
                Some(java) => Pack::Java(java_stages::JavaStages::new(
                    &self.device,
                    &self.queue,
                    format,
                    &pack.id,
                    java,
                )?),
                None => Pack::Horizon(pack_stages::PackStages::new(
                    &self.device,
                    &self.queue,
                    format,
                    pack,
                )?),
            }),
            None => None,
        };
        Ok(())
    }
    /// The folder name of the pack `set_shader_pack` gave.
    pub fn shader_pack_id(&self) -> Option<&str> {
        self.pack.as_ref().map(|p| match p {
            Pack::Horizon(p) => p.id.as_str(),
            Pack::Java(p) => p.id.as_str(),
        })
    }
    /// Horizon's resolution scale (world pixels per window pixel) and FXAA.
    pub fn set_world_post(&mut self, scale: f32, antialias: bool) {
        self.world_post.scale = scale.clamp(0.25, 2.0);
        self.world_post.antialias = antialias;
    }
    /// The Mipmap Levels option: block textures sample at most `levels`
    /// reduced copies (0: none).
    pub fn set_mipmap_levels(&mut self, levels: u32) {
        let levels = sprite_mipmap_limit(
            levels,
            self.sprite_textures[0].width(),
            self.sprite_minimum_size,
            self.mip_levels,
        );
        if levels == self.mipmaps {
            return;
        }
        self.mipmaps = levels;
        let views = sprite_views(&self.sprite_textures, &self.sprite_unused, Some(levels + 1));
        self.group = terrain_group(
            &self.device,
            "Horizon terrain resources",
            &self.terrain_layout,
            &self.globals,
            &views,
            &self.sprite_sampler,
            &self.lightmap_view,
            &self.lightmap_sampler,
        );
        self.shadow_group = terrain_group(
            &self.device,
            "Shader pack shadow terrain resources",
            &self.terrain_layout,
            &self.shadow_globals,
            &views,
            &self.sprite_sampler,
            &self.lightmap_view,
            &self.lightmap_sampler,
        );
    }
    pub fn view_projection(&self, frame: &Frame) -> Mat4 {
        self.projection(
            frame.fov_degrees,
            frame.yaw,
            frame.pitch,
            frame.bob,
            frame.depth_far,
        )
    }
    /// The view frustum `render` will cull with, for culling geometry built
    /// before the frame (entities, block entities) the same way.
    pub fn camera_frustum(
        &self,
        fov_degrees: f32,
        yaw: f32,
        pitch: f32,
        bob: Mat4,
        depth_far: f32,
    ) -> Frustum {
        Frustum {
            planes: frustum(self.projection(fov_degrees, yaw, pitch, bob, depth_far)),
        }
    }
    fn projection(&self, fov_degrees: f32, yaw: f32, pitch: f32, bob: Mat4, far: f32) -> Mat4 {
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        world_projection(fov_degrees, aspect, far) * bob * crate::camera::view_matrix(yaw, pitch)
    }
    fn update_lightmap(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        sky: &Sky,
        darkness: f32,
        gamma: f32,
        night_vision: f32,
        flicker: f32,
    ) {
        // lightmap.fsh: per (block, sky) level.
        let brightness = |level: f32| level / (4.0 - 3.0 * level);
        let mut pixels = [0u8; 16 * 16 * 4];
        for s in 0..16 {
            for b in 0..16 {
                let block_level = b as f32 / 15.0;
                let sky_level = s as f32 / 15.0;
                let block_brightness = brightness(block_level) * (1.4 + flicker);
                let sky_brightness = brightness(sky_level) * sky.sky_light_factor;
                // NightVisionColor defaults to 0x999999.
                let mut color =
                    Vec3::from(sky.ambient_light_color).max(Vec3::splat(0.6 * night_vision));
                color += Vec3::from(sky.sky_light_color) * sky_brightness;
                let parabolic = (2.0 * block_level - 1.0).powi(2);
                let block_color = Vec3::from(sky.block_light_tint).lerp(Vec3::ONE, 0.9 * parabolic);
                color += block_color * block_brightness;
                color -= Vec3::splat(darkness);
                color = color.clamp(Vec3::ZERO, Vec3::ONE);
                let max = color.max_element().max(1e-6);
                let inverted = 1.0 - max;
                let scaled = 1.0 - inverted.powi(4);
                let not_gamma = color * (scaled / max);
                color = color.lerp(not_gamma, gamma);
                let i = (s * 16 + b) * 4;
                pixels[i] = (color.x * 255.0) as u8;
                pixels[i + 1] = (color.y * 255.0) as u8;
                pixels[i + 2] = (color.z * 255.0) as u8;
                pixels[i + 3] = 255;
            }
        }
        if self.fullbright {
            pixels.fill(255);
        }
        // The lightmap changes with the time of day, flicker and effects,
        // not every frame: upload only what differs from the texture.
        if pixels == self.lightmap_pixels && self.lightmap_uploaded {
            return;
        }
        self.uploads.texture(
            encoder,
            wgpu::TexelCopyTextureInfo {
                texture: &self.lightmap,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            16,
            16,
            &pixels,
        );
        self.lightmap_pixels = pixels;
        self.lightmap_uploaded = true;
    }
    fn animate(&mut self, encoder: &mut wgpu::CommandEncoder, tick: u64) {
        self.animations.animate(encoder, &mut self.uploads, tick);
    }
    pub fn render(&mut self, frame: &Frame) -> Result<bool> {
        let rendered = self.draw_frame(frame);
        // A frame that was not drawn still submits the section copies.
        if let Some(copies) = self.take_section_copies() {
            self.queue.submit([copies]);
        }
        rendered
    }
    fn draw_frame(&mut self, frame: &Frame) -> Result<bool> {
        self.arena.trim();
        self.sorted_indices.trim();
        if let Some(flat) = &mut self.flat {
            flat.trim(|page| self.arena.has_page(page));
        }
        if self.suspended {
            return Ok(false);
        }
        let started = Instant::now();
        if self.first_frame == FirstFrame::Acquire {
            // Once: a minimized window acquires nothing for a while.
            startup_step(format_args!("acquiring the first frame"));
            self.first_frame = FirstFrame::Submit;
        }
        let acquire = Instant::now();
        let acquired = self.surface.as_ref().map(|s| s.get_current_texture());
        self.last_wait = acquire.elapsed();
        let (surface_frame, reconfigure) = match acquired {
            None => (None, false),
            Some(wgpu::CurrentSurfaceTexture::Success(f)) => (Some(f), false),
            Some(wgpu::CurrentSurfaceTexture::Suboptimal(f)) => (Some(f), true),
            Some(wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded) => {
                return Ok(false);
            }
            Some(wgpu::CurrentSurfaceTexture::Outdated) => {
                if let Some(surface) = &self.surface {
                    surface.configure(&self.device, &self.config);
                }
                return Ok(false);
            }
            Some(wgpu::CurrentSurfaceTexture::Lost) => {
                self.recreate_surface()?;
                return Ok(false);
            }
            Some(wgpu::CurrentSurfaceTexture::Validation) => {
                return Err("surface acquisition failed validation".into());
            }
        };
        let target_texture = match &surface_frame {
            Some(f) => f.texture.clone(),
            None => self.offscreen.clone().ok_or("no offscreen target")?,
        };
        // One command buffer for the frame, after the section copies staged
        // since the last one: Metal commits every command buffer wgpu-core
        // makes (one per run of copies and two per pass) on its own.
        let section_copies = self.section_copies.take();
        let has_section_copies = section_copies.is_some();
        let mut encoder = section_copies.unwrap_or_else(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Horizon frame"),
                })
        });
        if self.animated_textures {
            self.animate(&mut encoder, frame.ticks);
            self.hud
                .animate_fire(&mut self.uploads, &mut encoder, frame.ticks);
        }
        self.update_lightmap(
            &mut encoder,
            &frame.sky,
            frame.darkness.1,
            (self.gamma - frame.darkness.0).max(0.0),
            frame.night_vision,
            frame.block_flicker,
        );
        let view_proj = self.view_projection(frame);
        if frame.title {
            self.panorama.prepare(
                &self.queue,
                self.config.width,
                self.config.height,
                frame.yaw,
                frame.pitch,
            );
        }
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let view = crate::camera::view_matrix(frame.yaw, frame.pitch);
        let sun_angle = frame.sky.sun_angle.to_radians();
        // Celestial bodies rotate around the X axis; angle 0 = noon.
        let sun = Vec3::new(0.0, sun_angle.cos(), -sun_angle.sin());
        let (fog_color, fog) = if let Some(f) = frame.fog {
            (
                f.color,
                [
                    f.environmental_start,
                    f.environmental_end,
                    f.render_distance_start,
                    f.render_distance_end,
                ],
            )
        } else if frame.underwater {
            (
                [
                    frame.water_fog[0],
                    frame.water_fog[1],
                    frame.water_fog[2],
                    1.0,
                ],
                [-8.0, 96.0, -8.0, 96.0],
            )
        } else {
            let d = frame.distance;
            (
                [
                    frame.sky.fog_color[0],
                    frame.sky.fog_color[1],
                    frame.sky.fog_color[2],
                    1.0,
                ],
                // FogRenderer: render-distance fog starts at max(d - 16, 0)... d.
                {
                    let (start, end) = frame.environment_fog.unwrap_or((f32::MAX, f32::MAX));
                    [start, end, (d - d.min(64.0) * 0.25).max(0.0), d]
                },
            )
        };
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            fog_color,
            fog,
            sky_color: [
                frame.sky.sky_color[0],
                frame.sky.sky_color[1],
                frame.sky.sky_color[2],
                1.0,
            ],
            misc: [
                1.0 - frame.sky.sky_light_factor,
                frame.game_time,
                aspect,
                (frame.fov_degrees.to_radians() / 2.0).tan(),
            ],
            inverse_view: view.inverse().to_cols_array_2d(),
            sunrise: frame.sky.sunrise_color,
            sun: [sun.x, sun.y, sun.z, frame.sky.star_brightness],
            fog_ends: [
                frame.fog.map_or(frame.distance.min(512.0), |f| f.sky_end),
                frame.fog.map_or(2048.0, |f| f.cloud_end),
                if frame.dark_disc { 1.0 } else { 0.0 },
                0.0,
            ],
            end_portal_sprites: frame.end_portal_sprites,
            crumbling_sprite: frame.crumbling_sprite,
            time: [
                frame.shader_game_time,
                f32::from(frame.nether_lighting),
                frame.dynamic_lights.len().min(crate::dynamic_light::MAX) as f32,
                0.0,
            ],
            outline_params: outline_params(self.config.width, self.config.height),
            dynamic_lights: {
                let mut lights = [[0.0; 4]; crate::dynamic_light::MAX];
                for (to, from) in lights.iter_mut().zip(frame.dynamic_lights) {
                    *to = *from;
                }
                lights
            },
        };
        self.uploads
            .buffer(&mut encoder, &self.globals, 0, bytemuck::bytes_of(&globals));
        let pack_uniform = shader_pack_uniform(
            frame,
            view,
            world_projection(frame.fov_degrees, aspect, frame.depth_far) * frame.bob,
            sun,
            fog_color,
            globals.fog,
            globals.fog_ends,
        );
        // GameRenderer.render3dHud: the hand's projection at hudFov, with the
        // bob in its pose rather than the projection (same product).
        let hand_projection = Mat4::from_cols_array(&crate::camera::hand_projection(
            frame.hand_fov_degrees,
            self.config.width as f32,
            self.config.height.max(1) as f32,
            frame.depth_far,
        ));
        let hand_globals = Globals {
            view_proj: (hand_projection * frame.bob * view).to_cols_array_2d(),
            ..globals
        };
        self.uploads.buffer(
            &mut encoder,
            &self.hand_globals,
            0,
            bytemuck::bytes_of(&hand_globals),
        );
        // GameRenderer.render3dHud: DebugCrosshairRenderer draws with the
        // hand's projection but no bob, and FogMode.NONE (no fog colour).
        let crosshair_globals = Globals {
            view_proj: (hand_projection * view).to_cols_array_2d(),
            fog_color: [0.0; 4],
            ..globals
        };
        self.uploads.buffer(
            &mut encoder,
            &self.crosshair_globals,
            0,
            bytemuck::bytes_of(&crosshair_globals),
        );
        // Frustum culling and camera-relative origins.
        let view = Frustum {
            planes: frustum(view_proj),
        };
        let camera = frame.camera;
        let camera_section = camera_section(camera);
        let culling = !frame.title && frame.cave_culling;
        if culling {
            crate::spans::time("occlusion", || {
                self.start_occlusion(camera, frame.detail_distance)
            });
        }
        let reachable = &self.occlusion;
        let cutout = &self.lod_cutout;
        let mut visible = std::mem::take(&mut self.visible);
        visible.clear();
        crate::spans::time("visibility", || {
            self.sections.visible(
                camera,
                (frame.detail_distance / 16.0) as i32,
                &view,
                |k| !culling || reachable.contains(k),
                |x, z| !cutout.is_empty() && cutout.contains(&(x, z)),
                &mut visible,
            )
        });
        let mut visible_lods = std::mem::take(&mut self.visible_lods);
        visible_lods.clear();
        if frame.title {
            visible.clear();
        } else {
            visible_lods.extend(
                self.lods
                    .iter()
                    .filter_map(|(&key, tile)| tile.visible(key, camera, frame.distance, &view)),
            );
            visible_lods.sort_unstable_by_key(|v| (v.distance, v.key));
        }
        self.drawn_sections = visible.len() + visible_lods.len();
        if self.drawn_sections as u64 > self.instance_capacity {
            self.instance_capacity = (self.drawn_sections as u64).next_power_of_two();
            self.instances = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Section origins"),
                size: self.instance_capacity * 16,
                usage: wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
        }
        self.origins.clear();
        self.origins.extend(
            visible
                .iter()
                .map(|v| section_instance(v.origin))
                .chain(visible_lods.iter().map(|v| lod::instance(v.origin))),
        );
        if !self.origins.is_empty() {
            self.uploads.buffer(
                &mut encoder,
                &self.instances,
                0,
                bytemuck::cast_slice(&self.origins),
            );
        }
        // Solid sections front to back, then translucent ones back to front,
        // as one list of indirect draws over the section arena.
        self.draws.clear();
        self.draw_pages.clear();
        // The eye, camera-relative: view bobbing moves it off the camera.
        let eye = (frame.bob * crate::camera::view_matrix(frame.yaw, frame.pitch))
            .inverse()
            .transform_point3(Vec3::ZERO);
        // Occlusion culling leaves the sections its last result did not see
        // to phase 2, after the others, with instance counts of 0. Whether
        // the frame has draws enough goes by the last one's.
        let mut cull = self
            .cull
            .as_mut()
            .filter(|c| {
                self.cull_on && !frame.title && self.multi_draw && self.section_draws >= c.min_draws
            })
            .and_then(|cull| cull.begin(&self.device).then_some(cull));
        self.phase2_draws.clear();
        self.phase2_pages.clear();
        self.solid_runs.clear();
        self.phase2_runs.clear();
        for (i, v) in visible.iter().enumerate() {
            let s = self.sections.get(v.slot);
            let first_phase = cull.as_ref().is_none_or(|c| c.first(v.slot));
            let (draws, pages) = if first_phase {
                (&mut self.draws, &mut self.draw_pages)
            } else {
                (&mut self.phase2_draws, &mut self.phase2_pages)
            };
            let start = draws.len();
            let instances = u32::from(first_phase);
            let mut indices = 0;
            // Only the solid groups that can face the eye.
            s.faces.ranges(eye - Vec3::from(v.origin), |first, quads| {
                let first = s.start + u64::from(first) * 4;
                draws.push([quads * 6, instances, 0, first as u32, i as u32]);
                pages.push((s.page, None));
                indices += quads * 6;
            });
            let runs = if first_phase {
                &mut self.solid_runs
            } else {
                &mut self.phase2_runs
            };
            runs.push(
                v.distance,
                s.page,
                start,
                draws.len() - start,
                [i as u32, v.slot, indices],
            );
            if let Some(c) = &mut cull
                && first_phase
            {
                c.drawn(i, v.slot, indices);
            }
        }
        self.solid_runs
            .regroup(&mut self.draws, &mut self.draw_pages, 0);
        // Phase 2's draws are tested where grouping left them.
        let runs = self
            .phase2_runs
            .regroup(&mut self.phase2_draws, &mut self.phase2_pages, 0);
        if let Some(c) = &mut cull {
            for r in runs {
                let [i, slot, indices] = r.tag;
                c.deferred(i as usize, slot, r.start as usize, r.len as usize, indices);
            }
        }
        // Distant terrain tiles are never occlusion tested: phase 1.
        let tiles = self.draws.len();
        self.solid_runs.clear();
        for (i, v) in visible_lods.iter().enumerate() {
            let s = &self.lods[&v.key].mesh;
            let start = self.draws.len();
            s.faces.ranges(eye - Vec3::from(v.origin), |first, quads| {
                let first = s.start + u64::from(first) * 4;
                self.draws
                    .push([quads * 6, 1, 0, first as u32, (visible.len() + i) as u32]);
                self.draw_pages.push((s.page, None));
            });
            self.solid_runs
                .push(v.distance, s.page, start, self.draws.len() - start, [0; 3]);
        }
        self.solid_runs
            .regroup(&mut self.draws, &mut self.draw_pages, tiles);
        let phase1_draws = self.draws.len() as u32;
        self.draws.extend_from_slice(&self.phase2_draws);
        self.draw_pages.extend_from_slice(&self.phase2_pages);
        let culling = cull.is_some();
        let solid_draws = self.draws.len() as u32;
        // LevelRenderer: nearby sections each frame, plus a rotating eighth
        // (at least 15) of the visible list. Cached views avoid needless sorts.
        self.translucent_draws.clear();
        let count = visible.len() + visible_lods.len();
        let budget = (count / 8).max(15).min(count);
        self.translucency_cursor %= count.max(1);
        for (i, v) in visible.iter().enumerate().rev() {
            let s = self.sections.get_mut(v.slot);
            if let Some((index_page, index_start)) = s.sorted_indices {
                let nearby = (v.key.0 - camera_section.0).abs() <= 2
                    && (v.key.1 - camera_section.1).abs() <= 2
                    && (v.key.2 - camera_section.2).abs() <= 2;
                let scheduled = (i + count - self.translucency_cursor) % count < budget;
                let origin = [
                    v.key.0 * 16,
                    crate::chunk::MIN_Y + v.key.1 * 16,
                    v.key.2 * 16,
                ];
                if (!s.sort.is_sorted() || nearby || scheduled)
                    && let Some(indices) =
                        s.sort
                            .update(camera.to_array(), origin, &mut self.translucent_sorter)
                {
                    self.uploads.buffer(
                        &mut encoder,
                        self.sorted_indices.buffer(index_page),
                        index_start * 4,
                        bytemuck::cast_slice(indices),
                    );
                }
                let first = s.start + u64::from(s.solid) * 4;
                self.translucent_draws.push((
                    v.distance,
                    i,
                    [
                        s.translucent * 6,
                        1,
                        index_start as u32,
                        first as u32,
                        i as u32,
                    ],
                    (s.page, Some(index_page)),
                ));
            }
        }
        // Sections went back to front (`visible` is sorted by distance, then
        // key): tiles' water only needs sorting in when there is some.
        let section_translucent = self.translucent_draws.len();
        for (i, v) in visible_lods.iter().enumerate() {
            let instance = visible.len() + i;
            let tile = self.lods.get_mut(&v.key).expect("visible resident tile");
            let s = &mut tile.mesh;
            if let Some((index_page, index_start)) = s.sorted_indices {
                let scheduled = (instance + count - self.translucency_cursor) % count < budget;
                if (!s.sort.is_sorted() || scheduled)
                    && let Some(indices) = s.sort.update_lod(
                        camera.to_array(),
                        v.key.origin(),
                        &mut self.translucent_sorter,
                    )
                {
                    self.uploads.buffer(
                        &mut encoder,
                        self.sorted_indices.buffer(index_page),
                        index_start * 4,
                        bytemuck::cast_slice(indices),
                    );
                }
                let first = s.start + u64::from(s.solid) * 4;
                self.translucent_draws.push((
                    v.distance,
                    instance,
                    [
                        s.translucent * 6,
                        1,
                        index_start as u32,
                        first as u32,
                        instance as u32,
                    ],
                    (s.page, Some(index_page)),
                ));
            }
        }
        // Near water and distant water share one back-to-front draw order.
        if self.translucent_draws.len() > section_translucent {
            self.translucent_draws
                .sort_unstable_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        }
        for &(_, _, draw, pages) in &self.translucent_draws {
            self.draws.push(draw);
            self.draw_pages.push(pages);
        }
        self.translucency_cursor += budget;
        let translucent_draws = self.draws.len() as u32 - solid_draws;
        self.section_draws = self.draws.len();
        // Metal replays a multi-draw as one draw call per entry.
        self.section_draw_calls = if self.multi_draw && self.info.backend != wgpu::Backend::Metal {
            (0..self.draws.len())
                .filter(|&i| {
                    i == 0
                        || i == phase1_draws as usize
                        || i == solid_draws as usize
                        || self.draw_pages[i] != self.draw_pages[i - 1]
                })
                .count()
        } else {
            self.draws.len()
        };
        let debug_depth = frame.debug_lines.len() as u32;
        let debug_top = frame.debug_lines_on_top.len() as u32;
        let crosshair = frame.debug_crosshair.len() as u32;
        if debug_depth + debug_top + crosshair > 0 {
            let needed = u64::from(debug_depth + debug_top + crosshair);
            if needed > self.debug_line_capacity {
                self.debug_line_capacity = needed.next_power_of_two();
                self.debug_line_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Debug lines"),
                    size: 32 * self.debug_line_capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            let all: Vec<[f32; 8]> = frame
                .debug_lines
                .iter()
                .chain(frame.debug_lines_on_top)
                .chain(frame.debug_crosshair)
                .copied()
                .collect();
            self.uploads.buffer(
                &mut encoder,
                &self.debug_line_buffer,
                0,
                bytemuck::cast_slice(&all),
            );
        }
        self.weather.prepare(
            &self.device,
            &mut encoder,
            &mut self.uploads,
            frame.weather,
            frame.weather_time,
        );
        let outline_edges = frame.outline.len().min(MAX_OUTLINE_EDGES as usize);
        if outline_edges > 0 {
            let lines = outline_vertices(&frame.outline[..outline_edges], camera);
            self.uploads
                .buffer(&mut encoder, &self.outline, 0, bytemuck::cast_slice(&lines));
        }
        let [
            entity_quads,
            crumbling_quads,
            translucent_crumbling_quads,
            water_mask_quads,
        ] = self.entity_batch.update(
            frame.entities,
            frame.additive,
            frame.hand,
            frame.see_through,
        );
        let weather_quads = frame.weather.len() as u32 / 4;
        let world_quads =
            entity_quads + crumbling_quads + translucent_crumbling_quads + water_mask_quads;
        let cloud_quads = if let Some(input) = &frame.clouds {
            let (rebuilt, uniform) =
                crate::spans::time("clouds", || self.cloud_mesh.prepare(input));
            let vertices = self.cloud_mesh.vertices.len() as u64;
            if rebuilt && vertices > 0 {
                if vertices > self.cloud_capacity {
                    self.cloud_capacity = vertices.next_power_of_two();
                    self.cloud_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Cloud vertices"),
                        size: self.cloud_capacity * std::mem::size_of::<Vertex>() as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                }
                self.uploads.buffer(
                    &mut encoder,
                    &self.cloud_buffer,
                    0,
                    bytemuck::cast_slice(&self.cloud_mesh.vertices),
                );
            }
            self.uploads.buffer(
                &mut encoder,
                &self.cloud_uniform,
                0,
                bytemuck::bytes_of(&uniform),
            );
            (vertices / 4) as u32
        } else {
            0
        };
        self.lightning.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            &frame.lightning,
            camera,
        );
        let all_entities = &self.entity_batch.combined;
        let hand_quads = (frame.hand.len() / 4) as u32;
        let additive_quads = (frame.additive.len() / 4) as u32;
        let see_through_quads = (frame.see_through.len() / 4) as u32;
        let entity_vertices = all_entities.len() as u64;
        if entity_vertices > self.entity_capacity {
            self.entity_capacity = entity_vertices.next_power_of_two();
            self.entity_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Entity vertices"),
                size: self.entity_capacity * std::mem::size_of::<Vertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if entity_vertices > 0 {
            self.uploads.buffer(
                &mut encoder,
                &self.entity_buffer,
                0,
                bytemuck::cast_slice(all_entities),
            );
        }
        if let Some((rotation, brightness)) = frame.stars {
            self.stars.prepare(
                &mut self.uploads,
                &mut encoder,
                view_proj * rotation,
                brightness,
            );
        }
        let celestial_vertices = frame.celestial.len().min(2048 * 4);
        if frame.end_sky && !frame.title {
            self.end_sky.prepare(&self.device, frame.end_sky_sprite);
        }
        if celestial_vertices > 0 {
            self.uploads.buffer(
                &mut encoder,
                &self.celestial_buffer,
                0,
                bytemuck::cast_slice(&frame.celestial[..celestial_vertices]),
            );
        }
        let max_quads = (world_quads + additive_quads + hand_quads + see_through_quads)
            .max(cloud_quads)
            .max(if frame.lightning.bolts.is_empty() {
                0
            } else {
                lightning::QUADS
            })
            .max(weather_quads);
        if max_quads > self.index_quads {
            self.index_quads = max_quads.next_power_of_two();
            self.indices = Self::quad_indices(&self.device, self.index_quads);
        }
        if let Some(b) = &frame.border {
            if b.rebuilt {
                self.uploads.buffer(
                    &mut encoder,
                    &self.border_vertices,
                    0,
                    bytemuck::cast_slice(&b.vertices),
                );
            }
            let t = &b.transforms;
            let uniform = BorderUniform {
                color: t.color,
                offset: [
                    t.model_offset[0],
                    t.model_offset[1],
                    t.model_offset[2],
                    t.texture_offset,
                ],
            };
            self.uploads.buffer(
                &mut encoder,
                &self.border_uniform,
                0,
                bytemuck::bytes_of(&uniform),
            );
        }
        // Every upload before the frame's first pass: a pass ends wgpu-core's
        // command buffer, so copies between passes would need one more.
        if !frame.screen.is_empty() {
            let vertices: Vec<crate::screen_effects::ScreenVertex> =
                frame.screen.iter().flat_map(|q| q.vertices).collect();
            if vertices.len() as u64 > self.screen_capacity {
                self.screen_capacity = (vertices.len() as u64).next_power_of_two();
                self.screen_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Screen effect vertices"),
                    size: self.screen_capacity
                        * std::mem::size_of::<crate::screen_effects::ScreenVertex>() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            self.uploads.buffer(
                &mut encoder,
                &self.screen_buffer,
                0,
                bytemuck::cast_slice(&vertices),
            );
        }
        self.hud.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            frame.hud,
            [self.config.width, self.config.height],
        );
        if culling && let Some(cull) = &mut self.cull {
            cull.upload(
                &self.device,
                &mut self.uploads,
                &mut encoder,
                &self.draws[phase1_draws as usize..solid_draws as usize],
                &self.instances,
                &self.globals,
            );
        }
        let flat_frame = match &mut self.flat {
            Some(flat)
                if self.flat_on && !self.draws.is_empty() && self.draws.len() >= flat.min_draws =>
            {
                let sections = &self.sections;
                let lods = &self.lods;
                let sorted = &self.sorted_indices;
                let phase2 = self.cull.as_ref().filter(|_| culling).map(|c| &c.phase2);
                crate::spans::time("flatten", || {
                    flat.prepare(
                        &self.device,
                        &mut encoder,
                        &mut self.uploads,
                        &self.draws,
                        &self.draw_pages,
                        phase1_draws as usize,
                        solid_draws as usize,
                        phase2,
                        visible
                            .iter()
                            .filter_map(|v| {
                                let id = sections.get(v.slot).flat_id?;
                                Some((id, section_instance(v.origin)))
                            })
                            .chain(visible_lods.iter().filter_map(|v| {
                                let id = lods[&v.key].mesh.flat_id?;
                                Some((id, lod::instance(v.origin)))
                            })),
                        |page| sorted.buffer(page).clone(),
                    )
                })
            }
            _ => false,
        };
        if flat_frame {
            self.section_draw_calls = self.flat.as_ref().map_or(0, |f| f.calls.len());
        } else if self.multi_draw && !self.draws.is_empty() {
            if self.draws.len() as u64 > self.indirect_capacity {
                self.indirect_capacity = (self.draws.len() as u64).next_power_of_two();
                self.indirect = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Section draws"),
                    size: self.indirect_capacity * 20,
                    usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            self.uploads.buffer(
                &mut encoder,
                &self.indirect,
                0,
                bytemuck::cast_slice(&self.draws),
            );
        }
        let view_target = target_texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.config.format.remove_srgb_suffix()),
            ..Default::default()
        });
        if let Some(timer) = &mut self.gpu_timer {
            timer.begin(&mut encoder);
        }
        // The world's targets: the screen, or the scaled target of `post`.
        let pack = self.pack.is_some() && !frame.title;
        // What the pack reads beyond the world's colour and depth: each costs
        // a pass (the shadow map, the G-buffer) or a copy (the opaque depth).
        let (shadow_cfg, gbuffer, opaque_depth) = match self.pack.as_ref().filter(|_| pack) {
            Some(Pack::Horizon(s)) => (
                s.needs_shadow.then_some((s.shadow_res, s.shadow_dist)),
                s.needs_gbuffer,
                s.needs_opaque_depth,
            ),
            Some(Pack::Java(s)) => (None, false, s.reads_opaque_depth),
            None => (None, false, false),
        };
        if let Some((shadow_res, shadow_dist)) = shadow_cfg {
            self.world_post
                .prepare_shadow(&self.device, shadow_res, shadow_dist, sun, -sun);
            let shadow_globals = Globals {
                view_proj: (self.world_post.shadow_proj * self.world_post.shadow_view)
                    .to_cols_array_2d(),
                ..globals
            };
            self.uploads.buffer(
                &mut encoder,
                &self.shadow_globals,
                0,
                bytemuck::bytes_of(&shadow_globals),
            );
        } else {
            self.world_post.shadow_active = false;
        }
        let scaled = self.world_post.prepare(
            &self.device,
            [self.config.width, self.config.height],
            pack,
            world_post::Extras {
                opaque_depth,
                gbuffer,
            },
        );
        let (world_view, world_depth) = match &scaled {
            Some((color, depth)) => (color, depth),
            None => (&view_target, &self.depth),
        };
        if culling && let Some(cull) = &mut self.cull {
            cull.bind_depth(&self.device, world_depth);
        }
        let cull = self.cull.as_ref().filter(|_| culling && solid_draws > 0);
        if shadow_cfg.is_some() {
            let shadow_view = self.world_post.shadow_depth().clone();
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Horizon shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if solid_draws > 0 && !flat_frame {
                shadow_pass.set_bind_group(0, &self.shadow_group, &[]);
                shadow_pass.set_pipeline(&self.shadow_pipeline);
                shadow_pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                shadow_pass.set_vertex_buffer(1, self.instances.slice(..));
                self.draw_sections(&mut shadow_pass, &self.indirect, 0, 0, solid_draws);
            }
            if entity_quads > 0 {
                shadow_pass.set_pipeline(&self.shadow_entity_pipeline);
                shadow_pass.set_bind_group(0, &self.shadow_entity_group, &[]);
                shadow_pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                shadow_pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                shadow_pass.draw_indexed(0..entity_quads * 6, 0, 0..1);
            }
        }
        {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: world_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(
                        if frame.title || (frame.end_sky && !frame.underwater) {
                            wgpu::Color::BLACK
                        } else {
                            wgpu::Color {
                                r: f64::from(fog_color[0]),
                                g: f64::from(fog_color[1]),
                                b: f64::from(fog_color[2]),
                                a: 1.0,
                            }
                        },
                    ),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Horizon world"),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: world_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        // A shader pack reads the world's depth, and
                        // phase 2 of occlusion culling draws over it.
                        store: if pack || cull.is_some() {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.group, &[]);
            // LevelRenderer: no sky inside lava or powder snow (sky fog ends within 2 blocks).
            let sky_visible = match frame.fog {
                Some(f) => f.sky_end > 2.0,
                None => !frame.underwater,
            };
            if frame.title || (sky_visible && (frame.skybox || frame.end_sky)) {
                if !frame.title && frame.skybox {
                    pass.set_pipeline(&self.sky);
                    pass.draw(0..3, 0..1);
                }
                if frame.title {
                    self.panorama.draw(&mut pass);
                    pass.set_bind_group(0, &self.group, &[]);
                } else if celestial_vertices > 0 || frame.end_sky {
                    pass.set_pipeline(&self.celestial_pipeline);
                    pass.set_bind_group(0, &self.entity_group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    if frame.end_sky {
                        self.end_sky.draw(&mut pass);
                    }
                    if celestial_vertices > 0 {
                        pass.set_vertex_buffer(0, self.celestial_buffer.slice(..));
                        pass.draw_indexed(0..(celestial_vertices as u32 / 4) * 6, 0, 0..1);
                    }
                    pass.set_bind_group(0, &self.group, &[]);
                }
                if !frame.title && frame.stars.is_some() {
                    self.stars.draw(&mut pass);
                    pass.set_bind_group(0, &self.group, &[]);
                }
                if !frame.title && frame.dark_disc {
                    pass.set_pipeline(&self.dark_disc);
                    pass.draw(0..3, 0..1);
                }
            }
            match self.flat.as_ref().filter(|_| flat_frame) {
                Some(flat) => {
                    pass.set_pipeline(&flat.solid);
                    let vertices = |p| self.arena.buffer(p);
                    match cull {
                        // As below: phase 1, the test, then phase 2's draws
                        // that the test saw, copied after it.
                        Some(cull) => {
                            flat.draw(&mut pass, &flat.calls[..flat.phase1_calls], vertices);
                            cull.test(&mut pass);
                            drop(pass);
                            cull.test_pass(&mut encoder);
                            flat.flatten_phase2(&mut encoder);
                            pass = world_pass_after_culling(
                                &mut encoder,
                                world_view,
                                world_depth,
                                pack,
                            );
                            pass.set_bind_group(0, &self.group, &[]);
                            pass.set_pipeline(&flat.solid);
                            flat.draw_phase2(&mut pass, vertices);
                        }
                        None => flat.draw(&mut pass, &flat.calls[..flat.solid_calls], vertices),
                    }
                    pass.set_bind_group(0, &self.group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
                None => {
                    pass.set_pipeline(&self.solid);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.set_vertex_buffer(1, self.instances.slice(..));
                    match cull {
                        Some(cull) => {
                            // Phase 1 and the boxes against its depth (in
                            // its pass, or one of their own on Metal), then
                            // what they found in sight, in a pass that loads
                            // both targets (it reads the test's results).
                            self.draw_sections(&mut pass, &self.indirect, 0, 0, phase1_draws);
                            cull.test(&mut pass);
                            drop(pass);
                            cull.test_pass(&mut encoder);
                            pass = world_pass_after_culling(
                                &mut encoder,
                                world_view,
                                world_depth,
                                pack,
                            );
                            pass.set_bind_group(0, &self.group, &[]);
                            pass.set_pipeline(&self.solid);
                            pass.set_index_buffer(
                                self.indices.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.set_vertex_buffer(1, self.instances.slice(..));
                            self.draw_sections(
                                &mut pass,
                                &cull.phase2,
                                phase1_draws,
                                phase1_draws,
                                solid_draws - phase1_draws,
                            );
                        }
                        None => self.draw_sections(&mut pass, &self.indirect, 0, 0, solid_draws),
                    }
                }
            }
            if entity_quads > 0 {
                pass.set_pipeline(&self.entity_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(0..entity_quads * 6, 0, 0..1);
                pass.set_bind_group(0, &self.group, &[]);
            }
            self.weather.draw(&mut pass, &self.entity_group);
            pass.set_bind_group(0, &self.group, &[]);
            if crumbling_quads > 0 {
                pass.set_pipeline(&self.crumbling_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(0..crumbling_quads * 6, (entity_quads * 4) as i32, 0..1);
                pass.set_bind_group(0, &self.group, &[]);
            }
            // BoatRenderer's water patch (RenderTypes.waterMask): depth only,
            // before translucent terrain, so no water shows inside the boat.
            if water_mask_quads > 0 {
                pass.set_pipeline(&self.water_mask_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..water_mask_quads * 6,
                    ((entity_quads + crumbling_quads + translucent_crumbling_quads) * 4) as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            if !frame.lightning.bolts.is_empty() {
                pass.set_pipeline(&self.lightning_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                self.lightning.draw(&mut pass, &frame.lightning);
                pass.set_bind_group(0, &self.group, &[]);
            }
            if additive_quads > 0 {
                pass.set_pipeline(&self.additive_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(0..additive_quads * 6, (world_quads * 4) as i32, 0..1);
                pass.set_bind_group(0, &self.group, &[]);
            }
            if outline_edges > 0 && !frame.outline_after_terrain {
                pass.set_pipeline(&self.lines);
                pass.set_vertex_buffer(0, self.outline.slice(..));
                pass.draw(0..4, 0..outline_edges as u32);
            }
            if debug_depth > 0 {
                pass.set_pipeline(&self.debug_lines);
                pass.set_vertex_buffer(0, self.debug_line_buffer.slice(..));
                pass.draw(0..4, 0..debug_depth);
            }
            if debug_top > 0 {
                pass.set_pipeline(&self.debug_lines_on_top);
                pass.set_vertex_buffer(0, self.debug_line_buffer.slice(..));
                pass.draw(0..4, debug_depth..debug_depth + debug_top);
            }
            if opaque_depth {
                drop(pass);
                self.world_post.capture_opaque_depth(&mut encoder);
                let attachments = [Some(wgpu::RenderPassColorAttachment {
                    view: world_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })];
                pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Horizon world, translucent"),
                    color_attachments: &attachments,
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: world_depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_bind_group(0, &self.group, &[]);
            }
            match self.flat.as_ref().filter(|_| flat_frame) {
                Some(flat) => {
                    pass.set_pipeline(&flat.translucent);
                    flat.draw(&mut pass, &flat.calls[flat.solid_calls..], |p| {
                        self.arena.buffer(p)
                    });
                    pass.set_bind_group(0, &self.group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
                None => {
                    pass.set_pipeline(&self.translucent);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.set_vertex_buffer(1, self.instances.slice(..));
                    self.draw_sections(
                        &mut pass,
                        &self.indirect,
                        0,
                        solid_draws,
                        translucent_draws,
                    );
                }
            }
            // LevelRenderer.executeClassicTransparency: translucent targets'
            // outlines follow terrain, so its blending cannot wash them out.
            if outline_edges > 0 && frame.outline_after_terrain {
                pass.set_pipeline(&self.lines);
                pass.set_vertex_buffer(0, self.outline.slice(..));
                pass.draw(0..4, 0..outline_edges as u32);
            }
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            // LevelRenderer: the world border after translucent terrain and weather.
            if let Some(b) = frame.border.as_ref().filter(|_| !frame.title) {
                pass.set_pipeline(&self.border_pipeline);
                pass.set_bind_group(0, &self.border_group, &[]);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, self.border_vertices.slice(..));
                for &side in &b.sides {
                    let first = side as u32 * 6;
                    pass.draw_indexed(first..first + 6, 0, 0..1);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            if translucent_crumbling_quads > 0 {
                pass.set_pipeline(&self.crumbling_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..translucent_crumbling_quads * 6,
                    ((entity_quads + crumbling_quads) * 4) as i32,
                    0..1,
                );
            }
            if cloud_quads > 0 {
                pass.set_pipeline(&self.cloud_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_bind_group(1, &self.cloud_group, &[]);
                pass.set_vertex_buffer(0, self.cloud_buffer.slice(..));
                pass.draw_indexed(0..cloud_quads * 6, 0, 0..1);
            }
            // LevelRenderer.executeSeeThrough, after the rest of the world.
            if see_through_quads > 0 {
                pass.set_pipeline(&self.see_through_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..see_through_quads * 6,
                    ((world_quads + additive_quads + hand_quads) * 4) as i32,
                    0..1,
                );
            }
        }
        if gbuffer && scaled.is_some() {
            self.world_post.build_gbuffer(
                &self.queue,
                &mut encoder,
                &pack_uniform,
                frame.shader.eye_light,
                frame.shader.rain,
            );
        }
        if let (true, Some((color, depth)), Some(size), Some(stages)) =
            (pack, &scaled, self.world_post.size(), self.pack.as_mut())
        {
            let output = match stages {
                Pack::Horizon(stages) => stages.run(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    color,
                    depth,
                    size,
                    &self.world_post,
                    pack_uniform,
                    frame.shader.eye_light,
                ),
                Pack::Java(stages) => stages.run(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    color,
                    depth,
                    size,
                    &self.world_post,
                    pack_uniform,
                    java_stages::JavaFrame {
                        eye_light: frame.shader.eye_light,
                        sun_angle: frame.sky.sun_angle,
                        moon_angle: frame.sky.moon_angle,
                    },
                ),
            };
            self.world_post
                .finish_with(&mut encoder, &view_target, output);
        } else if scaled.is_some() {
            self.world_post.finish(&mut encoder, &view_target);
        }
        if hand_quads > 0 {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &view_target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("First-person hand"),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.entity_pipeline);
            pass.set_bind_group(0, &self.hand_group, &[]);
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
            pass.draw_indexed(
                0..hand_quads * 6,
                ((world_quads + additive_quads) * 4) as i32,
                0..1,
            );
        }
        if !frame.screen.is_empty() {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &view_target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Screen effects"),
                color_attachments: &attachments,
                ..Default::default()
            });
            pass.set_bind_group(0, &self.entity_group, &[]);
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_vertex_buffer(0, self.screen_buffer.slice(..));
            for (i, quad) in frame.screen.iter().enumerate() {
                let blend = match quad.blend {
                    crate::screen_effects::Blend::Alpha => 0,
                    crate::screen_effects::Blend::Vignette => 4,
                };
                pass.set_pipeline(&self.screen_pipelines[blend + (quad.sampling & 3) as usize]);
                pass.draw_indexed(0..6, (i * 4) as i32, 0..1);
            }
        }
        // GameRenderer.render3dHud: the debug crosshair after the hand and
        // the screen effects.
        if crosshair > 0 {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &view_target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d crosshair"),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.debug_lines_on_top);
            pass.set_bind_group(0, &self.crosshair_group, &[]);
            pass.set_vertex_buffer(0, self.debug_line_buffer.slice(..));
            let first = debug_depth + debug_top;
            pass.draw(0..4, first..first + crosshair);
        }
        let menu_blur = frame
            .menu_blur
            .filter(|_| self.config.usage.contains(wgpu::TextureUsages::COPY_SRC));
        let blur_at = menu_blur.map_or(0, |_| frame.hud.blur_at);
        self.hud.draw_range(
            &mut encoder,
            &view_target,
            &self.depth,
            frame.hud,
            [self.config.width, self.config.height],
            0..blur_at,
        );
        if let Some(radius) = menu_blur {
            let format = self.config.format;
            let device = &self.device;
            self.menu_blur
                .get_or_insert_with(|| crate::menu_blur::MenuBlur::new(device, format))
                .apply(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &target_texture,
                    &view_target,
                    [self.config.width, self.config.height],
                    radius,
                );
        }
        self.hud.draw_range(
            &mut encoder,
            &view_target,
            &self.depth,
            frame.hud,
            [self.config.width, self.config.height],
            blur_at..frame.hud.vertices.len() as u32,
        );
        if culling && let Some(cull) = &mut self.cull {
            cull.end(&self.device, &mut encoder);
        }
        if let Some(timer) = &mut self.gpu_timer {
            timer.end(
                &self.device,
                &mut encoder,
                (&view_target, self.config.format.remove_srgb_suffix()),
            );
        }
        let readback = self.screenshot.take().and_then(|path| {
            // The copy and PNG writer both assume 8-bit pixels; a float surface would make
            // the row stride too small for the copy and abort in validation.
            if self.config.format.block_copy_size(None) == Some(4) {
                Some((path, self.copy_frame(&mut encoder, &target_texture)))
            } else {
                self.screenshot_result = Some(Err(format!(
                    "screenshots need a 32-bit surface format, not {:?}",
                    self.config.format
                )));
                None
            }
        });
        self.uploads.finish(&encoder);
        if has_section_copies {
            self.section_copy_bytes = 0;
            self.section_uploads.finish(&encoder);
        }
        let commands = crate::spans::time("encode", || encoder.finish());
        let first_frame = self.first_frame == FirstFrame::Submit;
        if first_frame {
            startup_step(format_args!("submitting the first frame"));
            self.first_frame = FirstFrame::Drawn;
        }
        let submission = crate::spans::time("submit", || self.queue.submit([commands]));
        if let Some(cull) = &mut self.cull {
            cull.submitted();
        }
        if let Some(timer) = &mut self.gpu_timer {
            timer.submitted(&self.queue);
            timer.collect(&self.device, &mut self.gpu_times);
        }
        if let Some((path, buffer)) = readback {
            self.screenshot_result = Some(
                self.write_screenshot(&path, &buffer)
                    .map_err(|e| e.to_string()),
            );
        }
        let present = Instant::now();
        if let Some(surface_frame) = surface_frame {
            // On Wayland this ties RedrawRequested to the compositor's frame
            // callbacks, one per refresh. Only VSync (Fifo) waits for the
            // display: with it off, vanilla's window draws uncapped.
            if self.config.present_mode == wgpu::PresentMode::Fifo {
                self.window.pre_present_notify();
            }
            if first_frame {
                startup_step(format_args!("presenting the first frame"));
            }
            self.queue.present(surface_frame);
            if reconfigure && let Some(surface) = &self.surface {
                surface.configure(&self.device, &self.config);
            }
        } else {
            // No swapchain paces offscreen frames: keep two on the GPU at most.
            self.in_flight.push_back(submission);
            while self.in_flight.len() > 2 {
                let index = self.in_flight.pop_front();
                let _ = self.device.poll(wgpu::PollType::Wait {
                    submission_index: index,
                    timeout: Some(std::time::Duration::from_secs(5)),
                });
            }
        }
        self.last_wait += present.elapsed();
        self.last_encode = started.elapsed().saturating_sub(self.last_wait);
        let _ = self.started;
        self.visible = visible;
        self.visible_lods = visible_lods;
        Ok(true)
    }
}
impl Renderer {
    fn padded_row(&self) -> u32 {
        (self.config.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
    }
    fn copy_frame(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon screenshot"),
            size: u64::from(self.padded_row()) * u64::from(self.config.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row()),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            },
        );
        buffer
    }
    fn write_screenshot(&self, path: &std::path::Path, buffer: &wgpu::Buffer) -> Result<[u32; 2]> {
        if !self.config.usage.contains(wgpu::TextureUsages::COPY_SRC) {
            return Err("this surface does not support frame readback".into());
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = sender.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| e.to_string())?;
        receiver
            .recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let (width, height, row) = (
            self.config.width,
            self.config.height,
            self.padded_row() as usize,
        );
        let bgra = matches!(
            self.config.format.remove_srgb_suffix(),
            wgpu::TextureFormat::Bgra8Unorm
        );
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        {
            let data = buffer.get_mapped_range(..).map_err(|e| e.to_string())?;
            for y in 0..height as usize {
                for px in data[y * row..y * row + width as usize * 4]
                    .as_chunks::<4>()
                    .0
                {
                    let (r, b) = if bgra { (px[2], px[0]) } else { (px[0], px[2]) };
                    pixels.extend_from_slice(&[r, px[1], b, 255]);
                }
            }
        }
        buffer.unmap();
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(std::fs::File::create(path)?),
            width,
            height,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&pixels)?;
        Ok([width, height])
    }
}
/// Camera-relative clip planes (`Frustum`).
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    planes: [Vec4; 6],
}
impl Frustum {
    /// Whether the camera-relative box `min..max` is at least partly inside.
    pub fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        !self.planes.iter().any(|p| {
            let v = Vec3::new(
                if p.x >= 0.0 { max.x } else { min.x },
                if p.y >= 0.0 { max.y } else { min.y },
                if p.z >= 0.0 { max.z } else { min.z },
            );
            p.truncate().dot(v) + p.w < 0.0
        })
    }
    /// Whether the camera-relative box `min..max` is entirely inside. Then
    /// `intersects` accepts every box within it: each plane's distance only
    /// grows from this box's nearest corner to the inner box's farthest, and
    /// rounding keeps that order.
    pub fn contains(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|p| {
            let v = Vec3::new(
                if p.x >= 0.0 { min.x } else { max.x },
                if p.y >= 0.0 { min.y } else { max.y },
                if p.z >= 0.0 { min.z } else { max.z },
            );
            p.truncate().dot(v) + p.w >= 0.0
        })
    }
}
/// Vanilla's `Projection`: reversed depth (near maps to 1, far to 0, tests
/// are GREATER_THAN_OR_EQUAL), which keeps far faces a hair apart, like
/// fire_side's 0.01 pixel in front of its block, from z-fighting.
/// The prelude's `Frame` (`shader_pack_prelude.wgsl`); `resolution` and the
/// time are filled per stage.
fn shader_pack_uniform(
    frame: &Frame,
    view: Mat4,
    projection: Mat4,
    sun: Vec3,
    fog_color: [f32; 4],
    fog: [f32; 4],
    fog_ends: [f32; 4],
) -> pack_stages::FrameUniform {
    let s = frame.shader;
    let sky = frame.sky.sky_color;
    pack_stages::FrameUniform {
        view: view.to_cols_array_2d(),
        projection: projection.to_cols_array_2d(),
        inverse_view: view.inverse().to_cols_array_2d(),
        inverse_projection: projection.inverse().to_cols_array_2d(),
        camera_position: [
            frame.camera.x as f32,
            frame.camera.y as f32,
            frame.camera.z as f32,
            0.0,
        ],
        time: [0.0, 0.0, s.day_fraction, s.partial_tick],
        sun: [sun.x, sun.y, sun.z, frame.sky.sun_angle.to_radians()],
        moon: [-sun.x, -sun.y, -sun.z, f32::from(s.moon_phase)],
        sky_color: [sky[0], sky[1], sky[2], 1.0],
        fog_color,
        fog: [fog[0], fog[1], fog_ends[0], fog_ends[1]],
        weather: [s.rain, s.thunder, 0.0, 0.0],
        camera_state: [
            f32::from(s.submerged),
            frame.night_vision,
            frame.darkness.1,
            0.0,
        ],
        view_params: [
            0.05,
            frame.depth_far,
            frame.distance,
            frame.fov_degrees.to_radians(),
        ],
        dimension: [
            f32::from(s.dimension),
            f32::from(u8::from(s.has_sky_light)),
            0.0,
            0.0,
        ],
        ..Default::default()
    }
}

/// The selected shader pack on the GPU: a Horizon pack's WGSL stages, or a
/// Java pack's translated programs.
enum Pack {
    Horizon(pack_stages::PackStages),
    Java(java_stages::JavaStages),
}

/// The section the camera is in.
fn camera_section(camera: DVec3) -> SectionKey {
    (
        (camera.x.floor() as i32) >> 4,
        (camera.y.floor() as i32 - crate::chunk::MIN_Y) >> 4,
        (camera.z.floor() as i32) >> 4,
    )
}

fn world_projection(fov_degrees: f32, aspect: f32, far: f32) -> Mat4 {
    Mat4::perspective_rh(fov_degrees.to_radians(), aspect, far, 0.05)
}

fn frustum(m: Mat4) -> [Vec4; 6] {
    let r = |i| m.row(i);
    let planes = [
        r(3) + r(0),
        r(3) - r(0),
        r(3) + r(1),
        r(3) - r(1),
        r(2),
        r(3) - r(2),
    ];
    planes.map(|p| p / p.truncate().length())
}

#[cfg(test)]
mod tests {
    #[test]
    fn surface_is_8_bit_and_not_srgb() {
        use super::surface_format;
        use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm, Rgba16Float};
        // RADV's Wayland surfaces list the linear fp16 format first; choosing it makes the
        // compositor read gamma-encoded values as linear light.
        assert_eq!(
            surface_format(&[Rgba16Float, Bgra8UnormSrgb, Bgra8Unorm], Bgra8UnormSrgb),
            Bgra8Unorm
        );
        assert_eq!(
            surface_format(&[Bgra8UnormSrgb, Rgba8Unorm], Bgra8UnormSrgb),
            Rgba8Unorm
        );
        assert_eq!(surface_format(&[Rgba16Float], Bgra8UnormSrgb), Rgba16Float);
    }
    /// A Wayland surface on a compositor with colour management, as wgpu lists it (Mesa 26.2
    /// ANV on an Intel HD 620 under weston 16): wide formats first. The pre-#1075 choice,
    /// the first non-sRGB format, took `Rgba16Float` and the window looked washed out, as if
    /// under a white overlay (#1007); XWayland lists no such format, which is why it looked fine.
    #[test]
    fn a_colour_managed_wayland_surface_still_gets_8_bit() {
        use super::surface_format;
        use wgpu::TextureFormat::*;
        let wayland = [
            Rgba16Float,
            Rgba16Unorm,
            Rgb10a2Unorm,
            Bgra8UnormSrgb,
            Bgra8Unorm,
            Rgba8UnormSrgb,
            Rgba8Unorm,
        ];
        assert_eq!(surface_format(&wayland, Bgra8UnormSrgb), Bgra8Unorm);
        // Without Bgra8: Rgba8, still not a 16- or 10-bit format.
        let without_bgra = [
            Rgba16Float,
            Rgba16Unorm,
            Rgb10a2Unorm,
            Rgba8UnormSrgb,
            Rgba8Unorm,
        ];
        assert_eq!(surface_format(&without_bgra, Rgba8UnormSrgb), Rgba8Unorm);
    }
    #[test]
    fn special_material_batches_preserve_quad_order_and_offsets() {
        let quad = |layer, id| {
            [super::Vertex {
                pos: [0.0; 3],
                uv: [0.0; 2],
                layer,
                color: 0xffff_ffff,
                light: id,
                normal: [0; 4],
            }; 4]
        };
        let entities: Vec<_> = [(0, 0), (4, 1), (11, 2), (7, 3), (4, 4), (12, 7)]
            .into_iter()
            .flat_map(|(layer, id)| quad(layer, id))
            .collect();
        let mut batch = super::EntityBatch::default();
        let counts = batch.update(&entities, &quad(3, 5), &quad(0, 6), &quad(0, 8));
        let vertices = &batch.combined;
        assert_eq!(counts, [4, 1, 1, 0]);
        assert_eq!(
            vertices
                .as_chunks::<4>()
                .0
                .iter()
                .map(|q| q[0].light)
                .collect::<Vec<_>>(),
            [0, 1, 3, 4, 2, 7, 5, 6, 8]
        );
    }
    #[test]
    fn windows_tries_vulkan_then_directx_12() {
        use super::{GraphicsApi, backend_order};
        use wgpu::Backends;
        assert_eq!(
            backend_order(GraphicsApi::Auto, true, false),
            [Backends::VULKAN, Backends::DX12]
        );
        assert_eq!(
            backend_order(GraphicsApi::Vulkan, true, false),
            [Backends::VULKAN]
        );
        assert_eq!(
            backend_order(GraphicsApi::Dx12, true, false),
            [Backends::DX12]
        );
        // WGPU_BACKEND picks the backends itself; other systems have one.
        for api in GraphicsApi::ALL {
            assert_eq!(backend_order(api, true, true), [Backends::default()]);
            assert_eq!(backend_order(api, false, false), [Backends::default()]);
        }
    }
    /// Every WGSL shader file translates to HLSL as wgpu's DirectX 12 backend
    /// writes it for FXC (shader model 5.1), entry point by entry point.
    #[test]
    fn shaders_translate_to_hlsl_for_directx_12() {
        fn wgsl_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    wgsl_files(&path, out);
                } else if path.extension().is_some_and(|e| e == "wgsl") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        wgsl_files(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")),
            &mut files,
        );
        assert!(files.len() >= 10, "{files:?}");
        for path in files {
            let mut text = std::fs::read_to_string(&path).unwrap();
            // Api 2's prelude follows api 1's (`shader_pack::ShaderPack::load`).
            if path.ends_with("shader_pack_prelude_api2.wgsl") {
                text = format!("{}\n{text}", crate::shader_pack::PRELUDE);
            }
            // entity.wgsl comes after the overlay's constants (entity_shader).
            let module = naga::front::wgsl::parse_str(&text)
                .or_else(|_| {
                    naga::front::wgsl::parse_str(&(crate::overlay::wgsl_constants() + &text))
                })
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let info = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
            for entry in &module.entry_points {
                let (module, info) = naga::back::pipeline_constants::process_overrides(
                    &module,
                    &info,
                    Some((entry.stage, &entry.name)),
                    &Default::default(),
                )
                .unwrap();
                let mut hlsl = String::new();
                naga::back::hlsl::Writer::new(
                    &mut hlsl,
                    &naga::back::hlsl::Options::default(),
                    &naga::back::hlsl::PipelineOptions {
                        entry_point: Some((entry.stage, entry.name.clone())),
                    },
                )
                .write(&module, &info, None)
                .unwrap_or_else(|e| panic!("{} {}: {e}", path.display(), entry.name));
                assert!(!hlsl.is_empty());
            }
        }
    }
    #[test]
    fn entity_shader_validates_with_its_overlay_constants() {
        let module = naga::front::wgsl::parse_str(&super::entity_shader()).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
    #[test]
    fn terrain_attributes_follow_the_terrain_vertex() {
        use super::TerrainVertex;
        use std::mem::offset_of;
        let offsets = super::TERRAIN_ATTRIBUTES.map(|a| (a.shader_location, a.offset as usize));
        assert_eq!(
            offsets,
            [
                (0, offset_of!(TerrainVertex, pos)),
                (1, offset_of!(TerrainVertex, uv)),
                (2, offset_of!(TerrainVertex, layer)),
                (4, offset_of!(TerrainVertex, light)),
                (3, offset_of!(TerrainVertex, color)),
            ]
        );
        assert_eq!(std::mem::size_of::<TerrainVertex>(), 24);
    }
    #[test]
    fn cracks_multiply_instead_of_covering_the_block() {
        let blend = super::CRUMBLING_BLEND;
        assert_eq!(blend.color.src_factor, wgpu::BlendFactor::Dst);
        assert_eq!(blend.color.dst_factor, wgpu::BlendFactor::Src);
        assert_eq!(blend.alpha.src_factor, wgpu::BlendFactor::One);
        assert_eq!(blend.alpha.dst_factor, wgpu::BlendFactor::Zero);
    }
    #[test]
    fn shader_globals_match_the_rust_layout() {
        // terrain.wgsl and entity.wgsl bind the same buffer: same fields, same order.
        let fields = |source: &str| -> Vec<String> {
            let body = source.split("struct Globals {").nth(1).unwrap();
            body[..body.find('}').unwrap()]
                .lines()
                .filter_map(|l| l.trim().split_once(':').map(|(name, _)| name.to_string()))
                .filter(|name| !name.starts_with("//"))
                .collect()
        };
        let terrain = fields(include_str!("terrain.wgsl"));
        assert_eq!(terrain, fields(include_str!("entity.wgsl")));
        let rust = fields(include_str!("gfx.rs"));
        assert_eq!(terrain, rust);
    }
    #[test]
    fn selection_outline_uses_vanilla_pixel_width() {
        for width in [640, 1280, 1920] {
            assert_eq!(super::outline_width(width), 2.5);
        }
        assert_eq!(super::outline_width(2560), 2560.0 / 1920.0 * 2.5);
        assert_eq!(super::outline_width(3840), 5.0);
    }
    #[test]
    fn indirect_draws_are_validated_only_in_debug_builds_and_on_directx_12() {
        let validated = |backends| {
            super::instance_descriptor(backends)
                .flags
                .contains(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL)
        };
        if std::env::var_os("WGPU_VALIDATION_INDIRECT_CALL").is_none()
            && std::env::var_os("WGPU_BACKEND").is_none()
        {
            assert_eq!(validated(wgpu::Backends::default()), cfg!(debug_assertions));
            assert_eq!(validated(wgpu::Backends::VULKAN), cfg!(debug_assertions));
            assert_eq!(
                validated(wgpu::Backends::DX12),
                cfg!(debug_assertions) || cfg!(windows)
            );
        }
    }
    #[test]
    fn far_faces_a_hair_apart_keep_their_depth_order() {
        let m = super::world_projection(70.0, 16.0 / 9.0, 768.0);
        let depth = |distance: f32| {
            let clip = m * glam::Vec4::new(0.0, 0.0, -distance, 1.0);
            clip.z / clip.w
        };
        // fire_side at 150 blocks, 0.01 pixel in front of the block it burns on.
        let (block, fire) = (depth(150.0), depth(150.0 - 0.01 / 16.0));
        assert!(fire > block, "{fire} {block}");
        assert!((0.0..=1.0).contains(&block));
        assert!((depth(0.05) - 1.0).abs() < 1e-6);
    }
    #[test]
    fn selection_outline_has_vanilla_perspective_layering() {
        assert_eq!(
            super::outline_params(1280, 720),
            [1280.0, 720.0, 2.5, 0.99975586]
        );
        assert_eq!(
            super::outline_params(3840, 0),
            [3840.0, 1.0, 5.0, 0.99975586]
        );
    }
    #[test]
    fn selection_outline_preserves_shape_edges_without_inflation() {
        let edges = [[2.0, 97.5, 3.0, 3.0, 97.5, 3.0]];
        assert_eq!(
            super::outline_vertices(&edges, glam::DVec3::new(0.5, 98.62, 0.5)),
            vec![[
                1.5,
                (97.5 - 98.62) as f32,
                2.5,
                2.5,
                (97.5 - 98.62) as f32,
                2.5
            ]]
        );
        assert!(super::outline_vertices(&[], glam::DVec3::ZERO).is_empty());
    }
}

pub(crate) fn sprite_mipmap_limit(requested: u32, size: u32, minimum: u32, available: u32) -> u32 {
    // Only the atlas native minimum determines how many resampling-only mips to skip.
    let upscale = (size / minimum.max(1)).max(1).ilog2();
    requested.saturating_add(upscale).min(available - 1)
}
#[cfg(test)]
mod sprite_upload_tests {
    use super::*;
    #[test]
    fn real_rail_mips_do_not_change_with_larger_array_layers() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root
            .join("assets/minecraft/textures/block/rail.png")
            .exists()
        {
            return;
        }
        let mut sprites = crate::sprites::Sprites::default();
        let layers: Vec<_> = [
            "rail",
            "powered_rail",
            "powered_rail_on",
            "detector_rail",
            "activator_rail",
        ]
        .map(|name| {
            sprites
                .insert(&root, &format!("minecraft:block/{name}"))
                .unwrap()
        })
        .into();
        sprites.finish();
        assert_eq!(sprites.size, 16);
        let native: Vec<_> = layers
            .iter()
            .map(|&layer| {
                let start = layer as usize * 16 * 16 * 4;
                mip_chain(
                    &sprites.pixels[start..start + 16 * 16 * 4],
                    16,
                    &sprites.mips,
                    layer,
                    1,
                )
                .0
            })
            .collect();
        sprites.insert(&root, "minecraft:block/oak_sign").unwrap();
        sprites.finish();
        assert_eq!(sprites.size, 32);
        for (&layer, expected) in layers.iter().zip(native) {
            let start = layer as usize * 32 * 32 * 4;
            let (actual, _) = mip_chain(
                &sprites.pixels[start..start + 32 * 32 * 4],
                32,
                &sprites.mips,
                layer,
                1,
            );
            assert_eq!(&actual[32 * 32 * 4..], expected, "rail layer {layer}");
        }
    }
    #[test]
    fn mipmap_option_keeps_real_16px_reductions() {
        for levels in 0..=4 {
            assert_eq!(sprite_mipmap_limit(levels, 32, 16, 6), levels + 1);
            assert_eq!(sprite_mipmap_limit(levels, 16, 16, 5), levels);
        }
    }
    #[test]
    fn prepared_texture_uploads_preserve_layer_order_and_settings() {
        let size = 4;
        let layers = 5;
        let pixels: Vec<u8> = (0..size * size * layers)
            .flat_map(|i| [i as u8, (i * 3) as u8, 99, if i % 3 == 0 { 0 } else { 255 }])
            .collect();
        let settings: Vec<_> = (0..layers)
            .map(|i| crate::mipmap::Settings {
                strategy: if i % 2 == 0 {
                    crate::mipmap::Strategy::Mean
                } else {
                    crate::mipmap::Strategy::StrictCutout
                },
                ..Default::default()
            })
            .collect();
        let border = [20, 30, 40, 128].repeat(16);
        let expected = mip_chain(&pixels, size, &settings, 0, layers);
        // A partial final array and an exact boundary exercise the global
        // layer offset used to select each sprite's mipmap settings.
        for split in [1, 2, 5, 8] {
            let prepared = PreparedTextures::with_split(
                &pixels,
                size,
                layers,
                &settings,
                (&border, size),
                split,
            );
            assert_eq!(prepared.sprites.len(), layers.div_ceil(split) as usize);
            let actual: Vec<_> = prepared
                .sprites
                .iter()
                .flat_map(|(bytes, levels)| {
                    assert_eq!(*levels, expected.1);
                    bytes.iter().copied()
                })
                .collect();
            assert_eq!(actual, expected.0);
            assert_eq!(prepared.forcefield, mip_chain(&border, size, &[], 0, 1));
        }
    }

    #[test]
    fn mip_chain_allocates_exactly_its_size() {
        for size in [16, 64] {
            let animation = crate::sprites::test_animation(size, 0);
            let (chain, _) = mip_chain(animation.pixels(0), size, &[], 0, 1);
            assert_eq!(chain.capacity(), chain.len());
        }
    }
}
