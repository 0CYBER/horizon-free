use super::{
    Hdr, Output,
    tests::{gpu, texel},
};
use std::{cell::RefCell, collections::HashMap};
use wgpu::TextureFormat as F;
use wgpu::util::DeviceExt;

/// The look's gains, low to very bright (hdr_look.wgsl `HDR_GAIN`).
const GAINS: [f32; 4] = [1.6, 2.4, 2.8, 4.0];

const TERRAIN_VERTEX: &str = r#"
override TEST_TIER: u32 = 0u;
override TEST_OVERLAY: u32 = 0u;
@vertex fn test_vertex(@builtin(vertex_index) i: u32) -> TerrainWire {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    var o: TerrainOut;
    o.clip = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2(0.5);
    o.local = vec3(p.x, 0.0, p.y);
    // Its brightest channel is 1, as on a top face in full light, so a lifted texel has no
    // shading to leave behind (`hdr_unshade`); `misc.w` shades it as another face. A test's own
    // colour comes in the first coloured light's lane (none is counted, so nothing reads it).
    let shade = select(1.0, globals.misc.w, globals.misc.w > 0.0);
    let own = globals.colored_lights[0];
    o.color = vec4(select(vec3(0.8, 1.0, 0.7), own.rgb, own.a > 0.0) * shade,
        select(1.0, 0.5, globals.misc.z > 0.5));
    o.scene = vec2(15.0);
    o.spherical = globals.misc.x;
    o.cylindrical = globals.misc.x;
    o.visibility = globals.misc.y;
    o.emission = TEST_TIER;
    o.emissive_layer = select(0xffffffffu, 1u, TEST_OVERLAY != 0u);
    return to_wire(o);
}
"#;
const ENTITY_VERTEX: &str = r#"
override TEST_TIER: u32 = 0u;
override TEST_OVERLAY: u32 = 0u;
@vertex fn test_vertex(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.clip = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2(0.5);
    o.color = vec4(0.8, 0.9, 0.7, 1.0);
    o.back_color = o.color;
    o.light_color = vec4(0.7, 0.8, 0.9, 1.0);
    o.overlay_color = vec4(1.0);
    o.tier = TEST_TIER;
    o.overlay = TEST_OVERLAY;
    o.wrap = vec2(0.0, 1.0);
    o.proj = vec4(0.4, 0.6, 0.0, 1.0);
    o.fog_distance = vec2(globals.misc.x);
    return o;
}
"#;

/// A quad the entity stage lights as an entity (kind 0), unshaded, at full light.
const ENTITY_STAGE_VERTEX: &str = r#"
override TEST_TIER: u32 = 0u;
override TEST_OVERLAY: u32 = 0u;
override test_layer_alpha: f32 = 1.0;
@vertex fn test_vertex(@builtin(vertex_index) i: u32) -> horizon_entity_out {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    var o: horizon_entity_out;
    o.clip = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2(0.5);
    o.color = vec4(1.0, 1.0, 1.0, test_layer_alpha);
    o.back_color = vec4(1.0, 1.0, 0.0, 0.0);
    o.light_color = vec4(240.0, 240.0, 0.0, 0.0);
    o.overlay_color = vec4(1.0);
    o.overlay = TEST_OVERLAY;
    o.wrap = vec2(0.0, 1.0);
    o.glint_uv = vec2(0.5);
    o.flags = (1u << 3u) | (TEST_TIER << 8u);
    return o;
}
"#;

const TERRAIN_EXTRA_VERTICES: &str = r#"
@vertex fn test_model(@builtin(vertex_index) i: u32) -> NativeModelOut {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    var o: NativeModelOut;
    o.clip = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2(0.5);
    o.color = vec4(0.8, 0.9, 0.7, 1.0);
    return o;
}
@vertex fn test_fluid(@builtin(vertex_index) i: u32) -> NativeFluidOut {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    var o: NativeFluidOut;
    o.clip = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2(0.5);
    o.color = vec4(0.8, 0.9, 0.7, 1.0);
    return o;
}
@vertex fn test_sunrise(@builtin(vertex_index) i: u32) -> SunriseOut {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return SunriseOut(vec4(p * 2.0 - 1.0, 0.0, 1.0), 0.5);
}
"#;

