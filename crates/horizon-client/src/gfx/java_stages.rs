//! A Java (OptiFine/Iris) shader pack on the GPU (docs/shader-packs.md):
//! Iris's colour targets (`colortex0..`, each with a main and an alternate
//! side) and the pack's `begin`, `prepare`, `deferred`, `composite` and
//! `final` programs as full-screen passes over them, as Iris's
//! `CompositeRenderer` runs them. Horizon's world enters the target the
//! pack's terrain program draws to (`colortex0` for most packs) and
//! `depthtex0` after `prepare`, where Iris's gbuffers programs would have
//! drawn (`depthtex1`/`depthtex2`: its depth before translucent geometry),
//! and the last picture goes to `world_post`. The targets keep GL's layout
//! (row 0 at the bottom, which the translated vertex programs produce), so
//! the world is flipped going in and the picture coming out.
//!
//! When the pack has terrain programs (`gfx/java_terrain.rs`), Horizon's
//! world comes without its terrain: the pack's `gbuffers_terrain` draws it
//! into the pack's targets right after, against the world's depth, and its
//! `gbuffers_water` draws the translucent terrain after `deferred`, as in
//! Iris. `depthtex1` and `depthtex2` then hold the depth before it.
use super::java_terrain::{Atlas, JavaWorld, TerrainPipelines};
use super::pack_stages::{FrameUniform, noise, rgba_view};
use crate::java_pack::{JavaFolder, JavaPasses, NOISE_KEY, blend_state, pass_blends, texture_key};
use crate::shader_pack::Rgba;
use glam::{Mat3, Mat4, Vec3, Vec4};
use horizon_shaderpack::pipeline::GeometryProgram;
use horizon_shaderpack::pipeline::TargetSettings;
use horizon_shaderpack::pipeline::{Directives, Program, target_index};
use horizon_shaderpack::uniforms::{CustomUniforms, SmoothFloat, UniformState};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// What Iris's uniforms read from the game that the frame uniform doesn't
/// carry.
#[derive(Clone, Copy, Debug, Default)]
pub struct JavaFrame {
    /// The block and sky light at the camera's eye block, 0..15
    /// (`CommonUniforms.getEyeBrightness`).
    pub eye_light: [u8; 2],
    /// `visual/sun_angle` and `visual/moon_angle`, degrees (0: overhead).
    pub sun_angle: f32,
    pub moon_angle: f32,
}

/// The vertex buffers a pass may read (wgpu's default limit).
const MAX_ATTRIBUTES: usize = 8;

pub(super) const BLIT: &str = r#"
@group(0) @binding(0) var color: texture_2d<f32>;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var opaque_depth: texture_depth_2d;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
}

struct World {
    @location(0) color: vec4f,
    @location(1) depth: vec4f,
    @location(2) opaque_depth: vec4f,
}

// The world's colour, depth and pre-translucent depth, upside down into GL's
// layout; Horizon's reversed depth (near 1) as GL's (near 0).
fn world_texel(p: vec4f) -> World {
    let size = textureDimensions(depth);
    let xy = vec2u(u32(p.x), size.y - 1u - u32(p.y));
    return World(
        textureLoad(color, xy, 0),
        vec4f(1.0 - textureLoad(depth, xy, 0)),
        vec4f(1.0 - textureLoad(opaque_depth, xy, 0)),
    );
}

@fragment
fn world_in(@builtin(position) p: vec4f) -> World {
    return world_texel(p);
}

struct WorldDepth {
    @location(0) color: vec4f,
    @location(1) depth: vec4f,
    @location(2) opaque_depth: vec4f,
    @builtin(frag_depth) test: f32,
}

// The same, and the depth into the depth buffer the pack's geometry
// programs test against.
@fragment
fn world_in_depth(@builtin(position) p: vec4f) -> WorldDepth {
    let world = world_texel(p);
    return WorldDepth(world.color, world.depth, world.opaque_depth, world.depth.x);
}

struct Depths {
    @location(0) all: vec4f,
    @location(1) solid: vec4f,
}

// The depth buffer after the pack's geometry into `depthtex0` (and
// `depthtex1`, after solid terrain), both in GL's layout already.
@fragment
fn depth_out_both(@builtin(position) p: vec4f) -> Depths {
    let d = vec4f(textureLoad(depth, vec2u(p.xy), 0));
    return Depths(d, d);
}

@fragment
fn depth_out(@builtin(position) p: vec4f) -> @location(0) vec4f {
    return vec4f(textureLoad(depth, vec2u(p.xy), 0));
}

// A target, upside down back into wgpu's layout.
@fragment
fn present(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let size = textureDimensions(color);
    return textureLoad(color, vec2u(u32(p.x), size.y - 1u - u32(p.y)), 0);
}
"#;

/// A wgpu format for each of Iris's (`InternalTextureFormat`), renderable
/// everywhere: 8-bit unsigned normalized formats stay, signed normalized,
/// 16-bit and packed float ones become half floats, integer ones 32-bit.
/// `norm16`: the device has 16-bit normalized formats, which keep packed
/// G-buffers (Chocapic13' packs two 8-bit values per `RGBA16` channel)
/// exact where a half float rounds them.
fn target_format(iris: &str, norm16: bool) -> wgpu::TextureFormat {
    use wgpu::TextureFormat as F;
    let channels = if iris.starts_with("RGB") || iris.contains("_B") {
        4
    } else if iris.starts_with("RG") {
        2
    } else {
        1
    };
    let pick = |one, two, four| match channels {
        1 => one,
        2 => two,
        _ => four,
    };
    match iris {
        "R8" | "RG8" | "RGB8" | "RGBA8" | "RGBA" | "RGBA2" | "RGBA4" | "R3_G3_B2" | "RGB5_A1"
        | "RGB565" => pick(F::R8Unorm, F::Rg8Unorm, F::Rgba8Unorm),
        "R16" | "RG16" | "RGB16" | "RGBA16" if norm16 => {
            pick(F::R16Unorm, F::Rg16Unorm, F::Rgba16Unorm)
        }
        "R16_SNORM" | "RG16_SNORM" | "RGB16_SNORM" | "RGBA16_SNORM" if norm16 => {
            pick(F::R16Snorm, F::Rg16Snorm, F::Rgba16Snorm)
        }
        "RGB10_A2" => F::Rgb10a2Unorm,
        "RGB10_A2UI" => F::Rgb10a2Uint,
        f if f.ends_with("32F") => pick(F::R32Float, F::Rg32Float, F::Rgba32Float),
        f if f.ends_with("UI") => pick(F::R32Uint, F::Rg32Uint, F::Rgba32Uint),
        f if f.ends_with('I') => pick(F::R32Sint, F::Rg32Sint, F::Rgba32Sint),
        _ => pick(F::R16Float, F::Rg16Float, F::Rgba16Float),
    }
}

/// How a pass reads a texture of `format`.
fn sample_type(format: wgpu::TextureFormat) -> wgpu::TextureSampleType {
    match format.sample_type(None, None) {
        Some(wgpu::TextureSampleType::Float { .. }) => wgpu::TextureSampleType::Float {
            filterable: !matches!(
                format,
                wgpu::TextureFormat::R32Float
                    | wgpu::TextureFormat::Rg32Float
                    | wgpu::TextureFormat::Rgba32Float
            ),
        },
        Some(t) => t,
        None => wgpu::TextureSampleType::Float { filterable: false },
    }
}

/// The textures besides the targets a pass may read.
struct Views<'a> {
    depth: &'a wgpu::TextureView,
    depth_solid: &'a wgpu::TextureView,
    atlas: Option<&'a wgpu::TextureView>,
    lightmap: Option<&'a wgpu::TextureView>,
}

/// A colour target's two sides.
struct Target {
    format: wgpu::TextureFormat,
    main: wgpu::Texture,
    alt: wgpu::Texture,
    main_view: wgpu::TextureView,
    alt_view: wgpu::TextureView,
    clear: Option<wgpu::Color>,
}

/// What a sampler of the pack reads.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    Target(u32),
    /// `depthtex0` / `gdepthtex`: the world's depth (including translucent water).
    Depth,
    /// `depthtex1` / `depthtex2`: the depth before translucent terrain: after
    /// the pack's own solid terrain when it draws it, else Horizon's world's
    /// before translucent water.
    DepthSolid,
    Noise,
    /// `shadowtex*`: no shadow map yet, so a far depth (nothing in shadow);
    /// also Distant Horizons' and Voxy's depth, as without those mods.
    Shadow,
    /// Anything else (`shadowcolor*`, custom textures): white or black.
    Blank(bool),
    /// In geometry programs: the block atlas (`gtexture`), the lightmap,
    /// and a flat normal map for `normals` (sprites have no LabPBR maps).
    Atlas,
    Lightmap,
    FlatNormal,
}

/// What sampler `name` reads, in a geometry program or a full-screen one.
fn source(name: &str, geometry: bool) -> Source {
    match name {
        "texture" | "gtexture" | "tex" if geometry => Source::Atlas,
        "lightmap" if geometry => Source::Lightmap,
        "normals" if geometry => Source::FlatNormal,
        "specular" if geometry => Source::Blank(false),
        "depthtex0" | "gdepthtex" => Source::Depth,
        "depthtex1" | "depthtex2" => Source::DepthSolid,
        "noisetex" => Source::Noise,
        "shadowtex0" | "shadowtex1" | "shadow" | "watershadow" => Source::Shadow,
        n if n.starts_with("dhDepthTex") || n.starts_with("vxDepthTex") => Source::Shadow,
        n if n.starts_with("shadowcolor") => Source::Blank(true),
        n => target_index(n).map_or(Source::Blank(false), Source::Target),
    }
}

/// A sampler of a pass: its texture and sampler bindings.
struct SamplerBinding {
    name: String,
    source: Source,
    dimension: wgpu::TextureViewDimension,
    /// `isampler` (true) or `usampler` (false).
    signed: Option<bool>,
    texture: u32,
    sampler: u32,
    /// The shared sampler it uses (`iris_linear`), if any.
    shared: Option<String>,
    compare: bool,
}

