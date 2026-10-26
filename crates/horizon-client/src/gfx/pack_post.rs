//! A shader pack's post-processing over Horizon's own world (spec 2.1 rows 4, 6, 8, 9; 6.1; 6.2;
//! 6.4): the world renders into its own target as with a resolution scale, its colour becomes
//! colortex0 and its depth `depthtex0`, then the pack's `prepare`, `deferred`, `composite` and
//! `final` programs run in order, and `final` draws onto the screen under the hand and HUD.
//! Gbuffers programs come later; until then the pack's composites read Horizon's picture.
//!
//! Every colortex buffer a program reads or writes has two textures, A and B. Programs read
//! "main" and write "alt", and each buffer a program wrote flips afterwards (6.1.6). The
//! mapping carries over to the next frame, so a buffer that is not cleared reads back what was
//! written last (OQ-8). Buffers are cleared at the start of the frame, "main" only.
//! Pack buffers store rows as GL does (row 0 at the bottom: the translator negates clip y), so
//! the world is copied in upside down, and its reverse-Z depth becomes GL window depth.

use std::collections::HashMap;

use horizon_shaders::{Format, Group, Kind, Post};

use super::pack_uniforms::{self, FrameUniforms, PackInputs};

const BUFFERS: usize = horizon_shaders::BUFFERS;

/// The texture behind a pack sampler name.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    Color(usize),
    Depth,
    Noise,
    /// A 1×1 stand-in for names this slice does not provide yet (custom textures...).
    Dummy,
    /// A 1×1 depth of 1.0 behind a comparison sampler, until shadow maps exist: no shadow.
    Shadow,
}

struct Binding {
    source: Source,
    binding: u32,
    /// A filtering sampler (the texture is filterable).
    filtering: bool,
}

struct Pass {
    name: String,
    group: Group,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    ubo: wgpu::Buffer,
    block: Vec<u8>,
    /// (index into `pack_uniforms::NAMES`, declared type, offset).
    uniforms: Vec<(Option<usize>, String, u32)>,
    samplers: Vec<Binding>,
    /// (location, buffer) of each output that reaches a buffer.
    writes: Vec<(u32, usize)>,
    /// Buffers flipped after the pass.
    flip: u16,
    /// Buffers sampled, for the bind group cache key.
    reads: u16,
    /// Buffers written.
    written: u16,
    groups: HashMap<u16, wgpu::BindGroup>,
}

struct Targets {
    size: [u32; 2],
    /// A and B of each buffer used.
    color: [Option<[wgpu::TextureView; 2]>; BUFFERS],
    depth: wgpu::TextureView,
}

pub struct PackPost {
    pub notes: Vec<String>,
    passes: Vec<Pass>,
    deferred_pre: u16,
    composite_pre: u16,
    formats: [wgpu::TextureFormat; BUFFERS],
    clears: [Option<[f32; 4]>; BUFFERS],
    used: u16,
    /// Bit n set: B is buffer n's "main".
    main: u16,
    targets: Option<Targets>,
    noise: wgpu::TextureView,
    dummy: wgpu::TextureView,
    /// The stand-in shadow map and its comparison sampler; cleared on the first frame.
    shadow: (wgpu::TextureView, wgpu::Sampler),
    shadow_cleared: bool,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    repeat: wgpu::Sampler,
    blit: (wgpu::RenderPipeline, wgpu::BindGroupLayout),
    /// The blit also writes depthtex0 (some program reads it) and clears these buffers.
    blit_depth: bool,
    blit_clears: Vec<usize>,
    /// Buffers cleared in passes of their own (more than the blit's eight attachments).
    other_clears: Vec<usize>,
    /// The blit's bindings, for the world target of this size.
    blit_group: Option<([u32; 2], wgpu::BindGroup)>,
    /// Shows colortex0 when the pack has no `final` (OQ-5).
    copy: (wgpu::RenderPipeline, wgpu::BindGroupLayout),
    uniforms: FrameUniforms,
}