struct Draw<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    terrain: bool,
    shader: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    bindings: wgpu::BindGroupLayout,
    group: wgpu::BindGroup,
    raw: bool,
    hdr_lighting: bool,
    material_frame: Option<super::super::native_post::NativeFrame>,
    /// More pipeline constants, for the material wrappers' own.
    constants: Vec<(&'static str, f64)>,
    pipelines: RefCell<HashMap<PipelineKey, wgpu::RenderPipeline>>,
}
/// Everything that changes a pipeline built by `Draw::render`: the entry point,
/// the override constants, and the draw's own flags. The
/// shader and layout only change in `material`, which empties the cache.
#[derive(PartialEq, Eq, Hash)]
struct PipelineKey {
    entry: String,
    written: bool,
    tier: u32,
    overlay: u32,
    terrain: bool,
    raw: bool,
    hdr_lighting: bool,
    material: bool,
    /// `Draw::constants`, each value by its bits.
    constants: Vec<(&'static str, u64)>,
}
impl<'a> Draw<'a> {
    fn render_oit(
        &self,
        hdr: &mut Hdr,
        kind: &str,
        tier: u32,
        overlay: u32,
        written: bool,
    ) -> [f32; 4] {
        use super::super::oit::{Stage, stage_targets};
        let source = if self.terrain {
            super::super::terrain_shader()
                + include_str!("../oit.wgsl")
                + include_str!("../oit_world.wgsl")
                + TERRAIN_VERTEX
        } else {
            super::super::entity_shader()
                + include_str!("../oit.wgsl")
                + include_str!("../oit_features.wgsl")
                + include_str!("../oit_clouds.wgsl")
                + ENTITY_VERTEX
        }
        .replace(
            "o.clip = vec4(p * 2.0 - 1.0, 0.0, 1.0)",
            "o.clip = vec4(p * 2.0 - 1.0, 0.5, 1.0)",
        );
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("actual OIT fragments"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let screen_shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(
                    (super::super::terrain_shader()
                        + include_str!("../oit.wgsl")
                        + include_str!("../oit_world.wgsl")
                        + TERRAIN_VERTEX)
                        .into(),
                ),
            });
        let make = |format| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
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
        let bounds = make(F::Rgba32Float);
        let depth = make(F::Depth32Float);
        let coeff = [make(F::Rgba16Float), make(F::Rgba16Float)];
        let accumulated = make(F::Rgba16Float);
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&[0.0_f32, 1.0, 0.0, 0.0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let frame = hdr.frame(self.device, [1, 1]);
        let view = frame.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        super::tests::fill(&mut encoder, &frame, [2.5, 1.8, 0.9, 1.0]);
        let constants = [
            ("HDR_WRITE_LIGHT", f64::from(u8::from(written))),
            ("TEST_TIER", f64::from(tier)),
            ("TEST_OVERLAY", f64::from(overlay)),
        ];
        for stage in 0..4 {
            let textures: Vec<_> = match stage {
                0 => vec![],
                1 => vec![&bounds],
                2 => vec![&bounds, &coeff[0], &coeff[1]],
                _ => vec![&bounds, &coeff[0], &coeff[1], &accumulated],
            };
            let mut entries = vec![wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }];
            entries.extend(
                textures
                    .iter()
                    .enumerate()
                    .map(|(i, _)| wgpu::BindGroupLayoutEntry {
                        binding: i as u32 + 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    }),
            );
            let bindings = self
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: None,
                    entries: &entries,
                });
            let mut resources = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }];
            resources.extend(
                textures
                    .iter()
                    .enumerate()
                    .map(|(i, t)| wgpu::BindGroupEntry {
                        binding: i as u32 + 1,
                        resource: wgpu::BindingResource::TextureView(t),
                    }),
            );
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &bindings,
                entries: &resources,
            });
            let layout = self
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[Some(&self.bindings), None, Some(&bindings)],
                    ..Default::default()
                });
            let suffix = ["bounds", "transmittance", "accumulate", "composite"][stage];
            let entry = if stage == 3 {
                "fs_oit_composite".to_owned()
            } else if self.terrain {
                format!("fs_oit_{suffix}")
            } else {
                format!("fs_oit_{kind}_{suffix}")
            };
            let targets = if stage == 3 {
                vec![Some(wgpu::ColorTargetState {
                    format: F::Rgba16Float,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })]
            } else {
                stage_targets([Stage::DepthBounds, Stage::Transmittance, Stage::Accumulate][stage])
            };
            let shader = if stage == 3 { &screen_shader } else { &shader };
            let pipeline = self
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(&entry),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: shader,
                        entry_point: Some("test_vertex"),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &constants,
                            ..Default::default()
                        },
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: (stage == 3).then_some(wgpu::DepthStencilState {
                        format: F::Depth32Float,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::Always),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: shader,
                        entry_point: Some(&entry),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &constants,
                            ..Default::default()
                        },
                        targets: &targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                });
            let outputs = match stage {
                0 => vec![&bounds],
                1 => vec![&coeff[0], &coeff[1]],
                2 => vec![&accumulated],
                _ => vec![&view],
            };
            let attachments: Vec<_> = outputs
                .iter()
                .map(|v| {
                    Some(wgpu::RenderPassColorAttachment {
                        view: v,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: if stage == 3 {
                                wgpu::LoadOp::Load
                            } else {
                                wgpu::LoadOp::Clear(if stage == 0 {
                                    wgpu::Color {
                                        r: -(f32::MAX as f64),
                                        g: 0.0,
                                        b: 0.0,
                                        a: 0.0,
                                    }
                                } else {
                                    wgpu::Color::TRANSPARENT
                                })
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })
                })
                .collect();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &attachments,
                depth_stencil_attachment: (stage == 3).then_some(
                    wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    },
                ),
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_bind_group(2, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        texel(self.device, self.queue, encoder, &frame)
    }
    fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        terrain: bool,
        globals: &super::super::Globals,
        alpha: u8,
    ) -> Self {
        Self::with_lightmap(device, queue, terrain, globals, [200, 100, 50], alpha, 255)
    }
    /// `new` with the sprite's `texel`, and the terrain's lightmap at `full` in every channel at
    /// full light (sky 15, block 15), where `new` has white. Every other light level is black.
    fn with_lightmap(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        terrain: bool,
        globals: &super::super::Globals,
        [r, g, b]: [u8; 3],
        alpha: u8,
        full: u8,
    ) -> Self {
        let source = if terrain {
            super::super::terrain_shader() + TERRAIN_VERTEX + TERRAIN_EXTRA_VERTICES
        } else {
            super::super::entity_shader() + ENTITY_VERTEX
        };
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("actual HDR draw shader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let mut entries = vec![];
        for binding in 0..if terrain { 13 } else { 8 } {
            let sampler = if terrain {
                binding == 2 || binding == 4
            } else {
                binding == 2 || binding == 4 || binding == 6
            };
            let ty = if binding == 0 {
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                }
            } else if sampler {
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
            } else {
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    multisampled: false,
                    view_dimension: if terrain && binding != 3 && binding != 12 {
                        wgpu::TextureViewDimension::D2Array
                    } else {
                        wgpu::TextureViewDimension::D2
                    },
                }
            };
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty,
                count: None,
            });
        }
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bindings)],
            ..Default::default()
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(globals),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let texture = |layers, data: &[u8]| {
            device.create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: layers,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: F::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                data,
            )
        };
        let array = texture(2, &[r, g, b, alpha, 240, 210, 80, 128]);
        let array = array.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let image = texture(1, &[r, g, b, alpha]).create_view(&Default::default());
        let threshold = texture(1, &[0, 0, 0, 0]).create_view(&Default::default());
        let mut levels = [0u8; 16 * 16 * 4];
        levels[16 * 16 * 4 - 4..].copy_from_slice(&[full, full, full, 255]);
        let lightmap = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 16,
                        height: 16,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: F::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &levels,
            )
            .create_view(&Default::default());
        let sampler = device.create_sampler(&Default::default());
        let resources: Vec<_> = entries
            .iter()
            .map(|entry| wgpu::BindGroupEntry {
                binding: entry.binding,
                resource: match entry.ty {
                    wgpu::BindingType::Buffer { .. } => uniform.as_entire_binding(),
                    wgpu::BindingType::Sampler(_) => wgpu::BindingResource::Sampler(&sampler),
                    wgpu::BindingType::Texture {
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        ..
                    } => wgpu::BindingResource::TextureView(&array),
                    _ => wgpu::BindingResource::TextureView(match entry.binding {
                        12 => &threshold,
                        3 if terrain => &lightmap,
                        _ => &image,
                    }),
                },
            })
            .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bindings,
            entries: &resources,
        });
        Self {
            device,
            queue,
            terrain,
            shader,
            layout,
            bindings,
            group,
            raw: false,
            hdr_lighting: true,
            material_frame: None,
            constants: Vec::new(),
            pipelines: RefCell::default(),
        }
    }

    fn material(&mut self) {
        self.pack_stage(super::super::native_material::coverage_test_shader(false));
    }
    /// The entity stage's wrapper around a pack's `entity` that returns alpha `test_alpha`,
    /// lighting a quad as an entity with the vertex alpha `test_layer_alpha` (the layer's
    /// opacity).
    fn entity_stage(&mut self) {
        let entity = "override test_alpha: f32 = 0.7;\n\
            fn entity(input: EntityInput) -> Surface { return Surface(vec4(1.4, 0.3, 0.1, test_alpha)); }\n";
        self.pack_stage(super::super::native_entity::tests::shader(entity) + ENTITY_STAGE_VERTEX);
    }
    /// Draws through the pack wrapper `source` with the pack's frame, as the pack runs.
    fn pack_stage(&mut self, source: String) {
        self.shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("actual assembled pack wrappers"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let frame = super::super::native_post::NativeFrame::new(
            self.device,
            None,
            Vec::new(),
            Default::default(),
            None,
            false,
        );
        self.queue.write_buffer(
            &frame.buffer,
            0,
            bytemuck::bytes_of(&horizon_shader_api::Frame::default()),
        );
        self.layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&self.bindings), None, None, Some(&frame.layout)],
                ..Default::default()
            });
        self.material_frame = Some(frame);
        self.raw = true;
        self.pipelines.borrow_mut().clear();
    }
    fn render(
        &self,
        written: bool,
        entry: &str,
        tier: u32,
        overlay: u32,
        background: [f64; 4],
    ) -> [f32; 4] {
        let mut constants = vec![
            (
                match (self.material_frame.is_some(), self.terrain) {
                    (true, true) => "horizon_terrain_HDR_WRITE_LIGHT",
                    (true, false) => "horizon_vanilla_HDR_WRITE_LIGHT",
                    (false, _) => "HDR_WRITE_LIGHT",
                },
                f64::from(u8::from(written)),
            ),
            ("TEST_TIER", f64::from(tier)),
            ("TEST_OVERLAY", f64::from(overlay)),
        ];
        constants.extend(self.constants.iter().copied());
        if self.terrain {
            constants.push((
                if self.material_frame.is_some() {
                    "horizon_terrain_HDR_LIGHTING"
                } else {
                    "HDR_LIGHTING"
                },
                f64::from(u8::from(self.hdr_lighting)),
            ));
        }
        if self.material_frame.is_some() && self.terrain {
            constants.push((
                "horizon_displace_distance",
                if entry == "horizon_material_fs_displaced" {
                    10.0
                } else {
                    0.0
                },
            ));
        }
        let vertex = match entry {
            "fs_sky" | "fs_sky_base" => "vs_sky",
            "fs_sunrise" => "test_sunrise",
            "fs_native_model" => "test_model",
            "fs_native_fluid" => "test_fluid",
            _ => "test_vertex",
        };
        let format = F::Rgba16Float;
        let key = PipelineKey {
            entry: entry.to_owned(),
            written,
            tier,
            overlay,
            terrain: self.terrain,
            raw: self.raw,
            hdr_lighting: self.hdr_lighting,
            material: self.material_frame.is_some(),
            constants: self
                .constants
                .iter()
                .map(|&(name, value)| (name, value.to_bits()))
                .collect(),
        };
        let cached = self.pipelines.borrow().get(&key).cloned();
        let pipeline = cached.unwrap_or_else(|| {
            let built = self
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: None,
                    layout: Some(&self.layout),
                    vertex: wgpu::VertexState {
                        module: &self.shader,
                        entry_point: Some(vertex),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &constants,
                            ..Default::default()
                        },
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &self.shader,
                        entry_point: Some(entry),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &constants,
                            ..Default::default()
                        },
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: Some(
                                if self.raw || (self.terrain && entry != "fs_translucent") {
                                    wgpu::BlendState::REPLACE
                                } else {
                                    wgpu::BlendState::ALPHA_BLENDING
                                },
                            ),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
            self.pipelines.borrow_mut().insert(key, built.clone());
            built
        });
        let frame = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = frame.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let [r, g, b, a] = background;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, &self.group, &[]);
            if let Some(frame) = &self.material_frame {
                pass.set_bind_group(3, &frame.group, &[]);
            }
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }
        texel(self.device, self.queue, encoder, &frame)
    }
}