/// A scalar slot of the uniform block: byte offset, and whether it holds
/// an integer (`int`, `uint` or `bool`).
type Slot = (u32, bool);

struct PassGpu {
    program: String,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    samplers: Vec<SamplerBinding>,
    /// Uniform members by name, their scalar slots in order.
    uniforms: Vec<(String, Vec<Slot>)>,
    uniform: Option<(u32, wgpu::Buffer)>,
    /// Each vertex buffer slot's data: 0 position, 1 white, 2 up, 3 zero.
    attributes: Vec<usize>,
    draw_buffers: Vec<u32>,
    reads_alt: BTreeSet<u32>,
    scale: f32,
    /// Geometry programs: the flat draws' pipeline (where the renderer has
    /// them), and the targets written on their alternate side.
    flat_pipeline: Option<wgpu::RenderPipeline>,
    geometry: Option<BTreeSet<u32>>,
}

/// What a pass draws into.
enum Output<'a> {
    /// A full-screen pass: its targets, or for `final` the screen's format.
    Screen(Option<wgpu::TextureFormat>),
    /// A geometry program over Horizon's terrain.
    Geometry(GeometrySetup<'a>),
}

struct GeometrySetup<'a> {
    /// The vertex program for flat draws.
    flat_vertex: &'a horizon_shaderpack::translate::Translated,
    /// Each draw buffer's blending.
    blends: &'a [Option<wgpu::BlendState>],
    layouts: &'a GeometryLayouts<'a>,
}

/// The renderer's bind group layouts a geometry program's pipelines use:
/// the flat draws' group 1 (`gfx/flat.rs`, when it has them) and the page
/// group 3 ([`super::java_terrain::page_layout`]).
pub struct GeometryLayouts<'a> {
    pub flat: Option<&'a wgpu::BindGroupLayout>,
    pub page: &'a wgpu::BindGroupLayout,
}

/// A folder's terrain and water programs.
struct FolderGeometry {
    terrain: PassGpu,
    water: Option<PassGpu>,
    translucent_at: usize,
}

/// What every pass of the pack shares.
struct Common {
    linear: wgpu::Sampler,
    /// The block atlas's, as vanilla's: nearest, mipmapped.
    atlas: wgpu::Sampler,
    nearest: wgpu::Sampler,
    repeat: wgpu::Sampler,
    compare: wgpu::Sampler,
    noise: wgpu::TextureView,
    custom_textures: BTreeMap<String, wgpu::TextureView>,
    /// Full-screen triangle data per attribute kind (see `PassGpu::attributes`).
    vertices: [wgpu::Buffer; 4],
    world_layout: wgpu::BindGroupLayout,
    present_layout: wgpu::BindGroupLayout,
    /// The geometry's depth buffer, read into `depthtex0` and `depthtex1`.
    depth_layout: wgpu::BindGroupLayout,
    blit: wgpu::ShaderModule,
}

/// The textures of one folder at one size.
struct Bound {
    folder: usize,
    size: [u32; 2],
    /// `world_post`'s targets it binds (`Post::generation`).
    post_generation: u64,
    targets: BTreeMap<u32, Target>,
    depth: wgpu::TextureView,
    /// `depthtex1`: `depth` before translucent terrain, or `depth` itself.
    depth_solid: wgpu::TextureView,
    /// Per pass, then `final`.
    groups: Vec<wgpu::BindGroup>,
    geometry: Option<BoundGeometry>,
    world_in: (wgpu::RenderPipeline, wgpu::BindGroup),
    /// The world's target, and whether the world goes to its alternate side.
    world_target: u32,
    world_alt: bool,
    final_target: Option<wgpu::TextureView>,
    present: (wgpu::RenderPipeline, wgpu::BindGroup),
    output_view: wgpu::TextureView,
    output: wgpu::BindGroup,
    /// Blank textures, kept alive by the bind groups too.
    _blank: Vec<wgpu::Texture>,
}

/// What a folder's geometry programs draw with at one size.
struct BoundGeometry {
    /// The depth buffer the geometry tests against: the world's, then the
    /// terrain's.
    test: wgpu::TextureView,
    terrain: wgpu::BindGroup,
    water: Option<wgpu::BindGroup>,
    /// `world_in` that also fills `test`.
    world_in: wgpu::RenderPipeline,
    /// `test` into `depthtex0` and `depthtex1`, and into `depthtex0` alone.
    out_both: wgpu::RenderPipeline,
    out: wgpu::RenderPipeline,
    out_group: wgpu::BindGroup,
}

struct Folder {
    targets: Vec<TargetSettings>,
    world_at: usize,
    world_target: u32,
    flipped_after_prepare: BTreeSet<u32>,
    flipped_at_end: BTreeSet<u32>,
    passes: Vec<PassGpu>,
    final_pass: Option<PassGpu>,
    directives: Directives,
    geometry: Option<FolderGeometry>,
}

pub struct JavaStages {
    pub id: String,
    /// A pass samples `depthtex1`/`depthtex2`: the world's depth is kept
    /// before translucent geometry draws.
    pub reads_opaque_depth: bool,
    format: wgpu::TextureFormat,
    common: Common,
    folders: Vec<Folder>,
    folder_for: [Option<usize>; 3],
    bound: Option<Bound>,
    started: std::time::Instant,
    last: Option<std::time::Instant>,
    frames: u32,
    previous: Option<(Mat4, Mat4, Vec3)>,
    uniforms: CustomUniforms,
    uniform_state: UniformState,
    /// `eyeBrightnessSmooth`, block then sky.
    eye_smooth: [SmoothFloat; 2],
    /// The block atlas of the geometry programs.
    atlas: Option<Atlas>,
    /// The device has 16-bit normalized formats ([`target_format`]).
    norm16: bool,
    /// `wetness`: `rainStrength` smoothed with `wetnessHalflife` going up
    /// and `drynessHalflife` going down.
    wetness_smooth: SmoothFloat,
}

