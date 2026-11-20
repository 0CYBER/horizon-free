//! The post stage of Horizon's own shader packs (docs/shaders.md): each `[[post]]` pass is a
//! full-screen pipeline built from its checked naga module (`horizon_shader_api::check`), with
//! the pack's options as pipeline constants. A pass reads bind group 3: the frame, the world's
//! colour and depth, the previous pass's output and two samplers. The passes read and write
//! linear light in half-float targets, so values over 1 carry to the next; the tonemap stage
//! draws the last one's picture onto the screen. Every stage of a pack reads the same `frame`,
//! written once a frame (`NativeFrame`).

use glam::{Mat4, Vec3};
use horizon_shader_api::{
    Frame as FrameInputs, Primaries, StageKind, check::CheckedStage, stages::GROUP,
};

use super::Frame;

pub(super) const INTERMEDIATE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[path = "native_kept.rs"]
pub(super) mod kept;
/// The scene the post passes of a pack with `[ray_tracing]` trace.
#[path = "native_trace.rs"]
pub(super) mod trace;

/// What the renderer's own entries call where a stage's colours cross into and out of it:
/// vanilla's 2.2 power in, as the prelude's `horizon_decode`, and out as the frame stores them:
/// encoded, as `horizon_encode`, or where the frame holds linear light (`linear`,
/// `[frame] blend = "linear"`) as they are. The shadow pass has no prelude, so these stand alone.
pub fn line_helpers(linear: bool) -> &'static str {
    if linear {
        "fn horizon_line_in(c: vec3<f32>) -> vec3<f32> {
    return pow(select(vec3<f32>(0.0), c, c > vec3<f32>(0.0)), vec3<f32>(2.2));
}
fn horizon_line_out(c: vec3<f32>) -> vec3<f32> { return c; }
"
    } else {
        "fn horizon_line_in(c: vec3<f32>) -> vec3<f32> {
    return pow(select(vec3<f32>(0.0), c, c > vec3<f32>(0.0)), vec3<f32>(2.2));
}
fn horizon_line_out(c: vec3<f32>) -> vec3<f32> {
    return pow(select(vec3<f32>(0.0), c, c > vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}
"
    }
}

/// The decoding copy: a picture of the frame into linear light in `INTERMEDIATE`, of the same
/// size, alpha 1 (`scene_color`, `opaque_color`, the mirror). A frame of vanilla's gamma is
/// decoded; one that holds linear light (`linear`) is copied as it is.
#[derive(Clone)]
pub struct Decoder(wgpu::RenderPipeline);

impl Decoder {
    pub(super) fn new(device: &wgpu::Device, linear: bool) -> Self {
        let read = if linear { "c" } else { "horizon_line_in(c)" };
        let source = line_helpers(false).to_owned()
            + &format!(
                "@group(0) @binding(0) var horizon_from: texture_2d<f32>;
@vertex fn horizon_vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
    return vec4<f32>(vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0, 0.0, 1.0);
}}
@fragment fn horizon_fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {{
    let c = textureLoad(horizon_from, vec2<i32>(p.xy), 0).rgb;
    return vec4<f32>({read}, 1.0);
}}
"
            );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader API 0.2 decode"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let entries = (None, "horizon_vs", "horizon_fs");
        Self(fullscreen_pipeline(
            device,
            "Shader API 0.2 decode",
            &module,
            entries,
            INTERMEDIATE,
            &[],
        ))
    }

    /// Draws `from` decoded onto `to`.
    pub fn run(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        from: &wgpu::TextureView,
        to: &wgpu::TextureView,
    ) {
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader API 0.2 decode"),
            layout: &self.0.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(from),
            }],
        });
        fullscreen_pass(
            encoder,
            "Shader API 0.2 decode",
            to,
            &self.0,
            (0, &group),
            None,
        );
    }
}

/// A full-screen pipeline `label` of `module`'s entries `(vertex, fragment)`, unblended.
fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    (layout, vertex, fragment): (Option<&wgpu::PipelineLayout>, &str, &str),
    format: wgpu::TextureFormat,
    constants: &[(&str, f64)],
) -> wgpu::RenderPipeline {
    let options = wgpu::PipelineCompilationOptions {
        constants,
        ..Default::default()
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout,
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vertex),
            compilation_options: options.clone(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            compilation_options: options,
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// One full-screen triangle of `pipeline` onto `view`, cleared first.
fn fullscreen_pass(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    view: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    (index, group): (u32, &wgpu::BindGroup),
    timing: Option<&mut super::pack_timing::PassTiming>,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        timestamp_writes: timing.and_then(|t| t.writes(label)),
        ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(index, group, &[]);
    pass.draw(0..3, 0..1);
}

/// The tonemap stage's entries as the renderer draws them: the check's, with the result
/// encoded into the frame's gamma, where the camera overlays and the HUD draw on it. A pack's
/// `[tonemap]` is held to `frame.headroom`. Horizon's own (`default`) is a clip at white in
/// SDR; in HDR it leaves the light as it is for Horizon's tone curve (`Hdr::tonemap`), the
/// one HDR without a pack uses.
fn tonemap_entries(default: bool) -> String {
    let held = if default {
        "select(min(horizon_sanitize(out), vec3<f32>(frame.headroom)), horizon_sanitize(out), frame.hdr == 1u)"
    } else {
        "min(horizon_sanitize(out), vec3<f32>(frame.headroom))"
    };
    StageKind::Tonemap.entries().replace(
        "return vec4<f32>(min(horizon_sanitize(out), vec3<f32>(frame.headroom)), 1.0);",
        &format!("return vec4<f32>(horizon_encode({held}), 1.0);"),
    )
}

/// The tonemap of a pack without `[tonemap]`: the picture as it is, held as
/// `tonemap_entries` holds Horizon's.
const DEFAULT_TONEMAP: &str =
    "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color; }\n";

/// Whether TAA may upscale the world under a pack without a post chain whose `[tonemap]` is
/// `tonemap` (`None`: Horizon's, which reads only `input.color`). Upscaled, `previous` is
/// larger than the world, the size it promises the tonemap: only a tonemap that never reads
/// it can't tell.
pub(super) fn tonemap_lets_taa_upscale(tonemap: Option<&CheckedStage>) -> bool {
    tonemap.is_none_or(|s| !s.reads("previous"))
}

/// A colour target of `INTERMEDIATE` and `size`, drawn and read.
fn intermediate(device: &wgpu::Device, label: &str, size: [u32; 2]) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: INTERMEDIATE,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

/// A pack's display (docs/api/shaders-reference-0.2.md): the world decoded into
/// `scene_color` (the first pass's `previous`), the chain ending in `picture`, and the tonemap
/// stage drawing that onto the screen.
struct Display {
    decoder: Decoder,
    /// The pack's `[tonemap]` label, `None` for Horizon's.
    label: Option<String>,
    tonemap: wgpu::RenderPipeline,
    /// `scene_color` at the world's size, or without a chain at the size of what it decodes
    /// (`WorldPost::display`); `picture` at the window's (none without a chain: the tonemap
    /// reads `scene_color`).
    scene: Option<([u32; 2], wgpu::TextureView)>,
    picture: Option<([u32; 2], wgpu::TextureView)>,
}

/// The world's colour and depth views a set of bind groups was made for.
type WorldViews = (wgpu::TextureView, wgpu::TextureView, wgpu::TextureView);

/// Where a post pass draws.
#[derive(Clone, Copy, PartialEq)]
enum Writes {
    /// One of the chain's two targets.
    Chain(usize),
    /// A pack target (`[[post]] output`).
    Target(usize),
    /// A kept texture (`[[post]] output` naming one): this frame's version, at its size.
    Kept(usize),
    /// The chain's picture, at the window's size: the last pass.
    Picture,
}

/// A post pass's target, and the chain's picture it reads as `previous` (the world's for
/// none).
#[derive(Clone, Copy)]
struct PassPlan {
    writes: Writes,
    reads: Option<usize>,
}

/// The passes' plan for `outputs` (where each pass's `[[post]] output` draws): the chain's
/// passes write its targets in turn, each reading what the one before wrote; a pass with an
/// output writes that and reads the chain's picture, which goes on past it.
fn plan(outputs: &[Option<Writes>]) -> Vec<PassPlan> {
    let mut current = None;
    outputs
        .iter()
        .enumerate()
        .map(|(i, output)| {
            let reads = current;
            let writes = match output {
                _ if i + 1 == outputs.len() => Writes::Picture,
                Some(w) => *w,
                None => {
                    let slot = usize::from(current == Some(0));
                    current = Some(slot);
                    Writes::Chain(slot)
                }
            };
            PassPlan { writes, reads }
        })
        .collect()
}

/// A pack target's size: `scale` of the world's `size`, at least a texel.
fn scaled(size: [u32; 2], scale: f32) -> [u32; 2] {
    size.map(|s| ((s as f32 * scale).ceil() as u32).max(1))
}

pub struct NativePost {
    passes: Vec<(String, wgpu::RenderPipeline)>,
    layout: wgpu::BindGroupLayout,
    /// `layout`'s entries, which the scene's join (`trace`), the passes built against the
    /// scene where the device traces rays, and the scene's generation `groups` hold (none
    /// while they hold no scene).
    entries: Vec<wgpu::BindGroupLayoutEntry>,
    trace: Option<trace::NativeTrace>,
    traced: Option<u64>,
    /// The pack's textures.
    textures: Vec<(wgpu::TextureView, wgpu::Sampler)>,
    frame: wgpu::Buffer,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    /// Two targets of the world's size, written in turn (none with a single pass).
    targets: Option<([u32; 2], [wgpu::TextureView; 2])>,
    /// The frame's shadow map, its comparison sampler (bindings 6 and 7), the translucent map
    /// and its colour (bindings 9 and 10).
    shadow: (
        wgpu::TextureView,
        wgpu::Sampler,
        wgpu::TextureView,
        wgpu::TextureView,
    ),
    /// A black 1×1, in place of the pack's target a pass writes.
    blank: wgpu::Texture,
    /// One bind group per pass, for these world views.
    groups: Option<(WorldViews, Vec<wgpu::BindGroup>)>,
    /// What each pass writes and reads (`[[post]] output`).
    plan: Vec<PassPlan>,
    /// The pack's targets' scales (`Manifest::post_targets`), and their views for the world's
    /// size.
    scales: Vec<f32>,
    named: Option<([u32; 2], Vec<wgpu::TextureView>)>,
    /// The pack's `[[resource]]`s, sized to the world (none without).
    kept: Option<kept::KeptStore>,
    /// The screen's colour format, the world target's too.
    format: wgpu::TextureFormat,
    /// The decode, the picture and the tonemap.
    display: Display,
    /// `tonemap_lets_taa_upscale` of its `[tonemap]`.
    upscalable: bool,
}

/// The `frame` every stage of a pack reads (group 3, binding 0), written once a frame, and
/// what the material stage reads besides.
pub struct NativeFrame {
    /// The solid terrain's second target, the post stage's `scene_surface`.
    surface: wgpu::Texture,
    pub surface_view: wgpu::TextureView,
    pub(super) buffer: wgpu::Buffer,
    /// Group 3 of the sky and material stages, and its entries (a material stage that traces
    /// joins them with the scene's).
    pub layout: wgpu::BindGroupLayout,
    entries: Vec<wgpu::BindGroupLayoutEntry>,
    /// The pack's `[[textures]]` and their samplers, bound for every stage.
    pub textures: Vec<(wgpu::TextureView, wgpu::Sampler)>,
    /// The colour of block light around the camera (`[[lights]]`), bindings 9 and 10.
    pub block_light: super::block_light::BlockLight,
    /// The light sources a `[ray_tracing]` pack's rays aim at, where its files name them
    /// (`NativePack::build` adds them).
    pub ray_lights: Option<super::ray_lights::RayLights>,
    pub group: wgpu::BindGroup,
    /// The world's depth before translucent terrain (the material stage's `opaque_depth`,
    /// binding 6), copied each frame.
    pub opaque_depth: wgpu::Texture,
    /// The world's colour before translucent terrain (`opaque_color`, binding 11), copied
    /// with it; 1×1 while the world draws straight to the window.
    pub opaque_color: wgpu::Texture,
    /// The resource packs' LabPBR maps (bindings 12 and 13: normals, specular), a blank 1x1
    /// without them.
    pub(super) pbr: [wgpu::TextureView; 2],
    /// The material stage's `reflection` (binding 1): this frame's water mirror, else
    /// `blank`; and its sampler (binding 2).
    reflection: wgpu::TextureView,
    blank: wgpu::TextureView,
    reflection_sampler: wgpu::Sampler,
    /// The shadow map the stages read, and the camera that draws it when the pack asks for one.
    pub shadow_map: super::native_shadow::ShadowMap,
    shadow_camera: Option<horizon_shaders::Shadow>,
    /// Whether the pack's material reads `reflection` (`NativePack::new`): else no mirror
    /// is drawn and `has_reflection` stays 0.
    pub reflects: bool,
    /// Whether the pack's material reads `opaque_depth` or `opaque_color`
    /// (`NativePack::new`): else they stay 1×1 and unwritten, and the world pass doesn't break
    /// before translucent terrain to copy them.
    pub reads_opaque: bool,
    /// Last frame's view-projection, camera, time and jitter (`write`).
    last: Option<LastFrame>,
    /// The eased eye light (block, sky) and being in water and lava (`write`); NaN until the
    /// first frame sets them.
    eye: [f32; 4],
    /// `frame.ray_distance` while the pack traces this frame's scene (`NativePack::write`),
    /// set before `write`: `frame.has_ray_tracing` is 1 then.
    pub(super) ray_distance: Option<f32>,
    /// `frame.full_lightmap`, set before `write` from this frame's lightmap (`full_lightmap`).
    pub(super) full_lightmap: [f32; 3],
    /// `frame.jitter`, set before `write` from the world's jitter this frame.
    pub(super) jitter: [f32; 2],
    /// The lens the world is drawn with this frame (`FrameStart::projection`), set before
    /// `write`: `frame.projection` is it times the pose.
    pub(super) lens: Mat4,
    /// A pack with kept resources: their epochs (`NativePack::carry` moves them on).
    pub(super) epochs: Option<kept::Epochs>,
    start: std::time::Instant,
    frames: u32,
    /// `frame.wetness`, and when it was last written.
    wetness: (f32, Option<std::time::Instant>),
    /// Whether the world's frame holds linear light for the pack (`[frame] blend = "linear"`),
    /// the decoding copy and the mirror it decoded (`linear_mirror`).
    pub linear: bool,
    pub decoder: Decoder,
    mirror: Option<wgpu::TextureView>,
}

/// A depth texture of `size` the world's depth is copied into.
fn depth_copy(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    copy_target(
        device,
        "Native shader pack opaque depth",
        size,
        wgpu::TextureFormat::Depth32Float,
    )
}

/// A colour texture of `size` and `format` the world's colour is copied into.
fn color_copy(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    copy_target(
        device,
        "Native shader pack opaque colour",
        size,
        INTERMEDIATE,
    )
}

fn copy_target(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
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
        // RENDER_ATTACHMENT: `opaque_color` is drawn decoded rather than copied.
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

/// The uniform `frame` at binding 0, read by `visibility`.
fn frame_entry(visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size_of::<FrameInputs>() as u64),
        },
        count: None,
    }
}