/// The wgpu format of a pack format (6.7), the nearest one wgpu can render to.
fn texture_format(f: Format, features: wgpu::Features) -> wgpu::TextureFormat {
    use wgpu::TextureFormat as T;
    let pick = |r, rg, rgba| match f.channels {
        1 => r,
        2 => rg,
        _ => rgba,
    };
    let half = pick(T::R16Float, T::Rg16Float, T::Rgba16Float);
    match (f.kind, f.bits) {
        (Kind::Unorm, 10) => T::Rgb10a2Unorm,
        (Kind::Unorm, 16) if features.contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM) => {
            pick(T::R16Unorm, T::Rg16Unorm, T::Rgba16Unorm)
        }
        (Kind::Unorm, b) if b <= 8 => pick(T::R8Unorm, T::Rg8Unorm, T::Rgba8Unorm),
        (Kind::Float, 32) => pick(T::R32Float, T::Rg32Float, T::Rgba32Float),
        (Kind::Float, 11) if features.contains(wgpu::Features::RG11B10UFLOAT_RENDERABLE) => {
            T::Rg11b10Ufloat
        }
        (Kind::Int, 8) => pick(T::R8Sint, T::Rg8Sint, T::Rgba8Sint),
        (Kind::Int, 16) => pick(T::R16Sint, T::Rg16Sint, T::Rgba16Sint),
        (Kind::Int, _) => pick(T::R32Sint, T::Rg32Sint, T::Rgba32Sint),
        (Kind::Uint, 8) => pick(T::R8Uint, T::Rg8Uint, T::Rgba8Uint),
        (Kind::Uint, 10) => T::Rgb10a2Uint,
        (Kind::Uint, 16) => pick(T::R16Uint, T::Rg16Uint, T::Rgba16Uint),
        (Kind::Uint, _) => pick(T::R32Uint, T::Rg32Uint, T::Rgba32Uint),
        // Snorm, 16-bit norm without the feature, RGB9_E5: half floats hold them.
        _ => half,
    }
}

fn source(name: &str) -> Source {
    match name {
        "depthtex0" | "gdepthtex" | "depthtex1" | "depthtex2" => Source::Depth,
        "noisetex" => Source::Noise,
        n => horizon_shaders::buffer_index(n).map_or(Source::Dummy, Source::Color),
    }
}

/// 256×256 RGB white noise (6.4), from a fixed seed so every run looks the same.
fn noise(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let texels: Vec<u8> = (0..256 * 256 * 4)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            if i % 4 == 3 { 255 } else { (state >> 56) as u8 }
        })
        .collect();
    texture(device, queue, "Shader pack noisetex", 256, &texels)
}

fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    side: u32,
    texels: &[u8],
) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width: side,
        height: side,
        depth_or_array_layers: 1,
    };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        tex.as_image_copy(),
        texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(side * 4),
            rows_per_image: None,
        },
        size,
    );
    tex.create_view(&Default::default())
}

fn sampler(
    device: &wgpu::Device,
    filter: wgpu::FilterMode,
    address: wgpu::AddressMode,
) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Shader pack"),
        address_mode_u: address,
        address_mode_v: address,
        address_mode_w: address,
        mag_filter: filter,
        min_filter: filter,
        ..Default::default()
    })
}

/// Runs `create` and returns its validation error, if any.
fn checked<T>(device: &wgpu::Device, create: impl FnOnce() -> T) -> Result<T, String> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = create();
    match pollster::block_on(scope.pop()) {
        Some(e) => {
            // The details are in the error's sources.
            let mut message = e.to_string();
            let mut source = std::error::Error::source(&e);
            while let Some(s) = source {
                message = format!("{message}: {s}");
                source = s.source();
            }
            Err(message)
        }
        None => Ok(value),
    }
}

/// A full-screen helper pipeline of ours (`pack_post.wgsl`).
fn helper(
    device: &wgpu::Device,
    vertex: &str,
    entry: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
    targets: &[Option<wgpu::ColorTargetState>],
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Shader pack helpers"),
        source: wgpu::ShaderSource::Wgsl(include_str!("pack_post.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(entry),
        entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(entry),
        bind_group_layouts: &[Some(&layout)],
        ..Default::default()
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some(vertex),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets,
        }),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, layout)
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty,
        count: None,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}