#[test]
fn float_plain_fragments_keep_rgb_and_write_only_coverage() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = gpu() else {
        eprintln!("plain float GPU unavailable");
        return;
    };
    let hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
    let settings = hdr.draw_params(true);
    let mut cases = 0;
    for hdr_on in [false, true] {
        let globals = super::super::Globals {
            inverse_view: glam::Mat4::IDENTITY.to_cols_array_2d(),
            fog_color: [0.2, 0.3, 0.4, 1.0],
            sky_color: [0.7, 0.8, 0.9, 1.0],
            sunrise: [1.2, 0.7, 0.4, 0.6],
            fog: [0.0, 100.0, 0.0, 100.0],
            fog_ends: [100.0, 100.0, 0.0, 0.0],
            misc: [0.0, 1.0, 0.0, 0.0],
            filtering: [0.0, 1.0, 0.0, 0.0],
            hdr_light: [0.0, 0.0, f32::from(u8::from(hdr_on)), 0.0],
            hdr_settings: settings,
            ..bytemuck::Zeroable::zeroed()
        };
        for terrain in [true, false] {
            let mut draw = Draw::new(&device, &queue, terrain, &globals, 180);
            draw.raw = true;
            let coverage = 180.0 / 255.0;
            let entries = if terrain {
                vec![
                    ("fs_opaque", 1.0),
                    ("fs_solid", 1.0),
                    ("fs_solid_mirror", 1.0),
                    ("fs_translucent", coverage),
                    ("fs_native_model", 1.0),
                    ("fs_native_fluid", coverage),
                    ("fs_sky", 1.0),
                    ("fs_sky_base", 1.0),
                    ("fs_sunrise", 0.3),
                ]
            } else {
                vec![
                    ("fs_entity", coverage),
                    ("fs_celestial", coverage),
                    ("fs_sky_celestial", coverage),
                ]
            };
            // `HDR_LIGHTING` is only a pipeline constant for terrain, so entity
            // pipelines are identical for both values and need one pass.
            for plain in if terrain {
                vec![false, true]
            } else {
                vec![false]
            } {
                draw.hdr_lighting = !plain;
                for &(entry, alpha) in &entries {
                    // These vertex functions never read `TEST_TIER` or
                    // `TEST_OVERLAY`, so one combination covers them.
                    let fixed =
                        matches!(entry, "fs_native_model" | "fs_native_fluid" | "fs_sunrise");
                    for tier in if fixed { vec![0] } else { vec![0, 3, 4] } {
                        for overlay in if fixed {
                            vec![0]
                        } else if terrain {
                            vec![0, 1]
                        } else {
                            vec![0, 6]
                        } {
                            let old = draw.render(false, entry, tier, overlay, [0.0; 4]);
                            let float = draw.render(true, entry, tier, overlay, [0.0; 4]);
                            assert_eq!(
                                &float[..3],
                                &old[..3],
                                "{hdr_on} {entry} {tier} {overlay} {plain}"
                            );
                            assert!(
                                (float[3] - alpha).abs() < 0.001,
                                "{entry}: {float:?}, expected alpha {alpha}"
                            );
                            assert!(
                                float[..3].iter().any(|&c| c > 0.0),
                                "empty fixture: {entry}"
                            );
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    eprintln!("plain float GPU ran {cases} colour/coverage comparisons");
}

#[test]
fn float_stars_keep_the_original_rgb_with_brightness_as_coverage() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
    let settings = hdr.draw_params(true);
    // Frozen f030d9c80 fragment: the original written branch and its marked fallback.
    let source = super::super::wgsl_prefix::HDR_LIGHT.to_owned()
        + include_str!("stars.wgsl")
        + r#"
@vertex fn fixture(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4(vec2(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn reference() -> @location(0) vec4<f32> {
    let b = stars.color.a;
    if HDR_WRITE_LIGHT && stars.hdr_light.w != 0.0 {
        return vec4(hdr_written(stars.color.rgb * b, 0.175, stars.hdr_settings), b);
    }
    return vec4(stars.color.rgb * b, 0.175);
}
"#;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("actual stars with original RGB reference"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    for gain in [0.0, 1.0] {
        let mut data = [0.0_f32; 28];
        data[16..20].copy_from_slice(&[0.7, 0.8, 0.9, 0.35]);
        data[20..24].copy_from_slice(&settings);
        data[27] = gain;
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&data),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut draw = |entry| {
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("fixture"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[("HDR_WRITE_LIGHT", 1.0)],
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: F::Rgba16Float,
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
                    resource: uniform.as_entire_binding(),
                }],
            });
            let frame = hdr.frame(&device, [1, 1]);
            let view = frame.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            }
            texel(&device, &queue, encoder, &frame)
        };
        let old = draw("reference");
        let float = draw("fs_stars");
        assert_eq!(&float[..3], &old[..3]);
        assert!((float[3] - 0.35).abs() < 0.001, "{float:?}");
    }
}

/// A device for the real material layout, which binds the terrain arrays and the pack's frame
/// together.
fn material_gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = super::super::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    if adapter.limits().max_sampled_textures_per_shader_stage < 19 {
        return None;
    }
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_sampled_textures_per_shader_stage: 19,
            ..Default::default()
        },
        ..Default::default()
    }))
    .ok()
}