impl JavaStages {
    /// The pipelines of a loaded Java pack, or wgpu's error. `norm16`: the
    /// device has 16-bit normalized targets ([`target_format`]).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        id: &str,
        pack: &JavaPasses,
        layouts: &GeometryLayouts,
        norm16: bool,
    ) -> Result<Self, String> {
        let common = Common::new(device, queue, &pack.custom_textures);
        let folder_for = [-1, 0, 1].map(|d| pack.folder_for(d));
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let folders = pack
            .folders
            .iter()
            .map(|plan: &JavaFolder| {
                let formats: Vec<wgpu::TextureFormat> = plan
                    .targets
                    .iter()
                    .map(|t| target_format(&t.format, norm16))
                    .collect();
                let passes = plan
                    .passes
                    .iter()
                    .map(|p| PassGpu::new(device, p, &formats, Output::Screen(None)))
                    .collect::<Result<Vec<_>, _>>()?;
                let final_pass = plan
                    .final_pass
                    .as_ref()
                    .map(|p| PassGpu::new(device, p, &formats, Output::Screen(Some(format))))
                    .transpose()?;
                Ok::<_, String>(Folder {
                    targets: plan.targets.clone(),
                    world_at: plan.world_at,
                    world_target: plan.world_target,
                    flipped_after_prepare: plan.flipped_after_prepare.clone(),
                    flipped_at_end: plan.flipped_at_end.clone(),
                    passes,
                    final_pass,
                    directives: plan.directives,
                    geometry: folder_geometry(device, plan, &formats, layouts),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(error.to_string());
        }
        let reads_opaque_depth = folders
            .iter()
            .flat_map(|f| f.passes.iter().chain(&f.final_pass))
            .flat_map(|p| &p.samplers)
            .any(|s| s.source == Source::DepthSolid);
        Ok(Self {
            id: id.to_owned(),
            reads_opaque_depth,
            format,
            common,
            folders,
            folder_for,
            bound: None,
            started: std::time::Instant::now(),
            last: None,
            frames: 0,
            previous: None,
            uniforms: pack.uniforms.clone(),
            uniform_state: UniformState::default(),
            eye_smooth: [SmoothFloat::default(); 2],
            atlas: None,
            norm16,
            wetness_smooth: SmoothFloat::default(),
        })
    }

    /// Whether the pack draws the terrain in `dimension` (0 overworld, -1
    /// the Nether, 1 the End): the renderer then leaves it out of the world
    /// and gives `run` a [`JavaWorld`].
    pub fn draws_terrain(&self, dimension: i8) -> bool {
        let d = i32::from(dimension).clamp(-1, 1);
        let folder = self.folder_for[(d + 1) as usize].unwrap_or(0);
        self.folders
            .get(folder)
            .is_some_and(|f| f.geometry.is_some())
    }

    /// Run the passes of the world's dimension over its `color` and `depth`
    /// (`size`) and return `post`'s bindings of the picture. `world` is given
    /// when the world was drawn without its terrain ([`Self::draws_terrain`]):
    /// the pack's geometry programs draw it.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        size: [u32; 2],
        post: &super::world_post::Post,
        frame: FrameUniform,
        world: JavaFrame,
        terrain: Option<&JavaWorld>,
    ) -> &wgpu::BindGroup {
        let dimension = (frame.dimension[0] as i32).clamp(-1, 1);
        let folder = self.folder_for[(dimension + 1) as usize].unwrap_or(0);
        let terrain = terrain.filter(|_| self.folders[folder].geometry.is_some());
        if let Some(w) = terrain
            && self.atlas.as_ref().is_none_or(|a| !a.fits(w))
        {
            self.atlas = Atlas::new(device, queue, w);
            self.bound = None;
        }
        if self.bound.as_ref().is_none_or(|b| {
            b.folder != folder
                || b.size != size
                || b.post_generation != post.generation()
                || b.geometry.is_some() != terrain.is_some()
        }) {
            self.bound = Some(self.bind(device, queue, folder, color, depth, size, post, terrain));
        }
        let now = std::time::Instant::now();
        let frame_time = self.last.map_or(0.0, |t| (now - t).as_secs_f32());
        self.last = Some(now);
        let directives = self.folders[folder].directives;
        let mut inputs = Inputs::new(
            &frame,
            size,
            self.started.elapsed().as_secs_f32(),
            frame_time,
            self.frames,
            self.previous,
        );
        inputs.set_world(&world, directives.sun_path_rotation);
        if let Some(atlas) = &self.atlas {
            inputs.atlas_size = atlas.size().map(|v| v as f32);
        }
        let half_life = directives.eye_brightness_half_life;
        for (i, smooth) in self.eye_smooth.iter_mut().enumerate() {
            inputs.eye_smooth[i] = smooth.update(inputs.eye[i], half_life, half_life, frame_time);
        }
        inputs.wetness = self.wetness_smooth.update(
            frame.weather[0],
            directives.wetness_half_life,
            directives.dryness_half_life,
            frame_time,
        );
        self.uniforms
            .update(&mut self.uniform_state, frame_time, &|name| {
                inputs.builtin(name)
            });
        inputs.custom = self.uniform_state.values().clone();
        self.previous = Some((inputs.view, inputs.projection, inputs.camera));
        self.frames = self.frames.wrapping_add(1);
        let bound = self.bound.as_ref().expect("bound above");
        let folder = &self.folders[bound.folder];

        // Iris clears both sides of the targets that ask for it.
        for target in bound.targets.values() {
            if let Some(clear) = target.clear {
                for view in [&target.main_view, &target.alt_view] {
                    clear_view(encoder, view, clear);
                }
            }
        }
        if let (Some(w), Some(atlas)) = (terrain, &mut self.atlas) {
            atlas.update(device, encoder, w);
        }
        // The world after `prepare`, the terrain with it; translucent
        // terrain after `deferred`.
        let world_at = folder.world_at.min(folder.passes.len());
        let geometry = folder
            .geometry
            .as_ref()
            .zip(bound.geometry.as_ref())
            .zip(terrain);
        let translucent_at = geometry.map_or(usize::MAX, |((g, _), _)| {
            g.translucent_at.min(folder.passes.len())
        });
        for i in 0..=folder.passes.len() {
            if i == world_at {
                world_in(encoder, bound);
                if let Some(((g, b), w)) = geometry {
                    g.terrain.draw_geometry(
                        queue, encoder, bound, &b.terrain, &b.test, &inputs, w, false,
                    );
                    depth_out(encoder, bound, true);
                }
            }
            if i == translucent_at
                && let Some(((g, b), w)) = geometry
                && let (Some(water), Some(group)) = (&g.water, &b.water)
            {
                water.draw_geometry(queue, encoder, bound, group, &b.test, &inputs, w, true);
                depth_out(encoder, bound, false);
            }
            if let (Some(pass), Some(group)) = (folder.passes.get(i), bound.groups.get(i)) {
                pass.draw(queue, encoder, bound, group, &self.common, &inputs);
            }
        }
        if let (Some(pass), Some(view)) = (&folder.final_pass, &bound.final_target) {
            let group = bound.groups.last().expect("final's group");
            pass.draw_to(
                queue,
                encoder,
                &[Some(view)],
                size,
                group,
                &self.common,
                &inputs,
            );
        }
        {
            let attachments = [Some(color_attachment(&bound.output_view, None))];
            let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Java shader pack picture"),
                color_attachments: &attachments,
                ..Default::default()
            });
            render.set_pipeline(&bound.present.0);
            render.set_bind_group(0, &bound.present.1, &[]);
            render.draw(0..3, 0..1);
        }
        // The next frame starts unflipped: the last writes go back to main.
        for index in &folder.flipped_at_end {
            if let Some(t) = bound.targets.get(index) {
                encoder.copy_texture_to_texture(
                    t.alt.as_image_copy(),
                    t.main.as_image_copy(),
                    t.main.size(),
                );
            }
        }
        &bound.output
    }

    #[allow(clippy::too_many_arguments)]
    fn bind(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        folder_index: usize,
        color: &wgpu::TextureView,
        world_depth: &wgpu::TextureView,
        size: [u32; 2],
        post: &super::world_post::Post,
        world: Option<&JavaWorld>,
    ) -> Bound {
        let folder = &self.folders[folder_index];
        let plan = folder;
        let all = folder.passes.iter().chain(&folder.final_pass);
        let geometry = folder.geometry.as_ref().filter(|_| world.is_some());
        let geometry_passes = geometry
            .into_iter()
            .flat_map(|g| std::iter::once(&g.terrain).chain(&g.water));
        // Every target a pass reads or writes, colortex0 (shown without
        // `final`) and the world's.
        let mut used: BTreeSet<u32> = [0, folder.world_target].into();
        for pass in all.clone().chain(geometry_passes) {
            used.extend(pass.draw_buffers.iter().copied());
            used.extend(pass.samplers.iter().filter_map(|s| match s.source {
                Source::Target(i) => Some(i),
                _ => None,
            }));
        }
        let texture = |label, format, usage, size: [u32; 2]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0].max(1),
                    height: size[1].max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let target_usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST;
        let targets: BTreeMap<u32, Target> = used
            .iter()
            .filter_map(|&i| {
                let settings = plan.targets.get(i as usize)?;
                let format = target_format(&settings.format, self.norm16);
                let target_size = match settings.size {
                    Some(horizon_shaderpack::pipeline::TargetSize::Relative(w, h)) if i != 0 => [
                        (size[0] as f32 * w).round() as u32,
                        (size[1] as f32 * h).round() as u32,
                    ],
                    Some(horizon_shaderpack::pipeline::TargetSize::Pixels(w, h)) if i != 0 => {
                        [w, h]
                    }
                    _ => size,
                };
                let main = texture("Java shader pack target", format, target_usage, target_size);
                let alt = texture("Java shader pack target", format, target_usage, target_size);
                // Iris: colortex0 to the fog colour, colortex1 to white.
                let clear = settings.clear.then(|| {
                    let [r, g, b, a] = settings.clear_color.unwrap_or(match i {
                        1 => [1.0; 4],
                        _ => [0.0; 4],
                    });
                    wgpu::Color {
                        r: r.into(),
                        g: g.into(),
                        b: b.into(),
                        a: a.into(),
                    }
                });
                Some((
                    i,
                    Target {
                        format,
                        main_view: main.create_view(&Default::default()),
                        alt_view: alt.create_view(&Default::default()),
                        main,
                        alt,
                        clear,
                    },
                ))
            })
            .collect();
        let depth = texture(
            "Java shader pack depth",
            wgpu::TextureFormat::R32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            size,
        )
        .create_view(&Default::default());
        // Written by `world_in` (Horizon's depth before translucent water),
        // then by the pack's solid terrain when it draws it.
        let depth_solid = texture(
            "Java shader pack depth before translucent terrain",
            wgpu::TextureFormat::R32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            size,
        )
        .create_view(&Default::default());
        let views = Views {
            depth: &depth,
            depth_solid: &depth_solid,
            atlas: self.atlas.as_ref().map(|a| &a.view),
            lightmap: world.map(|w| w.lightmap),
        };
        let mut blank = Vec::new();
        let groups = all
            .clone()
            .map(|pass| pass.bind(device, queue, &targets, &views, &self.common, &mut blank))
            .collect();
        let geometry = geometry.map(|g| {
            let test = texture(
                "Java shader pack geometry depth",
                wgpu::TextureFormat::Depth32Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                size,
            )
            .create_view(&Default::default());
            let r32 = wgpu::TextureFormat::R32Float;
            let out_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Java shader pack geometry depth"),
                layout: &self.common.depth_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&test),
                }],
            });
            let mut bind = |pass: &PassGpu| {
                pass.bind(device, queue, &targets, &views, &self.common, &mut blank)
            };
            BoundGeometry {
                terrain: bind(&g.terrain),
                water: g.water.as_ref().map(&mut bind),
                world_in: blit_pipeline(
                    device,
                    &self.common,
                    &self.common.world_layout,
                    "world_in_depth",
                    &[targets[&folder.world_target].format, r32, r32],
                    true,
                ),
                out_both: blit_pipeline(
                    device,
                    &self.common,
                    &self.common.depth_layout,
                    "depth_out_both",
                    &[r32, r32],
                    false,
                ),
                out: blit_pipeline(
                    device,
                    &self.common,
                    &self.common.depth_layout,
                    "depth_out",
                    &[r32],
                    false,
                ),
                out_group,
                test,
            }
        });

        // The world goes into its target's side the gbuffers would write.
        let world_target = &targets[&folder.world_target];
        let world_pipeline = blit_pipeline(
            device,
            &self.common,
            &self.common.world_layout,
            "world_in",
            &[
                world_target.format,
                wgpu::TextureFormat::R32Float,
                wgpu::TextureFormat::R32Float,
            ],
            false,
        );
        let world_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Java shader pack world"),
            layout: &self.common.world_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(world_depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(post.opaque_depth()),
                },
            ],
        });

        let final_target = folder.final_pass.as_ref().map(|_| {
            texture(
                "Java shader pack final",
                self.format,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                size,
            )
            .create_view(&Default::default())
        });
        // Without `final`, Iris shows colortex0 as the last pass left it.
        let colortex0 = &targets[&0];
        let shown = final_target.clone().unwrap_or_else(|| {
            if plan.flipped_at_end.contains(&0) {
                colortex0.alt_view.clone()
            } else {
                colortex0.main_view.clone()
            }
        });
        let present_pipeline = blit_pipeline(
            device,
            &self.common,
            &self.common.present_layout,
            "present",
            &[self.format],
            false,
        );
        let present_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Java shader pack picture"),
            layout: &self.common.present_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&shown),
            }],
        });
        let output_view = texture(
            "Java shader pack picture",
            self.format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            size,
        )
        .create_view(&Default::default());
        let output = post.bind(device, &output_view);
        Bound {
            folder: folder_index,
            size,
            post_generation: post.generation(),
            targets,
            depth,
            depth_solid,
            groups,
            geometry,
            world_in: (world_pipeline, world_group),
            world_target: folder.world_target,
            world_alt: plan.flipped_after_prepare.contains(&folder.world_target),
            final_target,
            present: (present_pipeline, present_group),
            output_view,
            output,
            _blank: blank,
        }
    }
}

