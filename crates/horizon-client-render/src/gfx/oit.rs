//! Improved transparency: vanilla 26.3's order-independent transparency
//! (`LevelRenderer.executeOit`). The translucent layer is drawn three times
//! against the opaque world's depth, without writing it, into targets the
//! next stage reads (`oit.wgsl` says what each holds), then a full-screen pass
//! composites the sum. With the option off none of this exists and the world
//! draws as before.

use super::oit_features::Features;
use glam::Mat4;

/// The depth bounds' clear: `LevelRenderer.DEPTH_BOUNDS_CLEAR_COLOR`.
const BOUNDS_CLEAR: wgpu::Color = wgpu::Color {
    r: -(f32::MAX as f64),
    g: 0.0,
    b: 0.0,
    a: 0.0,
};
const BOUNDS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const COEFFICIENT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The shader's bind group: `terrain.wgsl` holds groups 0 and 1.
const GROUP: u32 = 2;

/// `OitStage`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    DepthBounds,
    Transmittance,
    Accumulate,
}

/// What the device must support: max blending on 32-bit float targets.
///
/// `FLOAT32_BLENDABLE` is a Vulkan/WebGPU feature. wgpu's Metal adapter reports
/// it, but Metal's physical format features refuse blending on `Rgba32Float`.
/// Our device requests `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`, so
/// `create_render_pipeline` validates blend state against those physical
/// features and rejects OIT's depth-bounds pipeline at world load. Ask the
/// format, not just the feature bit.
pub fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
    features_for(
        adapter.features(),
        adapter.get_texture_format_features(BOUNDS_FORMAT).flags,
    )
}

/// The feature bit alone is not enough: the depth-bounds target must actually
/// be blendable on this adapter. Kept separate so the decision is testable
/// without a GPU.
fn features_for(
    adapter_features: wgpu::Features,
    bounds_flags: wgpu::TextureFormatFeatureFlags,
) -> wgpu::Features {
    if bounds_flags.contains(wgpu::TextureFormatFeatureFlags::BLENDABLE) {
        adapter_features & wgpu::Features::FLOAT32_BLENDABLE
    } else {
        wgpu::Features::empty()
    }
}

pub fn supported(device: &wgpu::Device) -> bool {
    device
        .features()
        .contains(wgpu::Features::FLOAT32_BLENDABLE)
}

impl super::Renderer {
    /// Improved Transparency is on and nothing keeps the world off its path (but a world warp,
    /// decided per frame).
    pub(super) fn improved_transparency_runs(&self) -> bool {
        self.improved_transparency
            && self.pack_post.as_ref().is_none_or(|p| p.1.is_none())
            && self.executor.is_none()
            && supported(&self.device)
            && self.oit.as_ref().is_none_or(|oit| !oit.no_room)
    }

    /// The last frame drew a translucent block's crack through the transparency path, so it takes
    /// `DESTROY_TYPES_OIT`'s sprites; otherwise the classic multiply draws it, with the classic
    /// ones. The frame's own path, so a world warp or HDR's layered output is never missed; a
    /// change shows a frame late.
    pub fn translucent_cracks_transparent(&self) -> bool {
        self.cracks_transparent
    }
}

/// What a stage's pipelines write: `OIT_*_SNIPPET`'s colour targets.
pub(super) fn stage_targets(stage: Stage) -> Vec<Option<wgpu::ColorTargetState>> {
    let blend = |operation| wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation,
    };
    let target = |format, operation| {
        Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState {
                color: blend(operation),
                alpha: blend(operation),
            }),
            write_mask: wgpu::ColorWrites::ALL,
        })
    };
    let add = wgpu::BlendOperation::Add;
    match stage {
        Stage::DepthBounds => vec![target(BOUNDS_FORMAT, wgpu::BlendOperation::Max)],
        Stage::Transmittance => vec![
            target(COEFFICIENT_FORMAT, add),
            target(COEFFICIENT_FORMAT, add),
        ],
        Stage::Accumulate => vec![target(COEFFICIENT_FORMAT, add)],
    }
}

/// `ProjMat[2][2]` and `ProjMat[3][2]`: a device depth `d` is `y / (d + x)`
/// blocks along the view axis (`oit_linear_depth`).
fn projection_terms(projection: &Mat4) -> [f32; 2] {
    [projection.z_axis.z, projection.w_axis.z]
}

struct Targets {
    size: [u32; 2],
    bounds: wgpu::Texture,
    culled: wgpu::Texture,
    views: Views,
    /// The uniform alone, with the culled bounds, with those and the
    /// coefficients, for the cull with the unculled bounds, and the composite.
    alone: wgpu::BindGroup,
    sampled: wgpu::BindGroup,
    accumulate: wgpu::BindGroup,
    cull: wgpu::BindGroup,
    composite: wgpu::BindGroup,
    /// The depth the terrain's stages test against while a water mask is
    /// drawn (a copy of the world's, plus the masks), and the clouds' own.
    terrain_depth: Option<Depth>,
    cloud_depth: Option<Depth>,
}

struct Depth {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Depth {
    fn new(device: &wgpu::Device, label: &str, size: [u32; 2]) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Self {
            view: texture.create_view(&Default::default()),
            texture,
        }
    }
}

fn extent(size: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size[0].max(1),
        height: size[1].max(1),
        depth_or_array_layers: 1,
    }
}