#[test]
fn float_material_wrappers_keep_rgb_and_coverage_including_displacement() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = material_gpu() else {
        return;
    };
    let globals = super::super::Globals {
        filtering: [0.0, 1.0, 0.0, 0.0],
        ..bytemuck::Zeroable::zeroed()
    };
    let mut draw = Draw::new(&device, &queue, true, &globals, 255);
    draw.material();
    for entry in [
        "horizon_material_fs_solid",
        "horizon_material_fs_displaced",
        "horizon_material_fs_translucent",
    ] {
        let old = draw.render(false, entry, 3, 0, [0.0; 4]);
        let float = draw.render(true, entry, 3, 0, [0.0; 4]);
        assert_eq!(&float[..3], &old[..3], "{entry}");
        let alpha = if entry.ends_with("translucent") {
            0.7
        } else {
            1.0
        };
        assert!((float[3] - alpha).abs() < 0.001, "{entry}: {float:?}");
        assert!(
            float[0] > 1.0,
            "{entry}: clipping or an empty fixture: {float:?}"
        );
    }
    eprintln!("material GPU ran the solid, displaced and translucent wrappers");
}

#[test]
fn material_coverage_is_the_texture_s_unless_the_pack_declares_its_own() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = material_gpu() else {
        eprintln!("material coverage GPU unavailable");
        return;
    };
    let globals = super::super::Globals {
        filtering: [0.0, 1.0, 0.0, 0.0],
        ..bytemuck::Zeroable::zeroed()
    };
    let background = [0.0, 0.0, 0.25, 0.5];
    // (coverage = "pack", the texel's alpha, the alpha `material` returns, solid drawn,
    // translucent drawn): under "engine" the texture decides and the returned alpha is the
    // translucent pass's opacity only; under "pack" the returned alpha decides.
    for (pack, texel, returned, solid, translucent) in [
        (false, 255, 0.0, true, true),
        (false, 0, 1.0, false, false),
        (false, 255, 0.7, true, true),
        (true, 255, 0.0, false, false),
        (true, 0, 1.0, true, true),
        (true, 255, 0.7, true, true),
    ] {
        let mut draw = Draw::new(&device, &queue, true, &globals, texel);
        draw.material();
        draw.constants = vec![
            ("horizon_material_coverage_pack", f64::from(u8::from(pack))),
            ("test_alpha", returned),
        ];
        let case = format!("pack {pack}, texel alpha {texel}, returned {returned}");
        for (entry, drawn) in [
            ("horizon_material_fs_solid", solid),
            ("horizon_material_fs_translucent", translucent),
        ] {
            let got = draw.render(true, entry, 0, 0, background);
            if !drawn {
                assert_eq!(got, background.map(|c| c as f32), "{case} {entry}");
                continue;
            }
            assert!(got[0] > 1.0, "{case} {entry}: not drawn: {got:?}");
            // The test pipeline replaces: the alpha written is what the blend would weigh by.
            let alpha = if entry.ends_with("translucent") {
                returned as f32
            } else {
                1.0
            };
            assert!((got[3] - alpha).abs() < 0.001, "{case} {entry}: {got:?}");
        }
    }
}