/// A folder's terrain and water programs on the GPU, or `None` (with a
/// warning: the terrain is then Horizon's) when wgpu refuses them.
fn folder_geometry(
    device: &wgpu::Device,
    plan: &JavaFolder,
    formats: &[wgpu::TextureFormat],
    layouts: &GeometryLayouts,
) -> Option<FolderGeometry> {
    let terrain = plan.terrain.as_ref()?;
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let gpu = |g: &GeometryProgram, blends: &[Option<wgpu::BlendState>]| {
        PassGpu::new(
            device,
            &g.program,
            formats,
            Output::Geometry(GeometrySetup {
                flat_vertex: g.variants.first().unwrap_or(&g.program.vertex),
                blends,
                layouts,
            }),
        )
    };
    let made = gpu(terrain, &plan.terrain_blends).and_then(|terrain| {
        let water = plan
            .water
            .as_ref()
            .map(|w| gpu(w, &plan.water_blends))
            .transpose()?;
        Ok(FolderGeometry {
            terrain,
            water,
            translucent_at: plan.translucent_at,
        })
    });
    let error = pollster::block_on(scope.pop());
    match (made, error) {
        (Ok(geometry), None) => Some(geometry),
        (Err(e), _) => {
            log::warn!("Horizon: shader pack {}: {e}", terrain.program.pass.program);
            None
        }
        (_, Some(e)) => {
            log::warn!("Horizon: shader pack {}: {e}", terrain.program.pass.program);
            None
        }
    }
}

fn world_in(encoder: &mut wgpu::CommandEncoder, bound: &Bound) {
    let target = &bound.targets[&bound.world_target];
    let side = if bound.world_alt {
        &target.alt_view
    } else {
        &target.main_view
    };
    let attachments = [
        Some(color_attachment(side, None)),
        Some(color_attachment(&bound.depth, None)),
        Some(color_attachment(&bound.depth_solid, None)),
    ];
    let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Java shader pack world"),
        color_attachments: &attachments,
        depth_stencil_attachment: bound.geometry.as_ref().map(|g| {
            wgpu::RenderPassDepthStencilAttachment {
                view: &g.test,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }
        }),
        ..Default::default()
    });
    match &bound.geometry {
        Some(g) => render.set_pipeline(&g.world_in),
        None => render.set_pipeline(&bound.world_in.0),
    }
    render.set_bind_group(0, &bound.world_in.1, &[]);
    render.draw(0..3, 0..1);
}

/// The geometry's depth buffer into `depthtex0`, and `depthtex1` too after
/// solid terrain.
fn depth_out(encoder: &mut wgpu::CommandEncoder, bound: &Bound, solid: bool) {
    let Some(g) = &bound.geometry else {
        return;
    };
    let mut attachments = vec![Some(color_attachment(&bound.depth, None))];
    if solid {
        attachments.push(Some(color_attachment(&bound.depth_solid, None)));
    }
    let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Java shader pack geometry depth"),
        color_attachments: &attachments,
        ..Default::default()
    });
    render.set_pipeline(if solid { &g.out_both } else { &g.out });
    render.set_bind_group(0, &g.out_group, &[]);
    render.draw(0..3, 0..1);
}

fn color_attachment(
    view: &wgpu::TextureView,
    clear: Option<wgpu::Color>,
) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        depth_slice: None,
        ops: wgpu::Operations {
            load: clear.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear),
            store: wgpu::StoreOp::Store,
        },
    }
}

fn clear_view(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, color: wgpu::Color) {
    let attachments = [Some(color_attachment(view, Some(color)))];
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Java shader pack clear"),
        color_attachments: &attachments,
        ..Default::default()
    });
}

/// A full-screen pipeline of `BLIT`'s `entry`; `depth`: it writes the
/// geometry's depth buffer.
fn blit_pipeline(
    device: &wgpu::Device,
    common: &Common,
    layout: &wgpu::BindGroupLayout,
    entry: &str,
    formats: &[wgpu::TextureFormat],
    depth: bool,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Java shader pack blit"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    let targets: Vec<Option<wgpu::ColorTargetState>> =
        formats.iter().map(|&f| Some(f.into())).collect();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &common.blit,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: depth.then_some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &common.blit,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl Common {
    /// `custom` holds the pack's `texture.*` images (`JavaPasses::custom_textures`).
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, custom: &BTreeMap<String, Rgba>) -> Self {
        let sampler = |filter, address, compare| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Java shader pack"),
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                mag_filter: filter,
                min_filter: filter,
                compare,
                ..Default::default()
            })
        };
        use wgpu::{AddressMode::*, FilterMode::*};
        let noise = match custom.get(NOISE_KEY) {
            Some(image) => rgba_view(device, queue, "noisetex", image),
            None => rgba_view(device, queue, "noisetex", &noise()),
        };
        let custom_textures = custom
            .iter()
            .filter(|(key, _)| *key != NOISE_KEY)
            .map(|(key, image)| (key.clone(), rgba_view(device, queue, key, image)))
            .collect();
        // The full-screen triangle (0, 0), (2, 0), (0, 2): the pack's
        // projection maps 0..1 to the screen. Texture coordinates are the
        // position; colours white; normals up.
        let vertices = [
            [
                [0.0, 0.0, 0.0, 1.0],
                [2.0, 0.0, 0.0, 1.0],
                [0.0, 2.0, 0.0, 1.0],
            ],
            [[1.0f32; 4]; 3],
            [[0.0, 0.0, 1.0, 0.0]; 3],
            [[0.0; 4]; 3],
        ]
        .map(|data| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Java shader pack vertices"),
                size: std::mem::size_of_val(&data) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&data));
            buffer
        });
        let texture_entry = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let unfilterable = wgpu::TextureSampleType::Float { filterable: false };
        Self {
            linear: sampler(Linear, ClampToEdge, None),
            atlas: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Java shader pack block atlas"),
                mag_filter: Nearest,
                min_filter: Nearest,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            }),
            nearest: sampler(Nearest, ClampToEdge, None),
            repeat: sampler(Linear, Repeat, None),
            compare: sampler(Linear, ClampToEdge, Some(wgpu::CompareFunction::LessEqual)),
            noise,
            custom_textures,
            vertices,
            world_layout: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Java shader pack world"),
                entries: &[
                    texture_entry(0, unfilterable),
                    texture_entry(1, wgpu::TextureSampleType::Depth),
                    texture_entry(2, wgpu::TextureSampleType::Depth),
                ],
            }),
            present_layout: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Java shader pack picture"),
                entries: &[texture_entry(0, unfilterable)],
            }),
            depth_layout: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Java shader pack geometry depth"),
                entries: &[texture_entry(1, wgpu::TextureSampleType::Depth)],
            }),
            blit: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Java shader pack blit"),
                source: wgpu::ShaderSource::Wgsl(BLIT.into()),
            }),
        }
    }
}

/// The view dimension and integer kind of a GLSL sampler type.
fn sampler_kind(kind: &str) -> Option<(wgpu::TextureViewDimension, Option<bool>, bool)> {
    use wgpu::TextureViewDimension as D;
    let signed = match kind.as_bytes().first() {
        Some(b'i') => Some(true),
        Some(b'u') => Some(false),
        _ => None,
    };
    let rest = kind
        .trim_start_matches(['i', 'u'])
        .strip_prefix("sampler")?;
    let dimension = if rest.starts_with("2DArray") {
        D::D2Array
    } else if rest.starts_with("2D") || rest.starts_with("2DRect") {
        D::D2
    } else if rest.starts_with("3D") {
        D::D3
    } else if rest.starts_with("CubeArray") {
        D::CubeArray
    } else if rest.starts_with("Cube") {
        D::Cube
    } else if rest.starts_with("1D") {
        D::D1
    } else {
        return None;
    };
    Some((dimension, signed, rest.ends_with("Shadow")))
}

