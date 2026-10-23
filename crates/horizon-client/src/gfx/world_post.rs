//! Resolution scale, FXAA, and shader-pack G-buffer/shadow targets (Horizon
//! settings): the world renders into its own target at the scaled size, then
//! one full-screen pass copies or anti-aliases it onto the screen, under the
//! hand and HUD. For a shader pack that reads them, `Post` also keeps:
//! - `opaque_depth`: the depth before translucent terrain (water, stained
//!   glass, ice) draws, a Java pack's `depthtex1`/`depthtex2`;
//! - `normal` and `material`, an api 2 pack's `scene_normal` and
//!   `scene_material`: surface normals and material properties
//!   reconstructed from depth and scene colour;
//! - `shadow_depth`, an api 2 pack's: the sun/moon shadow map and its
//!   light-space `shadow_view` and `shadow_proj` matrices.
//!
//! With none of them on, the world renders straight to the screen as before.

use super::pack_stages::FrameUniform;
use glam::{Mat4, Vec3};

const GBUFFER_WGSL: &str = r#"
struct GbufferParams {
    inverse_view: mat4x4<f32>,
    inverse_projection: mat4x4<f32>,
    resolution: vec4<f32>,
    // x: eye block light (0..1), y: eye sky light (0..1), z: wetness (0..1), w: sun elevation
    env: vec4<f32>,
}

@group(0) @binding(0) var world_color: texture_2d<f32>;
@group(0) @binding(1) var world_depth: texture_depth_2d;
@group(0) @binding(2) var opaque_depth: texture_depth_2d;
@group(0) @binding(3) var nearest_sampler: sampler;
@group(0) @binding(4) var<uniform> params: GbufferParams;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_gbuffer(@builtin(vertex_index) i: u32) -> VsOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.clip = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

struct GbufferOut {
    @location(0) normal: vec4<f32>,
    @location(1) material: vec4<f32>,
}

fn world_pos(uv: vec2<f32>, d: f32) -> vec3<f32> {
    let v = params.inverse_projection * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let view = v.xyz / max(v.w, 1e-6);
    return (params.inverse_view * vec4<f32>(view, 1.0)).xyz;
}

@fragment
fn fs_gbuffer(in: VsOut) -> GbufferOut {
    let d_all = textureSampleLevel(world_depth, nearest_sampler, in.uv, 0);
    let d_opq = textureSampleLevel(opaque_depth, nearest_sampler, in.uv, 0);
    let color = textureSampleLevel(world_color, nearest_sampler, in.uv, 0.0);
    var out: GbufferOut;
    if d_all <= 0.0 && d_opq <= 0.0 {
        out.normal = vec4<f32>(0.5, 1.0, 0.5, 0.0);
        out.material = vec4<f32>(0.0, params.env.y, 0.0, 1.0);
        return out;
    }
    let d = select(d_all, d_opq, d_opq > 0.0);
    let step_uv = params.resolution.zw;
    let p0 = world_pos(in.uv, d);
    let uv_r = clamp(in.uv + vec2<f32>(step_uv.x, 0.0), vec2<f32>(0.0), vec2<f32>(1.0));
    let uv_l = clamp(in.uv - vec2<f32>(step_uv.x, 0.0), vec2<f32>(0.0), vec2<f32>(1.0));
    let uv_d = clamp(in.uv + vec2<f32>(0.0, step_uv.y), vec2<f32>(0.0), vec2<f32>(1.0));
    let uv_u = clamp(in.uv - vec2<f32>(0.0, step_uv.y), vec2<f32>(0.0), vec2<f32>(1.0));
    let dr = textureSampleLevel(opaque_depth, nearest_sampler, uv_r, 0);
    let dl = textureSampleLevel(opaque_depth, nearest_sampler, uv_l, 0);
    let dd = textureSampleLevel(opaque_depth, nearest_sampler, uv_d, 0);
    let du = textureSampleLevel(opaque_depth, nearest_sampler, uv_u, 0);
    let dx = select(world_pos(uv_r, dr) - p0, p0 - world_pos(uv_l, dl), abs(dl - d) < abs(dr - d));
    let dy = select(world_pos(uv_d, dd) - p0, p0 - world_pos(uv_u, du), abs(du - d) < abs(dd - d));
    let raw_n = cross(dy, dx);
    let n_len = length(raw_n);
    let n = select(vec3<f32>(0.0, 1.0, 0.0), raw_n / max(n_len, 1e-6), n_len > 1e-6);
    // Reversed depth: closer translucent geometry has larger depth than opaque geometry.
    let is_translucent = select(0.0, 1.0, d_all > d_opq + 1e-5);
    let luma = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let block_light = clamp(max(params.env.x, smoothstep(0.65, 0.98, luma)), 0.0, 1.0);
    let roughness = select(clamp(0.85 - params.env.z * 0.45, 0.1, 1.0), 0.05, is_translucent > 0.5);
    out.normal = vec4<f32>(n * 0.5 + 0.5, 1.0);
    out.material = vec4<f32>(block_light, params.env.y, is_translucent, roughness);
    return out;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GbufferUniform {
    inverse_view: [[f32; 4]; 4],
    inverse_projection: [[f32; 4]; 4],
    resolution: [f32; 4],
    env: [f32; 4],
}

/// What the frame's shader pack reads beyond the world's colour and depth:
/// `Post` keeps these targets only while one does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    pub opaque_depth: bool,
    pub gbuffer: bool,
}