fn target(format: wgpu::TextureFormat) -> Option<wgpu::ColorTargetState> {
    Some(wgpu::ColorTargetState {
        format,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    })
}

fn attach(
    view: &wgpu::TextureView,
    ops: wgpu::Operations<wgpu::Color>,
) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        depth_slice: None,
        ops,
    })
}

impl PackPost {
    /// Builds the pipelines of `post` for a screen of `screen` format.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        post: Post,
        screen: wgpu::TextureFormat,
    ) -> Self {
        let features = device.features();
        let formats: [wgpu::TextureFormat; BUFFERS] =
            std::array::from_fn(|i| texture_format(post.config.buffers[i].format, features));
        let clears = std::array::from_fn(|i| {
            let b = &post.config.buffers[i];
            b.clear.then(|| {
                b.clear_color
                    .unwrap_or(if i == 1 { [1.0; 4] } else { [0.0; 4] })
            })
        });
        let mut notes = post.notes;
        notes.extend(post.config.notes);
        let mask =
            |flips: &[(u32, bool)]| flips.iter().filter(|f| f.1).fold(0u16, |m, f| m | 1 << f.0);
        let mut used = 1u16;
        let mut passes = Vec::new();
        for p in post.programs {
            let interface = &p.program.interface;
            let mut entries = vec![entry(
                0,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            )];
            let mut samplers = Vec::new();
            let mut reads = 0u16;
            for s in &interface.samplers {
                let mut src = source(&s.name);
                let float = wgpu::TextureSampleType::Float { filterable: true };
                let mut sample_type = match src {
                    Source::Color(n) => formats[n]
                        .sample_type(None, Some(features))
                        .unwrap_or(float),
                    Source::Depth => wgpu::TextureSampleType::Float { filterable: false },
                    _ => float,
                };
                let int = !matches!(sample_type, wgpu::TextureSampleType::Float { .. });
                if s.ty == "sampler2DShadow" {
                    (src, sample_type) = (Source::Shadow, wgpu::TextureSampleType::Depth);
                }
                let wanted = match s.ty.as_str() {
                    "sampler2DShadow" => true,
                    "sampler2D" => !int,
                    "isampler2D" => sample_type == wgpu::TextureSampleType::Sint,
                    "usampler2D" => sample_type == wgpu::TextureSampleType::Uint,
                    _ => false,
                };
                if !wanted {
                    notes.push(format!(
                        "{}: `{} {}` is not provided yet",
                        p.name, s.ty, s.name
                    ));
                    (src, sample_type) = (Source::Dummy, float);
                }
                let filtering = sample_type == float;
                if let Source::Color(n) = src {
                    reads |= 1 << n;
                }
                entries.push(texture_entry(s.binding, sample_type));
                entries.push(entry(
                    s.binding + 1,
                    wgpu::BindingType::Sampler(if src == Source::Shadow {
                        wgpu::SamplerBindingType::Comparison
                    } else if filtering {
                        wgpu::SamplerBindingType::Filtering
                    } else {
                        wgpu::SamplerBindingType::NonFiltering
                    }),
                ));
                samplers.push(Binding {
                    source: src,
                    binding: s.binding,
                    filtering,
                });
            }
            let final_pass = p.group == Group::Final;
            let mut targets = Vec::new();
            let mut writes = Vec::new();
            for &location in &interface.outputs {
                let l = location as usize;
                if targets.len() <= l {
                    targets.resize(l + 1, None);
                }
                if final_pass {
                    if location == 0 {
                        targets[0] = target(screen);
                    }
                } else if let Some(&buffer) = p.buffers.get(l) {
                    targets[l] = target(formats[buffer as usize]);
                    writes.push((location, buffer as usize));
                }
            }
            if !final_pass && writes.is_empty() {
                notes.push(format!("{}: writes no buffer; skipped", p.name));
                continue;
            }
            let written = writes.iter().fold(0u16, |m, w| m | 1 << w.1);
            let kept = p
                .flips
                .iter()
                .filter(|f| !f.1)
                .fold(0u16, |m, f| m | 1 << f.0);
            let flip = if final_pass {
                0
            } else {
                (written & !kept) | mask(&p.flips)
            };
            used |= reads | written | flip;
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(&p.name),
                entries: &entries,
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&p.name),
                bind_group_layouts: &[Some(&layout)],
                ..Default::default()
            });
            let created = checked(device, || {
                let module = |m: naga::Module| {
                    device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some(&p.name),
                        source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(m)),
                    })
                };
                let (vs, fs) = (module(p.program.vertex), module(p.program.fragment));
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(&p.name),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &vs,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &fs,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        targets: &targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            });
            let pipeline = match created {
                Ok(p) => p,
                Err(e) => {
                    notes.push(format!("{}: skipped: {e}", p.name));
                    continue;
                }
            };
            let interface = &p.program.interface;
            passes.push(Pass {
                ubo: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&p.name),
                    size: u64::from(interface.uniform_size.max(16)),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                block: vec![0; interface.uniform_size as usize],
                uniforms: interface
                    .uniforms
                    .iter()
                    .map(|u| (pack_uniforms::index(&u.name), u.ty.clone(), u.offset))
                    .collect(),
                name: p.name,
                group: p.group,
                pipeline,
                layout,
                samplers,
                writes,
                flip,
                reads,
                written,
                groups: HashMap::new(),
            });
        }
        // A clear only matters when a program reads the buffer before one writes all of it
        // (composite passes cover their whole target); the others are skipped.
        let mut cleared = Vec::new();
        for b in 1..BUFFERS {
            let bit = 1u16 << b;
            let first = passes
                .iter()
                .find(|p| (p.reads | p.flip | p.written) & bit != 0);
            if clears[b].is_some()
                && first.is_some_and(|p| p.reads & bit != 0 || p.written & bit == 0)
            {
                cleared.push(b);
            }
        }
        let blit_depth = passes
            .iter()
            .any(|p| p.samplers.iter().any(|s| s.source == Source::Depth));
        let room = 8 - 1 - usize::from(blit_depth);
        let other_clears = cleared.split_off(room.min(cleared.len()));
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let mut blit_targets = vec![target(formats[0])];
        if blit_depth {
            blit_targets.push(target(wgpu::TextureFormat::R32Float));
        }
        // Cleared by the pass's load operation; the shader writes nothing there.
        blit_targets.extend(cleared.iter().map(|&b| {
            Some(wgpu::ColorTargetState {
                format: formats[b],
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
            })
        }));
        let blit = helper(
            device,
            "vs_full",
            if blit_depth {
                "fs_blit"
            } else {
                "fs_blit_color"
            },
            &[
                texture_entry(0, float),
                texture_entry(1, wgpu::TextureSampleType::Depth),
            ],
            &blit_targets,
        );
        let copy = helper(
            device,
            "vs_uv",
            "fs_copy",
            &[
                texture_entry(0, float),
                entry(
                    2,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
            &[target(screen)],
        );
        let white = [255u8; 4];
        Self {
            notes,
            passes,
            deferred_pre: mask(&post.deferred_pre),
            composite_pre: mask(&post.composite_pre),
            formats,
            clears,
            used,
            blit_depth,
            blit_clears: cleared,
            other_clears,
            blit_group: None,
            main: 0,
            targets: None,
            noise: noise(device, queue),
            dummy: texture(device, queue, "Shader pack stand-in", 1, &white),
            shadow: (
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("Shader pack shadow stand-in"),
                        size: wgpu::Extent3d {
                            width: 1,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Depth32Float,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    })
                    .create_view(&Default::default()),
                device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("Shader pack shadow"),
                    compare: Some(wgpu::CompareFunction::LessEqual),
                    ..Default::default()
                }),
            ),
            shadow_cleared: false,
            linear: sampler(
                device,
                wgpu::FilterMode::Linear,
                wgpu::AddressMode::ClampToEdge,
            ),
            nearest: sampler(
                device,
                wgpu::FilterMode::Nearest,
                wgpu::AddressMode::ClampToEdge,
            ),
            repeat: sampler(device, wgpu::FilterMode::Linear, wgpu::AddressMode::Repeat),
            blit,
            copy,
            uniforms: FrameUniforms::default(),
        }
    }

    /// The names of the programs that run, in order.
    pub fn programs(&self) -> Vec<&str> {
        self.passes.iter().map(|p| p.name.as_str()).collect()
    }

    fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let make = |label: &str, format| {
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
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let color = std::array::from_fn(|i| {
            (self.used & 1 << i != 0).then(|| {
                let label = format!("colortex{i}");
                [make(&label, self.formats[i]), make(&label, self.formats[i])]
            })
        });
        self.targets = Some(Targets {
            size,
            color,
            depth: make("depthtex0", wgpu::TextureFormat::R32Float),
        });
        for pass in &mut self.passes {
            pass.groups.clear();
        }
    }

    /// Runs the pack over the world drawn into `world` (colour) and `world_depth`, then draws
    /// the result onto `screen`.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world: &wgpu::TextureView,
        world_depth: &wgpu::TextureView,
        screen: &wgpu::TextureView,
        frame: &super::Frame,
        inputs: &PackInputs,
        size: [u32; 2],
        window: [u32; 2],
    ) {
        self.resize(device, size);
        self.uniforms.update(frame, inputs, size);
        let Some(targets) = &self.targets else { return };
        let main = |m: u16, i: usize| usize::from(m & 1 << i != 0);
        let view = |m: u16, i: usize, alt: bool| {
            targets.color[i]
                .as_ref()
                .map(|t| &t[main(m, i) ^ usize::from(alt)])
        };

        if !self.shadow_cleared {
            self.shadow_cleared = true;
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack shadow stand-in"),
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow.0,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
        }
        let clear_op = |b: usize| {
            let c = self.clears[b].unwrap_or_default();
            wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: c[0].into(),
                    g: c[1].into(),
                    b: c[2].into(),
                    a: c[3].into(),
                }),
                store: wgpu::StoreOp::Store,
            }
        };
        // Overwritten whole: clearing lets a tiled GPU skip loading the old content.
        let overwrite = wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        };
        for chunk in self.other_clears.chunks(8) {
            let attachments: Vec<_> = chunk
                .iter()
                .map(|&b| attach(view(self.main, b, false).expect("used buffer"), clear_op(b)))
                .collect();
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack clear"),
                color_attachments: &attachments,
                ..Default::default()
            });
        }

        // The world into colortex0 and depthtex0, flipped to GL's rows, and the clears.
        if self.blit_group.as_ref().is_none_or(|g| g.0 != size) {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Shader pack world"),
                layout: &self.blit.1,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(world),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(world_depth),
                    },
                ],
            });
            self.blit_group = Some((size, group));
        }
        {
            let mut attachments = vec![attach(
                view(self.main, 0, false).expect("colortex0"),
                overwrite,
            )];
            if self.blit_depth {
                attachments.push(attach(&targets.depth, overwrite));
            }
            for &b in &self.blit_clears {
                attachments.push(attach(
                    view(self.main, b, false).expect("used buffer"),
                    clear_op(b),
                ));
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack world"),
                color_attachments: &attachments,
                ..Default::default()
            });
            pass.set_pipeline(&self.blit.0);
            pass.set_bind_group(0, self.blit_group.as_ref().map(|g| &g.1), &[]);
            pass.draw(0..3, 0..1);
        }

        let fog = self.uniforms.fog();
        let mut seen = (false, false);
        let mut shown = false;
        for pass in &mut self.passes {
            if pass.group == Group::Deferred && !seen.0 {
                seen.0 = true;
                self.main ^= self.deferred_pre;
            }
            if pass.group == Group::Composite && !seen.1 {
                seen.1 = true;
                self.main ^= self.composite_pre;
            }
            let final_pass = pass.group == Group::Final;
            let out = if final_pass { window } else { size };
            // Our block members first (`hz_target`, `hz_fog_color`, `hz_fog`), then the pack's.
            let ours = [
                [
                    out[0] as f32,
                    out[1] as f32,
                    1.0 / out[0] as f32,
                    1.0 / out[1] as f32,
                ],
                fog[0].to_array(),
                fog[1].to_array(),
            ];
            let head: &[u8] = bytemuck::cast_slice(&ours);
            let n = head.len().min(pass.block.len());
            pass.block[..n].copy_from_slice(&head[..n]);
            for (index, ty, offset) in &pass.uniforms {
                if let Some(v) = index.and_then(|i| self.uniforms.values.get(i)) {
                    v.write(ty, &mut pass.block, *offset as usize);
                }
            }
            if !pass.block.is_empty() {
                queue.write_buffer(&pass.ubo, 0, &pass.block);
            }
            let key = self.main & pass.reads;
            let group = pass.groups.entry(key).or_insert_with(|| {
                let mut entries = vec![wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pass.ubo.as_entire_binding(),
                }];
                for s in &pass.samplers {
                    let filter = if s.filtering {
                        &self.linear
                    } else {
                        &self.nearest
                    };
                    let (texture, sampler) = match s.source {
                        Source::Color(i) => (view(key, i, false).unwrap_or(&self.dummy), filter),
                        Source::Depth => (&targets.depth, &self.nearest),
                        Source::Noise => (&self.noise, &self.repeat),
                        Source::Dummy => (&self.dummy, &self.linear),
                        Source::Shadow => (&self.shadow.0, &self.shadow.1),
                    };
                    entries.push(wgpu::BindGroupEntry {
                        binding: s.binding,
                        resource: wgpu::BindingResource::TextureView(texture),
                    });
                    entries.push(wgpu::BindGroupEntry {
                        binding: s.binding + 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    });
                }
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&pass.name),
                    layout: &pass.layout,
                    entries: &entries,
                })
            });
            let mut attachments: Vec<Option<wgpu::RenderPassColorAttachment>> = Vec::new();
            if final_pass {
                attachments.push(attach(screen, overwrite));
                shown = true;
            } else {
                for &(location, buffer) in &pass.writes {
                    let l = location as usize;
                    if attachments.len() <= l {
                        attachments.resize(l + 1, None);
                    }
                    attachments[l] =
                        view(self.main, buffer, true).and_then(|v| attach(v, overwrite));
                }
            }
            {
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(&pass.name),
                    color_attachments: &attachments,
                    ..Default::default()
                });
                rp.set_pipeline(&pass.pipeline);
                rp.set_bind_group(0, &*group, &[]);
                rp.draw(0..6, 0..1);
            }
            self.main ^= pass.flip;
        }
        if !shown {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Shader pack colortex0"),
                layout: &self.copy.1,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            view(self.main, 0, false).expect("colortex0"),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.linear),
                    },
                ],
            });
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack colortex0"),
                color_attachments: &[attach(screen, overwrite)],
                ..Default::default()
            });
            rp.set_pipeline(&self.copy.0);
            rp.set_bind_group(0, &group, &[]);
            rp.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::TextureFormat as T;

    #[test]
    fn pack_formats_map_to_renderable_ones() {
        let f = |name| texture_format(Format::parse(name).unwrap(), wgpu::Features::empty());
        assert_eq!(f("RGBA"), T::Rgba8Unorm);
        assert_eq!(f("RGB8"), T::Rgba8Unorm, "no three-channel formats in wgpu");
        assert_eq!(f("RG16F"), T::Rg16Float);
        assert_eq!(f("R32F"), T::R32Float);
        assert_eq!(f("RGB10_A2"), T::Rgb10a2Unorm);
        assert_eq!(f("R32UI"), T::R32Uint);
        assert_eq!(f("RGBA16"), T::Rgba16Float, "16-bit norm needs a feature");
        assert_eq!(f("RGBA8_SNORM"), T::Rgba16Float, "snorm is not renderable");
        assert_eq!(f("R11F_G11F_B10F"), T::Rgba16Float);
        let all =
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM | wgpu::Features::RG11B10UFLOAT_RENDERABLE;
        let g = |name| texture_format(Format::parse(name).unwrap(), all);
        assert_eq!(g("RGBA16"), T::Rgba16Unorm);
        assert_eq!(g("R11F_G11F_B10F"), T::Rg11b10Ufloat);
    }

    #[test]
    fn sampler_names() {
        assert!(source("gaux1") == Source::Color(4));
        assert!(source("gdepthtex") == Source::Depth);
        assert!(source("noisetex") == Source::Noise);
        assert!(source("shadowcolor0") == Source::Dummy);
    }
}