impl PassGpu {
    fn new(
        device: &wgpu::Device,
        program: &Program,
        formats: &[wgpu::TextureFormat],
        output: Output,
    ) -> Result<Self, String> {
        let (screen, setup) = match output {
            Output::Screen(screen) => (screen, None),
            Output::Geometry(setup) => (None, Some(setup)),
        };
        let name = program.pass.program.clone();
        let link = &program.link;
        let mut samplers = Vec::new();
        let mut seen = BTreeSet::new();
        for (sampler, kind) in program
            .vertex
            .interface
            .samplers
            .iter()
            .chain(&program.fragment.interface.samplers)
        {
            if !seen.insert(sampler.clone()) {
                continue;
            }
            let (dimension, signed, compare) = sampler_kind(kind)
                .ok_or_else(|| format!("{name}: {kind} {sampler} isn't supported"))?;
            let binding = |var: &str| {
                link.bindings
                    .get(var)
                    .copied()
                    .ok_or_else(|| format!("{name}: {sampler} has no binding"))
            };
            let var = program
                .fragment
                .interface
                .sampler_vars
                .get(sampler)
                .or_else(|| program.vertex.interface.sampler_vars.get(sampler))
                .cloned()
                .unwrap_or_else(|| format!("{sampler}_s"));
            samplers.push(SamplerBinding {
                name: sampler.clone(),
                source: source(sampler, setup.is_some()),
                dimension,
                signed,
                texture: binding(&format!("{sampler}_t"))?,
                sampler: binding(&var)?,
                shared: var.starts_with("iris_").then_some(var),
                compare,
            });
        }
        // The layout: the uniform block, then each sampler's texture and sampler.
        let block = [&program.fragment, &program.vertex]
            .into_iter()
            .find_map(|t| uniform_block(&t.module));
        let mut entries = Vec::new();
        if block.is_some() {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
        }
        let mut sampler_bindings = BTreeSet::new();
        for s in &samplers {
            let (sample_type, filtering) = match (s.compare, s.source) {
                (true, _) => (wgpu::TextureSampleType::Depth, None),
                (false, Source::Target(i)) => {
                    let format = formats
                        .get(i as usize)
                        .copied()
                        .unwrap_or(wgpu::TextureFormat::Rgba8Unorm);
                    let t = sample_type(format);
                    (
                        t,
                        Some(matches!(
                            t,
                            wgpu::TextureSampleType::Float { filterable: true }
                        )),
                    )
                }
                (false, Source::Depth | Source::DepthSolid | Source::Shadow) => (
                    wgpu::TextureSampleType::Float { filterable: false },
                    Some(false),
                ),
                (false, _) => (
                    wgpu::TextureSampleType::Float { filterable: true },
                    Some(true),
                ),
            };
            let (sample_type, filtering) = match s.signed {
                Some(true) => (wgpu::TextureSampleType::Sint, Some(false)),
                Some(false) => (wgpu::TextureSampleType::Uint, Some(false)),
                None => (sample_type, filtering),
            };
            // A shared sampler filters as its name says.
            let filtering = match s.shared.as_deref() {
                Some("iris_compare") => None,
                Some("iris_nearest") => Some(false),
                Some(_) => Some(true),
                None => filtering,
            };
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: s.texture,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type,
                    view_dimension: s.dimension,
                    multisampled: false,
                },
                count: None,
            });
            if !sampler_bindings.insert(s.sampler) {
                continue;
            }
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: s.sampler,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Sampler(match filtering {
                    None => wgpu::SamplerBindingType::Comparison,
                    Some(true) => wgpu::SamplerBindingType::Filtering,
                    Some(false) => wgpu::SamplerBindingType::NonFiltering,
                }),
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(&name),
            entries: &entries,
        });
        if let Some(setup) = setup {
            return Self::geometry(device, program, formats, setup, layout, samplers, block);
        }
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&name),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });

        // Vertex inputs, each from its own buffer of the full-screen triangle.
        let inputs = vertex_inputs(&program.vertex.module);
        if inputs.len() > MAX_ATTRIBUTES {
            return Err(format!(
                "{name}: more than {MAX_ATTRIBUTES} vertex attributes"
            ));
        }
        let attribute_layouts: Vec<[wgpu::VertexAttribute; 1]> = inputs
            .iter()
            .map(|(_, location, format)| {
                [wgpu::VertexAttribute {
                    format: *format,
                    offset: 0,
                    shader_location: *location,
                }]
            })
            .collect();
        let buffers: Vec<Option<wgpu::VertexBufferLayout>> = attribute_layouts
            .iter()
            .map(|attributes| {
                Some(wgpu::VertexBufferLayout {
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes,
                })
            })
            .collect();
        let attributes = inputs
            .iter()
            .map(|(name, _, _)| match name.as_str() {
                n if n == "iris_Vertex" || n.starts_with("iris_MultiTexCoord") => 0,
                "iris_Color" => 1,
                "iris_Normal" => 2,
                _ => 3,
            })
            .collect();

        // Output location `l` writes target `draw_buffers[l]` (or the screen).
        let outputs = &program.fragment.interface.outputs;
        let blends = pass_blends(&program.pass, &program.draw_buffers, None);
        let target_states: Vec<Option<wgpu::ColorTargetState>> = match screen {
            Some(format) => vec![Some(wgpu::ColorTargetState {
                format,
                blend: program.pass.blend.as_ref().and_then(blend_state),
                write_mask: if outputs.contains_key(&0) {
                    wgpu::ColorWrites::ALL
                } else {
                    wgpu::ColorWrites::empty()
                },
            })],
            None => program
                .draw_buffers
                .iter()
                .enumerate()
                .map(|(l, &i)| {
                    let format = formats
                        .get(i as usize)
                        .copied()
                        .unwrap_or(wgpu::TextureFormat::Rgba8Unorm);
                    // Only filterable float/unorm formats support blending.
                    let can_blend = matches!(
                        sample_type(format),
                        wgpu::TextureSampleType::Float { filterable: true }
                    );
                    Some(wgpu::ColorTargetState {
                        format,
                        blend: blends[l].filter(|_| can_blend),
                        write_mask: if outputs.contains_key(&(l as u32)) {
                            wgpu::ColorWrites::ALL
                        } else {
                            wgpu::ColorWrites::empty()
                        },
                    })
                })
                .collect(),
        };
        let module = |t: &horizon_shaderpack::translate::Translated, label: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(t.wgsl.as_str().into()),
            })
        };
        let vertex_module = module(&program.vertex, &format!("{name}.vsh"));
        let fragment_module = module(&program.fragment, &format!("{name}.fsh"));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&name),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment_module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &target_states,
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = block.as_ref().map(|(size, _)| {
            (
                *size,
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Java shader pack uniforms"),
                    size: u64::from(*size).max(16),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
            )
        });
        Ok(Self {
            program: name,
            pipeline,
            layout,
            samplers,
            uniforms: block.map(|(_, members)| members).unwrap_or_default(),
            uniform,
            attributes,
            draw_buffers: if screen.is_some() {
                Vec::new()
            } else {
                program.draw_buffers.clone()
            },
            reads_alt: program.pass.reads_alt.clone(),
            scale: program.pass.scale.clamp(0.0, 1.0),
            flat_pipeline: None,
            geometry: None,
        })
    }

    /// A geometry program's pipelines over Horizon's terrain vertices
    /// (instanced and flat draws), into its draw buffers on the side the
    /// next passes read, tested against the world's depth.
    #[allow(clippy::too_many_arguments)]
    fn geometry(
        device: &wgpu::Device,
        program: &Program,
        formats: &[wgpu::TextureFormat],
        setup: GeometrySetup,
        layout: wgpu::BindGroupLayout,
        samplers: Vec<SamplerBinding>,
        block: Option<(u32, Vec<(String, Vec<Slot>)>)>,
    ) -> Result<Self, String> {
        let name = program.pass.program.clone();
        let outputs = &program.fragment.interface.outputs;
        let targets: Vec<Option<wgpu::ColorTargetState>> = program
            .draw_buffers
            .iter()
            .enumerate()
            .map(|(l, &i)| {
                let format = formats
                    .get(i as usize)
                    .copied()
                    .unwrap_or(wgpu::TextureFormat::Rgba8Unorm);
                let blendable = matches!(
                    sample_type(format),
                    wgpu::TextureSampleType::Float { filterable: true }
                );
                Some(wgpu::ColorTargetState {
                    format,
                    blend: setup.blends.get(l).copied().flatten().filter(|_| blendable),
                    write_mask: if outputs.contains_key(&(l as u32)) {
                        wgpu::ColorWrites::ALL
                    } else {
                        wgpu::ColorWrites::empty()
                    },
                })
            })
            .collect();
        let vertex = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::mesher::TerrainVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &super::TERRAIN_ATTRIBUTES,
        };
        let instance_attributes = wgpu::vertex_attr_array![5 => Float32x4];
        let instance = wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &instance_attributes,
        };
        let fragment_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&format!("{name}.fsh")),
            source: wgpu::ShaderSource::Wgsl(program.fragment.wgsl.as_str().into()),
        });
        let pipeline = |vertex_program: &horizon_shaderpack::translate::Translated,
                        flat: Option<&wgpu::BindGroupLayout>| {
            let vertex_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&format!("{name}.vsh")),
                source: wgpu::ShaderSource::Wgsl(vertex_program.wgsl.as_str().into()),
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&name),
                bind_group_layouts: &[None, flat, Some(&layout), Some(setup.layouts.page)],
                ..Default::default()
            });
            let buffers = if flat.is_some() {
                vec![Some(vertex.clone())]
            } else {
                vec![Some(vertex.clone()), Some(instance.clone())]
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&name),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &vertex_module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                // The y flip into GL's layout turns the faces around.
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    front_face: wgpu::FrontFace::Cw,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &fragment_module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let instanced = pipeline(&program.vertex, None);
        let flat_pipeline = setup
            .layouts
            .flat
            .map(|flat| pipeline(setup.flat_vertex, Some(flat)));
        let uniform = block.as_ref().map(|(size, _)| {
            (
                *size,
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Java shader pack uniforms"),
                    size: u64::from(*size).max(16),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
            )
        });
        // It reads the side the next passes read, but the other side of the
        // targets it writes: wgpu can't read and write a texture at once.
        let flipped = program.pass.reads_alt.clone();
        let written: BTreeSet<u32> = program.draw_buffers.iter().copied().collect();
        let reads_alt = (0..horizon_shaderpack::pipeline::MAX_COLOR_BUFFERS)
            .filter(|i| flipped.contains(i) != written.contains(i))
            .collect();
        Ok(Self {
            program: name,
            pipeline: instanced,
            layout,
            samplers,
            uniforms: block.map(|(_, members)| members).unwrap_or_default(),
            uniform,
            attributes: Vec::new(),
            draw_buffers: program.draw_buffers.clone(),
            reads_alt,
            scale: 1.0,
            flat_pipeline,
            geometry: Some(flipped),
        })
    }

    /// The pass's bind group over `targets` and `views`.
    fn bind(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: &BTreeMap<u32, Target>,
        textures: &Views,
        common: &Common,
        blank: &mut Vec<wgpu::Texture>,
    ) -> wgpu::BindGroup {
        let depth = textures.depth;
        let mut views = Vec::new();
        for s in &self.samplers {
            let custom = (!s.compare && s.dimension == wgpu::TextureViewDimension::D2)
                .then(|| {
                    common
                        .custom_textures
                        .get(&texture_key(&self.program, &s.name))
                })
                .flatten();
            let view = if let Some(custom) = custom {
                custom.clone()
            } else {
                match (s.compare, s.source) {
                    (true, _) => blank_view(device, queue, blank, s, BlankKind::FarDepth),
                    (false, Source::Target(i)) if s.dimension == wgpu::TextureViewDimension::D2 => {
                        match targets.get(&i) {
                            Some(t) if self.reads_alt.contains(&i) => t.alt_view.clone(),
                            Some(t) => t.main_view.clone(),
                            None => blank_view(device, queue, blank, s, BlankKind::Color(false)),
                        }
                    }
                    (false, Source::Depth) if s.dimension == wgpu::TextureViewDimension::D2 => {
                        depth.clone()
                    }
                    (false, Source::DepthSolid)
                        if s.dimension == wgpu::TextureViewDimension::D2 =>
                    {
                        textures.depth_solid.clone()
                    }
                    (false, Source::Atlas) if s.dimension == wgpu::TextureViewDimension::D2 => {
                        match textures.atlas {
                            Some(v) => v.clone(),
                            None => blank_view(device, queue, blank, s, BlankKind::Color(true)),
                        }
                    }
                    (false, Source::Lightmap) if s.dimension == wgpu::TextureViewDimension::D2 => {
                        match textures.lightmap {
                            Some(v) => v.clone(),
                            None => blank_view(device, queue, blank, s, BlankKind::Color(true)),
                        }
                    }
                    (false, Source::FlatNormal) => {
                        blank_view(device, queue, blank, s, BlankKind::FlatNormal)
                    }
                    (false, Source::Shadow) => blank_view(device, queue, blank, s, BlankKind::Far),
                    (false, Source::Noise) if s.dimension == wgpu::TextureViewDimension::D2 => {
                        common.noise.clone()
                    }
                    (false, Source::Blank(white)) => {
                        blank_view(device, queue, blank, s, BlankKind::Color(white))
                    }
                    _ => blank_view(device, queue, blank, s, BlankKind::Color(false)),
                }
            };
            let sampler = match (s.compare, s.source) {
                _ if s.shared.is_some() => match s.shared.as_deref() {
                    Some("iris_compare") => &common.compare,
                    Some("iris_nearest") => &common.nearest,
                    Some("iris_repeat") => &common.repeat,
                    _ => &common.linear,
                },
                (true, _) => &common.compare,
                _ if s.signed.is_some() => &common.nearest,
                (false, Source::Noise) => &common.repeat,
                (false, Source::Depth | Source::DepthSolid | Source::Shadow) => &common.nearest,
                (false, Source::Atlas) => &common.atlas,
                (false, Source::Target(i)) => {
                    match targets.get(&i).map(|t| sample_type(t.format)) {
                        Some(wgpu::TextureSampleType::Float { filterable: true }) | None => {
                            &common.linear
                        }
                        _ => &common.nearest,
                    }
                }
                _ => &common.linear,
            };
            views.push((s.texture, view, s.sampler, sampler));
        }
        let mut entries = Vec::new();
        if let Some((_, buffer)) = &self.uniform {
            entries.push(wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            });
        }
        let mut sampler_bindings = BTreeSet::new();
        for (texture, view, sampler_binding, sampler) in &views {
            entries.push(wgpu::BindGroupEntry {
                binding: *texture,
                resource: wgpu::BindingResource::TextureView(view),
            });
            if !sampler_bindings.insert(*sampler_binding) {
                continue;
            }
            entries.push(wgpu::BindGroupEntry {
                binding: *sampler_binding,
                resource: wgpu::BindingResource::Sampler(sampler),
            });
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&self.program),
            layout: &self.layout,
            entries: &entries,
        })
    }

    /// This frame's values of its uniforms.
    fn write_uniforms(&self, queue: &wgpu::Queue, inputs: &Inputs) {
        let Some((block_size, buffer)) = &self.uniform else {
            return;
        };
        let mut bytes = vec![0u8; *block_size as usize];
        for (name, slots) in &self.uniforms {
            if let Some(values) = inputs.value(name, self.geometry.is_some()) {
                for (&(offset, int), value) in slots.iter().zip(values) {
                    let at = offset as usize..offset as usize + 4;
                    bytes[at].copy_from_slice(&if int {
                        (value as i32).to_ne_bytes()
                    } else {
                        value.to_ne_bytes()
                    });
                }
            }
        }
        queue.write_buffer(buffer, 0, &bytes);
    }

    /// A geometry program: Horizon's terrain (solid or `translucent`) into
    /// its draw buffers on the side the next passes read, against `depth`.
    #[allow(clippy::too_many_arguments)]
    fn draw_geometry(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        bound: &Bound,
        group: &wgpu::BindGroup,
        depth: &wgpu::TextureView,
        inputs: &Inputs,
        world: &JavaWorld,
        translucent: bool,
    ) {
        let Some(flipped) = &self.geometry else {
            return;
        };
        self.write_uniforms(queue, inputs);
        // Targets of another size than the world's can't be drawn with it.
        let size = wgpu::Extent3d {
            width: bound.size[0].max(1),
            height: bound.size[1].max(1),
            depth_or_array_layers: 1,
        };
        let attachments: Vec<Option<wgpu::RenderPassColorAttachment>> = self
            .draw_buffers
            .iter()
            .map(|i| {
                let t = bound.targets.get(i).filter(|t| t.main.size() == size)?;
                let side = if flipped.contains(i) {
                    &t.alt_view
                } else {
                    &t.main_view
                };
                Some(color_attachment(side, None))
            })
            .collect();
        let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(&self.program),
            color_attachments: &attachments,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        (world.draw)(
            &mut render,
            translucent,
            &TerrainPipelines {
                instanced: &self.pipeline,
                flat: self.flat_pipeline.as_ref(),
                group,
            },
        );
    }

    /// Draw into the targets of `draw_buffers`, on the side it doesn't read.
    fn draw(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        bound: &Bound,
        group: &wgpu::BindGroup,
        common: &Common,
        inputs: &Inputs,
    ) {
        let views: Vec<Option<&wgpu::TextureView>> = self
            .draw_buffers
            .iter()
            .map(|i| {
                bound.targets.get(i).map(|t| {
                    if self.reads_alt.contains(i) {
                        &t.main_view
                    } else {
                        &t.alt_view
                    }
                })
            })
            .collect();
        let size = self
            .draw_buffers
            .first()
            .and_then(|i| bound.targets.get(i))
            .map_or(bound.size, |t| {
                let s = t.main.size();
                [s.width, s.height]
            });
        self.draw_to(queue, encoder, &views, size, group, common, inputs);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_to(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        views: &[Option<&wgpu::TextureView>],
        size: [u32; 2],
        group: &wgpu::BindGroup,
        common: &Common,
        inputs: &Inputs,
    ) {
        self.write_uniforms(queue, inputs);
        let attachments: Vec<Option<wgpu::RenderPassColorAttachment>> = views
            .iter()
            .map(|v| v.map(|v| color_attachment(v, None)))
            .collect();
        let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(&self.program),
            color_attachments: &attachments,
            ..Default::default()
        });
        render.set_pipeline(&self.pipeline);
        render.set_bind_group(0, group, &[]);
        for (slot, &data) in self.attributes.iter().enumerate() {
            render.set_vertex_buffer(slot as u32, common.vertices[data].slice(..));
        }
        let [w, h] = size.map(|v| (v as f32 * self.scale).max(1.0));
        render.set_viewport(0.0, 0.0, w, h, 0.0, 1.0);
        render.draw(0..3, 0..1);
    }
}