struct WorldTarget {
    size: [u32; 2],
    extras: Extras,
    color: wgpu::TextureView,
    depth_tex: wgpu::Texture,
    depth: wgpu::TextureView,
    /// With `extras.opaque_depth` or `extras.gbuffer` (which reads it).
    opaque_depth: Option<(wgpu::Texture, wgpu::TextureView)>,
    gbuffer: Option<Gbuffer>,
    group: wgpu::BindGroup,
    opaque_captured: bool,
}

struct Gbuffer {
    normal: wgpu::TextureView,
    material: wgpu::TextureView,
    group: wgpu::BindGroup,
}

struct ShadowTarget {
    resolution: u32,
    depth: wgpu::TextureView,
}

pub struct Post {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    nearest: wgpu::Sampler,
    copy: wgpu::RenderPipeline,
    fxaa: wgpu::RenderPipeline,
    gbuffer_layout: wgpu::BindGroupLayout,
    gbuffer_pipeline: wgpu::RenderPipeline,
    gbuffer_uniform: wgpu::Buffer,
    format: wgpu::TextureFormat,
    /// World resolution relative to the window (0.5..=2.0).
    pub scale: f32,
    pub antialias: bool,
    target: Option<WorldTarget>,
    shadow: Option<ShadowTarget>,
    /// Bumped whenever a target is made: bindings of the old ones are stale.
    generation: u64,
    fallback_shadow: wgpu::TextureView,
    fallback_color: wgpu::TextureView,
    pub shadow_view: Mat4,
    pub shadow_proj: Mat4,
    pub shadow_distance: f32,
    pub shadow_resolution: u32,
    pub shadow_active: bool,
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon post"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Horizon post"),
                layout: Some(&pipeline_layout),
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Horizon post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let nearest = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Horizon post nearest"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let gbuffer_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon gbuffer"),
            source: wgpu::ShaderSource::Wgsl(GBUFFER_WGSL.into()),
        });
        let gbuffer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon gbuffer layout"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
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
        let gbuffer_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon gbuffer"),
            bind_group_layouts: &[Some(&gbuffer_layout)],
            ..Default::default()
        });
        let gbuffer_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Horizon gbuffer"),
            layout: Some(&gbuffer_pipe_layout),
            vertex: wgpu::VertexState {
                module: &gbuffer_shader,
                entry_point: Some("vs_gbuffer"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &gbuffer_shader,
                entry_point: Some("fs_gbuffer"),
                compilation_options: Default::default(),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
            }),
            multiview_mask: None,
            cache: None,
        });
        let gbuffer_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon gbuffer uniform"),
            size: std::mem::size_of::<GbufferUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let fallback_shadow = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Horizon fallback shadow"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let fallback_color = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Horizon fallback gbuffer"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Self {
            copy: pipeline("fs_copy"),
            fxaa: pipeline("fs_fxaa"),
            layout,
            sampler,
            nearest,
            gbuffer_layout,
            gbuffer_pipeline,
            gbuffer_uniform,
            format,
            scale: 1.0,
            antialias: false,
            target: None,
            shadow: None,
            generation: 0,
            fallback_shadow,
            fallback_color,
            shadow_view: Mat4::IDENTITY,
            shadow_proj: Mat4::IDENTITY,
            shadow_distance: 128.0,
            shadow_resolution: 2048,
            shadow_active: false,
        }
    }

    /// The world's colour and depth targets this frame, or `None` to draw
    /// it straight to the screen. `pack`: a shader pack runs this frame,
    /// reading `extras`.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        window: [u32; 2],
        pack: bool,
        extras: Extras,
    ) -> Option<(wgpu::TextureView, wgpu::TextureView)> {
        if self.scale == 1.0 && !self.antialias && !pack {
            self.target = None;
            self.shadow_active = false;
            return None;
        }
        let size = window.map(|v| ((v as f32 * self.scale).round() as u32).clamp(1, 8192));
        if self
            .target
            .as_ref()
            .is_none_or(|t| t.size != size || t.extras != extras)
        {
            self.generation += 1;
            let extent = wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            };
            let make_tex = |label, format, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let color = make_tex(
                "Scaled world",
                self.format,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            )
            .create_view(&Default::default());
            let depth_tex = make_tex(
                "Scaled world depth",
                wgpu::TextureFormat::Depth32Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
            );
            let depth = depth_tex.create_view(&Default::default());
            let opaque_depth = (extras.opaque_depth || extras.gbuffer).then(|| {
                let tex = make_tex(
                    "Opaque world depth",
                    wgpu::TextureFormat::Depth32Float,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                );
                let view = tex.create_view(&Default::default());
                (tex, view)
            });
            let gbuffer = opaque_depth
                .as_ref()
                .filter(|_| extras.gbuffer)
                .map(|(_, opaque)| {
                    let color_target = |label| {
                        make_tex(
                            label,
                            wgpu::TextureFormat::Rgba8Unorm,
                            wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::TEXTURE_BINDING,
                        )
                        .create_view(&Default::default())
                    };
                    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Horizon gbuffer"),
                        layout: &self.gbuffer_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&color),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&depth),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(opaque),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::Sampler(&self.nearest),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: self.gbuffer_uniform.as_entire_binding(),
                            },
                        ],
                    });
                    Gbuffer {
                        normal: color_target("World G-buffer normal"),
                        material: color_target("World G-buffer material"),
                        group,
                    }
                });
            let group = self.bind(device, &color);
            self.target = Some(WorldTarget {
                size,
                extras,
                color,
                depth_tex,
                depth,
                opaque_depth,
                gbuffer,
                group,
                opaque_captured: false,
            });
        }
        if let Some(t) = &mut self.target {
            t.opaque_captured = false;
        }
        self.target
            .as_ref()
            .map(|t| (t.color.clone(), t.depth.clone()))
    }

    /// Prepare the directional sun/moon shadow map target and compute its
    /// camera-relative orthographic `shadow_view` and `shadow_proj` matrices.
    pub fn prepare_shadow(
        &mut self,
        device: &wgpu::Device,
        resolution: u32,
        distance: f32,
        sun_dir: Vec3,
        moon_dir: Vec3,
    ) -> &wgpu::TextureView {
        let resolution = resolution.clamp(256, 4096);
        let distance = distance.clamp(16.0, 512.0);
        if self
            .shadow
            .as_ref()
            .is_none_or(|s| s.resolution != resolution)
        {
            let depth = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Horizon shadow map"),
                    size: wgpu::Extent3d {
                        width: resolution,
                        height: resolution,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Depth32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            self.shadow = Some(ShadowTarget { resolution, depth });
            self.generation += 1;
        }
        let light_dir = if sun_dir.y >= -0.05 {
            sun_dir.normalize_or_zero()
        } else {
            moon_dir.normalize_or_zero()
        };
        let dir = if light_dir.length_squared() > 1e-4 {
            light_dir
        } else {
            Vec3::Y
        };
        let up = if dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        self.shadow_view = Mat4::look_at_rh(dir * distance, Vec3::ZERO, up);
        // Reversed-depth orthographic projection for the world's GreaterEqual
        // pipelines (near maps to 1.0, far maps to 0.0), and standard 0..1
        // projection for shader sampling.
        self.shadow_proj = Mat4::orthographic_rh(
            -distance,
            distance,
            -distance,
            distance,
            distance * 2.5,
            0.05,
        );
        self.shadow_distance = distance;
        self.shadow_resolution = resolution;
        self.shadow_active = true;
        &self.shadow.as_ref().expect("created above").depth
    }

    /// Snapshot `world_depth` into `opaque_depth` (`depthtex1`/`depthtex2`)
    /// right before translucent water and weather draw.
    pub fn capture_opaque_depth(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(t) = &mut self.target else { return };
        let Some((opaque, _)) = &t.opaque_depth else {
            return;
        };
        encoder.copy_texture_to_texture(
            t.depth_tex.as_image_copy(),
            opaque.as_image_copy(),
            t.depth_tex.size(),
        );
        t.opaque_captured = true;
    }

    /// Reconstruct the G-buffer `normal` and `material` textures after the
    /// world pass finishes (with `Extras::gbuffer`).
    pub fn build_gbuffer(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameUniform,
        eye_light: [u8; 2],
        wetness: f32,
    ) {
        if !self.target.as_ref().is_some_and(|t| t.opaque_captured) {
            self.capture_opaque_depth(encoder);
        }
        let Some(t) = &self.target else { return };
        let Some(gbuffer) = &t.gbuffer else { return };
        let [w, h] = t.size.map(|v| v.max(1) as f32);
        let uniform = GbufferUniform {
            inverse_view: frame.inverse_view,
            inverse_projection: frame.inverse_projection,
            resolution: [w, h, 1.0 / w, 1.0 / h],
            env: [
                f32::from(eye_light[0]) / 15.0,
                f32::from(eye_light[1]) / 15.0,
                wetness,
                frame.sun[1],
            ],
        };
        queue.write_buffer(&self.gbuffer_uniform, 0, bytemuck::bytes_of(&uniform));
        let attachments = [
            Some(wgpu::RenderPassColorAttachment {
                view: &gbuffer.normal,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            }),
            Some(wgpu::RenderPassColorAttachment {
                view: &gbuffer.material,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            }),
        ];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon gbuffer"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(&self.gbuffer_pipeline);
        pass.set_bind_group(0, &gbuffer.group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The world target's size, once `prepare` made one.
    pub fn size(&self) -> Option<[u32; 2]> {
        self.target.as_ref().map(|t| t.size)
    }

    /// Changes whenever `prepare` or `prepare_shadow` makes new targets:
    /// bindings made from the views below must be made again.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The depth before translucent geometry (`capture_opaque_depth`), or
    /// the world's depth when nothing reads it.
    pub fn opaque_depth(&self) -> &wgpu::TextureView {
        match &self.target {
            Some(WorldTarget {
                opaque_depth: Some((_, view)),
                ..
            }) => view,
            Some(t) => &t.depth,
            None => &self.fallback_shadow,
        }
    }

    /// Reconstructed world-space normals (`scene_normal`).
    pub fn normal(&self) -> &wgpu::TextureView {
        self.gbuffer().map_or(&self.fallback_color, |g| &g.normal)
    }

    /// Reconstructed material and light (`scene_material`).
    pub fn material(&self) -> &wgpu::TextureView {
        self.gbuffer().map_or(&self.fallback_color, |g| &g.material)
    }

    fn gbuffer(&self) -> Option<&Gbuffer> {
        self.target.as_ref()?.gbuffer.as_ref()
    }

    /// The sun/moon shadow map (`shadow_depth` of api 2 packs).
    pub fn shadow_depth(&self) -> &wgpu::TextureView {
        self.shadow
            .as_ref()
            .map_or(&self.fallback_shadow, |s| &s.depth)
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

    /// Draw the scaled world onto `screen`.
    pub fn finish(&self, encoder: &mut wgpu::CommandEncoder, screen: &wgpu::TextureView) {
        let Some(target) = &self.target else { return };
        self.finish_with(encoder, screen, &target.group);
    }

    /// Draw what `group` binds (`bind`) onto `screen`.
    pub fn finish_with(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        screen: &wgpu::TextureView,
        group: &wgpu::BindGroup,
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
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(if self.antialias {
            &self.fxaa
        } else {
            &self.copy
        });
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}