/// The size of the water mirror for a world of `size`: half of it.
pub fn reflection_size(size: [u32; 2]) -> [u32; 2] {
    size.map(|s| s.div_ceil(2).max(1))
}

/// Seconds between frames past which the last one is forgotten (`NativeFrame::write`).
const MAX_FRAME_GAP: f32 = 0.5;

/// `value` moved toward `target` over `dt` seconds by a half-life in seconds; a NaN `value`
/// (the first frame) starts at `target`.
fn eased(value: f32, target: f32, half_life: f32, dt: f32) -> f32 {
    if value.is_nan() {
        target
    } else {
        target + (value - target) * 0.5f32.powf(dt / half_life)
    }
}

impl NativeFrame {
    /// The frame of a pack with the `[shadow]` settings `shadow`, if any, and the block light
    /// colours `lights`, onto a world's frame that holds linear light when `linear`.
    pub fn new(
        device: &wgpu::Device,
        shadow: Option<&horizon_shader_api::manifest::Shadow>,
        textures: Vec<(wgpu::TextureView, wgpu::Sampler)>,
        lights: super::block_light::LightTable,
        pbr: Option<&[wgpu::Texture; 2]>,
        linear: bool,
    ) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Native shader pack frame"),
            size: size_of::<FrameInputs>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut entries = vec![
            frame_entry(wgpu::ShaderStages::VERTEX_FRAGMENT),
            texture_entry(1, wgpu::TextureSampleType::Float { filterable: true }),
            sampler_entry(2),
            texture_entry(3, wgpu::TextureSampleType::Depth),
            comparison_entry(4),
            texture_entry(6, wgpu::TextureSampleType::Depth),
            texture_entry(7, wgpu::TextureSampleType::Depth),
            texture_entry(8, wgpu::TextureSampleType::Float { filterable: true }),
            texture_entry(11, wgpu::TextureSampleType::Float { filterable: false }),
            pbr_entry(12),
            pbr_entry(13),
        ];
        let block_light_entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 9,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 10,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        entries.extend(block_light_entries);
        entries.extend(pack_texture_entries(textures.len()));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Native shader pack frame"),
            entries: &entries,
        });
        let surface = surface_target(device, [1, 1]);
        let surface_view = surface.create_view(&Default::default());
        let opaque_depth = depth_copy(device, [1, 1]);
        let opaque_color = color_copy(device, [1, 1]);
        let blank = copy_target(
            device,
            "Native shader pack reflection blank",
            [1, 1],
            wgpu::TextureFormat::Rgba8Unorm,
        )
        .create_view(&Default::default());
        let reflection_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Native shader pack reflection sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shadow_map = super::native_shadow::ShadowMap::new(device, shadow);
        let block_light = super::block_light::BlockLight::new(device, lights);
        let array_view = |texture: &wgpu::Texture| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let pbr = match pbr {
            Some(maps) => maps.each_ref().map(array_view),
            None => {
                // Two layers: one would make a 2D texture on the OpenGL backend.
                let blank = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Native shader pack LabPBR (none)"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 2,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                [array_view(&blank), array_view(&blank)]
            }
        };
        let group = Self::group(
            device,
            &layout,
            &buffer,
            (&blank, &reflection_sampler),
            &shadow_map,
            (&opaque_depth, &opaque_color),
            &textures,
            &block_light,
            &pbr,
            &[],
        );
        Self {
            textures,
            block_light,
            ray_lights: None,
            pbr,
            buffer,
            layout,
            entries,
            group,
            surface,
            surface_view,
            opaque_depth,
            opaque_color,
            reflection: blank.clone(),
            blank,
            reflection_sampler,
            shadow_map,
            shadow_camera: shadow.map(super::native_shadow::camera),
            reflects: false,
            reads_opaque: false,
            linear,
            decoder: Decoder::new(device, linear),
            mirror: None,
            last: None,
            eye: [f32::NAN; 4],
            ray_distance: None,
            full_lightmap: [0.0; 3],
            jitter: [0.0; 2],
            lens: Mat4::IDENTITY,
            epochs: None,
            start: std::time::Instant::now(),
            frames: 0,
            wetness: (0.0, None),
        }
    }

    /// What the shadow camera sees this frame, for the renderer's shadow lists.
    pub fn shadow_view(&self, frame: &Frame) -> Option<super::external::ShadowView> {
        let camera = self.shadow_camera.as_ref()?;
        Some(super::external::ShadowView {
            cull: super::pack_uniforms::shadow_culling(frame, camera),
            light: super::pack_uniforms::shadow_light(frame, camera),
            // The light's box culls: on the ground it reaches further than `distance` along
            // the sun's azimuth while the sun is low.
            distance: None,
            unculled: None,
            every_face: false,
            view_casters: false,
            depth_planes: true,
        })
    }

    fn group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buffer: &wgpu::Buffer,
        (reflection, sampler): (&wgpu::TextureView, &wgpu::Sampler),
        shadow: &super::native_shadow::ShadowMap,
        (opaque_depth, opaque_color): (&wgpu::Texture, &wgpu::Texture),
        textures: &[(wgpu::TextureView, wgpu::Sampler)],
        block_light: &super::block_light::BlockLight,
        pbr: &[wgpu::TextureView; 2],
        scene: &[wgpu::BindGroupEntry<'_>],
    ) -> wgpu::BindGroup {
        let opaque_depth = opaque_depth.create_view(&Default::default());
        let opaque_color = opaque_color.create_view(&Default::default());
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(reflection),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&shadow.view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&shadow.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&shadow.translucent_view),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::TextureView(&shadow.color_view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&opaque_depth),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: wgpu::BindingResource::TextureView(&opaque_color),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: wgpu::BindingResource::TextureView(&pbr[0]),
            },
            wgpu::BindGroupEntry {
                binding: 13,
                resource: wgpu::BindingResource::TextureView(&pbr[1]),
            },
        ];
        entries.extend([
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(&block_light.view),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: block_light.grid.as_entire_binding(),
            },
        ]);
        entries.extend(pack_texture_group_entries(textures));
        entries.extend_from_slice(scene);
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Native shader pack frame"),
            layout,
            entries: &entries,
        })
    }

    /// Keeps the opaque copies the size of the world's depth `depth` and, where the world's
    /// colour `color` can be copied (the scaled target, not the window's own), its size and
    /// format; the group is made again when they change. Nothing for a pack that doesn't read
    /// them.
    pub fn fit_opaque(
        &mut self,
        device: &wgpu::Device,
        depth: &wgpu::Texture,
        color: Option<&wgpu::Texture>,
    ) {
        if !self.reads_opaque {
            return;
        }
        let size = [depth.width(), depth.height()];
        let color_size = color.map_or([1, 1], |c| [c.width(), c.height()]);
        let color_fits = [self.opaque_color.width(), self.opaque_color.height()] == color_size;
        let depth_fits = [self.opaque_depth.width(), self.opaque_depth.height()] == size;
        if depth_fits && color_fits {
            return;
        }
        if !color_fits {
            self.opaque_color = color_copy(device, color_size);
        }
        if !depth_fits {
            self.opaque_depth = depth_copy(device, size);
        }
        self.regroup(device);
    }

    /// Fills `opaque_color` from the world's colour `color`, decoded.
    pub fn copy_opaque(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
    ) {
        let to = self.opaque_color.create_view(&Default::default());
        self.decoder.run(device, encoder, color, &to);
    }

    /// The water mirror `view` as the material reads it: decoded into a copy.
    pub fn linear_mirror(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        view: wgpu::TextureView,
    ) -> wgpu::TextureView {
        let size = [view.texture().width(), view.texture().height()];
        let fits = |m: &wgpu::TextureView| [m.texture().width(), m.texture().height()] == size;
        if !self.mirror.as_ref().is_some_and(fits) {
            self.mirror = Some(intermediate(device, "Water mirror, linear", size));
        }
        let mirror = self.mirror.clone().unwrap();
        self.decoder.run(device, encoder, &view, &mirror);
        mirror
    }

    /// Sizes the surface target: the world's while a material stage writes it, else a blank.
    pub fn fit_surface(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if [self.surface.width(), self.surface.height()] != size {
            self.surface = surface_target(device, size);
            self.surface_view = self.surface.create_view(&Default::default());
        }
    }

    /// Binds `view` as this frame's `reflection` (`None`: the blank). The mirror is drawn
    /// before the stages read it.
    pub fn set_reflection(&mut self, device: &wgpu::Device, view: Option<&wgpu::TextureView>) {
        let view = view.unwrap_or(&self.blank);
        if *view != self.reflection {
            self.reflection = view.clone();
            self.regroup(device);
        }
    }

    /// Group 3's layout entries (`layout`), which a material stage's own layouts start with.
    pub(in crate::gfx) fn entries(&self) -> &[wgpu::BindGroupLayoutEntry] {
        &self.entries
    }

    fn regroup(&mut self, device: &wgpu::Device) {
        self.group = Self::group(
            device,
            &self.layout,
            &self.buffer,
            (&self.reflection, &self.reflection_sampler),
            &self.shadow_map,
            (&self.opaque_depth, &self.opaque_color),
            &self.textures,
            &self.block_light,
            &self.pbr,
            &[],
        );
    }

    /// Writes this frame's inputs, for a world of `size` drawn for `display`
    /// (`EffectiveOutput::display`).
    pub(crate) fn write(
        &mut self,
        queue: &wgpu::Queue,
        frame: &Frame,
        size: [u32; 2],
        display: super::output::DisplayValues,
    ) {
        let inputs = self.next_inputs(frame, size, display);
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&inputs));
    }

    /// This frame's inputs, moving the eased values and the frame count on.
    fn next_inputs(
        &mut self,
        frame: &Frame,
        size: [u32; 2],
        display: super::output::DisplayValues,
    ) -> FrameInputs {
        // Under `HORIZON_STILL` the world's ticks are the clock (`PackInputs::still_ticks`), and
        // eased values sit at their targets.
        // A scripted clock (`PackInputs::frame_timing`) comes first, and eased values
        // move by its delta.
        let now = std::time::Instant::now();
        let timing = frame.pack.frame_timing;
        // Seconds, frame index, and the seconds eased values move by when not the
        // measured ones.
        let (seconds, index, step) = match (timing, frame.pack.still_ticks) {
            (Some(t), _) => (t.elapsed, t.counter, Some(t.delta)),
            (None, Some(ticks)) => (
                (ticks as f32 / 20.0) % 3600.0,
                ticks as u32,
                Some(f32::INFINITY),
            ),
            (None, None) => (
                self.start.elapsed().as_secs_f32() % 3600.0,
                self.frames,
                None,
            ),
        };
        let since = step.unwrap_or_else(|| self.wetness.1.map_or(0.0, |t| (now - t).as_secs_f32()));
        self.wetness = (wetness(self.wetness.0, frame.pack.rain, since), Some(now));
        let shadow = self.shadow_camera.as_ref();
        let mut inputs = frame_inputs(frame, size, self.lens, seconds, index, shadow);
        inputs.wetness = self.wetness.0;
        inputs.full_lightmap = self.full_lightmap;
        inputs.jitter = self.jitter;
        if let Some(distance) = self.ray_distance {
            (inputs.has_ray_tracing, inputs.ray_distance) = (1, distance);
        }
        inputs.sky_color = decoded(inputs.sky_color);
        inputs.fog_color = decoded(inputs.fog_color);
        if display.hdr {
            inputs.hdr = 1;
            inputs.headroom = display.headroom;
            inputs.paper_white = display.paper_white;
            inputs.peak = display.peak;
            inputs.black_level = display.black_level;
        }
        if !self.reflects {
            inputs.has_reflection = 0;
        }
        let view_proj = (Mat4::from_cols_array_2d(&inputs.projection)
            * Mat4::from_cols_array_2d(&inputs.view))
        .to_cols_array_2d();
        inputs.previous_view_projection = view_proj;
        let (last, mut paused) = (self.last, false);
        // `write` skips title frames, so a gap means a pause or another world: the frame
        // before says nothing about this one, and the eased values start over.
        // A scripted clock's frames follow each other however long one takes to draw.
        match carried(self.last, now, timing.is_some()) {
            Some((previous, camera, time, jitter)) => {
                inputs.previous_view_projection = previous;
                inputs.previous_jitter = jitter;
                inputs.camera_motion = (frame.camera - camera).as_vec3().to_array();
                inputs.delta_time = (now - time).as_secs_f32();
            }
            _ => {
                paused = last.is_some();
                self.eye = [f32::NAN; 4];
            }
        }
        self.last = Some((view_proj, frame.camera, now, inputs.jitter));
        if let Some(epochs) = &mut self.epochs {
            let moved = last.map_or(0.0, |l| frame.camera.distance(l.1));
            [inputs.view_epoch, inputs.world_epoch, inputs.reset_reasons] =
                epochs.advance(size, inputs.hdr == 1, paused, moved);
        }
        // The eye's light and medium eased by half-lives in seconds, from where they are at
        // first.
        let targets = [
            (inputs.eye_light[0], 2.0),
            (inputs.eye_light[1], 2.0),
            (f32::from(u8::from(inputs.eye_in == 1)), 0.25),
            (f32::from(u8::from(inputs.eye_in == 2)), 0.25),
        ];
        let dt = step.unwrap_or(inputs.delta_time);
        inputs.delta_time = match (timing, step) {
            (Some(t), _) => t.delta,
            (None, Some(_)) => 0.05,
            (None, None) => inputs.delta_time,
        };
        for (value, (target, half_life)) in self.eye.iter_mut().zip(targets) {
            *value = eased(*value, target, half_life, dt);
        }
        let [block, sky, water, lava] = self.eye;
        inputs.eye_light_smooth = [block, sky];
        inputs.in_water = water;
        inputs.in_lava = lava;
        self.frames = self.frames.wrapping_add(1);
        inputs
    }

    /// Forgets the frames before, as a freshly loaded pack: eased values start at their
    /// targets, wetness dry, the frame count at 0.
    pub(super) fn reset_history(&mut self) {
        if let Some(epochs) = &mut self.epochs {
            epochs.note(horizon_shader_api::kept::REASON_RESET);
        }
        self.last = None;
        self.frames = 0;
        self.wetness = (0.0, None);
        self.start = std::time::Instant::now();
    }
}