enum BlankKind {
    Color(bool),
    /// A normal map's flat normal, (0.5, 0.5, 1).
    FlatNormal,
    /// An `R32Float` 1.0: the far plane.
    Far,
    /// A depth texture at the far plane, for comparison samplers.
    FarDepth,
}

/// A one-texel stand-in of the dimension `s` needs.
fn blank_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    keep: &mut Vec<wgpu::Texture>,
    s: &SamplerBinding,
    kind: BlankKind,
) -> wgpu::TextureView {
    use wgpu::TextureViewDimension as D;
    let (format, bytes): (wgpu::TextureFormat, Vec<u8>) = match (kind, s.signed) {
        (BlankKind::Color(_), Some(true)) => (wgpu::TextureFormat::R32Sint, vec![0; 4]),
        (BlankKind::Color(_), Some(false)) => (wgpu::TextureFormat::R32Uint, vec![0; 4]),
        (BlankKind::Color(white), None) => (
            wgpu::TextureFormat::Rgba8Unorm,
            vec![if white { 255 } else { 0 }; 4],
        ),
        (BlankKind::FlatNormal, _) => (wgpu::TextureFormat::Rgba8Unorm, vec![128, 128, 255, 255]),
        (BlankKind::Far, _) => (wgpu::TextureFormat::R32Float, 1.0f32.to_ne_bytes().to_vec()),
        (BlankKind::FarDepth, _) => (wgpu::TextureFormat::Depth32Float, Vec::new()),
    };
    let layers = match s.dimension {
        D::Cube | D::CubeArray => 6,
        _ => 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Java shader pack blank"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: match s.dimension {
            D::D1 => wgpu::TextureDimension::D1,
            D::D3 => wgpu::TextureDimension::D3,
            _ => wgpu::TextureDimension::D2,
        },
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    if bytes.is_empty() {
        // Depth can't be written by a copy: clear it to the far plane.
        let view = texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Java shader pack blank"),
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        queue.submit([encoder.finish()]);
    } else {
        for layer in 0..layers {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes.len() as u32),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(s.dimension),
        ..Default::default()
    });
    keep.push(texture);
    view
}

