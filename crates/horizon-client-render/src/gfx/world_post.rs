//! Resolution scale, FXAA and TAA (Horizon settings): the world renders into its
//! own target at the scaled size, then one full-screen pass copies or
//! anti-aliases it onto the screen, under the hand and HUD. With none on,
//! the world renders straight to the screen.
//!
//! TAA jitters the world's projection by a sub-pixel offset each frame and blends
//! the frame into a history reprojected through the depth buffer: its motion
//! vectors are the camera's, so what moves on its own leans on the neighbourhood
//! clamp alone.

use glam::{DMat4, DVec3, Mat4, Vec2};

/// The least a world is scaled to when memory runs out.
const MIN_SCALE: f32 = 0.25;
/// The TAA history format: 8 bits would band as frames accumulate.
const HISTORY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

struct WorldTarget {
    size: [u32; 2],
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    group: wgpu::BindGroup,
    taa: Option<History>,
    /// The world resolved, at `resolved_size`, made on first use (`resolved`); `Some(None)`
    /// where there was no video memory for it, until the target is made again.
    resolved: Option<Option<wgpu::TextureView>>,
}

/// TAA's two history textures at the output size (the world's, times the upscale), written
/// in turn: `resolve[i]` writes `history[i]` from the world and `history[1 - i]`; `show[i]`
/// copies `history[i]` to the screen.
struct History {
    size: [u32; 2],
    history: [wgpu::TextureView; 2],
    resolve: [wgpu::BindGroup; 2],
    show: [wgpu::BindGroup; 2],
}

/// `world_post.wgsl`'s `Taa`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TaaUniform {
    /// This frame's unjittered clip space to the previous frame's.
    reproject: [[f32; 4]; 4],
    /// xy: this frame's jitter in clip space; z: 1 when the history is unusable.
    jitter: [f32; 4],
}

/// TAA's GPU resources, absent when the device cannot compile the shader.
struct TaaResources {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
}

/// One frame handed to an upscaler backend: the jittered world at `render_size` and what
/// it needs to rebuild it at `output_size`. The motion is the camera's alone (`reproject`);
/// objects that move on their own carry no motion vectors yet.
// Only TAA reads it so far; a vendor backend reads the rest (#12918).
#[allow(dead_code)]
pub(super) struct UpscaleFrame<'a> {
    /// The world's colour, rendered with this frame's jitter.
    pub color: &'a wgpu::TextureView,
    /// The world's depth, at `render_size`.
    pub depth: &'a wgpu::TextureView,
    pub render_size: [u32; 2],
    pub output_size: [u32; 2],
    /// The jitter in clip space, as added to the projection.
    pub jitter_clip: [f32; 2],
    /// The same jitter in rendered pixels, y down.
    pub jitter_pixels: [f32; 2],
    /// This frame's unjittered clip space to the previous frame's.
    pub reproject: [[f32; 4]; 4],
    /// The history is unusable: a cut, a resize, a frame without the upscaler.
    pub reset: bool,
    /// Frames jittered so far.
    pub index: u32,
}

/// Something that turns the jittered low-resolution world into the output picture.
/// TAA (`TaaResources`) is the only backend; `Target` is the per-size state a backend
/// keeps beside the world target (TAA's history textures).
pub(super) trait UpscalerBackend {
    type Target;
    /// The name its GPU timing is shown under.
    fn name(&self) -> &'static str;
    /// Write this frame's output and return the bind group that shows it (`draw_onto`).
    fn resolve<'t>(
        &self,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        frame: &UpscaleFrame<'_>,
        target: &'t Self::Target,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) -> &'t wgpu::BindGroup;
}

impl UpscalerBackend for TaaResources {
    type Target = History;

    fn name(&self) -> &'static str {
        "TAA"
    }

    fn resolve<'t>(
        &self,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        frame: &UpscaleFrame<'_>,
        taa: &'t History,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) -> &'t wgpu::BindGroup {
        // The resolve bind group already samples `frame.color` and `frame.depth`.
        let i = (frame.index % 2) as usize;
        let uniform = TaaUniform {
            reproject: frame.reproject,
            jitter: [
                frame.jitter_clip[0],
                frame.jitter_clip[1],
                if frame.reset { 1.0 } else { 0.0 },
                0.0,
            ],
        };
        uploads.buffer(encoder, &self.uniform, 0, bytemuck::bytes_of(&uniform));
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: &taa.history[i],
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon TAA"),
            timestamp_writes: timing.and_then(|t| t.writes(self.name())),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &taa.resolve[i], &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        &taa.show[i]
    }
}