/// What the translucent layer's other drawers add to a frame: each draws with
/// a pipeline per stage where it has one.
#[derive(Default)]
pub struct Extras<'a> {
    /// `executeOitWaterMask`: the boats' water patches, depth only.
    pub water_mask: Option<&'a dyn Fn(&mut wgpu::RenderPass<'_>)>,
    /// `CloudRenderer.renderOit`: draws the clouds with the pipeline given.
    pub clouds: Option<&'a dyn Fn(&mut wgpu::RenderPass<'_>, &wgpu::RenderPipeline)>,
    /// "Features, World Border, Weather": draws what the entity, weather and
    /// border shaders put after the opaque world, with `Oit::features`'s
    /// pipelines of the stage.
    pub features: Option<&'a dyn Fn(&mut wgpu::RenderPass<'_>, Stage)>,
}

/// Counts of the translucent "Features, World Border, Weather" draws in a frame,
/// used to skip `Extras::features` completely when a frame emits none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct FeatureCounts {
    pub(super) text_quads: u32,
    pub(super) crumbling_quads: u32,
    pub(super) swirl_quads: u32,
    pub(super) shadow_quads: u32,
    pub(super) particle_quads: u32,
    pub(super) item_quads: u32,
    pub(super) entity_quads: u32,
    pub(super) additive_quads: u32,
    pub(super) weather_quads: u32,
    pub(super) lightning_bolts: bool,
    pub(super) world_border: bool,
}

impl FeatureCounts {
    /// Whether at least one OIT feature draw exists this frame.
    pub(super) fn any(self) -> bool {
        self.text_quads > 0
            || self.crumbling_quads > 0
            || self.swirl_quads > 0
            || self.shadow_quads > 0
            || self.particle_quads > 0
            || self.item_quads > 0
            || self.entity_quads > 0
            || self.additive_quads > 0
            || self.weather_quads > 0
            || self.lightning_bolts
            || self.world_border
    }
}

/// How `Extras::features` executes within each OIT stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FeaturesPass {
    /// Zero translucent feature draws this frame: omit feature recording.
    Skip,
    /// No active water mask (`terrain_depth` is `depth`): draw features in the
    /// same render pass as terrain, avoiding a full-screen store+load per stage.
    Coalesced,
    /// A water mask patched `terrain_depth`: features must run in a separate
    /// pass against the unmasked world `depth`.
    Separate,
}

fn features_pass(has_features: bool, has_water_mask: bool) -> FeaturesPass {
    if !has_features {
        FeaturesPass::Skip
    } else if has_water_mask {
        FeaturesPass::Separate
    } else {
        FeaturesPass::Coalesced
    }
}

struct Views {
    bounds: wgpu::TextureView,
    culled: wgpu::TextureView,
    coefficients: [wgpu::TextureView; 2],
    accumulated: wgpu::TextureView,
}

pub struct Oit {
    uniform: wgpu::Buffer,
    /// Layouts of the groups at `GROUP`: the uniform; with the bounds; with the
    /// coefficients; and with the accumulation too.
    layouts: [wgpu::BindGroupLayout; 4],
    /// One per `Stage`.
    terrain: [wgpu::RenderPipeline; 3],
    /// The terrain's flat section draws (`Flat`), where the device has them.
    flat: Option<[wgpu::RenderPipeline; 3]>,
    features: Features,
    clouds: [wgpu::RenderPipeline; 3],
    cull: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    targets: Option<Targets>,
    /// Its targets had no room (#13624): it is off (`improved_transparency_runs`) until the
    /// window's size changes (`Renderer::resize`).
    pub(super) no_room: bool,
}