#[test]
fn entity_coverage_is_the_texture_s_and_the_layer_s_unless_the_pack_declares_its_own() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = material_gpu() else {
        eprintln!("entity coverage GPU unavailable");
        return;
    };
    let globals = super::super::Globals {
        filtering: [0.0, 1.0, 0.0, 0.0],
        ..bytemuck::Zeroable::zeroed()
    };
    let background = [0.0, 0.0, 0.25, 0.5];
    // (coverage = "pack", overlay, the texel's alpha, the vertex alpha, the alpha `entity`
    // returns, the alpha drawn or None): under "engine" the texture's alpha times the layer's
    // opacity is cut below 0.1 and blends, as Horizon's own entities; a dissolve mask (overlay
    // 38, the texel's alpha here) cuts where the vertex alpha is below it and draws the rest
    // at the texture's alpha. Under "pack" the returned alpha does both.
    let half = 128.0 / 255.0;
    for (pack, overlay, texel, layer, returned, drawn) in [
        (false, 0, 255, 1.0, 0.0, Some(1.0)),
        (false, 0, 255, 0.5, 1.0, Some(0.5)),
        (false, 0, 0, 1.0, 1.0, None),
        (false, 0, 255, 0.05, 1.0, None),
        (false, 38, 128, 0.6, 0.0, Some(half)),
        (false, 38, 128, 0.4, 1.0, None),
        (true, 0, 255, 1.0, 0.0, None),
        (true, 0, 0, 1.0, 0.7, Some(0.7)),
        (true, 0, 255, 0.5, 0.7, Some(0.7)),
        (true, 38, 128, 0.4, 1.0, None),
    ] {
        let mut draw = Draw::new(&device, &queue, false, &globals, texel);
        draw.entity_stage();
        draw.constants = vec![
            ("horizon_entity_coverage_pack", f64::from(u8::from(pack))),
            ("test_alpha", returned),
            ("test_layer_alpha", layer),
        ];
        let case = format!(
            "pack {pack}, overlay {overlay}, texel alpha {texel}, layer {layer}, returned {returned}"
        );
        let got = draw.render(true, "horizon_entity_fs", 0, overlay, background);
        let Some(alpha) = drawn else {
            assert_eq!(got, background.map(|c| c as f32), "{case}");
            continue;
        };
        assert!(got[0] > 1.0, "{case}: not drawn: {got:?}");
        // The test pipeline replaces: the alpha written is what the blend would weigh by.
        assert!((got[3] - alpha).abs() < 0.001, "{case}: {got:?}");
    }
}