pub struct Post {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    copy: wgpu::RenderPipeline,
    fxaa: wgpu::RenderPipeline,
    /// Contrast-adaptive sharpening of TAA's upscaled picture: the jittered frames are
    /// spread over more output pixels, and the blend softens edges.
    sharpen: wgpu::RenderPipeline,
    /// The strength `sharpen` was built with (`set_sharpness`), and what rebuilding it takes.
    sharpness: f32,
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    /// World resolution relative to the window (0.5..=2.0).
    pub scale: f32,
    pub antialias: bool,
    pub taa: bool,
    /// Output pixels per rendered pixel while TAA is on (1: native): the world renders
    /// this much smaller and TAA rebuilds it at the resolution scale's size.
    pub upscale: f32,
    /// `upscale` is held at 1 (`hold_upscale`): a native pack's world gets TAA at its own size,
    /// where its `[tonemap]` reads `previous` (`Capabilities::engine_upscale`).
    upscale_held: bool,
    /// TAA runs this frame (`jitter`): not on the title screen, under a `shaders/` pack, or
    /// under a native one the renderer doesn't resolve (`Capabilities::engine_resolve`).
    taa_on: bool,
    target: Option<WorldTarget>,
    taa_resources: Option<TaaResources>,
    /// Frames jittered so far (the Halton index), and which history the next resolve writes.
    taa_frame: u32,
    /// The last TAA frame's unjittered `view_proj` and camera, `None` once a frame ran without it.
    taa_previous: Option<(DMat4, DVec3)>,
    taa_this_frame: TaaUniform,
    light_key: Option<LightKey>,
}

/// What the colour TAA's history keeps was drawn under: a change makes the history unusable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum LightKey {
    /// No pack: HDR's gain bakes `settings` into the colour (`[0; 4]` without it).
    Engine { gain: bool, settings: [f32; 4] },
    /// A native (API 0.2) pack's frame, lit for the display its `Frame` describes.
    Pack(super::output::DisplayValues),
}

impl LightKey {
    /// A frame without a pack. Without gain the settings don't change the colour: the
    /// history stays through a change of them alone.
    pub(super) fn engine(gain: bool, settings: [f32; 4]) -> Self {
        let settings = if gain { settings } else { [0.0; 4] };
        Self::Engine { gain, settings }
    }
}