/// A frame's view-projection, camera, time and `frame.jitter`, kept for the next one.
type LastFrame = ([[f32; 4]; 4], glam::DVec3, std::time::Instant, [f32; 2]);

/// `last` where it carries over into a frame drawn at `now`: always under a scripted clock,
/// else when it was drawn less than `MAX_FRAME_GAP` before. Without it the frame's
/// `previous_view_projection` is its own and `previous_jitter` zero, as after a reset.
fn carried(last: Option<LastFrame>, now: std::time::Instant, scripted: bool) -> Option<LastFrame> {
    last.filter(|&(_, _, time, _)| scripted || (now - time).as_secs_f32() < MAX_FRAME_GAP)
}

/// The option values of `constants` that `module` declares as overrides (wgpu refuses others).
pub fn present_constants<'a>(
    module: &naga::Module,
    constants: &'a [(String, f64)],
) -> Vec<(&'a str, f64)> {
    constants
        .iter()
        .filter(|(id, _)| {
            module
                .overrides
                .iter()
                .any(|(_, o)| o.name.as_ref() == Some(id))
        })
        .map(|(id, v)| (id.as_str(), *v))
        .collect()
}

/// The solid terrain's surface target of `size`, drawn and read.
fn surface_target(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Native shader pack surface"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: super::native_material::SURFACE_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// A LabPBR map's array, read by the material entries (`native_material.rs`).
fn pbr_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        // `[displacement]`'s grid moves its corners by the heights (gfx/displace.rs).
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}