/// The uniform block of a translated program: its size and each member's
/// scalar slots.
fn uniform_block(module: &naga::Module) -> Option<(u32, Vec<(String, Vec<Slot>)>)> {
    let (_, var) = module
        .global_variables
        .iter()
        .find(|(_, v)| v.space == naga::AddressSpace::Uniform)?;
    let naga::TypeInner::Struct { members, span } = &module.types[var.ty].inner else {
        return None;
    };
    let members = members
        .iter()
        .map(|m| {
            let mut slots = Vec::new();
            scalar_slots(module, m.ty, m.offset, &mut slots);
            (m.name.clone().unwrap_or_default(), slots)
        })
        .collect();
    Some((*span, members))
}

/// The scalars of a value of type `ty` at `offset`: vectors in order,
/// matrices column by column, arrays element by element.
fn scalar_slots(
    module: &naga::Module,
    ty: naga::Handle<naga::Type>,
    offset: u32,
    out: &mut Vec<Slot>,
) {
    let int = |s: naga::Scalar| s.kind != naga::ScalarKind::Float;
    match &module.types[ty].inner {
        naga::TypeInner::Scalar(s) => out.push((offset, int(*s))),
        naga::TypeInner::Vector { size, scalar } => {
            out.extend((0..*size as u32).map(|i| (offset + 4 * i, int(*scalar))))
        }
        naga::TypeInner::Matrix { columns, rows, .. } => {
            let stride = if *rows == naga::VectorSize::Bi { 8 } else { 16 };
            for c in 0..*columns as u32 {
                out.extend((0..*rows as u32).map(|r| (offset + c * stride + 4 * r, false)));
            }
        }
        naga::TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(n),
            stride,
        } => {
            for i in 0..n.get() {
                scalar_slots(module, *base, offset + i * stride, out);
            }
        }
        _ => {}
    }
}