impl Post {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../world_post.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon post bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon post"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = |entry: &str, layout: &wgpu::PipelineLayout, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Horizon post"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_post"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let taa_resources = {
            let taa_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Horizon TAA bindings"),
                entries: &[
                    texture(0, float),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    texture(2, float),
                    // A depth view through an unfilterable float binding: the
                    // texel is the same, and the GLSL backend can read it.
                    texture(3, wgpu::TextureSampleType::Float { filterable: false }),
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
            let taa_pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Horizon TAA"),
                    bind_group_layouts: &[Some(&taa_layout)],
                    ..Default::default()
                });
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
            let taa_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Horizon TAA"),
                layout: Some(&taa_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_post"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_taa"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HISTORY_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            let internal = pollster::block_on(internal.pop());
            match internal.or(pollster::block_on(validation.pop())) {
                None => {
                    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Horizon TAA"),
                        size: std::mem::size_of::<TaaUniform>() as u64,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    Some(TaaResources {
                        layout: taa_layout,
                        pipeline: taa_pipeline,
                        uniform,
                    })
                }
                Some(e) => {
                    log::warn!("Horizon renderer: temporal anti-aliasing unavailable ({e})");
                    None
                }
            }
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Horizon post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            shader: shader.clone(),
            pipeline_layout: pipeline_layout.clone(),
            copy: pipeline("fs_copy", &pipeline_layout, format),
            fxaa: pipeline("fs_fxaa", &pipeline_layout, format),
            sharpen: sharpen_pipeline(device, &shader, &pipeline_layout, format, 0.85),
            sharpness: 0.85,
            layout,
            sampler,
            format,
            scale: 1.0,
            antialias: false,
            taa: false,
            upscale: 1.0,
            upscale_held: false,
            taa_on: false,
            target: None,
            taa_resources,
            taa_frame: 0,
            taa_previous: None,
            taa_this_frame: bytemuck::Zeroable::zeroed(),
            light_key: None,
        }
    }

    // History stores baked light, so a change must reject it before jitter records the next frame.
    pub(super) fn baked_light(&mut self, key: LightKey) {
        if self.light_key != Some(key) {
            self.taa_previous = None;
            self.light_key = Some(key);
        }
    }

    /// TAA's next frame starts without history: what it kept was drawn by another pipeline.
    pub(super) fn forget_history(&mut self) {
        self.taa_previous = None;
    }

    /// Holds TAA's upscaling at 1 (`held`) or lets the Upscaling setting apply again. Set
    /// before a native pack's first frame, so that its world's size, TAA's history and the
    /// culling agree from it on.
    pub(super) fn hold_upscale(&mut self, held: bool) {
        self.upscale_held = held;
    }

    /// The upscaling TAA applies: `upscale`, or 1 while it is held.
    fn taa_upscale(&self) -> f32 {
        if self.upscale_held { 1.0 } else { self.upscale }
    }

    /// Whether TAA is supported on this GPU/backend.
    pub fn supports_taa(&self) -> bool {
        self.taa_resources.is_some()
    }

    /// The world's rendered size and the size it is shown at (TAA's history), smaller by the
    /// upscaling ratio `pack` renders it for (`prepare`).
    fn sizes(&self, window: [u32; 2], pack: bool) -> ([u32; 2], [u32; 2]) {
        let upscale = if pack {
            self.upscale
        } else if self.taa_on {
            self.taa_upscale()
        } else {
            1.0
        };
        self.sizes_with_upscale(window, upscale)
    }

    fn sizes_with_upscale(&self, window: [u32; 2], upscale: f32) -> ([u32; 2], [u32; 2]) {
        sizes(window, self.scale, upscale)
    }

    /// The size a native (API 0.2) pack's world draws at: it always has a target (`prepare`'s
    /// `always`) at TAA's sizes, not a `shaders/` pack's. Smaller by the Upscaling ratio only
    /// where TAA upscales it (`upscaled`: `Capabilities::engine_upscale`); `upscale` is 1
    /// while TAA is off.
    pub(super) fn pack_render_size(&self, window: [u32; 2], upscaled: bool) -> [u32; 2] {
        let upscale = if upscaled { self.upscale } else { 1.0 };
        self.sizes_with_upscale(window, upscale).0
    }

    /// The scaled world's depth where it was drawn at `size`, the window's: what post effects
    /// read when 4x MSAA drew the world there.
    pub(super) fn depth_at(&self, size: [u32; 2]) -> Option<&wgpu::TextureView> {
        (self.target.as_ref())
            .filter(|t| t.size == size)
            .map(|t| &t.depth)
    }

    /// The next TAA frame's input size, even before its first jitter or resize.
    pub(super) fn taa_render_size(&self, window: [u32; 2]) -> [u32; 2] {
        self.sizes_with_upscale(window, self.taa_upscale()).0
    }

    /// TAA's sub-pixel jitter this frame, as a clip-space matrix to multiply into the
    /// world's projection (identity while `on` is false); `view_proj` is the unjittered
    /// projection, relative to `camera`. Call it before `prepare`, once per frame.
    pub fn jitter(&mut self, window: [u32; 2], on: bool, view_proj: Mat4, camera: DVec3) -> Mat4 {
        self.taa_on = on && self.taa && self.taa_resources.is_some();
        if !self.taa_on {
            self.taa_previous = None;
            return Mat4::IDENTITY;
        }
        self.taa_frame = self.taa_frame.wrapping_add(1);
        let (size, _) = self.sizes(window, false);
        // Halton (2, 3) over 8 frames per output pixel (FSR's count), in rendered
        // pixels around the centre.
        let upscale = self.taa_upscale();
        let phases = (8.0 * upscale * upscale).ceil() as u32;
        let i = self.taa_frame % phases + 1;
        let offset = Vec2::new(halton(i, 2), halton(i, 3)) - 0.5;
        let jitter = offset * 2.0 / Vec2::new(size[0] as f32, size[1] as f32);
        let current = view_proj.as_dmat4();
        let (reproject, reset) = match self.taa_previous {
            Some((previous, previous_camera)) => (
                reprojection(previous, previous_camera, current, camera),
                0.0,
            ),
            None => (DMat4::IDENTITY, 1.0),
        };
        self.taa_previous = Some((current, camera));
        self.taa_this_frame = TaaUniform {
            reproject: reproject.as_mat4().to_cols_array_2d(),
            jitter: [jitter.x, jitter.y, reset, 0.0],
        };
        Mat4::from_translation(jitter.extend(0.0))
    }

    /// This frame's jitter in clip space, the translation `jitter` multiplies into the
    /// projection (`frame.jitter` of a 0.2 pack); zero without TAA.
    pub fn jitter_clip(&self) -> [f32; 2] {
        let j = self.taa_this_frame.jitter;
        if self.taa_on { [j[0], j[1]] } else { [0.0; 2] }
    }

    /// This frame's jitter in rendered pixels, y down (FSR's `jitterOffset`); zero
    /// without TAA.
    pub fn jitter_pixels(&self) -> [f32; 2] {
        let Some(size) = self.size().filter(|_| self.taa_on) else {
            return [0.0; 2];
        };
        let j = self.taa_this_frame.jitter;
        [j[0] * size[0] as f32 * 0.5, -j[1] * size[1] as f32 * 0.5]
    }

    /// The world's colour and depth targets this frame, or `None` to draw
    /// it straight to the screen.
    /// `always`: a target even at scale 1 without anti-aliasing (a shader pack reads it).
    /// `pack`: a `shaders/` pack reads the world, so it renders at the Upscaling ratio's share
    /// of the window — the pack's own copy of it stretches the smaller picture back, and TAA,
    /// which would divide for the same reason, never runs under it. Frame generation and a
    /// native (API 0.2) pack also read the world but stretch nothing themselves: `false` keeps
    /// them, and a frame without a pack, at TAA's sizes.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        window: [u32; 2],
        always: bool,
        pack: bool,
    ) -> Option<(wgpu::TextureView, wgpu::TextureView)> {
        if self.scale == 1.0 && !self.antialias && !self.taa_on && !always {
            self.target = None;
            return None;
        }
        // The old target goes first: a slider drag would otherwise hold the old and the new
        // targets together, and the sum is what runs a small GPU out of memory.
        loop {
            let (size, output) = self.sizes(window, pack);
            let history_size = self.taa_on.then_some(output);
            if self
                .target
                .as_ref()
                .is_some_and(|t| t.size == size && t.taa.as_ref().map(|h| h.size) == history_size)
            {
                break;
            }
            self.target = None;
            // Errors raised are fatal, so catch an out-of-memory one and scale down. A texture
            // that failed is invalid, so its view fails validation too: catch that as well.
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let target = self.build_target(device, size, history_size);
            let error = pollster::block_on(memory.pop()).or(pollster::block_on(validation.pop()));
            if error.is_none() {
                // New textures: no history yet.
                self.taa_this_frame.jitter[2] = 1.0;
                self.target = Some(target);
                break;
            }
            drop(target);
            if self.scale <= MIN_SCALE {
                log::warn!(
                    "Horizon renderer: no video memory for the scaled world at {size:?} ({}); drawing at the window's size",
                    error.map_or_else(String::new, |e| e.to_string())
                );
                self.scale = 1.0;
                self.target = None;
                return None;
            }
            let reduced = (self.scale * 0.75).max(MIN_SCALE);
            log::warn!(
                "Horizon renderer: out of video memory for the scaled world at {size:?}; resolution scale {:.2} -> {reduced:.2} ({})",
                self.scale,
                error.map_or_else(String::new, |e| e.to_string())
            );
            self.scale = reduced;
            if self.scale == 1.0 && !self.antialias && !self.taa_on && !always {
                return None;
            }
        }
        self.target
            .as_ref()
            .map(|t| (t.color.clone(), t.depth.clone()))
    }

    /// The world's colour and depth targets at `size`, and TAA's history when `history_size`
    /// is set.
    fn build_target(
        &self,
        device: &wgpu::Device,
        size: [u32; 2],
        history_size: Option<[u32; 2]>,
    ) -> WorldTarget {
        {
            let make_tex = |label, size: [u32; 2], format, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
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
            // COPY_SRC: a native pack's material stage reads copies of them (`opaque_depth`,
            // `opaque_color`).
            let sampled = wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC;
            let color = make_tex("Scaled world", size, self.format, sampled)
                .create_view(&Default::default());
            // TEXTURE_BINDING: occlusion culling and TAA read it.
            let depth = make_tex(
                "Scaled world depth",
                size,
                wgpu::TextureFormat::Depth32Float,
                sampled | wgpu::TextureUsages::COPY_SRC,
            )
            .create_view(&Default::default());
            let group = self.bind(device, &color);
            let taa = history_size.and_then(|output| {
                let res = self.taa_resources.as_ref()?;
                let history = [0, 1].map(|_| {
                    make_tex("TAA history", output, HISTORY_FORMAT, sampled)
                        .create_view(&Default::default())
                });
                let resolve = [0, 1].map(|i| {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Horizon TAA"),
                        layout: &res.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&color),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(&history[1 - i]),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&depth),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: res.uniform.as_entire_binding(),
                            },
                        ],
                    })
                });
                let show = [0, 1].map(|i| self.bind(device, &history[i]));
                Some(History {
                    size: output,
                    history,
                    resolve,
                    show,
                })
            });
            WorldTarget {
                size,
                color,
                depth,
                group,
                taa,
                resolved: None,
            }
        }
    }

    /// The Upscaling Sharpness setting, 0..=1: rebuilds the sharpen pipeline when it changed.
    pub fn set_sharpness(&mut self, device: &wgpu::Device, sharpness: f32) {
        let sharpness = sharpness.clamp(0.0, 1.0);
        if sharpness != self.sharpness {
            self.sharpen = sharpen_pipeline(
                device,
                &self.shader,
                &self.pipeline_layout,
                self.format,
                sharpness,
            );
            self.sharpness = sharpness;
        }
    }

    /// A target in the world's format for `finish` or the sharpening to draw onto, where
    /// something after it reads the world resolved (a native pack without a post chain,
    /// #16126), at `resolved_size`. Made on first use, and dropped with the world's target.
    /// None where there is no video memory for it: the pack then reads the world without FXAA,
    /// or TAA's history unsharpened.
    pub(super) fn resolved(&mut self, device: &wgpu::Device) -> Option<wgpu::TextureView> {
        let format = self.format;
        let size = self.resolved_size()?;
        let target = self.target.as_mut()?;
        let view = target.resolved.get_or_insert_with(|| {
            // As the world's own target: an error raised is fatal, so catch running out of
            // memory, and the failed texture's view failing validation.
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let view = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Resolved world"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            let error = pollster::block_on(memory.pop()).or(pollster::block_on(validation.pop()));
            if let Some(error) = error {
                log::warn!(
                    "Horizon renderer: no video memory for the shader pack's FXAA or sharpening at {size:?}; drawing it without ({error})"
                );
                return None;
            }
            Some(view)
        });
        view.clone()
    }

    /// The size of the world's own target, when it has one.
    pub fn size(&self) -> Option<[u32; 2]> {
        self.target.as_ref().map(|t| t.size)
    }

    /// The size of the world resolved (`resolve_taa`, `resolved`): TAA's history's, which its
    /// upscaling makes larger than the world's, else the world's target's.
    pub(super) fn resolved_size(&self) -> Option<[u32; 2]> {
        let target = self.target.as_ref()?;
        Some(target.taa.as_ref().map_or(target.size, |h| h.size))
    }

    /// Bindings that copy (or anti-alias) `view` onto the screen in `finish_with`.
    pub fn bind(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon post"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Draw the scaled world onto `screen`, through TAA when this frame's `jitter` had it on.
    pub fn finish(
        &self,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        screen: &wgpu::TextureView,
        mut timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        let Some(target) = &self.target else { return };
        let Some((res, frame, taa)) = self.taa_frame() else {
            self.finish_with(encoder, screen, &target.group, timing.as_deref_mut());
            return;
        };
        let shown = res.resolve(uploads, encoder, &frame, taa, timing.as_deref_mut());
        let pipeline = if self.taa_upscale() > 1.0 {
            &self.sharpen
        } else {
            &self.copy
        };
        self.draw_onto(encoder, screen, shown, pipeline, timing);
    }

    /// TAA's resolve of this frame, when `jitter` had it on: the world anti-aliased, for a
    /// native pack's output to read in the world's place (#16126). At the world's size while
    /// the upscaling is held (`hold_upscale`), the history itself. Upscaled, the history
    /// sharpened as without a pack, at its size onto `resolved`, or unsharpened where there is
    /// no video memory for that.
    pub(super) fn resolve_taa_staged(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        mut timing: Option<&mut super::pack_timing::PassTiming>,
    ) -> Option<wgpu::TextureView> {
        // Made only for a frame TAA resolves.
        let sharpened = (self.taa_frame().is_some() && self.taa_upscale() > 1.0)
            .then(|| self.resolved(device))
            .flatten();
        let (res, frame, taa) = self.taa_frame()?;
        let shown = res.resolve(uploads, encoder, &frame, taa, timing.as_deref_mut());
        let Some(sharpened) = sharpened else {
            return Some(taa.history[(frame.index % 2) as usize].clone());
        };
        self.draw_onto(encoder, &sharpened, shown, &self.sharpen, timing);
        Some(sharpened)
    }

    #[cfg(test)]
    pub(super) fn resolve_taa(
        &mut self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) -> Option<wgpu::TextureView> {
        let mut uploads = super::upload::Uploads::new(device);
        let view = self.resolve_taa_staged(device, &mut uploads, encoder, timing);
        uploads.finish(encoder);
        view
    }

    /// What TAA resolves this frame, when `jitter` had it on and the target has a history.
    fn taa_frame(&self) -> Option<(&TaaResources, UpscaleFrame<'_>, &History)> {
        let target = self.target.as_ref()?;
        let taa = target
            .taa
            .as_ref()
            .filter(|_| self.taa_previous.is_some())?;
        let res = self.taa_resources.as_ref()?;
        let frame = UpscaleFrame {
            color: &target.color,
            depth: &target.depth,
            render_size: target.size,
            output_size: taa.size,
            jitter_clip: [self.taa_this_frame.jitter[0], self.taa_this_frame.jitter[1]],
            jitter_pixels: self.jitter_pixels(),
            reproject: self.taa_this_frame.reproject,
            reset: self.taa_this_frame.jitter[2] > 0.5,
            index: self.taa_frame,
        };
        Some((res, frame, taa))
    }

    /// Draw what `group` binds (`bind`) onto `screen`.
    pub fn finish_with(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        screen: &wgpu::TextureView,
        group: &wgpu::BindGroup,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        let pipeline = if self.antialias {
            &self.fxaa
        } else {
            &self.copy
        };
        self.draw_onto(encoder, screen, group, pipeline, timing);
    }

    /// Draw what `group` binds onto `screen` through `pipeline`.
    fn draw_onto(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        screen: &wgpu::TextureView,
        group: &wgpu::BindGroup,
        pipeline: &wgpu::RenderPipeline,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: screen,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon post"),
            timestamp_writes: timing.and_then(|t| t.writes("Post")),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// `fs_sharpen` at `sharpness` (its override), drawing onto `format`.
fn sharpen_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    sharpness: f32,
) -> wgpu::RenderPipeline {
    let constants = [("sharpness", f64::from(sharpness))];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Horizon sharpen"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_post"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_sharpen"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            },
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

/// This frame's clip space to the previous frame's, for camera-relative projections:
/// a point `camera - previous_camera` further from the previous camera than from this one.
pub(super) fn reprojection(
    previous: DMat4,
    previous_camera: DVec3,
    current: DMat4,
    camera: DVec3,
) -> DMat4 {
    previous * DMat4::from_translation(camera - previous_camera) * current.inverse()
}

/// The radical inverse of `i` in `base`: a low-discrepancy sequence in 0..1.
/// The world's size and the output's for `window` at resolution scale `scale`, the world
/// drawn smaller by TAA's `upscale` (1 without it).
fn sizes(window: [u32; 2], scale: f32, upscale: f32) -> ([u32; 2], [u32; 2]) {
    let output = window.map(|v| ((v as f32 * scale).round() as u32).clamp(1, 8192));
    (
        output.map(|v| ((v as f32 / upscale).round() as u32).max(1)),
        output,
    )
}

fn halton(mut i: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0, 0.0);
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reprojection_finds_where_a_block_was_last_frame() {
        let projection = DMat4::perspective_rh(1.2, 16.0 / 9.0, 1024.0, 0.05);
        let previous = projection * DMat4::from_rotation_y(0.3);
        let current = projection * DMat4::from_rotation_y(0.35);
        let (previous_camera, camera) = (DVec3::new(10.0, 64.0, 5.0), DVec3::new(10.5, 64.2, 4.0));
        let block = DVec3::new(14.0, 66.0, -8.0);
        let clip = |m: DMat4, eye: DVec3| m * (block - eye).extend(1.0);
        let now = clip(current, camera);
        let then = reprojection(previous, previous_camera, current, camera) * (now / now.w);
        let expected = clip(previous, previous_camera);
        assert!((then.truncate() / then.w - expected.truncate() / expected.w).length() < 1e-9);
    }

    /// A native (API 0.2) pack's kept resources are made before its first frame at the size
    /// it draws at: TAA's render size where TAA upscales it, else none of TAA's upscaling
    /// (`hold_upscale`), and the scale applied with or without a target yet.
    #[test]
    fn a_pack_draws_at_the_scaled_window_without_taa_s_upscaling() {
        for (window, scale, upscale, before, pack) in [
            ([1280, 720], 1.0, 2.0, [640, 360], [1280, 720]),
            ([1281, 721], 0.5, 2.0, [321, 181], [641, 361]),
            ([1920, 1080], 0.75, 1.0, [1440, 810], [1440, 810]),
            ([10000, 10000], 2.0, 4.0, [2048, 2048], [8192, 8192]),
        ] {
            assert_eq!(sizes(window, scale, upscale).0, before);
            assert_eq!(sizes(window, scale, 1.0).0, pack);
        }
    }

    /// A `shaders/` pack's own copy of the world stretches the smaller picture back to the
    /// window, so the world renders at the Upscaling ratio's share of it: 1280×720 at
    /// Performance's 2 is 640×360 (#16757). Without the pack the sizes stay what they were.
    #[test]
    fn a_shaders_pack_s_world_renders_at_the_upscaling_ratio() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = test_device() else {
            eprintln!("Upscaling pack GPU skipped: no test device");
            return;
        };
        let mut post = Post::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        post.upscale = 2.0;
        let _ = post.prepare(&device, [1280, 720], true, true);
        assert_eq!(post.size(), Some([640, 360]));
        let _ = post.prepare(&device, [1280, 720], true, false);
        assert_eq!(post.size(), Some([1280, 720]));
    }

    #[test]
    fn jitter_stays_inside_the_pixel() {
        for i in 1..=128 {
            let (x, y) = (halton(i, 2), halton(i, 3));
            assert!((0.0..1.0).contains(&x) && (0.0..1.0).contains(&y));
        }
        assert_eq!(
            [halton(1, 2), halton(2, 2), halton(3, 2)],
            [0.5, 0.25, 0.75]
        );
    }

    #[test]
    fn culling_uses_the_next_taa_size_before_the_first_jitter_or_resize() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = test_device() else {
            return;
        };
        let mut post = Post::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        assert!(!post.taa_on);
        assert!(post.size().is_none());
        for (window, scale, upscale, expected) in [
            ([1280, 720], 1.0, 1.0, [1280, 720]),
            ([1281, 721], 0.5, 2.0, [321, 181]),
            ([3840, 2160], 0.25, 4.0, [240, 135]),
            ([10000, 10000], 2.0, 4.0, [2048, 2048]),
            ([0, 0], 0.5, 4.0, [1, 1]),
        ] {
            post.scale = scale;
            post.upscale = upscale;
            post.taa_on = false;
            assert_eq!(post.taa_render_size(window), expected);
            post.taa_on = true;
            assert_eq!(post.sizes(window, false).0, expected);
            // A native pack's frame without TAA: what its kept resources are made for.
            post.taa_on = false;
            assert_eq!(
                post.pack_render_size(window, false),
                post.sizes(window, false).0
            );
            // One TAA upscales: they are made at TAA's render size.
            assert_eq!(post.pack_render_size(window, true), expected);
        }
    }

    /// Under a native pack the renderer resolves, TAA's upscaling is held at 1 from the pack's
    /// install on (#16126): the world draws, TAA's history keeps and the culling widens for
    /// the scaled window, the size the pack's kept resources are made for.
    #[test]
    fn a_resolved_pack_gets_taa_at_its_world_s_size() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = test_device() else {
            eprintln!("TAA upscale hold GPU skipped: no test device");
            return;
        };
        let mut post = Post::new(&device, wgpu::TextureFormat::Rgba16Float);
        post.taa = post.supports_taa();
        let (window, scaled) = ([1281, 721], [641, 361]);
        post.scale = 0.5;
        post.upscale = 2.0;
        assert_eq!(post.taa_render_size(window), [321, 181]);
        post.hold_upscale(true);
        assert_eq!(post.taa_render_size(window), scaled);
        assert_eq!(post.pack_render_size(window, false), scaled);
        post.jitter(window, true, Mat4::IDENTITY, DVec3::ZERO);
        let _ = post.prepare(&device, window, true, false);
        assert_eq!(post.size(), Some(scaled));
        let history = post.target.as_ref().and_then(|t| t.taa.as_ref());
        assert_eq!(history.map(|h| h.size), post.taa.then_some(scaled));
        // A `shaders/` pack's world keeps the Upscaling ratio's share of the window.
        assert_eq!(post.sizes(window, true).0, [321, 181]);
        post.hold_upscale(false);
        assert_eq!(post.taa_render_size(window), [321, 181]);
    }

    /// Under a native pack TAA may upscale (`Capabilities::engine_upscale`, #16126), the
    /// world draws at TAA's render size, the size the pack's kept resources are made for,
    /// while TAA's history and the sharpened picture it resolves to keep the scaled window's.
    /// At the Upscaling ratio 1 nothing is sharpened: the pack reads the history itself.
    #[test]
    fn an_upscaled_pack_draws_small_and_resolves_sharpened_at_the_scaled_window() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = test_device() else {
            eprintln!("TAA upscaling under a pack GPU skipped: no test device");
            return;
        };
        let mut post = Post::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        if !post.supports_taa() {
            eprintln!("TAA upscaling under a pack GPU skipped: TAA unsupported");
            return;
        }
        post.taa = true;
        let (window, scaled, drawn) = ([1281, 721], [641, 361], [321, 181]);
        post.scale = 0.5;
        let resolve = |post: &mut Post, upscale| {
            post.upscale = upscale;
            post.jitter(window, true, Mat4::IDENTITY, DVec3::ZERO);
            let _ = post.prepare(&device, window, true, false);
            let mut encoder = device.create_command_encoder(&Default::default());
            let resolved = post.resolve_taa(&device, &queue, &mut encoder, None);
            queue.submit([encoder.finish()]);
            let view = resolved.expect("TAA resolves the pack's world");
            let texture = view.texture();
            ([texture.width(), texture.height()], texture.format())
        };
        post.upscale = 2.0;
        assert_eq!(post.pack_render_size(window, true), drawn);
        let sharpened = resolve(&mut post, 2.0);
        assert_eq!(post.size(), Some(drawn));
        assert_eq!(post.resolved_size(), Some(scaled));
        // The sharpening's target is in the world's format; the history is `Rgba16Float`.
        assert_eq!(sharpened, (scaled, wgpu::TextureFormat::Rgba8Unorm));
        let history = resolve(&mut post, 1.0);
        assert_eq!(post.size(), Some(scaled));
        assert_eq!(history, (scaled, HISTORY_FORMAT));
    }

    #[test]
    fn baked_light_changes_and_returning_from_a_pack_reject_taa_history() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = test_device() else {
            eprintln!("TAA baked-light GPU skipped: no test device");
            return;
        };
        let mut post = Post::new(&device, wgpu::TextureFormat::Rgba16Float);
        post.taa = true;
        if !post.supports_taa() {
            eprintln!("TAA baked-light GPU skipped: TAA unsupported");
            return;
        }
        let key = [5.0, 4.0, 1.0, 0.0];
        for gain in [true, false, true] {
            post.baked_light(LightKey::engine(gain, key));
            post.jitter([32, 32], true, Mat4::IDENTITY, DVec3::ZERO);
            assert_eq!(post.taa_this_frame.jitter[2], 1.0);
            post.baked_light(LightKey::engine(gain, key));
            post.jitter([32, 32], true, Mat4::IDENTITY, DVec3::ZERO);
            assert_eq!(post.taa_this_frame.jitter[2], 0.0);
        }
        for field in 0..4 {
            let mut changed = key;
            changed[field] += 0.01;
            post.baked_light(LightKey::engine(true, changed));
            post.jitter([32, 32], true, Mat4::IDENTITY, DVec3::ZERO);
            assert_eq!(post.taa_this_frame.jitter[2], 1.0);
        }
        post.baked_light(LightKey::engine(false, key));
        post.jitter([32, 32], true, Mat4::IDENTITY, DVec3::ZERO);
        assert_eq!(post.taa_this_frame.jitter[2], 1.0);
        post.baked_light(LightKey::engine(false, [0.0; 4]));
        post.jitter([32, 32], true, Mat4::IDENTITY, DVec3::ZERO);
        assert_eq!(post.taa_this_frame.jitter[2], 0.0);
        // A pack disables TAA even when the light settings stay the same. Its first
        // world frame after the pack is dropped must not reuse the pre-pack history.
        post.jitter([32, 32], false, Mat4::IDENTITY, DVec3::ZERO);
        post.jitter([32, 32], true, Mat4::IDENTITY, DVec3::ZERO);
        assert_eq!(post.taa_this_frame.jitter[2], 1.0);
        post.jitter([32, 32], true, Mat4::IDENTITY, DVec3::ZERO);
        assert_eq!(post.taa_this_frame.jitter[2], 0.0);
        // A pack the renderer resolves keeps TAA on: its install, the display's values and
        // the return to no pack each drop the history, and its frames alone keep it.
        let display = super::super::output::DisplayValues::SDR;
        post.forget_history();
        post.baked_light(LightKey::engine(false, [0.0; 4]));
        post.jitter([32, 32], true, Mat4::IDENTITY, DVec3::ZERO);
        assert_eq!(post.taa_this_frame.jitter[2], 1.0);
        for (key, reset) in [
            (LightKey::Pack(display), 1.0),
            (LightKey::Pack(display), 0.0),
            (
                LightKey::Pack(super::super::output::DisplayValues {
                    hdr: true,
                    ..display
                }),
                1.0,
            ),
            (LightKey::engine(false, [0.0; 4]), 1.0),
        ] {
            post.baked_light(key);
            post.jitter([32, 32], true, Mat4::IDENTITY, DVec3::ZERO);
            assert_eq!(post.taa_this_frame.jitter[2], reset, "{key:?}");
        }
        eprintln!(
            "TAA baked-light GPU checked gain, all settings, a pack's display and the first frame after a pack"
        );
    }

    fn test_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    #[test]
    fn world_post_pipelines_compile_or_fall_back() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = test_device() else {
            return;
        };
        let post = Post::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        // Copy and FXAA pipelines always compile; TAA compiles if supported by backend.
        let _ = post.supports_taa();
    }
}