impl Oit {
    /// `terrain`: the terrain bind group layout that `vs_terrain` reads,
    /// `flat` the flat draws' second group, `features` the entity, weather
    /// and world border layouts,
    /// `clouds` the cloud pipeline's two, `sprites` the terrain's override
    /// constants, `format` the world target's.
    pub fn new(
        device: &wgpu::Device,
        terrain: &wgpu::BindGroupLayout,
        flat: Option<&wgpu::BindGroupLayout>,
        features: [&wgpu::BindGroupLayout; 3],
        clouds: [&wgpu::BindGroupLayout; 2],
        sprites: [f64; 2],
        format: wgpu::TextureFormat,
    ) -> Self {
        let instance = wgpu::vertex_attr_array![5=>Float32x4];
        let buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<super::TerrainVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &super::TERRAIN_ATTRIBUTES,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 16,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance,
            }),
        ];
        let buffers = &buffers[..];
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon improved transparency"),
            source: wgpu::ShaderSource::Wgsl(
                (super::terrain_shader()
                    + include_str!("../oit.wgsl")
                    + include_str!("../oit_world.wgsl"))
                .into(),
            ),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Improved transparency"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entry = |binding: u32, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let texture = |binding| {
            entry(
                binding,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            )
        };
        let params = entry(
            0,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        );
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let layouts = [
            layout("Improved transparency", &[params]),
            layout("Improved transparency bounds", &[params, texture(1)]),
            layout(
                "Improved transparency coefficients",
                &[params, texture(1), texture(2), texture(3)],
            ),
            layout(
                "Improved transparency composite",
                &[params, texture(1), texture(2), texture(3), texture(4)],
            ),
        ];
        let pipeline_layout = |first: Option<&wgpu::BindGroupLayout>, group: usize| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Improved transparency"),
                bind_group_layouts: &[first, None, Some(&layouts[group])],
                ..Default::default()
            })
        };
        let constants = [
            ("SPRITE_ARRAYS", sprites[0]),
            ("SPRITE_SPLIT", sprites[1]),
            (
                "HDR_WRITE_LIGHT",
                f64::from(u8::from(format == super::LINEAR_FRAME)),
            ),
        ];
        let add = wgpu::BlendOperation::Add;
        let depth = |write| {
            Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(write),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            })
        };
        let pipeline = |label,
                        layout: &wgpu::PipelineLayout,
                        vertex: &str,
                        fragment: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        targets: &[Option<wgpu::ColorTargetState>],
                        depth_stencil| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &constants,
                        ..Default::default()
                    },
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: vertex.starts_with("vs_terrain").then_some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &constants,
                        ..Default::default()
                    },
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let terrain_pipeline = |label, fragment, stage| {
            pipeline(
                label,
                &pipeline_layout(Some(terrain), stage as usize),
                "vs_terrain",
                fragment,
                buffers,
                &stage_targets(stage),
                depth(false),
            )
        };
        let terrain_pipelines = [
            terrain_pipeline("OIT depth bounds", "fs_oit_bounds", Stage::DepthBounds),
            terrain_pipeline(
                "OIT transmittance",
                "fs_oit_transmittance",
                Stage::Transmittance,
            ),
            terrain_pipeline("OIT accumulate", "fs_oit_accumulate", Stage::Accumulate),
        ];
        // The same fragments over `vs_terrain_flat`: one vertex buffer, and
        // the draw table at group 1.
        let flat_pipelines = flat.map(|draw| {
            let flat_buffers = &buffers[..1];
            [
                (
                    "OIT depth bounds (flat)",
                    "fs_oit_bounds",
                    Stage::DepthBounds,
                ),
                (
                    "OIT transmittance (flat)",
                    "fs_oit_transmittance",
                    Stage::Transmittance,
                ),
                (
                    "OIT accumulate (flat)",
                    "fs_oit_accumulate",
                    Stage::Accumulate,
                ),
            ]
            .map(|(label, fragment, stage)| {
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Improved transparency flat terrain"),
                    bind_group_layouts: &[
                        Some(terrain),
                        Some(draw),
                        Some(&layouts[stage as usize]),
                    ],
                    ..Default::default()
                });
                pipeline(
                    label,
                    &layout,
                    "vs_terrain_flat",
                    fragment,
                    flat_buffers,
                    &stage_targets(stage),
                    depth(false),
                )
            })
        });
        let screen = |label, fragment, group, targets: &[_], depth_stencil| {
            pipeline(
                label,
                &pipeline_layout(None, group),
                "vs_sky",
                fragment,
                &[],
                targets,
                depth_stencil,
            )
        };
        let replace = |format| {
            Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        let cull = screen(
            "OIT cull depth bounds",
            "fs_oit_cull",
            1,
            &[replace(BOUNDS_FORMAT)],
            depth(true),
        );
        // `RenderPipelines.OIT_COMPOSITE`: premultiplied alpha over the world,
        // the nearest see-through depth written.
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: add,
        };
        let composite = screen(
            "OIT composite",
            "fs_oit_composite",
            3,
            &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState {
                    color: premultiplied,
                    alpha: premultiplied,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
        );
        // The clouds: entity.wgsl's vertex shader and vertex format, with the
        // stages' fragments.
        let cloud_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon improved transparency clouds"),
            source: wgpu::ShaderSource::Wgsl(
                (super::entity_shader()
                    + include_str!("../oit.wgsl")
                    + include_str!("../oit_clouds.wgsl"))
                .into(),
            ),
        });
        let cloud_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
        let cloud_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<super::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &cloud_attributes,
        })];
        let cloud_pipeline = |label, fragment, stage: Stage| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(
                    &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("Improved transparency clouds"),
                        bind_group_layouts: &[
                            Some(clouds[0]),
                            Some(clouds[1]),
                            Some(&layouts[stage as usize]),
                        ],
                        ..Default::default()
                    }),
                ),
                vertex: wgpu::VertexState {
                    module: &cloud_shader,
                    entry_point: Some("vs_cloud"),
                    compilation_options: Default::default(),
                    buffers: &cloud_buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                // `OIT_CLOUDS`: only the bounds stage writes the cloud depth,
                // so the later stages keep the nearest face of each cloud.
                depth_stencil: depth(stage == Stage::DepthBounds),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &cloud_shader,
                    entry_point: Some(fragment),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &constants[2..],
                        ..Default::default()
                    },
                    targets: &stage_targets(stage),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let cloud_pipelines = [
            cloud_pipeline(
                "OIT cloud depth bounds",
                "fs_oit_cloud_bounds",
                Stage::DepthBounds,
            ),
            cloud_pipeline(
                "OIT cloud transmittance",
                "fs_oit_cloud_transmittance",
                Stage::Transmittance,
            ),
            cloud_pipeline(
                "OIT cloud accumulate",
                "fs_oit_cloud_accumulate",
                Stage::Accumulate,
            ),
        ];
        let features = Features::new(
            device,
            &layouts,
            features[0],
            features[1],
            features[2],
            format,
        );
        Self {
            uniform,
            layouts,
            terrain: terrain_pipelines,
            flat: flat_pipelines,
            features,
            clouds: cloud_pipelines,
            cull,
            composite,
            targets: None,
            no_room: false,
        }
    }

    /// The pipeline the terrain's translucent layer draws with in `stage`.
    pub fn terrain(&self, stage: Stage) -> &wgpu::RenderPipeline {
        &self.terrain[stage as usize]
    }

    /// The pipeline the terrain's translucent layer draws with in `stage`
    /// as flat section draws, where they run.
    pub fn flat(&self, stage: Stage) -> Option<&wgpu::RenderPipeline> {
        self.flat.as_ref().map(|f| &f[stage as usize])
    }

    pub fn features(&self) -> &Features {
        &self.features
    }

    /// Sizes the targets to the world's `size`, before the frame chooses how it draws translucent
    /// things: `false` where there was no room for them (#13624), and the frame draws them the
    /// ordinary way. Nothing that failed is kept, the old targets go first, and it stays off
    /// (`no_room`) until the window's size changes.
    pub fn fit(&mut self, device: &wgpu::Device, size: [u32; 2]) -> bool {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return true;
        }
        self.targets = None;
        self.targets =
            super::frame_targets::made(device, super::frame_targets::Kind::Transparency, || {
                self.targets_at(device, size)
            });
        self.no_room = self.targets.is_none();
        !self.no_room
    }

    /// Lets the targets go while it is off for want of room (`no_room`).
    pub fn let_go(&mut self) {
        self.targets = None;
    }

    /// Writes this frame's projection, and makes the depths the water mask and the clouds
    /// `wants`. Without the room for one, this frame's `run` leaves that part out and it is off
    /// from the next frame (`no_room`).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        projection: &Mat4,
        wants: [bool; 2],
    ) {
        let [p0, p1] = projection_terms(projection);
        let terms = [p0, p1, 0.0, 0.0];
        uploads.buffer(encoder, &self.uniform, 0, bytemuck::cast_slice(&terms));
        let [mask, clouds] = wants;
        let Some(t) = self.targets.as_mut() else {
            return;
        };
        let size = t.size;
        let depth = |label| {
            super::frame_targets::made(device, super::frame_targets::Kind::Transparency, || {
                Depth::new(device, label, size)
            })
        };
        if mask && t.terrain_depth.is_none() {
            t.terrain_depth = depth("OIT terrain depth");
        }
        if clouds && t.cloud_depth.is_none() {
            t.cloud_depth = depth("OIT cloud depth");
        }
        if (mask && t.terrain_depth.is_none()) || (clouds && t.cloud_depth.is_none()) {
            self.no_room = true;
        }
    }

    fn targets_at(&self, device: &wgpu::Device, size: [u32; 2]) -> Targets {
        let make = |label, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let attachment =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let bounds = make(
            "OIT depth bounds",
            BOUNDS_FORMAT,
            attachment | wgpu::TextureUsages::COPY_SRC,
        );
        let culled = make(
            "OIT culled depth bounds",
            BOUNDS_FORMAT,
            attachment | wgpu::TextureUsages::COPY_DST,
        );
        let coefficients =
            [0, 1].map(|_| make("OIT transmittance", COEFFICIENT_FORMAT, attachment));
        let accumulated = make("OIT accumulate", COEFFICIENT_FORMAT, attachment);
        let views = Views {
            bounds: bounds.create_view(&Default::default()),
            culled: culled.create_view(&Default::default()),
            coefficients: coefficients.map(|t| t.create_view(&Default::default())),
            accumulated: accumulated.create_view(&Default::default()),
        };
        let group = |layout: &wgpu::BindGroupLayout, views: &[&wgpu::TextureView]| {
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniform.as_entire_binding(),
            }];
            entries.extend(views.iter().enumerate().map(|(i, v)| wgpu::BindGroupEntry {
                binding: i as u32 + 1,
                resource: wgpu::BindingResource::TextureView(v),
            }));
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Improved transparency"),
                layout,
                entries: &entries,
            })
        };
        let [c0, c1] = &views.coefficients;
        Targets {
            size,
            alone: group(&self.layouts[0], &[]),
            sampled: group(&self.layouts[1], &[&views.culled]),
            accumulate: group(&self.layouts[2], &[&views.culled, c0, c1]),
            cull: group(&self.layouts[1], &[&views.bounds]),
            composite: group(
                &self.layouts[3],
                &[&views.bounds, c0, c1, &views.accumulated],
            ),
            terrain_depth: None,
            cloud_depth: None,
            bounds,
            culled,
            views,
        }
    }

    /// `LevelRenderer.executeOit`: `draw` records the terrain's translucent
    /// layer into each stage's pass with that stage's pipeline, `extras` the
    /// water masks and clouds, then the composite goes over `color`. `depth`
    /// is the opaque world's: the stages only read it; the cull and the
    /// composite write it.
    pub fn run(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        draw: &dyn Fn(&mut wgpu::RenderPass<'_>, Stage),
        extras: &Extras<'_>,
        mut timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        let Some(t) = &self.targets else { return };
        let attachment = |view, load| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let clear = wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT);
        let load = wgpu::LoadOp::Load;
        let read_only = |view| {
            Some(wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: None,
                stencil_ops: None,
            })
        };
        let water_mask = extras.water_mask.zip(t.terrain_depth.as_ref());
        let cloud_depth = extras.clouds.zip(t.cloud_depth.as_ref());
        if let Some((mask, _)) = water_mask {
            self.execute_water_mask(encoder, t, color, depth, mask, timing.as_deref_mut());
        }
        let terrain_depth = water_mask.map_or(depth, |(_, d)| &d.view);
        let features_mode = features_pass(extras.features.is_some(), water_mask.is_some());
        let [c0, c1] = &t.views.coefficients;
        let culled = &t.views.culled;
        let bounds_first = [attachment(
            &t.views.bounds,
            wgpu::LoadOp::Clear(BOUNDS_CLEAR),
        )];
        let bounds_features = [attachment(&t.views.bounds, load)];
        // The clouds' bounds join the culled ones.
        let bounds_clouds = [attachment(culled, load)];
        let transmittance_first = [attachment(c0, clear), attachment(c1, clear)];
        let transmittance_load = [attachment(c0, load), attachment(c1, load)];
        let accumulate_first = [attachment(&t.views.accumulated, clear)];
        let accumulate_load = [attachment(&t.views.accumulated, load)];
        for (stage, label, group, first, features_targets, cloud_targets) in [
            (
                Stage::DepthBounds,
                "OIT depth bounds",
                &t.alone,
                bounds_first.as_slice(),
                bounds_features.as_slice(),
                bounds_clouds.as_slice(),
            ),
            (
                Stage::Transmittance,
                "OIT transmittance",
                &t.sampled,
                transmittance_first.as_slice(),
                transmittance_load.as_slice(),
                transmittance_load.as_slice(),
            ),
            (
                Stage::Accumulate,
                "OIT accumulate",
                &t.accumulate,
                accumulate_first.as_slice(),
                accumulate_load.as_slice(),
                accumulate_load.as_slice(),
            ),
        ] {
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    timestamp_writes: timing.as_deref_mut().and_then(|t| t.writes(label)),
                    color_attachments: first,
                    depth_stencil_attachment: read_only(terrain_depth),
                    ..Default::default()
                });
                let pixels = super::pixel_count(terrain_depth);
                let mut pass =
                    super::pack_timing::counted(timing.as_deref_mut(), pass, pixels, label);
                if features_mode == FeaturesPass::Coalesced {
                    pass.stage(timing.as_deref_mut(), "OIT terrain");
                }
                pass.set_bind_group(GROUP, group, &[]);
                draw(&mut pass, stage);
                if features_mode == FeaturesPass::Coalesced
                    && let Some(features) = extras.features
                {
                    pass.split(timing.as_deref_mut(), "OIT features");
                    pass.stage(timing.as_deref_mut(), "OIT features");
                    pass.set_bind_group(GROUP, group, &[]);
                    features(&mut pass, stage);
                }
            }
            if features_mode == FeaturesPass::Separate
                && let Some(features) = extras.features
            {
                // Over the world's own depth, even where a water mask patched
                // the terrain's: the masks only hide water.
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    timestamp_writes: timing.as_deref_mut().and_then(|t| t.writes("OIT features")),
                    color_attachments: features_targets,
                    depth_stencil_attachment: read_only(depth),
                    ..Default::default()
                });
                pass.set_bind_group(GROUP, group, &[]);
                features(&mut pass, stage);
            }
            if stage == Stage::DepthBounds {
                self.execute_depth_bounds_cull(encoder, t, depth, timing.as_deref_mut());
            }
            if let Some((clouds, cloud_depth)) = cloud_depth {
                self.render_clouds(
                    encoder,
                    t,
                    stage,
                    depth,
                    (label, group, cloud_targets),
                    (clouds, cloud_depth),
                    timing.as_deref_mut(),
                );
            }
            if stage == Stage::DepthBounds
                && let Some((mask, _)) = water_mask
            {
                self.execute_water_mask(encoder, t, color, depth, mask, timing.as_deref_mut());
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OIT composite"),
            timestamp_writes: timing.and_then(|t| t.writes("OIT composite")),
            color_attachments: &[attachment(color, wgpu::LoadOp::Load)],
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
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(GROUP, &t.composite, &[]);
        pass.draw(0..3, 0..1);
    }

    /// `CloudRenderer.renderOit` for one stage. The bounds stage first copies
    /// the world's depth (after the cull) and draws into it, so the later
    /// stages, which only read it, keep each pixel's nearest cloud face.
    #[allow(clippy::too_many_arguments)]
    fn render_clouds(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        stage: Stage,
        depth: &wgpu::TextureView,
        (label, group, targets): (
            &str,
            &wgpu::BindGroup,
            &[Option<wgpu::RenderPassColorAttachment<'_>>],
        ),
        (draw, cloud_depth): (
            &dyn Fn(&mut wgpu::RenderPass<'_>, &wgpu::RenderPipeline),
            &Depth,
        ),
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        if stage == Stage::DepthBounds {
            encoder.copy_texture_to_texture(
                depth.texture().as_image_copy(),
                cloud_depth.texture.as_image_copy(),
                extent(t.size),
            );
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            timestamp_writes: timing.and_then(|t| t.writes("OIT clouds")),
            color_attachments: targets,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &cloud_depth.view,
                depth_ops: (stage == Stage::DepthBounds).then_some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(GROUP, group, &[]);
        draw(&mut pass, &self.clouds[stage as usize]);
    }

    /// `LevelRenderer.executeOitWaterMask`: the terrain's depth is the world's
    /// again, with the boats' water patches on top.
    fn execute_water_mask(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        mask: &dyn Fn(&mut wgpu::RenderPass<'_>),
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        let Some(terrain) = &t.terrain_depth else {
            return;
        };
        encoder.copy_texture_to_texture(
            depth.texture().as_image_copy(),
            terrain.texture.as_image_copy(),
            extent(t.size),
        );
        // The mask pipeline writes depth only but has a colour target.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OIT water mask"),
            timestamp_writes: timing.and_then(|t| t.writes("OIT water mask")),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &terrain.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        mask(&mut pass);
    }

    /// `LevelRenderer.executeDepthBoundsCull`: the bounds are copied, then where
    /// the nearest fully opaque see-through fragment is in front of the world,
    /// the far bound stops there and its depth replaces the world's.
    fn execute_depth_bounds_cull(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        depth: &wgpu::TextureView,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        encoder.copy_texture_to_texture(
            t.bounds.as_image_copy(),
            t.culled.as_image_copy(),
            wgpu::Extent3d {
                width: t.size[0].max(1),
                height: t.size[1].max(1),
                depth_or_array_layers: 1,
            },
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OIT cull depth bounds"),
            timestamp_writes: timing.and_then(|t| t.writes("OIT cull depth bounds")),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &t.views.culled,
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
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.cull);
        pass.set_bind_group(GROUP, &t.cull, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Metal reports `FLOAT32_BLENDABLE` but its physical `Rgba32Float`
    /// features are not `BLENDABLE`. Requesting the feature there made OIT
    /// build an invalid depth-bounds pipeline at world load
    /// (`crash-2026-11-12_22.06.31-client.txt`).
    #[test]
    fn a_reported_feature_does_not_enable_oit_without_blendable_bounds() {
        let reported = wgpu::Features::FLOAT32_BLENDABLE;
        assert_eq!(
            features_for(reported, wgpu::TextureFormatFeatureFlags::empty()),
            wgpu::Features::empty()
        );
        assert_eq!(
            features_for(reported, wgpu::TextureFormatFeatureFlags::BLENDABLE),
            wgpu::Features::FLOAT32_BLENDABLE
        );
    }

    #[test]
    fn blendable_bounds_without_the_feature_request_nothing() {
        assert_eq!(
            features_for(
                wgpu::Features::empty(),
                wgpu::TextureFormatFeatureFlags::BLENDABLE
            ),
            wgpu::Features::empty()
        );
    }

    fn validate(source: &str, entries: &[&str]) {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        for entry in entries {
            assert!(
                module.entry_points.iter().any(|e| e.name == *entry),
                "{entry}"
            );
        }
    }

    #[test]
    fn the_terrain_shader_validates() {
        validate(
            &(crate::gfx::terrain_shader()
                + include_str!("../oit.wgsl")
                + include_str!("../oit_world.wgsl")),
            &[
                "fs_oit_bounds",
                "fs_oit_transmittance",
                "fs_oit_accumulate",
                "fs_oit_cull",
                "fs_oit_composite",
            ],
        );
    }

    #[test]
    fn the_cloud_shader_validates() {
        validate(
            &(super::super::entity_shader()
                + include_str!("../oit.wgsl")
                + include_str!("../oit_clouds.wgsl")),
            &[
                "fs_oit_cloud_bounds",
                "fs_oit_cloud_transmittance",
                "fs_oit_cloud_accumulate",
            ],
        );
    }

    #[test]
    fn the_feature_shaders_validate() {
        let stages = |name: &str| {
            ["bounds", "transmittance", "accumulate"].map(|s| format!("fs_oit_{name}_{s}"))
        };
        let entity = ["entity", "shadow", "weather", "swirl"]
            .map(stages)
            .concat();
        validate(
            &(super::super::entity_shader()
                + include_str!("../oit.wgsl")
                + include_str!("../oit_features.wgsl")),
            &entity.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let border = stages("border");
        validate(
            &(super::super::border_shader()
                + include_str!("../oit.wgsl")
                + include_str!("../oit_border.wgsl")),
            &border.iter().map(String::as_str).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn a_device_depth_gives_the_distance_along_the_view_axis() {
        // The world's reversed-Z projection: near plane at depth 1.
        let projection = super::super::world_projection(70.0, 16.0 / 9.0, 512.0);
        let [x, y] = projection_terms(&projection);
        for distance in [0.1_f32, 1.0, 7.5, 64.0, 500.0] {
            let clip = projection * glam::Vec4::new(0.0, 0.0, -distance, 1.0);
            let device = clip.z / clip.w;
            let linear = y / (device + x);
            assert!(
                (linear - distance).abs() < distance * 2e-3,
                "{distance}: {linear}"
            );
        }
    }

    #[test]
    fn features_pass_coalesces_without_water_mask_and_splits_with_one() {
        assert_eq!(features_pass(false, false), FeaturesPass::Skip);
        assert_eq!(features_pass(false, true), FeaturesPass::Skip);
        assert_eq!(features_pass(true, false), FeaturesPass::Coalesced);
        assert_eq!(features_pass(true, true), FeaturesPass::Separate);
    }

    #[test]
    fn empty_feature_counts_skip_and_each_source_enables_features() {
        assert!(!FeatureCounts::default().any());
        let cases = [
            FeatureCounts {
                text_quads: 1,
                ..Default::default()
            },
            FeatureCounts {
                crumbling_quads: 1,
                ..Default::default()
            },
            FeatureCounts {
                swirl_quads: 1,
                ..Default::default()
            },
            FeatureCounts {
                shadow_quads: 1,
                ..Default::default()
            },
            FeatureCounts {
                particle_quads: 1,
                ..Default::default()
            },
            FeatureCounts {
                item_quads: 1,
                ..Default::default()
            },
            FeatureCounts {
                entity_quads: 1,
                ..Default::default()
            },
            FeatureCounts {
                additive_quads: 1,
                ..Default::default()
            },
            FeatureCounts {
                weather_quads: 1,
                ..Default::default()
            },
            FeatureCounts {
                lightning_bolts: true,
                ..Default::default()
            },
            FeatureCounts {
                world_border: true,
                ..Default::default()
            },
        ];
        for counts in cases {
            assert!(counts.any(), "{counts:?}");
        }
    }

    #[test]
    fn oit_shader_unrolls_haar_wavelets_without_exp2_or_local_arrays() {
        fn has_loop(block: &naga::Block) -> bool {
            block.iter().any(|stmt| match stmt {
                naga::Statement::Loop { .. } => true,
                naga::Statement::Block(inner) => has_loop(inner),
                naga::Statement::If { accept, reject, .. } => has_loop(accept) || has_loop(reject),
                _ => false,
            })
        }

        let wgsl = include_str!("../oit.wgsl");
        let height_line = wgsl
            .lines()
            .find(|line| line.starts_with("const OIT_WAVELET_HEIGHT_1: f32 = "))
            .expect("OIT_WAVELET_HEIGHT_1 constant");
        let parsed: f32 = height_line
            .trim_start_matches("const OIT_WAVELET_HEIGHT_1: f32 = ")
            .trim_end_matches(';')
            .parse()
            .expect("valid f32 literal");
        assert_eq!(parsed.to_bits(), std::f32::consts::SQRT_2.to_bits());
        assert_eq!(parsed.to_bits(), 0.5_f32.exp2().to_bits());

        let source = crate::gfx::terrain_shader() + wgsl + include_str!("../oit_world.wgsl");
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        for name in ["oit_add_transmittance", "oit_sample_absorbance"] {
            let (_, function) = module
                .functions
                .iter()
                .find(|(_, f)| f.name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("missing WGSL function {name}"));
            assert!(
                !function.expressions.iter().any(|(_, expr)| matches!(
                    expr,
                    naga::Expression::Math {
                        fun: naga::MathFunction::Exp2,
                        ..
                    }
                )),
                "{name} must not emit runtime exp2"
            );
            assert!(
                !function.local_variables.iter().any(|(_, local)| matches!(
                    module.types[local.ty].inner,
                    naga::TypeInner::Array { .. }
                )),
                "{name} must not spill a local array"
            );
            assert!(!has_loop(&function.body), "{name} must be unrolled");
        }
    }

    #[test]
    fn unrolled_haar_wavelet_matches_looped_reference_bit_for_bit() {
        const BINS: i32 = 8;

        fn ref_add(absorbance: f32, original: f32) -> [f32; 8] {
            let mut c = [0.0_f32; 8];
            let depth = original * ((BINS - 1) as f32) / (BINS as f32);
            c[0] = absorbance * (1.0 - depth);
            let mut tree = (original * ((BINS - 1) as f32)).floor() as i32 + BINS;
            for level in 0..=2 {
                let power = 2 - level;
                let parent = tree >> 1;
                let index_at_parent = (parent & ((1 << power) - 1)) as f32;
                let right = tree & 1;
                let side = 1 - (right << 1);
                let width = (-(power as f32)).exp2();
                let height = ((power as f32) * 0.5).exp2();
                let offset = depth - width * index_at_parent;
                c[parent as usize] =
                    ((right as f32) * width + (side as f32) * offset) * height * absorbance;
                tree = parent;
            }
            c
        }

        fn unrolled_add(absorbance: f32, original: f32) -> [f32; 8] {
            let in_bins = original * ((BINS - 1) as f32);
            let depth = in_bins / (BINS as f32);
            let c0 = absorbance * (1.0 - depth);
            let bin = (in_bins.floor() as i32).clamp(0, BINS - 1);

            let p0 = bin >> 1;
            let r0 = (bin & 1) != 0;
            let offset0 = depth - 0.25 * (p0 as f32);
            let v0 = if r0 { 0.25 - offset0 } else { offset0 } * 2.0 * absorbance;

            let p1 = bin >> 2;
            let r1 = (p0 & 1) != 0;
            let offset1 = depth - if p1 != 0 { 0.5 } else { 0.0 };
            let v1 =
                if r1 { 0.5 - offset1 } else { offset1 } * std::f32::consts::SQRT_2 * absorbance;

            let r2 = p1 != 0;
            let v2 = if r2 { 1.0 - depth } else { depth } * absorbance;

            [
                c0,
                v2,
                if r2 { 0.0 } else { v1 },
                if r2 { v1 } else { 0.0 },
                if p0 == 0 { v0 } else { 0.0 },
                if p0 == 1 { v0 } else { 0.0 },
                if p0 == 2 { v0 } else { 0.0 },
                if p0 == 3 { v0 } else { 0.0 },
            ]
        }

        fn lane(v: [f32; 4], index: i32) -> f32 {
            let lo = if (index & 1) != 0 { v[1] } else { v[0] };
            let hi = if (index & 1) != 0 { v[3] } else { v[2] };
            if (index & 2) != 0 { hi } else { lo }
        }

        fn wgsl_fract(x: f32) -> f32 {
            x - x.floor()
        }

        fn wgsl_mix(a: f32, b: f32, t: f32) -> f32 {
            a * (1.0 - t) + b * t
        }

        fn ref_sample(c: [f32; 8], original: f32, current: f32) -> f32 {
            let mut average = c[0];
            if average == 0.0 {
                return 0.0;
            }
            let in_bins = original * ((BINS - 1) as f32);
            let depth = in_bins / (BINS as f32);
            average -= current * (1.0 - depth);
            let mut tree_b = (in_bins.floor() as i32).clamp(0, BINS - 1);
            let sample_a_too = tree_b >= 1;
            let mut tree_a = if sample_a_too { tree_b - 1 } else { tree_b };
            tree_b += BINS;
            tree_a += BINS;
            let mut sample_b = average;
            let mut sample_a = if sample_a_too { average } else { 0.0 };
            for level in 0..=2 {
                let power = 2 - level;
                let width = (-(power as f32)).exp2();
                let height = ((power as f32) * 0.5).exp2();
                let parent_b = tree_b >> 1;
                let right_b = tree_b & 1;
                let side_b = 1 - (right_b << 1);
                let index_at_parent = (parent_b & ((1 << power) - 1)) as f32;
                let offset = depth - width * index_at_parent;
                let own = ((right_b as f32) * width + (side_b as f32) * offset) * height * current;
                let difference_b = c[parent_b as usize] - own;
                sample_b -= height * difference_b * (side_b as f32);
                tree_b = parent_b;
                if sample_a_too {
                    let parent_a = tree_a >> 1;
                    let difference_a = if parent_a == parent_b {
                        difference_b
                    } else {
                        c[parent_a as usize]
                    };
                    let side_a = 1 - ((tree_a & 1) << 1);
                    sample_a -= height * difference_a * (side_a as f32);
                    tree_a = parent_a;
                }
            }
            let blend = if in_bins >= BINS as f32 {
                1.0
            } else {
                wgsl_fract(in_bins)
            };
            wgsl_mix(sample_a, sample_b, blend)
        }

        fn unrolled_sample(c0: [f32; 4], c1: [f32; 4], original: f32, current: f32) -> f32 {
            let mut average = c0[0];
            if average == 0.0 {
                return 0.0;
            }
            let in_bins = original * ((BINS - 1) as f32);
            let depth = in_bins / (BINS as f32);
            average -= current * (1.0 - depth);
            let bin_b = (in_bins.floor() as i32).clamp(0, BINS - 1);
            let sample_a_too = bin_b >= 1;
            let bin_a = if sample_a_too { bin_b - 1 } else { bin_b };
            let mut sample_b = average;
            let mut sample_a = if sample_a_too { average } else { 0.0 };

            let p0_b = bin_b >> 1;
            let r0_b = (bin_b & 1) != 0;
            let offset0 = depth - 0.25 * (p0_b as f32);
            let own0 = if r0_b { 0.25 - offset0 } else { offset0 } * 2.0 * current;
            let diff0_b = lane(c1, p0_b) - own0;
            let step0_b = 2.0 * diff0_b;
            sample_b -= if r0_b { -step0_b } else { step0_b };
            if sample_a_too {
                let p0_a = bin_a >> 1;
                let diff0_a = if p0_a == p0_b {
                    diff0_b
                } else {
                    lane(c1, p0_a)
                };
                let step0_a = 2.0 * diff0_a;
                sample_a -= if (bin_a & 1) != 0 { -step0_a } else { step0_a };
            }

            let p1_b = bin_b >> 2;
            let r1_b = (p0_b & 1) != 0;
            let offset1 = depth - if p1_b != 0 { 0.5 } else { 0.0 };
            let own1 =
                if r1_b { 0.5 - offset1 } else { offset1 } * std::f32::consts::SQRT_2 * current;
            let diff1_b = if p1_b != 0 { c0[3] } else { c0[2] } - own1;
            let step1_b = std::f32::consts::SQRT_2 * diff1_b;
            sample_b -= if r1_b { -step1_b } else { step1_b };
            if sample_a_too {
                let p1_a = bin_a >> 2;
                let diff1_a = if p1_a == p1_b {
                    diff1_b
                } else if p1_a != 0 {
                    c0[3]
                } else {
                    c0[2]
                };
                let step1_a = std::f32::consts::SQRT_2 * diff1_a;
                sample_a -= if ((bin_a >> 1) & 1) != 0 {
                    -step1_a
                } else {
                    step1_a
                };
            }

            let r2_b = p1_b != 0;
            let own2 = if r2_b { 1.0 - depth } else { depth } * current;
            let diff2_b = c0[1] - own2;
            sample_b -= if r2_b { -diff2_b } else { diff2_b };
            if sample_a_too {
                sample_a -= if (bin_a >> 2) != 0 { -diff2_b } else { diff2_b };
            }

            let blend = if in_bins >= BINS as f32 {
                1.0
            } else {
                wgsl_fract(in_bins)
            };
            wgsl_mix(sample_a, sample_b, blend)
        }

        let mut originals = vec![0.0_f32, 1e-6, 1.0 - 1e-6, 1.0];
        for k in 1..7 {
            let edge = k as f32 / 7.0;
            originals.extend([edge - 1e-6, edge, edge + 1e-6, (k as f32 - 0.5) / 7.0]);
        }
        let absorbances = [0.0_f32, 1e-4, 0.35, 1.25, 4.0];
        for &orig in &originals {
            for abs in absorbances {
                let expected = ref_add(abs, orig);
                let actual = unrolled_add(abs, orig);
                assert_eq!(
                    expected.map(f32::to_bits),
                    actual.map(f32::to_bits),
                    "add mismatch at orig={orig} abs={abs}"
                );
            }
        }

        // Zero average early-out matches bit-for-bit.
        assert_eq!(
            ref_sample([0.0; 8], 0.5, 1.0).to_bits(),
            unrolled_sample([0.0; 4], [0.0; 4], 0.5, 1.0).to_bits()
        );

        // Multi-fragment sums across all bins plus composite far depth (100000.0).
        let layers: &[&[(f32, f32)]] = &[
            &[(0.8, 0.0)],
            &[(0.4, 0.05), (1.1, 0.42), (0.7, 0.95)],
            &[
                (0.2, 0.14),
                (0.6, 0.29),
                (1.3, 0.57),
                (0.9, 0.72),
                (2.4, 1.0),
            ],
            &[(4.0, 0.5), (0.01, 0.5)],
        ];
        let mut query_depths = originals.clone();
        query_depths.extend([-0.5_f32, 8.0 / 7.0, 100_000.0]);
        for fragments in layers {
            let mut sum = [0.0_f32; 8];
            for &(abs, orig) in *fragments {
                let contrib = ref_add(abs, orig);
                for (dst, src) in sum.iter_mut().zip(contrib) {
                    *dst += src;
                }
            }
            let c0 = [sum[0], sum[1], sum[2], sum[3]];
            let c1 = [sum[4], sum[5], sum[6], sum[7]];
            for &orig in &query_depths {
                for current in [0.0_f32, fragments[0].0, 1.75] {
                    let expected = ref_sample(sum, orig, current);
                    let actual = unrolled_sample(c0, c1, orig, current);
                    assert_eq!(
                        expected.to_bits(),
                        actual.to_bits(),
                        "sample mismatch at orig={orig} current={current}"
                    );
                }
            }
        }
    }
}