#[test]
fn actual_translucent_writers_blend_their_own_light() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = gpu() else {
        eprintln!("HDR blend GPU unavailable");
        return;
    };
    let hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
    let settings = hdr.draw_params(true);
    let globals = super::super::Globals {
        fog: [0.0, 100.0, 0.0, 100.0],
        fog_ends: [100.0, 100.0, 0.0, 0.0],
        misc: [0.0, 1.0, 0.0, 0.0],
        filtering: [0.0, 1.0, 0.0, 0.0],
        hdr_light: [0.0, 0.0, 1.0, 1.0],
        hdr_settings: settings,
        ..bytemuck::Zeroable::zeroed()
    };
    // Texel alpha 100 is clear glass's kind of texel: it gets its own light too.
    for (terrain, entry, overlay, tier, tint_alpha, texel_alpha) in [
        (true, "fs_translucent", 0, 0, 1.0, 180),
        (true, "fs_translucent", 0, 0, 0.5, 180),
        (true, "fs_translucent", 0, 0, 1.0, 100),
        (false, "fs_entity", 0, 3, 1.0, 180),
        (false, "fs_entity", 4, 0, 1.0, 180),
    ] {
        let mut globals = globals;
        globals.misc[2] = if tint_alpha < 1.0 { 1.0 } else { 0.0 };
        let draw = Draw::new(&device, &queue, terrain, &globals, texel_alpha);
        let src = draw.render(true, entry, tier, overlay, [0.0; 4]);
        let dst = [2.5, 1.8, 0.9, 1.0];
        let got = draw.render(true, entry, tier, overlay, dst);
        let alpha = f32::from(texel_alpha) / 255.0 * tint_alpha;
        assert!(
            (src[3] - alpha).abs() < 0.001,
            "{entry}: {src:?}, coverage {alpha}"
        );
        if terrain {
            // Full block and sky light give no gain, whatever the texel's alpha, even when
            // vertex coverage is halved.
            let expected = 200.0 / 255.0 * 0.8 * alpha;
            assert!(
                (src[0] - expected).abs() < 0.002,
                "coverage {tint_alpha}: {src:?}, expected {expected}"
            );
        }
        for i in 0..3 {
            let want = src[i] + dst[i] as f32 * (1.0 - alpha);
            assert!(
                (got[i] - want).abs() < 0.003,
                "{entry} {overlay}: {got:?} expected {want}"
            );
        }
        assert!(got[0] > 1.0);
    }
    eprintln!("HDR actual blend GPU ran terrain, particle and cloud equations");
}

/// HDR Lighting lifts light sources and the sun, not what they light: under full block and sky
/// light, the sun straight on the face, a surface writes the light of its colour as drawn; a
/// light source's tier still gains.
#[test]
fn lit_surfaces_stay_at_their_sdr_light() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = gpu() else {
        eprintln!("HDR lit surface GPU unavailable");
        return;
    };
    let hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
    let settings = hdr.draw_params(true);
    let globals = super::super::Globals {
        fog: [0.0, 100.0, 0.0, 100.0],
        fog_ends: [100.0, 100.0, 0.0, 0.0],
        misc: [0.0, 1.0, 0.0, 0.0],
        filtering: [0.0, 1.0, 0.0, 0.0],
        // The test face lies flat (`TERRAIN_VERTEX`, at light level 15): a noon sun meets it
        // straight on.
        sun: [0.0, 1.0, 0.0, 0.0],
        hdr_light: [0.0, 0.0, 1.0, 1.0],
        hdr_settings: settings,
        ..bytemuck::Zeroable::zeroed()
    };
    // The texel (200, 100, 50) times the test vertex's colour, and an entity's light, in gamma.
    let texel = [200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0];
    let terrain = [texel[0] * 0.8, texel[1], texel[2] * 0.7];
    let entity = [
        texel[0] * 0.8 * 0.7,
        texel[1] * 0.9 * 0.8,
        texel[2] * 0.7 * 0.9,
    ];
    for (is_terrain, entry, overlay, colour) in [
        (true, "fs_opaque", 0, terrain),
        (true, "fs_solid", 0, terrain),
        (true, "fs_translucent", 0, terrain),
        (false, "fs_entity", 0, entity),
    ] {
        let draw = Draw::new(&device, &queue, is_terrain, &globals, 255);
        let got = draw.render(true, entry, 0, overlay, [0.0; 4]);
        for i in 0..3 {
            assert!(
                (got[i] - colour[i]).abs() < 0.002,
                "{entry}: {got:?}, drawn {colour:?}"
            );
        }
    }
    // The very bright tier all over (emission 8), bright enough to be fully lifted: its gain.
    let draw = Draw::new(&device, &queue, true, &globals, 255);
    let lifted = draw.render(true, "fs_opaque", 8, 0, [0.0; 4]);
    for i in 0..3 {
        let want = terrain[i] * GAINS[3].powf(1.0 / 2.2);
        assert!(
            (lifted[i] - want).abs() < 0.003,
            "{lifted:?}: expected {want}"
        );
    }
}