/// The vertex entry point's inputs: name, location and a vertex format
/// matching their scalar kind.
fn vertex_inputs(module: &naga::Module) -> Vec<(String, u32, wgpu::VertexFormat)> {
    let Some(entry) = module.entry_points.first() else {
        return Vec::new();
    };
    let format = |ty: naga::Handle<naga::Type>| {
        let kind = match &module.types[ty].inner {
            naga::TypeInner::Scalar(s) | naga::TypeInner::Vector { scalar: s, .. } => s.kind,
            _ => naga::ScalarKind::Float,
        };
        match kind {
            naga::ScalarKind::Sint => wgpu::VertexFormat::Sint32x4,
            naga::ScalarKind::Uint => wgpu::VertexFormat::Uint32x4,
            _ => wgpu::VertexFormat::Float32x4,
        }
    };
    let mut out = Vec::new();
    for arg in &entry.function.arguments {
        match (&arg.binding, &module.types[arg.ty].inner) {
            (Some(naga::Binding::Location { location, .. }), _) => out.push((
                arg.name.clone().unwrap_or_default(),
                *location,
                format(arg.ty),
            )),
            (None, naga::TypeInner::Struct { members, .. }) => {
                for m in members {
                    if let Some(naga::Binding::Location { location, .. }) = &m.binding {
                        out.push((m.name.clone().unwrap_or_default(), *location, format(m.ty)));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The game state Iris's uniforms describe, this frame.
struct Inputs {
    view: Mat4,
    /// GL's: near at -1, far at 1.
    projection: Mat4,
    camera: Vec3,
    previous: (Mat4, Mat4, Vec3),
    size: [f32; 2],
    time: f32,
    frame_time: f32,
    frames: u32,
    frame: FrameUniform,
    /// `eyeBrightness` and `eyeBrightnessSmooth`: block, sky light × 16.
    eye: [f32; 2],
    eye_smooth: [f32; 2],
    /// `wetness` (`JavaStages::wetness_smooth`).
    wetness: f32,
    /// `CelestialUniforms`, in view space.
    sun_position: Vec3,
    moon_position: Vec3,
    up_position: Vec3,
    /// `sunAngle` and `shadowAngle`, 0..1.
    sun_angle: f32,
    shadow_angle: f32,
    is_day: bool,
    /// The pack's custom uniforms this frame.
    custom: HashMap<String, Vec<f32>>,
    /// `atlasSize`, in geometry programs.
    atlas_size: [f32; 2],
}

/// `CelestialUniforms.getSunAngle`: the body's angle plus 90, in 0..360
/// (0 at sunrise for the sun).
fn celestial_angle(degrees: f32) -> f32 {
    let c = degrees + 90.0;
    if c < 0.0 {
        c + 360.0
    } else if c > 360.0 {
        c - 360.0
    } else {
        c
    }
}

impl Inputs {
    fn new(
        frame: &FrameUniform,
        size: [u32; 2],
        time: f32,
        frame_time: f32,
        frames: u32,
        previous: Option<(Mat4, Mat4, Vec3)>,
    ) -> Self {
        let view = Mat4::from_cols_array_2d(&frame.view);
        // Horizon's reversed depth (z/w from 1 near to 0 far) to GL's
        // (-1 near, 1 far): z' = w - 2z.
        let to_gl = Mat4::from_cols(
            Vec4::X,
            Vec4::Y,
            Vec4::new(0.0, 0.0, -2.0, 0.0),
            Vec4::new(0.0, 0.0, 1.0, 1.0),
        );
        let projection = to_gl * Mat4::from_cols_array_2d(&frame.projection);
        let camera = Vec4::from(frame.camera_position).truncate();
        let mut out = Self {
            view,
            projection,
            camera,
            previous: previous.unwrap_or((view, projection, camera)),
            size: size.map(|v| v as f32),
            time,
            frame_time,
            frames,
            frame: *frame,
            eye: [0.0; 2],
            eye_smooth: [0.0; 2],
            wetness: frame.weather[0],
            sun_position: Vec3::ZERO,
            moon_position: Vec3::ZERO,
            up_position: Vec3::ZERO,
            sun_angle: 0.0,
            shadow_angle: 0.0,
            is_day: true,
            custom: HashMap::new(),
            atlas_size: [0.0; 2],
        };
        out.set_world(&JavaFrame::default(), 0.0);
        out
    }

    /// The light at the eye and the sun and moon, tilted by the pack's
    /// `sunPathRotation` as Iris tilts them.
    fn set_world(&mut self, world: &JavaFrame, sun_path_rotation: f32) {
        self.eye = world.eye_light.map(|l| f32::from(l) * 16.0);
        self.eye_smooth = self.eye;
        // CelestialUniforms.getCelestialPosition: the sky's Y -90, the
        // tilt, the body's angle, then (0, 100, 0) as a point.
        let sky = self.view * Mat4::from_rotation_y((-90f32).to_radians());
        let body = |angle: f32| {
            let m = sky
                * Mat4::from_rotation_z(sun_path_rotation.to_radians())
                * Mat4::from_rotation_x(angle.to_radians());
            m.transform_point3(Vec3::new(0.0, 100.0, 0.0))
        };
        self.sun_position = body(world.sun_angle);
        self.moon_position = body(world.moon_angle);
        self.up_position = sky.transform_vector3(Vec3::new(0.0, 100.0, 0.0));
        let sun = celestial_angle(world.sun_angle);
        self.is_day = sun < 180.0;
        self.sun_angle = sun / 360.0;
        self.shadow_angle = if self.is_day {
            sun
        } else {
            celestial_angle(world.moon_angle)
        } / 360.0;
    }

    /// The value of uniform `name`, as its scalars in order: Iris's own,
    /// else the pack's custom uniform; `None` for one Horizon doesn't
    /// provide (it stays zero).
    fn value(&self, name: &str, geometry: bool) -> Option<Vec<f32>> {
        geometry
            .then(|| self.geometry(name))
            .flatten()
            .or_else(|| self.builtin(name))
            .or_else(|| self.custom.get(name).cloned())
    }

    /// The uniforms whose value differs in geometry programs: the
    /// camera's matrices (camera-relative positions, as Iris's terrain),
    /// the lightmap's texture matrix, the atlas's size.
    fn geometry(&self, name: &str) -> Option<Vec<f32>> {
        let mat = |m: Mat4| m.to_cols_array().to_vec();
        Some(match name {
            "iris_ModelViewMatrix" | "modelViewMatrix" => mat(self.view),
            "iris_ModelViewMatrixInverse" | "modelViewMatrixInverse" => mat(self.view.inverse()),
            "iris_ProjectionMatrix" | "projectionMatrix" => mat(self.projection),
            "iris_ProjectionMatrixInverse" | "projectionMatrixInverse" => {
                mat(self.projection.inverse())
            }
            "iris_ModelViewProjectionMatrix" => mat(self.projection * self.view),
            "iris_NormalMatrix" | "normalMatrix" => Mat3::from_mat4(self.view)
                .inverse()
                .transpose()
                .to_cols_array()
                .to_vec(),
            // Unit 1, the lightmap: 0..240 to texel centres (Iris's
            // `LIGHTMAP_TEXTURE_MATRIX`).
            "iris_TextureMatrix" => {
                let lightmap = Mat4::from_scale(Vec3::splat(1.0 / 256.0))
                    * Mat4::from_translation(Vec3::splat(8.0));
                (0..8)
                    .flat_map(|i| mat(if i == 1 { lightmap } else { Mat4::IDENTITY }))
                    .collect()
            }
            "textureMatrix" => mat(Mat4::IDENTITY),
            "chunkOffset" => vec![0.0; 3],
            "alphaTestRef" => vec![0.1],
            "atlasSize" => self.atlas_size.to_vec(),
            "entityId" | "blockEntityId" | "currentRenderedItemId" => vec![-1.0],
            _ => return None,
        })
    }

    /// The value of Iris uniform `name`; `None` for one Horizon doesn't
    /// provide.
    fn builtin(&self, name: &str) -> Option<Vec<f32>> {
        if let Some(flag) = name.strip_prefix("iris_bool_") {
            return self.value(flag, false);
        }
        let f = &self.frame;
        let mat = |m: Mat4| m.to_cols_array().to_vec();
        let vec3 = |v: Vec3| v.to_array().to_vec();
        let day = f.time[2].rem_euclid(1.0);
        // The full-screen passes' own matrices: 0..1 onto the screen.
        let screen = Mat4::orthographic_rh_gl(0.0, 1.0, 0.0, 1.0, -1.0, 1.0);
        Some(match name {
            "gbufferModelView" => mat(self.view),
            "gbufferModelViewInverse" => mat(self.view.inverse()),
            "gbufferProjection" => mat(self.projection),
            "gbufferProjectionInverse" => mat(self.projection.inverse()),
            "gbufferPreviousModelView" => mat(self.previous.0),
            "gbufferPreviousProjection" => mat(self.previous.1),
            "cameraPosition" => vec3(self.camera),
            "previousCameraPosition" => vec3(self.previous.2),
            "cameraPositionInt" => vec3(self.camera.floor()),
            "cameraPositionFract" => vec3(self.camera - self.camera.floor()),
            "eyeAltitude" => vec![self.camera.y],
            "viewWidth" => vec![self.size[0]],
            "viewHeight" => vec![self.size[1]],
            "aspectRatio" => vec![self.size[0] / self.size[1].max(1.0)],
            "near" => vec![f.view_params[0]],
            "far" => vec![f.view_params[2]],
            "frameTimeCounter" => vec![self.time % 3600.0],
            "frameTime" => vec![self.frame_time],
            "frameCounter" => vec![(self.frames % 720_720) as f32],
            "worldTime" => vec![(day * 24000.0).floor()],
            "worldDay" => vec![0.0],
            "sunAngle" => vec![self.sun_angle],
            "shadowAngle" => vec![self.shadow_angle],
            "sunPosition" => vec3(self.sun_position),
            "moonPosition" => vec3(self.moon_position),
            "shadowLightPosition" => vec3(if self.is_day {
                self.sun_position
            } else {
                self.moon_position
            }),
            "upPosition" => vec3(self.up_position),
            "moonPhase" => vec![f.moon[3]],
            "rainStrength" => vec![f.weather[0]],
            "wetness" => vec![self.wetness],
            "thunderStrength" => vec![f.weather[1]],
            "fogColor" => f.fog_color[..3].to_vec(),
            "skyColor" => f.sky_color[..3].to_vec(),
            "isEyeInWater" => vec![f.camera_state[0]],
            "nightVision" => vec![f.camera_state[1]],
            "darknessFactor" => vec![f.camera_state[2]],
            "screenBrightness" => vec![1.0],
            "eyeBrightness" => self.eye.to_vec(),
            // An ivec2: the smoothed value truncated.
            "eyeBrightnessSmooth" => self.eye_smooth.map(f32::trunc).to_vec(),
            "hasSkylight" => vec![f.dimension[1]],
            "iris_ModelViewMatrix" | "iris_ModelViewMatrixInverse" => mat(Mat4::IDENTITY),
            "iris_ProjectionMatrix" | "iris_ModelViewProjectionMatrix" => mat(screen),
            "iris_ProjectionMatrixInverse" => mat(screen.inverse()),
            "iris_TextureMatrix" => (0..8).flat_map(|_| mat(Mat4::IDENTITY)).collect(),
            "iris_NormalMatrix" => Mat3::IDENTITY.to_cols_array().to_vec(),
            "shadowModelView"
            | "shadowModelViewInverse"
            | "shadowProjection"
            | "shadowProjectionInverse" => mat(Mat4::IDENTITY),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iris_formats_become_renderable_wgpu_formats() {
        use wgpu::TextureFormat as F;
        for (iris, wgpu) in [
            ("RGBA", F::Rgba8Unorm),
            ("RGB8", F::Rgba8Unorm),
            ("R8", F::R8Unorm),
            ("RG16F", F::Rg16Float),
            ("R11F_G11F_B10F", F::Rgba16Float),
            ("RGB8_SNORM", F::Rgba16Float),
            ("RGBA16", F::Rgba16Float),
            ("R32F", F::R32Float),
            ("RGBA32F", F::Rgba32Float),
            ("RGB10_A2", F::Rgb10a2Unorm),
            ("R32UI", F::R32Uint),
            ("RGBA16I", F::Rgba32Sint),
        ] {
            assert_eq!(target_format(iris, false), wgpu, "{iris}");
        }
        assert_eq!(target_format("RGBA16", true), F::Rgba16Unorm);
        assert_eq!(target_format("RG16_SNORM", true), F::Rg16Snorm);
        assert_eq!(target_format("RGBA16F", true), F::Rgba16Float);
        assert_eq!(
            sample_type(F::R32Float),
            wgpu::TextureSampleType::Float { filterable: false }
        );
    }

    #[test]
    fn samplers_read_what_iris_binds() {
        assert_eq!(source("colortex4", false), Source::Target(4));
        assert_eq!(source("gaux1", false), Source::Target(4));
        assert_eq!(source("gdepthtex", false), Source::Depth);
        assert_eq!(source("shadowtex1", false), Source::Shadow);
        assert_eq!(source("dhDepthTex0", false), Source::Shadow);
        assert_eq!(source("shadowcolor0", false), Source::Blank(true));
        assert_eq!(source("cloudTexture", false), Source::Blank(false));
        // Before translucent terrain; geometry programs' own textures.
        assert_eq!(source("depthtex1", false), Source::DepthSolid);
        assert_eq!(source("gtexture", true), Source::Atlas);
        assert_eq!(source("texture", true), Source::Atlas);
        assert_eq!(source("lightmap", true), Source::Lightmap);
        assert_eq!(source("normals", true), Source::FlatNormal);
        assert_eq!(source("colortex1", true), Source::Target(1));
        let (d, signed, shadow) = sampler_kind("usampler2D").unwrap();
        assert_eq!(
            (d, signed, shadow),
            (wgpu::TextureViewDimension::D2, Some(false), false)
        );
        assert!(sampler_kind("sampler2DShadow").unwrap().2);
        assert_eq!(
            sampler_kind("sampler3D").unwrap().0,
            wgpu::TextureViewDimension::D3
        );
    }

    #[test]
    fn celestial_and_eye_uniforms_follow_iris() {
        let frame = FrameUniform {
            view: Mat4::IDENTITY.to_cols_array_2d(),
            ..Default::default()
        };
        let close = |a: Option<Vec<f32>>, b: [f32; 3]| {
            let a = a.unwrap();
            assert!(
                a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3),
                "{a:?} != {b:?}"
            );
        };
        let mut inputs = Inputs::new(&frame, [300, 200], 0.0, 0.0, 0, None);
        // Sunrise: the sun in the east, at angle 0; the moon in the west.
        let world = |sun_angle: f32| JavaFrame {
            eye_light: [3, 15],
            sun_angle,
            moon_angle: sun_angle + 180.0,
        };
        inputs.set_world(&world(-90.0), 0.0);
        close(inputs.value("sunPosition", false), [100.0, 0.0, 0.0]);
        close(inputs.value("moonPosition", false), [-100.0, 0.0, 0.0]);
        close(inputs.value("upPosition", false), [0.0, 100.0, 0.0]);
        assert_eq!(inputs.value("sunAngle", false), Some(vec![0.0]));
        assert_eq!(
            inputs.value("eyeBrightness", false),
            Some(vec![48.0, 240.0])
        );
        // Complementary Unbound's sunPathRotation of -40 at noon: the sun
        // where its GetSunVector puts it, (0, cos 40, sin 40).
        let tilt = (-40f32).to_radians();
        inputs.set_world(&world(0.0), -40.0);
        close(
            inputs.value("sunPosition", false),
            [0.0, 100.0 * tilt.cos(), -100.0 * tilt.sin()],
        );
        assert_eq!(inputs.value("sunAngle", false), Some(vec![0.25]));
        close(inputs.value("shadowLightPosition", false), {
            let s = inputs.sun_position;
            [s.x, s.y, s.z]
        });
        // Night: the moon casts the shadows.
        inputs.set_world(&world(150.0), 0.0);
        assert!(!inputs.is_day);
        assert_eq!(
            inputs.value("shadowLightPosition", false),
            inputs.value("moonPosition", false)
        );
        assert!((inputs.value("shadowAngle", false).unwrap()[0] - 60.0 / 360.0).abs() < 1e-6);
        // Custom uniforms come after Iris's own.
        inputs.custom.insert("eyeBrightnessM".into(), vec![0.5]);
        inputs.custom.insert("sunAngle".into(), vec![9.0]);
        assert_eq!(inputs.value("eyeBrightnessM", false), Some(vec![0.5]));
        assert_ne!(inputs.value("sunAngle", false), Some(vec![9.0]));
    }

    #[test]
    fn gl_projection_from_reversed_depth() {
        // A point at the near plane is GL's -1, at the far plane 1.
        let (near, far) = (0.05, 512.0);
        let frame = FrameUniform {
            view: Mat4::IDENTITY.to_cols_array_2d(),
            projection: Mat4::perspective_rh(1.2, 1.5, far, near).to_cols_array_2d(),
            ..Default::default()
        };
        let inputs = Inputs::new(&frame, [300, 200], 0.0, 0.0, 0, None);
        let depth = |d: f32| {
            let clip = inputs.projection * Vec4::new(0.0, 0.0, -d, 1.0);
            clip.z / clip.w
        };
        assert!((depth(near) + 1.0).abs() < 1e-4, "{}", depth(near));
        assert!((depth(far) - 1.0).abs() < 1e-3, "{}", depth(far));
        assert_eq!(inputs.value("viewWidth", false), Some(vec![300.0]));
        assert_eq!(
            inputs.value("iris_bool_hasSkylight", false),
            inputs.value("hasSkylight", false)
        );
    }
}