fn comparison_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// A post pass's pipeline from its checked `module`, with the option values `constants`, drawing
/// a target of `format`.
fn post_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    module: naga::Module,
    constants: &[(String, f64)],
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let present = present_constants(&module, constants);
    let present: Vec<(&str, f64)> = present.iter().map(|(id, v)| (*id, *v)).collect();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&super::dx12_compiler::module_label(label)),
        source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
    });
    let options = wgpu::PipelineCompilationOptions {
        constants: &present,
        ..Default::default()
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("horizon_vs_post"),
            compilation_options: options.clone(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("horizon_fs_post"),
            compilation_options: options,
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl NativePost {
    /// Pipelines for `stages` (`[[post]]`'s entries in order, then `[[before_world]]`'s; the kept
    /// store runs the compute ones) with the option values `constants`, the last onto `picture`,
    /// and `tonemap` (else Horizon's) onto a screen of `format`. Errors name the pass wgpu
    /// refused.
    pub fn new(
        device: &wgpu::Device,
        stages: Vec<CheckedStage>,
        manifest: &horizon_shader_api::Manifest,
        constants: &[(String, f64)],
        format: wgpu::TextureFormat,
        frame: &NativeFrame,
        tonemap: Option<&CheckedStage>,
    ) -> Result<Self, String> {
        let targets = manifest.post_targets();
        let plan = plan(&kept::outputs(manifest, &targets));
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let mut entries = vec![
            frame_entry(wgpu::ShaderStages::FRAGMENT),
            texture_entry(1, float),
            texture_entry(2, wgpu::TextureSampleType::Depth),
            texture_entry(3, float),
            sampler_entry(4),
            sampler_entry(5),
            texture_entry(6, wgpu::TextureSampleType::Depth),
            comparison_entry(7),
            texture_entry(9, wgpu::TextureSampleType::Depth),
            texture_entry(10, float),
            texture_entry(11, wgpu::TextureSampleType::Uint),
        ];
        entries.extend(pack_texture_entries(frame.textures.len()));
        entries
            .extend((0..targets.len() as u32).map(|j| {
                texture_entry(horizon_shader_api::stages::POST_TARGET_BINDING + j, float)
            }));
        let (stages, kept) = kept::chain(device, manifest, stages, &entries, constants)?;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Native shader pack post"),
            entries: &entries,
        });
        let mut groups: [Option<&wgpu::BindGroupLayout>; GROUP as usize + 1] = Default::default();
        groups[GROUP as usize] = Some(&layout);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Native shader pack post"),
            bind_group_layouts: &groups,
            ..Default::default()
        });
        let mut passes = Vec::with_capacity(stages.len());
        for (i, stage) in stages.into_iter().enumerate() {
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
            let target = kept::target(&kept, plan[i].writes);
            let layout = kept::pass(&kept, Some(i)).map_or(&pipeline_layout, |p| &p.pipeline);
            let label = &stage.label;
            let pipeline = post_pipeline(device, label, layout, stage.module, constants, target);
            let internal = pollster::block_on(internal.pop());
            if let Some(e) = internal.or(pollster::block_on(validation.pop())) {
                return Err(format!("{}: {e}", stage.file));
            }
            passes.push((stage.label, pipeline));
        }
        let source = tonemap.map_or_else(
            || StageKind::Tonemap.prelude() + DEFAULT_TONEMAP,
            |s| s.source.clone(),
        ) + &tonemap_entries(tonemap.is_none());
        let file = tonemap.map_or("<tonemap>", |s| s.file.as_str());
        let module = naga::front::wgsl::parse_str(&source)
            .map_err(|e| format!("{file}: {}", e.emit_to_string(&source)))?;
        let present = present_constants(&module, constants);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Tonemap"),
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        });
        let stages = (
            Some(&pipeline_layout),
            "horizon_vs_tonemap",
            "horizon_fs_tonemap",
        );
        let pipeline = fullscreen_pipeline(device, "Tonemap", &module, stages, format, &present);
        let internal = pollster::block_on(internal.pop());
        if let Some(e) = internal.or(pollster::block_on(validation.pop())) {
            return Err(format!("{file}: {e}"));
        }
        let display = Display {
            decoder: frame.decoder.clone(),
            label: tonemap.map(|s| s.label.clone()),
            tonemap: pipeline,
            scene: None,
            picture: None,
        };
        let sampler = |label, filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        Ok(Self {
            passes,
            layout,
            entries,
            trace: None,
            traced: None,
            textures: frame.textures.clone(),
            frame: frame.buffer.clone(),
            shadow: (
                frame.shadow_map.view.clone(),
                frame.shadow_map.sampler.clone(),
                frame.shadow_map.translucent_view.clone(),
                frame.shadow_map.color_view.clone(),
            ),
            linear: sampler("Native shader pack linear", wgpu::FilterMode::Linear),
            nearest: sampler("Native shader pack nearest", wgpu::FilterMode::Nearest),
            targets: None,
            blank: blank_target(device),
            groups: None,
            scales: targets.iter().map(|t| t.1).collect(),
            named: None,
            plan,
            kept,
            format,
            display,
            upscalable: tonemap_lets_taa_upscale(tonemap),
        })
    }

    /// Whether the pack has a `[tonemap]` of its own, which replaces Horizon's tone curve.
    pub fn tonemaps(&self) -> bool {
        self.display.label.is_some()
    }

    /// Whether the pack has post passes, the last of which draws the picture; without them
    /// the tonemap reads the decoded world.
    pub fn chain(&self) -> bool {
        !self.passes.is_empty()
    }

    /// Whether its tonemap lets TAA upscale the world (`tonemap_lets_taa_upscale`).
    pub fn upscalable(&self) -> bool {
        self.upscalable
    }

    /// The passes' labels, in order, then the pack's tonemap's.
    pub fn labels(&self) -> Vec<&str> {
        let tonemap = self.display.label.as_deref();
        self.passes
            .iter()
            .map(|(l, _)| l.as_str())
            .chain(tonemap)
            .collect()
    }

    /// Forgets the pack's own targets (`[[post]] output`): made again, blank, on the next frame,
    /// as for a pack just made. Its pipelines stay.
    pub(super) fn reset_history(&mut self) {
        self.named = None;
        self.groups = None;
    }

    fn prepare(
        &mut self,
        device: &wgpu::Device,
        world: (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView),
        size: [u32; 2],
        display: [u32; 2],
        window: [u32; 2],
        scene: Option<&trace::Traced<'_>>,
    ) {
        let chain = !self.passes.is_empty();
        // Without a chain `scene_color` is what the output decodes: the world resolved.
        let scene_size = if chain { size } else { display };
        let d = &mut self.display;
        if d.scene.as_ref().is_none_or(|s| s.0 != scene_size) {
            d.scene = Some((
                scene_size,
                intermediate(device, "Shader API 0.2 scene_color", scene_size),
            ));
            self.groups = None;
        }
        if chain && d.picture.as_ref().is_none_or(|p| p.0 != window) {
            d.picture = Some((
                window,
                intermediate(device, "Shader API 0.2 picture", window),
            ));
            self.groups = None;
        }
        if !self.scales.is_empty() && self.named.as_ref().is_none_or(|n| n.0 != size) {
            let views = self
                .scales
                .iter()
                .map(|&scale| {
                    let size = scaled(size, scale);
                    device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some("Native shader pack target"),
                            size: wgpu::Extent3d {
                                width: size[0],
                                height: size[1],
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: INTERMEDIATE,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        })
                        .create_view(&Default::default())
                })
                .collect();
            self.named = Some((size, views));
            self.groups = None;
        }
        let chain = self
            .plan
            .iter()
            .any(|p| matches!(p.writes, Writes::Chain(_)));
        if chain && self.targets.as_ref().is_none_or(|t| t.0 != size) {
            let target = |label| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: INTERMEDIATE,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            };
            self.targets = Some((
                size,
                [
                    target("Native shader pack A"),
                    target("Native shader pack B"),
                ],
            ));
            self.groups = None;
        }
        // The scene's TLAS and store are replaced as they grow: groups holding old ones go.
        let traced = self.trace.as_ref().zip(scene);
        let generation = traced.map(|(_, scene)| scene.generation);
        if self
            .groups
            .as_ref()
            .is_some_and(|((color, depth, surface), _)| {
                color == world.0
                    && depth == world.1
                    && surface == world.2
                    && self.traced == generation
            })
        {
            return;
        }
        let blank = self.blank.create_view(&Default::default());
        // The passes read the world decoded.
        let scene = &self.display.scene.as_ref().expect("made above").1;
        let group = |previous: &wgpu::TextureView, pass: Option<usize>, parity| {
            let writes = pass.and_then(|i| self.plan.get(i)).map(|p| p.writes);
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.frame.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(scene),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(world.1),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(previous),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.nearest),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&self.shadow.0),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(&self.shadow.1),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&self.shadow.2),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::TextureView(world.2),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&self.shadow.3),
                },
            ];
            entries.extend(pack_texture_group_entries(&self.textures));
            // The pack's targets, a blank one in place of the one this pass writes.
            if let Some((_, views)) = &self.named {
                entries.extend(views.iter().enumerate().map(|(j, view)| {
                    let view = if writes == Some(Writes::Target(j)) {
                        &blank
                    } else {
                        view
                    };
                    wgpu::BindGroupEntry {
                        binding: horizon_shader_api::stages::POST_TARGET_BINDING + j as u32,
                        resource: wgpu::BindingResource::TextureView(view),
                    }
                }));
            }
            // The passes built against the scene read it too; the tonemap has no `trace`.
            entries.extend(kept::entries(&self.kept, pass, parity));
            let with_scene = traced.and_then(|(t, scene)| {
                let pass = pass?;
                Some((t.layout(pass)?, t, scene, pass))
            });
            let layout = match with_scene {
                Some((layout, trace, scene, pass)) => {
                    entries.extend(trace.entries(pass, scene));
                    layout
                }
                None => kept::pass(&self.kept, pass).map_or(&self.layout, |p| &p.layout),
            };
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Native shader pack post"),
                layout,
                entries: &entries,
            })
        };
        // One set a parity of the kept textures' versions.
        let mut groups = Vec::new();
        for parity in 0..1 + usize::from(self.kept.is_some()) {
            groups.extend(kept::order(&self.kept, self.passes.len()).map(|(i, at)| {
                // The chain's picture so far: the world's before its first pass.
                let previous = match (&self.targets, self.plan[at].reads) {
                    (Some((_, t)), Some(slot)) => &t[slot],
                    _ => scene,
                };
                group(previous, Some(i), parity)
            }));
            // The tonemap's, last: the chain's picture, else the decoded world.
            let picture = self.display.picture.as_ref().map_or(scene, |p| &p.1);
            groups.push(group(picture, None, parity));
        }
        self.groups = Some(((world.0.clone(), world.1.clone(), world.2.clone()), groups));
        self.traced = generation;
    }

    /// The whole post stage in one call: `run_chain`, then `run_display` onto `screen`, of the
    /// window's size.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        world: (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView),
        screen: &wgpu::TextureView,
        size: [u32; 2],
        mut timing: Option<&mut super::pack_timing::PassTiming>,
        scene: Option<&trace::Traced<'_>>,
    ) {
        let window = [screen.texture().width(), screen.texture().height()];
        let timed = timing.as_deref_mut();
        self.run_chain(device, encoder, world, size, size, window, timed, scene);
        self.run_display(device, encoder, screen, None, timing);
    }

    /// Runs the passes over the world target `world` (colour, depth) of `size`, the last onto
    /// the picture of the `window`'s size, timed by `timing` when it times this frame. With
    /// `scene`, the terrain's scene and the camera (`trace` said this frame's `frame` has it),
    /// the passes built against it draw. Without passes the world, or the world resolved at
    /// `display` (`WorldPost::display`), is decoded in `run_display`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run_chain(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        world: (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView),
        size: [u32; 2],
        display: [u32; 2],
        window: [u32; 2],
        mut timing: Option<&mut super::pack_timing::PassTiming>,
        scene: Option<&trace::Traced<'_>>,
    ) {
        self.begin_kept(device, encoder, size);
        self.prepare(device, world, size, display, window, scene);
        if let Some(t) = timing.as_deref_mut() {
            t.mark(device, encoder, (world.0, self.format));
        }
        if self.passes.is_empty() {
            return;
        }
        let Some((_, groups)) = &self.groups else {
            return;
        };
        let groups = kept::current(&self.kept, groups);
        let d = &self.display;
        let (Some((_, scene)), Some((_, passes))) = (&d.scene, groups.split_last()) else {
            return;
        };
        d.decoder.run(device, encoder, world.0, scene);
        for (i, ((label, pipeline), group)) in self.passes.iter().zip(passes).enumerate() {
            let computes = self.trace.as_ref().filter(|_| self.traced.is_some());
            let traced = computes.map(|t| t.computes.as_slice());
            let timed = timing.as_deref_mut();
            kept::dispatch(&self.kept, encoder, (passes, traced), i, size, timed);
            let view = match (self.plan[i].writes, &self.targets, &self.named, &d.picture) {
                (Writes::Chain(slot), Some((_, t)), _, _) => &t[slot],
                (Writes::Target(j), _, Some((_, n)), _) => &n[j],
                (Writes::Kept(r), ..) => kept::output(&self.kept, r),
                (Writes::Picture, _, _, Some((_, picture))) => picture,
                _ => unreachable!("`prepare` makes each pass's target"),
            };
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &attachments,
                timestamp_writes: timing.as_deref_mut().and_then(|t| t.writes(label)),
                ..Default::default()
            });
            pass.set_pipeline(match (&self.trace, self.traced) {
                (Some(trace), Some(_)) => &trace.pipelines[i],
                _ => pipeline,
            });
            pass.set_bind_group(GROUP, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// The tonemap onto `screen`, after `run_chain` in the same frame, which set this frame's
    /// version of the kept resources. Without passes it decodes the world first, or
    /// `resolved`, the world anti-aliased at `run_chain`'s `display`, in its place.
    pub(super) fn run_display(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        screen: &wgpu::TextureView,
        resolved: Option<&wgpu::TextureView>,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        let Some(((world, _, _), groups)) = &self.groups else {
            return;
        };
        let groups = kept::current(&self.kept, groups);
        let d = &self.display;
        let (Some((_, scene)), Some((group, _))) = (&d.scene, groups.split_last()) else {
            return;
        };
        if self.passes.is_empty() {
            d.decoder
                .run(device, encoder, resolved.unwrap_or(world), scene);
        }
        let label = d.label.as_deref().unwrap_or("Tonemap");
        fullscreen_pass(encoder, label, screen, &d.tonemap, (GROUP, group), timing);
    }
}

/// A black 1×1 of the pack's targets' format (`NativePost::blank`).
fn blank_target(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Native shader pack blank"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: INTERMEDIATE,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// Vanilla's gamma colour `c` in linear light (the API's decode; negatives and NaN made 0).
fn decoded(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| if v > 0.0 { v.powf(2.2) } else { 0.0 })
}

/// `wetness` after `dt` seconds of `rain`: halfway there in 30 seconds while it rains
/// harder, in 10 while it dries (the half-lives of the `shaders/` format's `wetness`).
fn wetness(wetness: f32, rain: f32, dt: f32) -> f32 {
    let half_life = if rain > wetness { 30.0 } else { 10.0 };
    rain + (wetness - rain) * 0.5f32.powf(dt / half_life)
}

/// The frame as packs read it (docs/api/shaders-reference-0.2.md), from the renderer's `Frame`,
/// for a world of `size` drawn through `lens` (`FrameStart::projection`). The lens is the
/// renderer's own, not one made again from `size`: the world's size is the window's scaled
/// and rounded, so its aspect can differ from the window's, which the world is drawn with.
pub fn frame_inputs(
    frame: &Frame,
    size: [u32; 2],
    lens: Mat4,
    seconds: f32,
    index: u32,
    shadow: Option<&horizon_shaders::Shadow>,
) -> FrameInputs {
    let view = crate::camera::view_matrix(frame.yaw, frame.pitch);
    let projection = lens * frame.bob * frame.spin;
    // A celestial body's direction at `angle` degrees (SkyRenderer: Y -90°, then X by the
    // angle, of straight up): overhead at 0, rising in the east (+x).
    let body = |angle: f32| {
        let a = angle.to_radians();
        Vec3::new(-a.sin(), a.cos(), 0.0)
    };
    let fog_color = match &frame.fog {
        Some(f) => [f.color[0], f.color[1], f.color[2]],
        None => frame.sky.fog_color,
    };
    // The two terms terrain.wgsl's `apply_fog` takes the larger of, as the renderer has them.
    let (_, fog) = super::fog_uniform(frame);
    let pack = &frame.pack;
    let level = |l: i32| l as f32 / 15.0;
    let m = |m: Mat4| m.to_cols_array_2d();
    FrameInputs {
        view: m(view),
        projection: m(projection),
        view_inverse: m(view.inverse()),
        projection_inverse: m(projection.inverse()),
        camera_position: frame.camera.as_vec3().to_array(),
        time: seconds,
        sun_direction: body(frame.sky.sun_angle).to_array(),
        day_time: pack.day_time.rem_euclid(24000) as f32 / 24000.0,
        moon_direction: body(frame.sky.moon_angle).to_array(),
        game_time: frame.shader_game_time,
        sky_color: frame.sky.sky_color,
        rain: pack.rain,
        fog_color,
        thunder: pack.thunder,
        resolution: [size[0] as f32, size[1] as f32],
        eye_light: [level(pack.eye_light[0]), level(pack.eye_light[1])],
        fog_gap: [0.0; 2],
        render_distance: frame.distance,
        far: frame.depth_far,
        night_vision: frame.night_vision,
        blindness: pack.blindness,
        darkness: frame.darkness.1.clamp(0.0, 1.0),
        brightness: pack.brightness,
        eye_in: pack.eye_in as u32,
        dimension: if frame.end_sky {
            1
        } else if frame.nether_lighting {
            -1
        } else {
            0
        },
        frame_index: index,
        hide_gui: u32::from(pack.hide_gui),
        precipitation: pack.biome_precipitation.clamp(0, 2) as u32,
        temperature: pack.temperature,
        downfall: pack.rainfall,
        wetness: pack.rain,
        // Filled by `NativeFrame::write`, which remembers the frames before.
        previous_view_projection: m(Mat4::IDENTITY),
        camera_motion: [0.0; 3],
        delta_time: 0.0,
        eye_light_smooth: [level(pack.eye_light[0]), level(pack.eye_light[1])],
        in_water: f32::from(u8::from(pack.eye_in as u32 == 1)),
        in_lava: f32::from(u8::from(pack.eye_in as u32 == 2)),
        water_plane: frame.water_plane.unwrap_or(0.0),
        has_reflection: u32::from(frame.water_plane.is_some()),
        reflection_resolution: reflection_size(size).map(|s| s as f32),
        mods: pack.mod_params,
        light_direction: match shadow {
            Some(s) => super::pack_uniforms::shadow_light(frame, s),
            None => body(super::pack_uniforms::light_angle(frame)),
        }
        .to_array(),
        has_shadow: u32::from(shadow.is_some()),
        shadow_view_proj: m(shadow.map_or(Mat4::IDENTITY, |s| {
            super::pack_uniforms::shadow_culling(frame, s)
        })),
        shadow_distance: shadow.map_or(0.0, |s| s.distance),
        shadow_distortion: shadow.map_or(0.0, |s| {
            super::native_shadow::distortion(s.distance, s.resolution)
        }),
        shadow_resolution: shadow.map_or(0.0, |s| s.resolution as f32),
        shadow_center_texel: shadow.map_or(0.0, |s| {
            let k = super::native_shadow::distortion(s.distance, s.resolution);
            2.0 * s.distance / s.resolution.max(1) as f32 * (1.0 - k)
        }),
        // The display: SDR's values, which `NativeFrame::write` replaces on an HDR display.
        hdr: 0,
        headroom: 1.0,
        paper_white: 80.0,
        peak: 80.0,
        black_level: 0.0,
        // No pack is served another space yet (`[frame] primaries`).
        primaries: Primaries::Bt709 as u32,
        // `NativeFrame::write` sets them while the pack traces the scene.
        has_ray_tracing: 0,
        ray_distance: 0.0,
        luminance: Primaries::Bt709.luminance(),
        // Kept resources' epochs: no pack with `[[resource]]` runs yet.
        view_epoch: 0,
        world_epoch: 0,
        reset_reasons: 0,
        fog_environment: [fog[0], fog[1]],
        fog_render: [fog[2], fog[3]],
        moon_phase: frame.sky.moon_phase,
        padding: [0; 1],
        // `NativeFrame::write` sets it from the renderer's lightmap.
        full_lightmap: [0.0; 3],
        full_lightmap_padding: [0; 1],
        // `NativeFrame::next_inputs` sets them from the world's jitter.
        jitter: [0.0; 2],
        previous_jitter: [0.0; 2],
    }
}