/// Terrain's light sources on the float frame: a fully lifted texel writes the same light on a
/// top face (shade 1), a north or south one (0.8), an east or west one (0.6), a bottom one (0.5)
/// and in dim light, its tier's gain times the texel and the face's hue; a half-lifted one sheds
/// half its shading; an unlifted one, and a face that gives no light, keep all of it. So does
/// every face with HDR Lighting off, where nothing is lifted. What dims the lightmap at full
/// light (the Darkness effect) dims a lifted texel as much on every face. A coloured shade keeps
/// its hue, and one without a channel stays without it.
#[test]
fn a_lifted_texel_shines_the_same_on_every_face() {
    let Some((device, queue)) = gpu() else {
        eprintln!("HDR face shading GPU unavailable");
        return;
    };
    let hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
    let settings = hdr.draw_params(true);
    // The test texels, by their brightness before lighting (hdr_look.wgsl `brightness`): one
    // past every tier's window (0.40) and one before them all (0.03).
    const FULL: [u8; 3] = [200, 100, 50];
    const DIM: [u8; 3] = [60, 40, 20];
    // And one about halfway through the tier's window, the low tier's (0.08 to 0.09) or the
    // others' (0.14 to 0.38), with its lift.
    let half = |emission: u32| {
        let (texel, start, full) = if matches!(emission, 1 | 5) {
            ([96u8, 58, 30], 0.08, 0.09)
        } else {
            ([170, 85, 40], 0.14, 0.38)
        };
        let light = texel.map(|c| (f32::from(c) / 255.0).powf(2.2));
        let y = 0.5 * (0.2126 * light[0] + 0.7152 * light[1] + 0.0722 * light[2] + light[0]);
        let t = (y - start) / (full - start);
        (texel, t * t * (3.0 - 2.0 * t))
    };
    // `own`: the vertex colour before `shade`, where a is above 0. `full`: the lightmap at full
    // light.
    let lit = |settings: [f32; 4],
               texel: [u8; 3],
               shade: f32,
               emission: u32,
               entry: &str,
               own: [f32; 4],
               full: u8| {
        let mut globals = super::super::Globals {
            fog: [0.0, 100.0, 0.0, 100.0],
            fog_ends: [100.0, 100.0, 0.0, 0.0],
            misc: [0.0, 1.0, 0.0, shade],
            filtering: [0.0, 1.0, 0.0, 0.0],
            hdr_light: [0.0, 0.0, 1.0, 1.0],
            hdr_settings: settings,
            ..bytemuck::Zeroable::zeroed()
        };
        globals.colored_lights[0][0] = own;
        let draw = Draw::with_lightmap(&device, &queue, true, &globals, texel, 255, full);
        draw.render(true, entry, emission, 0, [0.0; 4])
    };
    let draw = |settings, texel, shade, emission, entry: &str| {
        lit(settings, texel, shade, emission, entry, [0.0; 4], 255)
    };
    let face = |texel, shade, emission, entry: &str| draw(settings, texel, shade, emission, entry);
    let texel = [200.0 / 255.0 * 0.8, 100.0 / 255.0, 50.0 / 255.0 * 0.7];
    let near = |a: f32, b: f32| (a - b).abs() < 0.003 * b.max(1.0);
    let shades = [0.8, 0.6, 0.5, 0.2];
    for entry in ["fs_opaque", "fs_solid"] {
        // Each tier, by its bright parts (1 to 4) and all over (5 to 8).
        for emission in 1..=8 {
            let gain = GAINS[(emission as usize - 1) % 4].powf(1.0 / 2.2);
            let top = face(FULL, 1.0, emission, entry);
            for shade in shades {
                let side = face(FULL, shade, emission, entry);
                for i in 0..3 {
                    // The mark's 8 bits take up to 1% of a lift (`hdr_quantized_mark`).
                    assert!(
                        (top[i] - texel[i] * gain).abs() < 0.006 && near(side[i], top[i]),
                        "{entry} emission {emission} shade {shade}: {side:?}, top {top:?}"
                    );
                }
            }
            // A texel halfway through the tier's window: about half lifted, so it keeps about
            // half its shading. And one before it: not lifted, so shaded as any surface.
            let (half, half_lift) = half(emission);
            for shade in shades {
                let (lit, unlit) = (
                    face(half, shade, emission, entry),
                    face(half, 1.0, emission, entry),
                );
                let want = shade + (1.0 - shade) * half_lift;
                let (dark, bright) = (
                    face(DIM, shade, emission, entry),
                    face(DIM, 1.0, emission, entry),
                );
                for i in 0..3 {
                    assert!(
                        near(lit[i], unlit[i] * want) && near(dark[i], bright[i] * shade),
                        "{entry} emission {emission} shade {shade}: half {lit:?} of {unlit:?}, none {dark:?} of {bright:?}"
                    );
                }
            }
        }
        // The lightmap at 0.3 in full light, as under the Darkness effect. A top face is then
        // shaded 0.3 and a bottom one 0.15, and fully lifted both write 0.3 of the undimmed
        // top's light (0.3 is 0.3 in the frame too: it stores what the game draws). An unlifted
        // texel is drawn exactly as with any lightmap, since its shade is all it has.
        let dim = 77;
        let dimmed = f32::from(dim) / 255.0;
        for emission in [2, 7] {
            let top = face(FULL, 1.0, emission, entry);
            for shade in [dimmed, 0.5 * dimmed, 0.2 * dimmed] {
                let lifted = lit(settings, FULL, shade, emission, entry, [0.0; 4], dim);
                for i in 0..3 {
                    assert!(
                        near(lifted[i], top[i] * dimmed),
                        "{entry} emission {emission} shade {shade}, dimmed: {lifted:?} of {top:?}"
                    );
                }
                assert_eq!(
                    lit(settings, DIM, shade, emission, entry, [0.0; 4], dim),
                    face(DIM, shade, emission, entry),
                    "{entry} emission {emission} shade {shade}, dimmed and unlifted"
                );
            }
            // A shade brighter than the lightmap at full light (a coloured light adds past
            // it): lifting doesn't darken it.
            assert_eq!(
                lit(settings, FULL, 0.5, emission, entry, [0.0; 4], dim),
                lit(settings, FULL, 0.5, emission, entry, [0.0; 4], 0),
                "{entry} emission {emission}: a shade above the dimmed lightmap"
            );
        }
        // A coloured shade: its brightest channel goes to 1 and the others keep their ratio to
        // it, at any shade. One without blue stays without it, and all are finite.
        let gain = GAINS[2].powf(1.0 / 2.2);
        for own in [[0.9f32, 0.6, 0.3], [0.5, 0.25, 0.0], [0.0, 0.0, 0.0]] {
            let brightest = own[0].max(1e-4);
            for shade in [1.0, 0.5] {
                let lifted = lit(
                    settings,
                    FULL,
                    shade,
                    3,
                    entry,
                    [own[0], own[1], own[2], 1.0],
                    255,
                );
                let texel = [200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0];
                for i in 0..3 {
                    let want = texel[i] * own[i] / brightest * gain;
                    assert!(
                        lifted[i].is_finite()
                            && (lifted[i] - want).abs() < 0.006
                            && (own[i] != 0.0 || lifted[i] == 0.0),
                        "{entry} shade {own:?} x {shade}: {lifted:?}, want {want}"
                    );
                }
            }
        }
        // HDR Lighting off (the look's lane at 0): a light source is drawn as a plain surface.
        let unlit = [settings[0], settings[1], 0.0, settings[3]];
        for shade in shades {
            for emission in [3, 8] {
                assert_eq!(
                    draw(unlit, FULL, shade, emission, entry),
                    draw(unlit, FULL, shade, 0, entry),
                    "{entry} emission {emission} shade {shade}, HDR Lighting off"
                );
            }
        }
        for shade in shades {
            let surface = face(FULL, shade, 0, entry);
            for i in 0..3 {
                assert!(
                    near(surface[i], texel[i] * shade),
                    "{entry} shade {shade}: {surface:?}"
                );
            }
        }
    }
}

