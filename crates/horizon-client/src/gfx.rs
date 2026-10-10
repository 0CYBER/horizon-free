//! GPU renderer for streamed terrain sections, sky, block outline and HUD.
//! Section origins are supplied relative to the camera each frame so large
//! world coordinates keep full precision.
use crate::{
    Result,
    environment::Sky,
    hud::{HudBatch, HudRenderer},
    mesher::{SectionMesh, Vertex},
    sprites::SpriteAnimation,
};
use glam::{DVec3, Mat4, Vec3, Vec4};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Instant};
use wgpu::util::DeviceExt;
use winit::window::Window;

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
}
pub type SectionKey = (i32, i32, i32);
struct SectionGpu {
    buffer: wgpu::Buffer,
    solid: u32,
    translucent: u32,
}
/// Line segments the hit outline can hold.
const MAX_OUTLINE_EDGES: u64 = 256;
pub struct Frame<'a> {
    pub camera: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_degrees: f32,
    /// GameRenderer.bobView: (walk distance, bob amount), interpolated.
    pub bob: Option<(f32, f32)>,
    pub sky: Sky,
    /// Render distance in blocks (fog end).
    pub distance: f32,
    pub underwater: bool,
    /// Skip sections the camera cannot see through open faces (vanilla's
    /// occlusion graph; off in spectator-like views inside blocks).
    pub cave_culling: bool,
    /// Title screen: only the panorama (in `celestial`) and the HUD.
    pub title: bool,
    /// Dimensions without a skybox (the Nether) clear to the fog colour.
    pub skybox: bool,
    /// The End: clear to black and draw `celestial` (the End sky box) behind the world.
    pub end_sky: bool,
    /// Environmental fog start/end (Nether 10..96), if any.
    pub environment_fog: Option<(f32, f32)>,
    pub water_fog: [f32; 3],
    /// `FogRenderer.setupFog` result; replaces the underwater/environment fields when set.
    pub fog: Option<crate::fog::FogData>,
    /// Camera eye below the horizon height and not under water (`shouldRenderDarkDisc`).
    pub dark_disc: bool,
    /// The hit block outline as line segments (from, to) in world space.
    pub outline: Vec<[f64; 6]>,
    pub hud: &'a HudBatch,
    /// Camera-relative entity geometry textured from the shared atlas.
    pub entities: &'a [Vertex],
    /// Additive translucent geometry after the entities (lightning, dragon rays).
    pub additive: &'a [Vertex],
    /// Camera-relative sun, moon and stars (drawn behind everything, additive).
    pub celestial: &'a [Vertex],
    /// First-person hand and held item, drawn after clearing depth.
    pub hand: &'a [Vertex],
    /// Full-screen camera effects drawn over the world and hand, under the HUD.
    pub screen: &'a [crate::screen_effects::ScreenQuad],
    pub game_time: f32,
    pub ticks: u64,
    /// The darkness effect on the lightmap: the brightness it takes off the
    /// gamma option and the pulsing scale subtracted from the light.
    pub darkness: (f32, f32),
    /// Night vision (or conduit water vision) intensity on the lightmap.
    pub night_vision: f32,
    /// Torch flicker added to the block light factor of 1.4.
    pub block_flicker: f32,
}
pub struct Renderer {
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    window: Arc<Window>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    solid: wgpu::RenderPipeline,
    translucent: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    dark_disc: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    entity_pipeline: wgpu::RenderPipeline,
    additive_pipeline: wgpu::RenderPipeline,
    celestial_pipeline: wgpu::RenderPipeline,
    celestial_buffer: wgpu::Buffer,
    /// Screen effects: alpha-blended and vignette (`dst * (1 - src)`) pipelines.
    screen_pipelines: [wgpu::RenderPipeline; 2],
    screen_buffer: wgpu::Buffer,
    screen_capacity: u64,
    entity_group: wgpu::BindGroup,
    entity_buffer: wgpu::Buffer,
    entity_capacity: u64,
    group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    lightmap: wgpu::Texture,
    sprite_texture: wgpu::Texture,
    mip_levels: u32,
    animations: Vec<(SpriteAnimation, usize)>,
    indices: wgpu::Buffer,
    index_quads: u32,
    instances: wgpu::Buffer,
    instance_capacity: u64,
    outline: wgpu::Buffer,
    depth: wgpu::TextureView,
    sections: HashMap<SectionKey, SectionGpu>,
    /// Block-entity blocks per meshed section (world position, state id).
    block_entities: HashMap<SectionKey, Vec<([i32; 3], u32)>>,
    /// VisGraph face connectivity per meshed section (cave culling).
    visibility: HashMap<SectionKey, u64>,
    reachable: std::collections::HashSet<SectionKey>,
    reachable_from: Option<SectionKey>,
    visibility_dirty: bool,
    pub hud: HudRenderer,
    pub info: wgpu::AdapterInfo,
    pub suspended: bool,
    pub drawn_sections: usize,
    pub uploaded_bytes: u64,
    started: Instant,
    /// Write the next presented frame to this PNG path (control socket).
    pub screenshot: Option<PathBuf>,
    /// Outcome of the last screenshot: frame size, or the error.
    pub screenshot_result: Option<std::result::Result<[u32; 2], String>>,
    /// Brightness option (lightmap gamma), 0..1.
    pub gamma: f32,
    present_modes: Vec<wgpu::PresentMode>,
}
fn mip_chain(pixels: &[u8], size: u32, layers: u32) -> (Vec<u8>, u32) {
    let levels = size.trailing_zeros() + 1;
    let mut out = Vec::new();
    for layer in 0..layers as usize {
        let base =
            &pixels[layer * (size * size * 4) as usize..(layer + 1) * (size * size * 4) as usize];
        let mut level = base.to_vec();
        let mut s = size as usize;
        out.extend_from_slice(&level);
        for _ in 1..levels {
            let n = s / 2;
            let mut next = vec![0u8; n * n * 4];
            for y in 0..n {
                for x in 0..n {
                    let mut acc = [0u32; 4];
                    let mut weight = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let p = &level[((y * 2 + dy) * s + x * 2 + dx) * 4..][..4];
                        let a = u32::from(p[3]);
                        for c in 0..3 {
                            acc[c] += u32::from(p[c]) * a;
                        }
                        acc[3] += a;
                        weight += a;
                    }
                    let o = &mut next[(y * n + x) * 4..][..4];
                    for c in 0..3 {
                        o[c] = acc[c].checked_div(weight).unwrap_or(0) as u8;
                    }
                    o[3] = (acc[3] / 4) as u8;
                }
            }
            out.extend_from_slice(&next);
            level = next;
            s = n;
        }
    }
    (out, levels)
}
impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        sprite_pixels: &[u8],
        sprite_size: u32,
        layers: u32,
        animations: &[SpriteAnimation],
        hud_atlas: (&[u8], u32, u32),
    ) -> Result<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await?;
        let info = adapter.get_info();
        eprintln!(
            "Horizon renderer: {} ({:?}, {:?})",
            info.name, info.backend, info.device_type
        );
        let limits = adapter.limits();
        if limits.max_texture_array_layers < layers {
            return Err(format!(
                "GPU supports {} texture layers; {layers} required",
                limits.max_texture_array_layers
            )
            .into());
        }
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Horizon device"),
                required_limits: limits.clone(),
                ..Default::default()
            })
            .await?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("surface has no supported configuration")?;
        let caps = surface.get_capabilities(&adapter);
        // Vanilla blends and tints in gamma space: use a non-sRGB target.
        config.format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(config.format);
        config.view_formats = vec![config.format.remove_srgb_suffix()];
        // Screenshots copy the presented frame back from the swapchain.
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        let present_modes = caps.present_modes.clone();
        config.present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        surface.configure(&device, &config);
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
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon terrain layout"),
            bind_group_layouts: &[Some(&bindings)],
            ..Default::default()
        });
        let vertex_attributes =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4];
        let instance_attributes = wgpu::vertex_attr_array![5=>Float32x4];
        let terrain_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &vertex_attributes,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 16,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attributes,
            }),
        ];
        let line_attributes = wgpu::vertex_attr_array![0=>Float32x3];
        let line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &line_attributes,
        })];
        let alpha_blend = wgpu::BlendState::ALPHA_BLENDING;
        let pipeline = |label: &str,
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
                layout: Some(&layout),
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
                        wgpu::CompareFunction::LessEqual
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
                    compilation_options: Default::default(),
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
            false,
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
            wgpu::PrimitiveTopology::LineList,
            None,
            false,
            true,
            Some(alpha_blend),
        );
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (mips, mip_levels) = mip_chain(sprite_pixels, sprite_size, layers);
        let sprite_texture = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("Block sprites"),
                size: wgpu::Extent3d {
                    width: sprite_size,
                    height: sprite_size,
                    depth_or_array_layers: layers,
                },
                mip_level_count: mip_levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &mips,
        );
        let sprite_view = sprite_texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
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
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon terrain resources"),
            layout: &bindings,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&sprite_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sprite_sampler),
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
        let index_quads = 1 << 16;
        let indices = Self::quad_indices(&device, index_quads);
        let instance_capacity = 4096;
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Section origins"),
            size: instance_capacity * 16,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let outline = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Outline"),
            size: 24 * MAX_OUTLINE_EDGES,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let depth = Self::depth(&device, &config);
        let (atlas_pixels, atlas_width, atlas_height) = hud_atlas;
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
            source: wgpu::ShaderSource::Wgsl(include_str!("entity.wgsl").into()),
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
        let entity_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Entity layout"),
            bind_group_layouts: &[Some(&entity_bindings)],
            ..Default::default()
        });
        let entity_vertex = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &vertex_attributes,
        })];
        let entity_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Entities"),
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
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &entity_shader,
                entry_point: Some("fs_entity"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format.remove_srgb_suffix(),
                    blend: Some(alpha_blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // BlendFunction.LIGHTNING: src alpha, one; no depth writes.
        let additive_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
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
        let screen_pipelines =
            [("fs_screen", alpha_blend), ("fs_vignette", vignette_blend)].map(|(entry, blend)| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Screen effects"),
                    layout: Some(&entity_layout),
                    vertex: wgpu::VertexState {
                        module: &screen_shader,
                        entry_point: Some("vs_screen"),
                        compilation_options: Default::default(),
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
                        compilation_options: Default::default(),
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
        Ok(Self {
            instance,
            surface,
            window,
            device,
            queue,
            config,
            solid,
            translucent,
            sky,
            dark_disc,
            lines,
            entity_pipeline,
            additive_pipeline,
            celestial_pipeline,
            celestial_buffer,
            screen_pipelines,
            screen_buffer,
            screen_capacity,
            entity_group,
            entity_buffer,
            entity_capacity,
            group,
            globals,
            lightmap,
            sprite_texture,
            mip_levels,
            animations: animations.iter().map(|a| (a.clone(), usize::MAX)).collect(),
            indices,
            index_quads,
            instances,
            instance_capacity,
            outline,
            depth,
            sections: HashMap::new(),
            block_entities: HashMap::new(),
            visibility: HashMap::new(),
            reachable: std::collections::HashSet::new(),
            reachable_from: None,
            visibility_dirty: true,
            hud,
            info,
            suspended: size.width == 0 || size.height == 0,
            drawn_sections: 0,
            uploaded_bytes: 0,
            started: Instant::now(),
            screenshot: None,
            screenshot_result: None,
            gamma: 0.5,
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
    fn depth(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::TextureView {
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
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }
    /// VSync on: tear-free presentation (Mailbox, else Fifo); off: Immediate when available.
    pub fn set_vsync(&mut self, vsync: bool) {
        let has = |m| self.present_modes.contains(&m);
        let mode = match (
            vsync,
            has(wgpu::PresentMode::Mailbox),
            has(wgpu::PresentMode::Immediate),
        ) {
            (true, true, _) => wgpu::PresentMode::Mailbox,
            (true, false, _) => wgpu::PresentMode::Fifo,
            (false, _, true) => wgpu::PresentMode::Immediate,
            (false, true, false) => wgpu::PresentMode::Mailbox,
            _ => wgpu::PresentMode::Fifo,
        };
        if self.config.present_mode != mode {
            self.config.present_mode = mode;
            self.surface.configure(&self.device, &self.config);
        }
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        self.suspended = width == 0 || height == 0;
        if self.suspended {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth = Self::depth(&self.device, &self.config);
    }
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }
    pub fn upload(&mut self, key: SectionKey, mesh: &SectionMesh) {
        if self.visibility.insert(key, mesh.visibility) != Some(mesh.visibility) {
            self.visibility_dirty = true;
        }
        if mesh.block_entities.is_empty() {
            self.block_entities.remove(&key);
        } else {
            self.block_entities.insert(key, mesh.block_entities.clone());
        }
        let solid = (mesh.solid.len() / 4) as u32;
        let translucent = (mesh.translucent.len() / 4) as u32;
        if solid + translucent == 0 {
            self.sections.remove(&key);
            return;
        }
        let max = solid.max(translucent);
        if max > self.index_quads {
            self.index_quads = max.next_power_of_two();
            self.indices = Self::quad_indices(&self.device, self.index_quads);
        }
        let mut data = Vec::with_capacity(mesh.solid.len() + mesh.translucent.len());
        data.extend_from_slice(&mesh.solid);
        data.extend_from_slice(&mesh.translucent);
        let bytes: &[u8] = bytemuck::cast_slice(&data);
        self.uploaded_bytes += bytes.len() as u64;
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Section mesh"),
                contents: bytes,
                usage: wgpu::BufferUsages::VERTEX,
            });
        self.sections.insert(
            key,
            SectionGpu {
                buffer,
                solid,
                translucent,
            },
        );
    }
    /// Block-entity blocks of every meshed section (see `SectionMesh::block_entities`).
    pub fn block_entities(&self) -> impl Iterator<Item = &([i32; 3], u32)> {
        self.block_entities.values().flatten()
    }
    pub fn clear_sections(&mut self) {
        self.block_entities.clear();
        self.sections.clear();
        self.visibility.clear();
        self.visibility_dirty = true;
    }
    /// SectionOcclusionGraph: breadth-first from the camera section through
    /// faces its VisGraph connects, never stepping back towards the camera.
    fn update_reachable(&mut self, from: SectionKey, radius: i32) {
        const STEPS: [(i32, i32, i32); 6] = [
            (0, -1, 0),
            (0, 1, 0),
            (0, 0, -1),
            (0, 0, 1),
            (-1, 0, 0),
            (1, 0, 0),
        ];
        let opposite = |d: usize| d ^ 1;
        self.reachable.clear();
        let mut queue = std::collections::VecDeque::new();
        // (section, entered face, directions travelled)
        queue.push_back((from, None::<usize>, 0u8));
        self.reachable.insert(from);
        let sections = crate::chunk::HEIGHT / 16;
        while let Some((key, entered, travelled)) = queue.pop_front() {
            let vis = self
                .visibility
                .get(&key)
                .copied()
                .unwrap_or(crate::mesher::ALL_VISIBLE);
            for (d, step) in STEPS.iter().enumerate() {
                if travelled & (1 << opposite(d)) != 0 {
                    continue;
                }
                if let Some(e) = entered
                    && vis & (1 << (e * 6 + d)) == 0
                {
                    continue;
                }
                let next = (key.0 + step.0, key.1 + step.1, key.2 + step.2);
                if next.1 < -1
                    || next.1 > sections
                    || (next.0 - from.0).abs() > radius
                    || (next.2 - from.2).abs() > radius
                {
                    continue;
                }
                if self.reachable.insert(next) {
                    queue.push_back((next, Some(opposite(d)), travelled | (1 << d)));
                }
            }
        }
        self.reachable_from = Some(from);
        self.visibility_dirty = false;
    }
    pub fn remove(&mut self, key: SectionKey) {
        self.block_entities.remove(&key);
        self.visibility.remove(&key);
        self.visibility_dirty = true;
        self.sections.remove(&key);
    }
    pub fn view_projection(&self, frame: &Frame) -> Mat4 {
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let (yaw, pitch) = (frame.yaw.to_radians(), frame.pitch.to_radians());
        // Minecraft yaw: 0 = +Z (south), 90 = -X (west); pitch positive looks down.
        let forward = Vec3::new(
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        let far = (frame.distance * 4.0).max(1024.0);
        let bob = frame.bob.map_or(Mat4::IDENTITY, |(walk, bob)| {
            let f = -walk * std::f32::consts::PI;
            Mat4::from_translation(Vec3::new(f.sin() * bob * 0.5, -(f.cos() * bob).abs(), 0.0))
                * Mat4::from_rotation_z((f.sin() * bob * 3.0).to_radians())
                * Mat4::from_rotation_x(
                    ((f - 0.2).cos() * bob).abs().mul_add(5.0, 0.0).to_radians(),
                )
        });
        Mat4::perspective_rh(frame.fov_degrees.to_radians(), aspect, 0.05, far)
            * bob
            * Mat4::look_to_rh(Vec3::ZERO, forward, Vec3::Y)
    }
    fn update_lightmap(
        &self,
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
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.lightmap,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(64),
                rows_per_image: Some(16),
            },
            wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
        );
    }
    fn animate(&mut self, tick: u64) {
        for (animation, previous) in &mut self.animations {
            let frame = animation.frame(tick);
            if frame == *previous {
                continue;
            }
            *previous = frame;
            let (mips, levels) = mip_chain(animation.pixels(frame), 16, 1);
            let mut offset = 0;
            for level in 0..levels.min(self.mip_levels) {
                let size = 16 >> level;
                let bytes = (size * size * 4) as usize;
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.sprite_texture,
                        mip_level: level,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: animation.layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &mips[offset..offset + bytes],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(size * 4),
                        rows_per_image: Some(size),
                    },
                    wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 1,
                    },
                );
                offset += bytes;
            }
        }
    }
    pub fn render(&mut self, frame: &Frame) -> Result<bool> {
        if self.suspended {
            return Ok(false);
        }
        let (surface_frame, reconfigure) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => (f, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => (f, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface = self.instance.create_surface(self.window.clone())?;
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("surface acquisition failed validation".into());
            }
        };
        self.animate(frame.ticks);
        self.update_lightmap(
            &frame.sky,
            frame.darkness.1,
            (self.gamma - frame.darkness.0).max(0.0),
            frame.night_vision,
            frame.block_flicker,
        );
        let view_proj = self.view_projection(frame);
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let (yaw, pitch) = (frame.yaw.to_radians(), frame.pitch.to_radians());
        let forward = Vec3::new(
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        let view = Mat4::look_to_rh(Vec3::ZERO, forward, Vec3::Y);
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
        };
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        // Frustum culling and camera-relative origins.
        let planes = frustum(view_proj);
        let camera = frame.camera;
        let camera_section = (
            (camera.x.floor() as i32) >> 4,
            (camera.y.floor() as i32 - crate::chunk::MIN_Y) >> 4,
            (camera.z.floor() as i32) >> 4,
        );
        let culling = !frame.title && frame.cave_culling;
        if culling && (self.visibility_dirty || self.reachable_from != Some(camera_section)) {
            let radius = (frame.distance / 16.0) as i32 + 2;
            self.update_reachable(camera_section, radius);
        }
        let reachable = &self.reachable;
        let mut visible: Vec<(SectionKey, f64, [f32; 3])> = self
            .sections
            .keys()
            .filter(|k| !culling || reachable.contains(k))
            .filter_map(|&(sx, sy, sz)| {
                let origin = DVec3::new(
                    f64::from(sx * 16),
                    f64::from(crate::chunk::MIN_Y + sy * 16),
                    f64::from(sz * 16),
                ) - camera;
                let center = origin + DVec3::splat(8.0);
                let horizontal = (center.x * center.x + center.z * center.z).sqrt();
                if horizontal > f64::from(frame.distance) + 24.0 {
                    return None;
                }
                let min = origin.as_vec3();
                let max = min + Vec3::splat(16.0);
                if planes.iter().any(|p| {
                    let v = Vec3::new(
                        if p.x >= 0.0 { max.x } else { min.x },
                        if p.y >= 0.0 { max.y } else { min.y },
                        if p.z >= 0.0 { max.z } else { min.z },
                    );
                    p.truncate().dot(v) + p.w < 0.0
                }) {
                    return None;
                }
                Some((
                    (sx, sy, sz),
                    center.length_squared(),
                    origin.as_vec3().to_array(),
                ))
            })
            .collect();
        visible.sort_by(|a, b| a.1.total_cmp(&b.1));
        self.drawn_sections = visible.len();
        if visible.len() as u64 > self.instance_capacity {
            self.instance_capacity = (visible.len() as u64).next_power_of_two();
            self.instances = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Section origins"),
                size: self.instance_capacity * 16,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        let origins: Vec<[f32; 4]> = visible
            .iter()
            .map(|(_, _, o)| [o[0], o[1], o[2], 0.0])
            .collect();
        if !origins.is_empty() {
            self.queue
                .write_buffer(&self.instances, 0, bytemuck::cast_slice(&origins));
        }
        let outline_edges = frame.outline.len().min(MAX_OUTLINE_EDGES as usize);
        if outline_edges > 0 {
            // A hair outside the block, like vanilla's line depth offset.
            let lines: Vec<[f32; 3]> = frame.outline[..outline_edges]
                .iter()
                .flat_map(|e| {
                    let center = [
                        (e[0] + e[3]) / 2.0,
                        (e[1] + e[4]) / 2.0,
                        (e[2] + e[5]) / 2.0,
                    ];
                    let point = |i: usize| -> [f32; 3] {
                        std::array::from_fn(|a| {
                            let v = e[i + a];
                            let grow = if v < center[a] {
                                -0.002
                            } else if v > center[a] {
                                0.002
                            } else {
                                0.0
                            };
                            (v - [camera.x, camera.y, camera.z][a]) as f32 + grow
                        })
                    };
                    [point(0), point(3)]
                })
                .collect();
            self.queue
                .write_buffer(&self.outline, 0, bytemuck::cast_slice(&lines));
        }
        let mut combined;
        let all_entities: &[Vertex] = if frame.hand.is_empty() && frame.additive.is_empty() {
            frame.entities
        } else {
            combined =
                Vec::with_capacity(frame.entities.len() + frame.additive.len() + frame.hand.len());
            combined.extend_from_slice(frame.entities);
            combined.extend_from_slice(frame.additive);
            combined.extend_from_slice(frame.hand);
            &combined
        };
        let hand_quads = (frame.hand.len() / 4) as u32;
        let additive_quads = (frame.additive.len() / 4) as u32;
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
            self.queue
                .write_buffer(&self.entity_buffer, 0, bytemuck::cast_slice(all_entities));
        }
        let celestial_vertices = frame.celestial.len().min(2048 * 4);
        if celestial_vertices > 0 {
            self.queue.write_buffer(
                &self.celestial_buffer,
                0,
                bytemuck::cast_slice(&frame.celestial[..celestial_vertices]),
            );
        }
        let entity_quads = (entity_vertices / 4) as u32 - hand_quads - additive_quads;
        if entity_quads + additive_quads + hand_quads > self.index_quads {
            self.index_quads = (entity_quads + additive_quads + hand_quads).next_power_of_two();
            self.indices = Self::quad_indices(&self.device, self.index_quads);
        }
        let view_target = surface_frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor {
                format: Some(self.config.format.remove_srgb_suffix()),
                ..Default::default()
            });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Horizon frame"),
            });
        {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &view_target,
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
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
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
            if sky_visible && (frame.skybox || frame.end_sky) {
                if !frame.title && frame.skybox {
                    pass.set_pipeline(&self.sky);
                    pass.draw(0..3, 0..1);
                }
                if celestial_vertices > 0 {
                    pass.set_pipeline(&self.celestial_pipeline);
                    pass.set_bind_group(0, &self.entity_group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.set_vertex_buffer(0, self.celestial_buffer.slice(..));
                    pass.draw_indexed(0..(celestial_vertices as u32 / 4) * 6, 0, 0..1);
                    pass.set_bind_group(0, &self.group, &[]);
                }
                if !frame.title && frame.dark_disc {
                    pass.set_pipeline(&self.dark_disc);
                    pass.draw(0..3, 0..1);
                }
            }
            if frame.title {
                visible.clear();
            }
            pass.set_pipeline(&self.solid);
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_vertex_buffer(1, self.instances.slice(..));
            for (i, (key, _, _)) in visible.iter().enumerate() {
                let s = &self.sections[key];
                if s.solid == 0 {
                    continue;
                }
                pass.set_vertex_buffer(0, s.buffer.slice(..));
                pass.draw_indexed(0..s.solid * 6, 0, i as u32..i as u32 + 1);
            }
            if entity_quads > 0 {
                pass.set_pipeline(&self.entity_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(0..entity_quads * 6, 0, 0..1);
                pass.set_bind_group(0, &self.group, &[]);
            }
            if additive_quads > 0 {
                pass.set_pipeline(&self.additive_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(0..additive_quads * 6, (entity_quads * 4) as i32, 0..1);
                pass.set_bind_group(0, &self.group, &[]);
            }
            if outline_edges > 0 {
                pass.set_pipeline(&self.lines);
                pass.set_vertex_buffer(0, self.outline.slice(..));
                pass.draw(0..outline_edges as u32 * 2, 0..1);
            }
            pass.set_pipeline(&self.translucent);
            pass.set_vertex_buffer(1, self.instances.slice(..));
            for (i, (key, _, _)) in visible.iter().enumerate().rev() {
                let s = &self.sections[key];
                if s.translucent == 0 {
                    continue;
                }
                pass.set_vertex_buffer(0, s.buffer.slice(..));
                pass.draw_indexed(
                    0..s.translucent * 6,
                    (s.solid * 4) as i32,
                    i as u32..i as u32 + 1,
                );
            }
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
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.entity_pipeline);
            pass.set_bind_group(0, &self.entity_group, &[]);
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
            pass.draw_indexed(
                0..hand_quads * 6,
                ((entity_quads + additive_quads) * 4) as i32,
                0..1,
            );
        }
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
            self.queue
                .write_buffer(&self.screen_buffer, 0, bytemuck::cast_slice(&vertices));
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
                let pipeline = match quad.blend {
                    crate::screen_effects::Blend::Alpha => &self.screen_pipelines[0],
                    crate::screen_effects::Blend::Vignette => &self.screen_pipelines[1],
                };
                pass.set_pipeline(pipeline);
                pass.draw_indexed(0..6, (i * 4) as i32, 0..1);
            }
        }
        self.hud.draw(
            &self.device,
            &self.queue,
            &mut encoder,
            &view_target,
            frame.hud,
            [self.config.width, self.config.height],
        );
        let readback = self
            .screenshot
            .take()
            .map(|path| (path, self.copy_frame(&mut encoder, &surface_frame.texture)));
        self.queue.submit([encoder.finish()]);
        if let Some((path, buffer)) = readback {
            self.screenshot_result = Some(
                self.write_screenshot(&path, &buffer)
                    .map_err(|e| e.to_string()),
            );
        }
        self.window.pre_present_notify();
        self.queue.present(surface_frame);
        if reconfigure {
            self.surface.configure(&self.device, &self.config);
        }
        let _ = self.started;
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