/// `frame.full_lightmap` from `lightmap`, the renderer's lightmap as it uploads it
/// (`Renderer::lightmap_pixels`: RGBA rows of sky level by block level, in vanilla's gamma):
/// its texel at block and sky light 15, which terrain.wgsl's `sample_lightmap(15.0, 15.0)` reads,
/// decoded as the stages' `lightmap` is.
pub(super) fn full_lightmap(lightmap: &[u8; 16 * 16 * 4]) -> [f32; 3] {
    let at = (15 * 16 + 15) * 4;
    decoded([0, 1, 2].map(|i| f32::from(lightmap[at + i]) / 255.0))
}

/// The pack's `[[textures]]` (`check::Image`s), uploaded with samplers as the manifest says.
pub fn pack_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    report: &horizon_shader_api::Report,
) -> Vec<(wgpu::TextureView, wgpu::Sampler)> {
    report
        .textures
        .iter()
        .map(|(t, image)| {
            let texture = wgpu::util::DeviceExt::create_texture_with_data(
                device,
                queue,
                &wgpu::TextureDescriptor {
                    label: Some(&t.id),
                    size: wgpu::Extent3d {
                        width: image.width,
                        height: image.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &image.rgba,
            );
            let filter = if t.linear {
                wgpu::FilterMode::Linear
            } else {
                wgpu::FilterMode::Nearest
            };
            let address = if t.repeat {
                wgpu::AddressMode::Repeat
            } else {
                wgpu::AddressMode::ClampToEdge
            };
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(&t.id),
                address_mode_u: address,
                address_mode_v: address,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            });
            (texture.create_view(&Default::default()), sampler)
        })
        .collect()
}