#[test]
fn unmarked_entity_users_keep_their_source_gain() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = gpu() else {
        return;
    };
    let hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
    let settings = hdr.draw_params(true);
    let globals = super::super::Globals {
        fog: [0.0, 100.0, 0.0, 100.0],
        fog_ends: [100.0, 100.0, 0.0, 0.0],
        hdr_light: [0.0, 0.0, 1.0, 1.0],
        hdr_settings: settings,
        ..bytemuck::Zeroable::zeroed()
    };
    // HDR Lighting off (the look's lane at 0): nothing is lifted.
    let unlit = super::super::Globals {
        hdr_settings: [settings[0], settings[1], 0.0, settings[3]],
        ..globals
    };
    let draw = Draw::new(&device, &queue, false, &globals, 255);
    let plain = Draw::new(&device, &queue, false, &unlit, 255);
    for (entry, overlay) in [
        ("fs_entity_unmarked", 0),
        ("fs_entity_shadow", 0),
        ("fs_energy_swirl", 0),
        ("fs_entity", 11),
        ("fs_entity", 12),
    ] {
        let raw = draw.render(true, entry, 0, overlay, [0.0; 4]);
        let unlifted = plain.render(true, entry, 0, overlay, [0.0; 4]);
        for i in 0..3 {
            assert!(
                (raw[i] - unlifted[i]).abs() < 0.001,
                "{entry} {overlay}: {raw:?} {unlifted:?}"
            );
        }
    }
}

#[test]
fn additive_texels_below_the_mark_cutoff_keep_their_gain() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = gpu() else {
        return;
    };
    let hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
    let settings = hdr.draw_params(true);
    let globals = super::super::Globals {
        fog: [0.0, 100.0, 0.0, 100.0],
        hdr_light: [0.0, 0.0, 1.0, 1.0],
        hdr_settings: settings,
        ..bytemuck::Zeroable::zeroed()
    };
    let draw = Draw::new(&device, &queue, false, &globals, 16);
    let background = [0.0, 0.0, 0.0, 1.0];
    let written = draw.render(true, "fs_celestial", 4, 0, background);
    // A texel this faint is not lifted (`hdr_entity_mark`); its alpha still adds its colour.
    let red = (200.0 / 255.0) * 0.8 * 0.7 * (16.0 / 255.0);
    assert!(
        (written[0] - red).abs() < 0.0001,
        "{written:?}: expected {red}"
    );
}

fn check_oit_writer(terrain: bool, kind: &str, overlay: u32, tier: u32, alphas: &[u8]) {
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("OIT GPU unavailable");
        return;
    };
    if !adapter
        .features()
        .contains(wgpu::Features::FLOAT32_BLENDABLE)
    {
        eprintln!("OIT unsupported on {:?}", adapter.get_info());
        return;
    }
    eprintln!(
        "OIT GPU {:?}: {kind}, alphas {alphas:?}",
        adapter.get_info()
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::FLOAT32_BLENDABLE,
        ..Default::default()
    }))
    .unwrap();
    let mut hdr = Hdr::with_debug(&device, &queue, F::Rgba16Float, Output::Pq, true, None);
    let settings = hdr.draw_params(true);
    let globals = super::super::Globals {
        fog: [0.0, 100.0, 0.0, 100.0],
        fog_ends: [100.0, 100.0, 0.0, 0.0],
        misc: [0.0, 1.0, 0.0, 0.0],
        filtering: [0.0, 1.0, 0.0, 0.0],
        hdr_light: [0.0, 0.0, 1.0, 1.0],
        hdr_settings: settings,
        ..bytemuck::Zeroable::zeroed()
    };
    for &alpha in alphas {
        let draw = Draw::new(&device, &queue, terrain, &globals, alpha);
        let got = draw.render_oit(&mut hdr, kind, tier, overlay, true);
        let unconverted = draw.render_oit(&mut hdr, kind, tier, overlay, false);
        let entry = if terrain {
            "fs_translucent"
        } else if kind == "lightning" {
            "fs_celestial"
        } else {
            "fs_entity"
        };
        let mut expected = draw.render(
            true,
            entry,
            tier,
            overlay,
            if kind == "lightning" {
                [0.0; 4]
            } else {
                [2.5, 1.8, 0.9, 1.0]
            },
        );
        if kind == "lightning" {
            for (c, background) in expected[..3].iter_mut().zip([2.5, 1.8, 0.9]) {
                *c += background;
            }
        }
        for i in 0..3 {
            assert!(
                (got[i] - expected[i]).abs() < 0.006,
                "{kind} alpha {alpha}: {got:?} expected {expected:?}"
            );
        }
        assert_eq!(got[3], 1.0);
        assert!(got[0] > 1.0, "OIT clipped light: {got:?}");
        // A light source's conversion lifts it; a surface's or a cloud's leaves it as drawn.
        assert_eq!(
            (got[0] - unconverted[0]).abs() > 0.01,
            tier > 0,
            "{kind}: {got:?} unconverted {unconverted:?}"
        );
    }
}

#[test]
fn actual_oit_terrain_writes_its_own_light_including_clear_glass() {
    let _gpu = crate::gfx::test_gpu::lock();
    check_oit_writer(true, "terrain", 0, 0, &[180, 100]);
}
#[test]
fn actual_oit_particle_writes_its_own_light() {
    let _gpu = crate::gfx::test_gpu::lock();
    check_oit_writer(false, "entity", 0, 3, &[180]);
}
#[test]
fn actual_oit_cloud_writes_its_own_light() {
    let _gpu = crate::gfx::test_gpu::lock();
    check_oit_writer(false, "cloud", 4, 0, &[180]);
}
#[test]
fn actual_oit_additive_adds_converted_light_without_coverage() {
    let _gpu = crate::gfx::test_gpu::lock();
    check_oit_writer(false, "lightning", 0, 4, &[180]);
}