/// Layout entries of `count` pack textures and their samplers.
pub(super) fn pack_texture_entries(count: usize) -> Vec<wgpu::BindGroupLayoutEntry> {
    use horizon_shader_api::stages::{TEXTURE_BINDING, TEXTURE_SAMPLER_BINDING};
    (0..count as u32)
        .flat_map(|i| {
            [
                wgpu::BindGroupLayoutEntry {
                    binding: TEXTURE_BINDING + i,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: TEXTURE_SAMPLER_BINDING + i,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ]
        })
        .collect()
}

/// Group entries of the pack's textures, as `pack_texture_entries` lays them out.
pub(super) fn pack_texture_group_entries(
    textures: &[(wgpu::TextureView, wgpu::Sampler)],
) -> Vec<wgpu::BindGroupEntry<'_>> {
    use horizon_shader_api::stages::{TEXTURE_BINDING, TEXTURE_SAMPLER_BINDING};
    textures
        .iter()
        .enumerate()
        .flat_map(|(i, (view, sampler))| {
            [
                wgpu::BindGroupEntry {
                    binding: TEXTURE_BINDING + i as u32,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: TEXTURE_SAMPLER_BINDING + i as u32,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_pass_with_an_output_leaves_the_chain_as_it_was() {
        use super::{Writes, plan};
        // Bloom into a target, a pass of the chain, a kept texture, the picture.
        let (bloom, kept) = (Some(Writes::Target(0)), Some(Writes::Kept(1)));
        let p = plan(&[bloom, None, kept, None, None]);
        let writes: Vec<_> = p.iter().map(|p| p.writes).collect();
        assert!(writes[0] == Writes::Target(0));
        assert!(writes[1] == Writes::Chain(0));
        assert!(writes[2] == Writes::Kept(1));
        assert!(writes[3] == Writes::Chain(1));
        assert!(writes[4] == Writes::Picture);
        let reads: Vec<_> = p.iter().map(|p| p.reads).collect();
        assert_eq!(reads, [None, None, Some(0), Some(0), Some(1)]);
    }
    #[test]
    fn eased_values_start_at_the_target_and_halve_the_distance() {
        use super::eased;
        assert_eq!(eased(f32::NAN, 0.25, 2.0, 0.0), 0.25);
        assert_eq!(eased(1.0, 0.0, 2.0, 2.0), 0.5);
        assert_eq!(eased(1.0, 0.0, 2.0, 0.0), 1.0);
    }
    use super::*;
    use crate::gfx::output::DisplayValues;
    use horizon_shader_api::{MANIFEST, Pack, check};

    const SDR: DisplayValues = DisplayValues::SDR;

    /// `light_direction` is the sun by day and the moon by night, as the same frame's
    /// `sun_direction` and `moon_direction` have them, also for a pack without a shadow map; a
    /// pack with `[shadow]` gets the shadow camera's light as before, to the bit.
    #[test]
    fn the_light_is_the_sun_by_day_and_the_moon_by_night_with_or_without_a_shadow_map() {
        let hud = crate::hud::HudBatch::scratch([4, 4]);
        let mut frame = crate::gfx::pack_uniforms::initial_history_tests::frame(&hud);
        let shadow = crate::gfx::native_shadow::camera(&Default::default());
        let light = |frame: &Frame, shadow| {
            frame_inputs(frame, [4, 4], Mat4::IDENTITY, 0.0, 0, shadow)
                .light_direction
                .map(f32::to_bits)
        };
        // Morning and night, where the sun, the moon and straight up all differ.
        for (sun, moon, by_day, shadowed) in [
            (35.0, 215.0, true, [0xbf12_d5e7, 0x3f51_b3f3, 0xb2d7_5d86]),
            (200.0, 20.0, false, [0xbeaf_1d43, 0x3f70_8fb3, 0xb280_6bc8]),
        ] {
            (frame.sky.sun_angle, frame.sky.moon_angle) = (sun, moon);
            let inputs = frame_inputs(&frame, [4, 4], Mat4::IDENTITY, 0.0, 0, None);
            let body = if by_day {
                inputs.sun_direction
            } else {
                inputs.moon_direction
            };
            assert_eq!(light(&frame, None), body.map(f32::to_bits), "{sun}");
            assert_eq!(light(&frame, Some(&shadow)), shadowed, "{sun}");
        }
    }

    /// A pack rebuilds positions from depth with the projection the world is drawn with
    /// (unjittered): at an odd window with upscaling, the world's rounded size has another
    /// aspect than the window's, and `frame.projection` still is the renderer's lens.
    #[test]
    fn the_pack_s_projection_is_the_renderer_s_at_an_odd_upscaled_size() {
        let hud = crate::hud::HudBatch::scratch([4, 4]);
        let mut frame = crate::gfx::pack_uniforms::initial_history_tests::frame(&hud);
        (frame.yaw, frame.pitch, frame.fov_degrees) = (30.0, -10.0, 70.0);
        // A 1281×721 window at scale 0.5 and the Upscaling ratio 2 draws its world at 321×181.
        let (window, drawn) = ([1281.0f32, 721.0], [321, 181]);
        let renderer = crate::gfx::view::View::screen(
            frame.yaw,
            frame.pitch,
            frame.fov_degrees,
            window[0] / window[1],
            frame.depth_far,
        );
        let redone = drawn[0] as f32 / drawn[1] as f32;
        let at_drawn = crate::gfx::world_projection(frame.fov_degrees, redone, frame.depth_far);
        assert_ne!(at_drawn, renderer.projection, "the sizes' aspects differ");
        let inputs = frame_inputs(&frame, drawn, renderer.projection, 0.0, 0, None);
        let m = Mat4::from_cols_array_2d;
        let pose = frame.bob * frame.spin;
        let pack = m(&inputs.projection) * m(&inputs.view);
        let drawn_with = renderer.view_proj(pose);
        assert!(pack.abs_diff_eq(drawn_with, 1e-6), "{pack} != {drawn_with}");
        let inverse = m(&inputs.projection_inverse) * m(&inputs.projection);
        assert!(inverse.abs_diff_eq(Mat4::IDENTITY, 1e-4), "{inverse}");
        assert_eq!(inputs.resolution, [321.0, 181.0]);
    }

    #[test]
    fn wetness_rises_slower_than_it_dries() {
        assert_eq!(wetness(0.0, 1.0, 30.0), 0.5);
        assert_eq!(wetness(1.0, 0.0, 10.0), 0.5);
        assert_eq!(wetness(0.25, 0.25, 5.0), 0.25);
    }

    /// `previous_jitter` is last frame's jitter where last frame carries over, as
    /// `previous_view_projection` is, and zero after a gap or a reset (`last` is `None`).
    #[test]
    fn the_previous_jitter_is_last_frame_s_unless_a_gap_or_reset_came_between() {
        let now = std::time::Instant::now();
        let jitter = [0.25, -0.125];
        let drawn = |ago: f32| {
            let time = now - std::time::Duration::from_secs_f32(ago);
            Some((
                Mat4::IDENTITY.to_cols_array_2d(),
                glam::DVec3::ZERO,
                time,
                jitter,
            ))
        };
        let previous = |last, scripted| carried(last, now, scripted).map_or([0.0; 2], |l| l.3);
        assert_eq!(previous(drawn(0.016), false), jitter);
        assert_eq!(previous(drawn(2.0), false), [0.0; 2]);
        assert_eq!(previous(drawn(2.0), true), jitter);
        assert_eq!(previous(None, false), [0.0; 2]);
    }

    /// A scripted clock comes before STILL's ticks, eased values and wetness move by its
    /// delta, and a long wait between its frames is not a gap that starts them over.
    #[test]
    fn a_scripted_clock_drives_time_frame_index_and_eased_values() {
        let _gpu = crate::gfx::test_gpu::lock();
        use crate::gfx::pack_uniforms::FrameTiming;
        let Some((device, _)) = device(false) else {
            return;
        };
        let hud = crate::hud::HudBatch::scratch([4, 4]);
        let mut frame = crate::gfx::pack_uniforms::initial_history_tests::frame(&hud);
        let mut native =
            NativeFrame::new(&device, None, Vec::new(), Default::default(), None, false);
        frame.pack.still_ticks = Some(47_811);
        frame.pack.eye_light = [0, 15];
        frame.pack.rain = 1.0;
        frame.pack.frame_timing = Some(FrameTiming::nth(1, 2000.0));
        let first = native.next_inputs(&frame, [4, 4], SDR);
        assert_eq!(
            (first.time, first.frame_index, first.delta_time),
            (0.0, 1, 0.0)
        );
        // Wetness starts dry, as on a fresh pack; frame 1's delta of 0 leaves it there.
        assert_eq!((first.eye_light_smooth[1], first.wetness), (1.0, 0.0));
        native.last.as_mut().unwrap().2 -= std::time::Duration::from_secs(5);
        frame.pack.eye_light = [0, 0];
        frame.pack.frame_timing = Some(FrameTiming::nth(2, 2000.0));
        let second = native.next_inputs(&frame, [4, 4], SDR);
        assert_eq!(
            (second.time, second.frame_index, second.delta_time),
            (2.0, 2, 2.0)
        );
        // Two seconds: the eye light's 2 s half-life halves it, and wetness rises
        // by 1 - 0.5^(2/30) (a 30 s half-life while it rains harder).
        assert_eq!(second.eye_light_smooth[1], 0.5);
        assert!(
            (second.wetness - 0.045_158_4).abs() < 1e-6,
            "{}",
            second.wetness
        );
    }

    /// After a reset, the same frames give the same inputs, to the bit, as a pack just made,
    /// however the frames before moved the eased values, wetness and the frame count, and
    /// whatever their jitter (`previous_jitter` starts at zero).
    #[test]
    fn a_reset_frame_gives_a_fresh_pack_s_inputs() {
        let _gpu = crate::gfx::test_gpu::lock();
        use crate::gfx::pack_uniforms::FrameTiming;
        let Some((device, _)) = device(false) else {
            return;
        };
        let hud = crate::hud::HudBatch::new(
            [4, 4],
            crate::hud::GuiScale {
                requested: 0,
                unicode: false,
            },
        );
        let mut frame = crate::gfx::pack_uniforms::initial_history_tests::frame(&hud);
        let new = || NativeFrame::new(&device, None, Vec::new(), Default::default(), None, false);
        let (mut used, mut fresh) = (new(), new());
        frame.pack.rain = 1.0;
        frame.pack.eye_light = [12, 3];
        frame.pack.eye_in = 1;
        used.jitter = [0.25, -0.125];
        for n in 1..=5 {
            frame.pack.frame_timing = Some(FrameTiming::nth(n, 700.0));
            used.next_inputs(&frame, [4, 4], SDR);
        }
        used.reset_history();
        frame.pack.eye_light = [2, 15];
        frame.pack.eye_in = 0;
        used.jitter = [0.5, 0.5];
        fresh.jitter = used.jitter;
        for n in 1..=3 {
            frame.pack.frame_timing = Some(FrameTiming::nth(n, 50.0));
            let inputs = [&mut used, &mut fresh].map(|f| f.next_inputs(&frame, [4, 4], SDR));
            assert_eq!(
                bytemuck::bytes_of(&inputs[0]),
                bytemuck::bytes_of(&inputs[1]),
                "frame {n}"
            );
        }
    }

    /// After a reset a pass reads its pack's own targets blank, as in a pack just made: here
    /// `fb`, which a pass reads before the pass that writes it, so it holds last frame's.
    #[test]
    fn a_reset_post_reads_its_own_targets_blank() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device(true) else {
            return;
        };
        let manifest = "schema = 1
id = \"t\"
name = \"T\"
version = \"1\"
api = \"0.2\"
            [[post]]
    file = \"a.wgsl\"
    [[post]]
    file = \"b.wgsl\"
    output = \"fb\"
            [[post]]
    file = \"c.wgsl\"
    ";
        let a = "fn post(input: PostInput) -> vec4<f32> { return vec4<f32>(textureLoad(fb, vec2<i32>(input.pixel), 0).rgb + 0.25, 1.0); }
    ";
        let pass = "fn post(input: PostInput) -> vec4<f32> { return input.color; }
    ";
        let files = [
            (MANIFEST, manifest),
            ("a.wgsl", a),
            ("b.wgsl", pass),
            ("c.wgsl", pass),
        ]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let frame = sdr_frame(&device, &queue);
        let mut post = NativePost::new(
            &device,
            report.stages,
            &report.manifest,
            &[],
            format,
            &frame,
            None,
        )
        .unwrap();
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let color = texture(&device, format, usage).create_view(&Default::default());
        let depth = texture(&device, wgpu::TextureFormat::Depth32Float, usage)
            .create_view(&Default::default());
        let screen = texture(
            &device,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let view = screen.create_view(&Default::default());
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let red = |post: &mut NativePost| {
            let mut encoder = device.create_command_encoder(&Default::default());
            let world = (&color, &depth, &frame.surface_view);
            post.run(&device, &mut encoder, world, &view, [4, 4], None, None);
            encoder.copy_texture_to_buffer(
                screen.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                        rows_per_image: Some(4),
                    },
                },
                screen.size(),
            );
            queue.submit([encoder.finish()]);
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let red = buffer.slice(..).get_mapped_range().unwrap()[0];
            buffer.unmap();
            red
        };
        let near = |got: u8, want: u8| got.abs_diff(want) <= 1;
        // A quarter, a half and three quarters of white in linear light, encoded.
        for want in [136, 186, 224] {
            let got = red(&mut post);
            assert!(near(got, want), "{got} before the reset, {want} expected");
        }
        post.reset_history();
        let got = red(&mut post);
        assert!(
            near(got, 136),
            "{got} after the reset: fb kept last frame's"
        );
    }

    fn device(needs_depth_load: bool) -> Option<(wgpu::Device, wgpu::Queue)> {
        let adapter = crate::gfx::test_gpu::adapter()?;
        if needs_depth_load && !crate::gfx::test_gpu::supports_depth_load(&adapter) {
            return None;
        }
        Some(crate::gfx::test_gpu::device(&adapter))
    }

    /// A pack's frame with nothing but an SDR display written (`headroom` 1): the tonemap
    /// holds the picture to the headroom, and a zeroed buffer has none.
    fn sdr_frame(device: &wgpu::Device, queue: &wgpu::Queue) -> NativeFrame {
        let frame = NativeFrame::new(device, None, Vec::new(), Default::default(), None, false);
        let inputs = FrameInputs {
            headroom: 1.0,
            ..Default::default()
        };
        queue.write_buffer(&frame.buffer, 0, bytemuck::bytes_of(&inputs));
        frame
    }

    fn texture(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    }

    /// Two passes: the first doubles the world's light (over 1, so the half-float target must
    /// keep it) times an option, the second halves it and writes the depth as blue.
    #[test]
    fn passes_chain_through_half_float_targets_with_options_and_depth() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device(true) else {
            return;
        };
        let manifest = "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
            [[post]]\nfile = \"a.wgsl\"\n[[post]]\nfile = \"b.wgsl\"\n\
            [[options]]\nid = \"gain\"\nname = \"Gain\"\ntype = \"float\"\ndefault = 0.0\nmin = 0.0\nmax = 2.0\n";
        let a = "override gain: f32;\nfn post(input: PostInput) -> vec4<f32> { return vec4<f32>(input.color.rgb * 2.0 * gain, 1.0); }\n";
        let b = "fn post(input: PostInput) -> vec4<f32> { return vec4<f32>(input.color.rg * 0.5, input.depth, 1.0); }\n";
        let files = [(MANIFEST, manifest), ("a.wgsl", a), ("b.wgsl", b)]
            .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
            .into();
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let frame = sdr_frame(&device, &queue);
        let mut post = NativePost::new(
            &device,
            report.stages,
            &report.manifest,
            &[("gain".into(), 1.0)],
            format,
            &frame,
            None,
        )
        .unwrap();
        assert_eq!(post.labels(), ["a.wgsl", "b.wgsl"]);

        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let color = texture(&device, format, usage).create_view(&Default::default());
        let depth = texture(&device, wgpu::TextureFormat::Depth32Float, usage)
            .create_view(&Default::default());
        let screen = texture(
            &device,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.8,
                        g: 0.4,
                        b: 0.2,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.5),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        let view = screen.create_view(&Default::default());
        post.run(
            &device,
            &mut encoder,
            (&color, &depth, &frame.surface_view),
            &view,
            [4, 4],
            None,
            None,
        );
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            screen.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    rows_per_image: Some(4),
                },
            },
            screen.size(),
        );
        queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixel = buffer.slice(..).get_mapped_range().unwrap()[..4].to_vec();
        let near = |got: u8, want: u8| got.abs_diff(want) <= 1;
        // The world's red and green as they went in, the depth 0.5 as light, encoded (186).
        assert!(
            near(pixel[0], 204) && near(pixel[1], 102) && near(pixel[2], 186) && pixel[3] == 255,
            "{pixel:?}"
        );
    }

    /// OpenGL refuses `Rgba32Float` as a render attachment (GLES 3 semantics, absent
    /// `EXT_color_buffer_float`) but always renders to the integer formats, so the surface
    /// is an integer format, and `surface_at` gets the floats back from the stored bits.
    #[test]
    fn the_surface_is_an_integer_target_that_the_post_stage_reads_bit_exact() {
        let _gpu = crate::gfx::test_gpu::lock();
        assert_eq!(
            super::super::native_material::SURFACE_FORMAT.sample_type(None, None),
            Some(wgpu::TextureSampleType::Uint)
        );
        let Some((device, queue)) = device(true) else {
            return;
        };
        let manifest = "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
            [[post]]\nfile = \"a.wgsl\"\n";
        // The surface's fields on the colour: normal x, smoothness and data, all 0 when absent.
        let a = "fn post(input: PostInput) -> vec4<f32> {\n    \
            let s = surface_at(vec2<i32>(input.pixel));\n    \
            return vec4<f32>(s.normal.x, s.smoothness, s.data, 1.0);\n}\n";
        let files = [(MANIFEST, manifest), ("a.wgsl", a)]
            .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
            .into();
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let frame = sdr_frame(&device, &queue);
        let mut post = NativePost::new(
            &device,
            report.stages,
            &report.manifest,
            &[],
            format,
            &frame,
            None,
        )
        .unwrap();
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let color = texture(&device, format, usage).create_view(&Default::default());
        let depth = texture(&device, wgpu::TextureFormat::Depth32Float, usage)
            .create_view(&Default::default());
        let screen = texture(
            &device,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let view = screen.create_view(&Default::default());
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // What the material stage stores, as the bits of floats: the normal +x (octahedral
        // (1, 0), two snorm16), smoothness 0.75, data 0.25 and the fragment's depth 0.5.
        let writer = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                    return vec4<f32>(f32((i << 1u) & 2u) * 2.0 - 1.0, f32(i & 2u) * 2.0 - 1.0, 0.0, 1.0);
                }
                @fragment fn fs() -> @location(0) vec4<u32> {
                    return bitcast<vec4<u32>>(vec4<f32>(
                        bitcast<f32>(pack2x16snorm(vec2<f32>(1.0, 0.0))), 0.75, 0.25, 0.5));
                }"
                .into(),
            ),
        });
        let writer = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &writer,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &writer,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: super::super::native_material::SURFACE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let mut read = |depth_of_world: f64| {
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame.surface_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&writer);
                pass.draw(0..3, 0..1);
            }
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(depth_of_world as f32),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            post.run(
                &device,
                &mut encoder,
                (&color, &depth, &frame.surface_view),
                &view,
                [4, 4],
                None,
                None,
            );
            encoder.copy_texture_to_buffer(
                screen.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                        rows_per_image: Some(4),
                    },
                },
                screen.size(),
            );
            queue.submit([encoder.finish()]);
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let pixel = buffer.slice(..).get_mapped_range().unwrap()[..4].to_vec();
            buffer.unmap();
            pixel
        };
        // Present where the world's depth is the stored one: +x, 0.75, 0.25, shown as light
        // (encoded: 255, 223.7, 135.8). None sits halfway between two 8-bit levels, where
        // drivers round either way.
        assert_eq!(read(0.5), [255, 224, 136, 255]);
        // Hidden by something nearer drawn since: the stored values no longer show.
        assert_eq!(read(0.25), [0, 0, 0, 255]);
    }

    /// The surface holds float bit patterns, some of which are NaNs (a normal's two snorm16
    /// can form one). The material stage writes `bitcast<vec4<u32>>(vec4<f32>(..))`; this
    /// runs that same expression (not the whole material fragment, which needs a full terrain
    /// pipeline) on bits fed as integers and checks the texture holds them exactly: payloads,
    /// signs and denormals included.
    #[test]
    fn the_surface_keeps_every_bit_of_the_floats_written_through_the_material_stage_bitcast() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device(false) else {
            return;
        };
        let cases: [[u32; 4]; 4] = [
            [0x7fc0_1234, 0xffbf_ffff, 0x7fff_8001, 0x7f80_0001],
            [0x8000_0000, 0x0000_0001, 0x007f_ffff, 0x8000_0001],
            [0xffff_ffff, 0x7f80_0000, 0xff80_0000, 0x3f80_0000],
            [0x7fff_8001, 0x0000_0000, 0x1234_5678, 0xdead_beef],
        ];
        let mut bits = [[0u32; 4]; 16];
        bits[..4].copy_from_slice(&cases);
        let inputs = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&inputs, 0, bytemuck::cast_slice(&bits));
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                "@group(0) @binding(0) var<uniform> bits: array<vec4<u32>, 16>;
                @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                    return vec4<f32>(f32((i << 1u) & 2u) * 2.0 - 1.0, f32(i & 2u) * 2.0 - 1.0, 0.0, 1.0);
                }
                @fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<u32> {
                    let v = bitcast<vec4<f32>>(bits[u32(p.y) * 4u + u32(p.x)]);
                    return bitcast<vec4<u32>>(v);
                }"
                .into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: super::super::native_material::SURFACE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: inputs.as_entire_binding(),
            }],
        });
        let surface = texture(
            &device,
            super::super::native_material::SURFACE_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let row = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: row * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let view = surface.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            surface.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    rows_per_image: Some(4),
                },
            },
            surface.size(),
        );
        queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        buffer.unmap();
        for (i, expected) in bits.iter().enumerate() {
            let (x, y) = (i % 4, i / 4);
            let at = y * row as usize + x * 16;
            let got: [u32; 4] = std::array::from_fn(|c| {
                u32::from_ne_bytes(data[at + c * 4..at + c * 4 + 4].try_into().unwrap())
            });
            assert_eq!(
                got.map(|v| format!("{v:08x}")),
                expected.map(|v| format!("{v:08x}")),
                "texel {x},{y}"
            );
        }
    }

    /// The world decoded, through a chain that passes it on or none, and
    /// Horizon's tonemap or the pack's identity one, comes back encoded as it went in (within
    /// an 8-bit step), at alpha 1.
    #[test]
    fn the_world_comes_back_through_decode_tonemap_and_encode() {
        let _gpu = crate::gfx::test_gpu::lock();
        assert!(tonemap_entries(false).contains("horizon_encode(min(horizon_sanitize(out)"));
        let Some((device, queue)) = device(true) else {
            return;
        };
        let post = "[[post]]\nfile = \"a.wgsl\"\n[[post]]\nfile = \"b.wgsl\"\n";
        let curve = "[tonemap]\nfile = \"t.wgsl\"\n";
        // A pack needs a stage: the first has only a sky, and no chain or tonemap.
        let sky = "[sky]\nfile = \"s.wgsl\"\n";
        for extra in [
            sky.into(),
            post.into(),
            curve.into(),
            format!("{post}{curve}"),
        ] {
            let manifest = format!(
                "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n{extra}"
            );
            let pass = "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
            let identity = "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color; }\n";
            let mut files = vec![(MANIFEST, manifest.as_str())];
            if extra.contains("[[post]]") {
                files.extend([("a.wgsl", pass), ("b.wgsl", pass)]);
            }
            if extra.contains("[tonemap]") {
                files.push(("t.wgsl", identity));
            }
            if extra.contains("[sky]") {
                files.push((
                    "s.wgsl",
                    "fn sky(input: SkyInput) -> vec4<f32> { return input.color; }\n",
                ));
            }
            let files = files
                .into_iter()
                .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
                .collect();
            let report = check(&Pack::from_files(files).unwrap()).unwrap();
            let (passes, others): (Vec<_>, Vec<_>) = report
                .stages
                .into_iter()
                .partition(|s| s.kind == StageKind::Post);
            let format = wgpu::TextureFormat::Rgba8Unorm;
            let frame =
                NativeFrame::new(&device, None, Vec::new(), Default::default(), None, false);
            let inputs = FrameInputs {
                headroom: 1.0,
                ..Default::default()
            };
            queue.write_buffer(&frame.buffer, 0, bytemuck::bytes_of(&inputs));
            let tonemap = others.iter().find(|s| s.kind == StageKind::Tonemap);
            let mut post = NativePost::new(
                &device,
                passes,
                &report.manifest,
                &[],
                format,
                &frame,
                tonemap,
            )
            .unwrap();
            assert_eq!(post.labels().last() == Some(&"t.wgsl"), tonemap.is_some());
            let usage =
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let color = texture(&device, format, usage).create_view(&Default::default());
            let depth = texture(&device, wgpu::TextureFormat::Depth32Float, usage)
                .create_view(&Default::default());
            let screen = texture(
                &device,
                format,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            let clear = wgpu::Color {
                r: 0.8,
                g: 0.4,
                b: 0.2,
                a: 0.5,
            };
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            }));
            let view = screen.create_view(&Default::default());
            let world = (&color, &depth, &frame.surface_view);
            post.run(&device, &mut encoder, world, &view, [4, 4], None, None);
            let buffer = crate::gfx::copy_frame(&device, &mut encoder, &screen, [4, 4]);
            queue.submit([encoder.finish()]);
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let pixel = buffer.slice(..).get_mapped_range().unwrap()[..4].to_vec();
            let want = [204, 102, 51, 255];
            assert!(
                pixel
                    .iter()
                    .zip(want)
                    .all(|(&got, want)| got.abs_diff(want) <= 1),
                "{extra}: {pixel:?}"
            );
        }
    }

    /// A pack without `[[post]]` whose `[tonemap]` returns its input, over an SDR frame: its
    /// output is the world it reads, or the world the renderer resolved in its place.
    fn identity_tonemap_pack(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> (NativePost, NativeFrame) {
        let identity = "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color; }\n";
        tonemap_pack(device, queue, format, identity)
    }

    /// The check's report of a pack without `[[post]]` whose `[tonemap]` is `tonemap` (none:
    /// Horizon's).
    fn tonemap_report(tonemap: Option<&str>) -> horizon_shader_api::Report {
        let stage = if tonemap.is_some() {
            "[tonemap]\nfile = \"t.wgsl\"\n"
        } else {
            ""
        };
        let manifest = format!(
            "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
             [sky]\nfile = \"s.wgsl\"\n{stage}"
        );
        let sky = "fn sky(input: SkyInput) -> vec4<f32> { return input.color; }\n";
        let files = [(MANIFEST, manifest.as_str()), ("s.wgsl", sky)]
            .into_iter()
            .chain(tonemap.map(|t| ("t.wgsl", t)))
            .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
            .collect();
        check(&Pack::from_files(files).unwrap()).unwrap()
    }

    /// A pack without `[[post]]` whose `[tonemap]` is `tonemap`, over an SDR frame.
    fn tonemap_pack(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        tonemap: &str,
    ) -> (NativePost, NativeFrame) {
        let report = tonemap_report(Some(tonemap));
        let tonemap = report.stages.iter().find(|s| s.kind == StageKind::Tonemap);
        let frame = sdr_frame(device, queue);
        let post = NativePost::new(
            device,
            Vec::new(),
            &report.manifest,
            &[],
            format,
            &frame,
            tonemap,
        )
        .unwrap();
        assert!(!post.chain());
        (post, frame)
    }

    /// A square world of `size` in RGBA8, black left of the diagonal moved right by `shift`
    /// and white from it on: a staircase edge.
    fn staircase(size: u32, shift: u32) -> Vec<u8> {
        (0..size * size)
            .flat_map(|i| {
                let v = if i % size >= i / size + shift { 255 } else { 0 };
                [v, v, v, 255]
            })
            .collect()
    }

    /// The rows of the square `screen` of `size`, once the commands in `encoder` ran.
    fn read_screen(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        screen: &wgpu::Texture,
        size: u32,
    ) -> Vec<u8> {
        let buffer = crate::gfx::copy_frame(device, &mut encoder, screen, [size, size]);
        queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let row = crate::gfx::padded_row(size) as usize;
        let mapped = buffer.slice(..).get_mapped_range().unwrap();
        (0..size as usize)
            .flat_map(|y| mapped[y * row..y * row + size as usize * 4].to_vec())
            .collect()
    }

    /// A pack without `[[post]]` shows the player's FXAA when the renderer hands it the world
    /// resolved (#16126): a staircase edge comes out blended, and without the resolve the
    /// output is the world exactly.
    #[test]
    fn a_pack_without_a_chain_shows_the_world_through_fxaa_only_when_it_is_on() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device(true) else {
            return;
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let (mut post, frame) = identity_tonemap_pack(&device, &queue, format);
        // An 8×8 world with a staircase edge.
        const SIZE: u32 = 8;
        let world_pixels = staircase(SIZE, 0);
        let sized = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
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
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let world = sized(format, usage | wgpu::TextureUsages::COPY_DST);
        queue.write_texture(
            world.as_image_copy(),
            &world_pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: None,
            },
            world.size(),
        );
        let world = world.create_view(&Default::default());
        let depth =
            sized(wgpu::TextureFormat::Depth32Float, usage).create_view(&Default::default());
        let screen = sized(format, usage | wgpu::TextureUsages::COPY_SRC);
        let screen_view = screen.create_view(&Default::default());
        // The renderer's resolve: `world_post` with FXAA on, onto its world-sized target.
        let mut world_post = crate::gfx::world_post::Post::new(&device, format);
        world_post.antialias = true;
        assert!(
            (world_post.prepare(&device, [SIZE, SIZE], true, false)).is_some(),
            "a pack's world has a target"
        );
        let resolved = world_post.resolved(&device).unwrap();
        let mut show = |fxaa: bool| {
            let mut encoder = device.create_command_encoder(&Default::default());
            if fxaa {
                let group = world_post.bind(&device, &world);
                world_post.finish_with(&mut encoder, &resolved, &group, None);
            }
            let size = [SIZE, SIZE];
            let inputs = (&world, &depth, &frame.surface_view);
            post.run_chain(&device, &mut encoder, inputs, size, size, size, None, None);
            let resolved = fxaa.then_some(&resolved);
            post.run_display(&device, &mut encoder, &screen_view, resolved, None);
            read_screen(&device, &queue, encoder, &screen, SIZE)
        };
        let off = show(false);
        assert_eq!(
            off, world_pixels,
            "without the resolve the output is the world"
        );
        let on = show(true);
        let blended = on.chunks(4).filter(|p| (16..=239).contains(&p[0])).count();
        assert!(blended > 0, "FXAA left the edge as it was: {on:?}");
    }

    /// A pack without `[[post]]` shows the player's TAA when the renderer hands it TAA's
    /// history (#16126): the first frame after a reset is the world exactly, and the next,
    /// drawn with another jitter that moved the edge a pixel, comes out blended with it.
    #[test]
    fn a_pack_without_a_chain_shows_the_world_through_taa() {
        use glam::{DVec3, Mat4};
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device(true) else {
            eprintln!("TAA under a pack GPU skipped: no test device");
            return;
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut world_post = crate::gfx::world_post::Post::new(&device, format);
        if !world_post.supports_taa() {
            eprintln!("TAA under a pack GPU skipped: TAA unsupported");
            return;
        }
        world_post.taa = true;
        world_post.hold_upscale(true);
        let (mut post, frame) = identity_tonemap_pack(&device, &queue, format);
        const SIZE: u32 = 8;
        let window = [SIZE, SIZE];
        let sized = |usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
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
        let source = sized(wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        let source_view = source.create_view(&Default::default());
        let screen = sized(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        let screen_view = screen.create_view(&Default::default());
        let mut show = |pixels: &[u8]| {
            world_post.jitter(window, true, Mat4::IDENTITY, DVec3::ZERO);
            let (color, depth) = world_post.prepare(&device, window, true, false).unwrap();
            queue.write_texture(
                source.as_image_copy(),
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: None,
                },
                source.size(),
            );
            // The world as drawn this frame, copied onto the target TAA reads.
            let mut encoder = device.create_command_encoder(&Default::default());
            let group = world_post.bind(&device, &source_view);
            world_post.finish_with(&mut encoder, &color, &group, None);
            let resolved = world_post.resolve_taa(&device, &queue, &mut encoder, None);
            let resolved = resolved.expect("TAA resolves the pack's world");
            let inputs = (&color, &depth, &frame.surface_view);
            post.run_chain(
                &device,
                &mut encoder,
                inputs,
                window,
                window,
                window,
                None,
                None,
            );
            post.run_display(&device, &mut encoder, &screen_view, Some(&resolved), None);
            read_screen(&device, &queue, encoder, &screen, SIZE)
        };
        let first = staircase(SIZE, 0);
        assert_eq!(show(&first), first, "the reset frame is the world");
        let next = show(&staircase(SIZE, 1));
        let blended = next
            .chunks(4)
            .filter(|p| (16..=239).contains(&p[0]))
            .count();
        assert!(blended > 0, "TAA kept nothing of the first frame: {next:?}");
    }

    /// TAA upscales the world only under a pack whose `[tonemap]` can't tell (#16126):
    /// Horizon's and one reading `input.color` can't; one reading `previous`, itself or
    /// through a function it calls, would find it larger than the world.
    #[test]
    fn taa_upscales_only_under_a_tonemap_that_never_reads_previous() {
        let lets = |tonemap: Option<&str>| {
            let report = tonemap_report(tonemap);
            let stage = report.stages.iter().find(|s| s.kind == StageKind::Tonemap);
            assert_eq!(stage.is_some(), tonemap.is_some());
            tonemap_lets_taa_upscale(stage)
        };
        assert!(lets(None));
        assert!(lets(Some(
            "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color * 0.5; }\n"
        )));
        assert!(!lets(Some(READS_PREVIOUS)));
        assert!(!lets(Some(
            "fn around(uv: vec2<f32>) -> vec3<f32> {\n\
             \x20   return textureSampleLevel(previous, linear_sampler, uv, 0.0).rgb;\n}\n\
             fn tonemap(input: TonemapInput) -> vec3<f32> { return around(input.uv); }\n"
        )));
    }

    /// A `[tonemap]` that reads `previous` itself, at the pixel's `uv`.
    const READS_PREVIOUS: &str = "fn tonemap(input: TonemapInput) -> vec3<f32> {\n\
        \x20   return textureSampleLevel(previous, nearest_sampler, input.uv, 0.0).rgb;\n}\n";

    /// A square texture of `size` in `format`, for `usage`.
    fn square(
        device: &wgpu::Device,
        size: u32,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    }

    /// One frame of a pack without `[[post]]` through the renderer's resolve slot, as
    /// `Renderer::render` runs it: `world_post` jitters and prepares for an 8×8 window, the
    /// square world `pixels` is drawn onto its target, TAA resolves it (`resolve_taa`) and
    /// the pack's output reads that, decoded at `resolved_size`, onto the window. Returns the
    /// window's rows and what TAA resolved.
    fn through_taa(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        (post, frame): (&mut NativePost, &NativeFrame),
        world_post: &mut crate::gfx::world_post::Post,
        pixels: &[u8],
    ) -> (Vec<u8>, wgpu::TextureView) {
        use glam::{DVec3, Mat4};
        const WINDOW: u32 = 8;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let window = [WINDOW, WINDOW];
        world_post.jitter(window, true, Mat4::IDENTITY, DVec3::ZERO);
        let (color, depth) = world_post.prepare(device, window, true, false).unwrap();
        let size = world_post.size().unwrap();
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let source = square(device, size[0], format, usage);
        queue.write_texture(
            source.as_image_copy(),
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: None,
            },
            source.size(),
        );
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let screen = square(device, WINDOW, format, usage);
        let screen_view = screen.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        let group = world_post.bind(device, &source.create_view(&Default::default()));
        world_post.finish_with(&mut encoder, &color, &group, None);
        let resolved = world_post.resolve_taa(device, queue, &mut encoder, None);
        let resolved = resolved.expect("TAA resolves the pack's world");
        let display = world_post.resolved_size().unwrap();
        let inputs = (&color, &depth, &frame.surface_view);
        post.run_chain(
            device,
            &mut encoder,
            inputs,
            size,
            display,
            window,
            None,
            None,
        );
        post.run_display(device, &mut encoder, &screen_view, Some(&resolved), None);
        (
            read_screen(device, queue, encoder, &screen, WINDOW),
            resolved,
        )
    }

    /// Under a pack whose `[tonemap]` never reads `previous`, TAA upscales as without a pack
    /// (#16126): a 4×4 world at the Upscaling ratio 2 is resolved and sharpened at 8×8, the
    /// window's size, before the decode. On the second frame, whose history blend leaves the
    /// edge between its two sides (the reset frame copies the world's nearest pixels, which
    /// CAS's clamp leaves as they are), a midtone edge (black and white leave CAS no room)
    /// comes out differently at the Upscaling Sharpness 0 and 100%, while the flat corners
    /// stay as they are.
    #[test]
    fn a_pack_without_a_chain_shows_taa_s_upscaled_and_sharpened_world() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device(true) else {
            eprintln!("TAA upscaling under a pack GPU skipped: no test device");
            return;
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        if !crate::gfx::world_post::Post::new(&device, format).supports_taa() {
            eprintln!("TAA upscaling under a pack GPU skipped: TAA unsupported");
            return;
        }
        // A 4×4 staircase edge from 64 below the diagonal to 192 on and above it.
        let world: Vec<u8> = staircase(4, 0)
            .chunks(4)
            .flat_map(|p| {
                let v = if p[0] == 255 { 192 } else { 64 };
                [v, v, v, 255]
            })
            .collect();
        let (mut post, frame) = identity_tonemap_pack(&device, &queue, format);
        assert!(post.upscalable());
        // A fresh resolve each: the same jitters, the history's first frame and its second.
        let mut shown = |sharpness| {
            let mut world_post = crate::gfx::world_post::Post::new(&device, format);
            world_post.taa = true;
            world_post.upscale = 2.0;
            world_post.set_sharpness(&device, sharpness);
            through_taa(
                &device,
                &queue,
                (&mut post, &frame),
                &mut world_post,
                &world,
            );
            let pack = (&mut post, &frame);
            let (rows, resolved) = through_taa(&device, &queue, pack, &mut world_post, &world);
            assert_eq!(world_post.size(), Some([4, 4]));
            let texture = resolved.texture();
            assert_eq!([texture.width(), texture.height()], [8, 8]);
            assert_eq!(post.display.scene.as_ref().map(|s| s.0), Some([8, 8]));
            rows
        };
        let soft = shown(0.0);
        let sharp = shown(1.0);
        assert_ne!(
            soft, sharp,
            "the sharpening left the midtone edge as it was"
        );
        // Top right and bottom left, far from the edge, are flat in both.
        for (x, y, v) in [(7, 0, 192), (0, 7, 64)] {
            let at = |rows: &[u8]| rows[(y * 8 + x) * 4];
            assert!(at(&soft).abs_diff(v) <= 1, "{soft:?}");
            assert!(at(&sharp).abs_diff(v) <= 1, "{sharp:?}");
        }
    }

    /// Under a pack whose `[tonemap]` reads `previous`, TAA's upscaling is held
    /// (`hold_upscale`): at the Upscaling ratio 2 its 4×4 world (an 8×8 window at scale 0.5)
    /// keeps a 4×4 history, nothing sharpens it, and `previous` is the world's 4×4 picture.
    #[test]
    fn a_tonemap_that_reads_previous_keeps_taa_at_the_world_s_size() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device(true) else {
            eprintln!("TAA upscale hold under a pack GPU skipped: no test device");
            return;
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut world_post = crate::gfx::world_post::Post::new(&device, format);
        if !world_post.supports_taa() {
            eprintln!("TAA upscale hold under a pack GPU skipped: TAA unsupported");
            return;
        }
        world_post.taa = true;
        world_post.scale = 0.5;
        world_post.upscale = 2.0;
        let (mut post, frame) = tonemap_pack(&device, &queue, format, READS_PREVIOUS);
        assert!(!post.upscalable());
        // What `replace_executor` holds for it (`engine_resolve` without `engine_upscale`).
        world_post.hold_upscale(true);
        let pack = (&mut post, &frame);
        let world = staircase(4, 0);
        let (_, resolved) = through_taa(&device, &queue, pack, &mut world_post, &world);
        assert_eq!(world_post.size(), Some([4, 4]));
        let texture = resolved.texture();
        let history = wgpu::TextureFormat::Rgba16Float;
        assert_eq!(
            ([texture.width(), texture.height()], texture.format()),
            ([4, 4], history),
            "TAA's history, unsharpened"
        );
        assert_eq!(post.display.scene.as_ref().map(|s| s.0), Some([4, 4]));
    }

    /// A pack's `Frame` describes the HDR display its picture goes to, and SDR
    /// without one: `hdr` 0, `headroom` 1, `paper_white` and `peak` 80, `black_level` 0.
    #[test]
    fn the_frame_describes_the_display_it_goes_to() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device(false) else {
            return;
        };
        let hud = crate::hud::HudBatch::scratch([4, 4]);
        let frame = crate::gfx::pack_uniforms::initial_history_tests::frame(&hud);
        let mut native =
            NativeFrame::new(&device, None, Vec::new(), Default::default(), None, false);
        let fields = |i: &FrameInputs| (i.hdr, i.headroom, i.paper_white, i.peak, i.black_level);
        assert_eq!(
            fields(&native.next_inputs(&frame, [4, 4], SDR)),
            (0, 1.0, 80.0, 80.0, 0.0)
        );
        let hdr = |headroom, paper_white, peak, black_level| DisplayValues {
            hdr: true,
            headroom,
            paper_white,
            peak,
            black_level,
        };
        assert_eq!(
            fields(&native.next_inputs(&frame, [4, 4], hdr(4.0, 250.0, 1000.0, 0.05))),
            (1, 4.0, 250.0, 1000.0, 0.05)
        );
        // Read every frame: a change of display shows on the next one.
        let next = native.next_inputs(&frame, [4, 4], hdr(2.0, 300.0, 600.0, 0.0));
        assert_eq!(fields(&next).1, 2.0);
        assert_eq!(fields(&native.next_inputs(&frame, [4, 4], SDR)).0, 0);
    }

    /// On the layered HDR frame a pack's light is tone mapped once: by Horizon's curve,
    /// the one HDR without a pack uses, when the pack has no `[tonemap]`, else by the pack's
    /// alone, held to the headroom. In SDR Horizon's is a clip at white. An HDR output switched
    /// to scRGB and back to HDR10 (`Hdr::retargeted`) maps it once all the same.
    #[test]
    fn a_pack_is_tone_mapped_once_by_its_own_curve_or_horizons() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device(true) else {
            return;
        };
        // Light 8 times paper white, as the world's float frame stores it, in gamma.
        let light = 8.0f32;
        let (paper, peak) = (203.0, 1000.0);
        let headroom = peak / paper;
        // The picture shown for that light, in linear light: (a pack's [tonemap], HDR, the
        // output switched).
        let shown = |pack_curve: bool, hdr: bool, switched: bool| {
            let (stage, file, source) = if pack_curve {
                (
                    "[tonemap]",
                    "t.wgsl",
                    "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color; }\n",
                )
            } else {
                (
                    "[sky]",
                    "s.wgsl",
                    "fn sky(input: SkyInput) -> vec4<f32> { return input.color; }\n",
                )
            };
            let manifest = format!(
                "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
                 {stage}\nfile = \"{file}\"\n"
            );
            let files = [(MANIFEST, manifest.as_str()), (file, source)]
                .into_iter()
                .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
                .collect();
            let report = check(&Pack::from_files(files).unwrap()).unwrap();
            let tonemap = report.stages.iter().find(|s| s.kind == StageKind::Tonemap);
            let format = wgpu::TextureFormat::Rgba16Float;
            let frame =
                NativeFrame::new(&device, None, Vec::new(), Default::default(), None, false);
            let inputs = FrameInputs {
                hdr: u32::from(hdr),
                headroom: if hdr { headroom } else { 1.0 },
                ..Default::default()
            };
            queue.write_buffer(&frame.buffer, 0, bytemuck::bytes_of(&inputs));
            let mut post = NativePost::new(
                &device,
                Vec::new(),
                &report.manifest,
                &[],
                format,
                &frame,
                tonemap,
            )
            .unwrap();
            assert_eq!(post.tonemaps(), pack_curve);
            let mut display = super::super::hdr::Hdr::new(
                &device,
                &queue,
                format,
                super::super::hdr::Output::Pq,
                true,
            );
            (display.paper_white, display.peak) = (paper, peak);
            // As the renderer has it under a pack that draws: Horizon lights none of it.
            display.lighting = false;
            if switched {
                use super::super::hdr::Output::{Pq, ScRgb};
                display = display.retargeted(&device, &queue, ScRgb);
                display = display.retargeted(&device, &queue, Pq);
            }
            let screen = display.frame(&device, [4, 4]);
            let usage =
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let color = texture(&device, format, usage);
            let depth = texture(&device, wgpu::TextureFormat::Depth32Float, usage);
            let [color, depth] = [&color, &depth].map(|t| t.create_view(&Default::default()));
            let mut encoder = device.create_command_encoder(&Default::default());
            let g = f64::from(light.powf(1.0 / 2.2));
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: g,
                            g,
                            b: g,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            }));
            let view = screen.create_view(&Default::default());
            let world = (&color, &depth, &frame.surface_view);
            post.run(&device, &mut encoder, world, &view, [4, 4], None, None);
            // The renderer's rule, then its curve, as it runs them after the pack's chain.
            if crate::gfx::horizon_tone_curve(hdr, true, post.tonemaps()) {
                display.begin(&queue, true);
                display.tonemap(&mut encoder, None);
            }
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 256,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                screen.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let bytes = buffer.get_mapped_range(..).unwrap();
            half::f16::from_le_bytes([bytes[0], bytes[1]])
                .to_f32()
                .powf(2.2)
        };
        let near = |got: f32, want: f32| (got - want).abs() <= 0.01 * want;
        // Horizon's curve once: BT.2390's EETF at 203 and 1000 nits puts light 8 at 4.82
        // (worked out apart from the shader, as in the HDR tone curve's own test). Clipped at
        // the headroom first, or mapped twice, it would land lower.
        for switched in [false, true] {
            let horizon = shown(false, true, switched);
            assert!(near(horizon, 4.82043), "{switched}: {horizon}");
            // The pack's identity curve alone: the light held to the headroom, not eased again.
            let pack = shown(true, true, switched);
            assert!(near(pack, headroom), "{switched}: {pack}");
        }
        // SDR: a clip at white, the pack's or Horizon's.
        for pack_curve in [false, true] {
            let sdr = shown(pack_curve, false, false);
            assert!(near(sdr, 1.0), "{pack_curve}: {sdr}");
        }
    }
}
