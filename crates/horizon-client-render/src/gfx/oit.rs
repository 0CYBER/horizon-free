//! Improved transparency: vanilla 26.3's order-independent transparency
//! (`LevelRenderer.executeOit`). The translucent layer is drawn three times
//! against the opaque world's depth, without writing it, into targets the
//! next stage reads (`oit.wgsl` says what each holds), then a full-screen pass
//! composites the sum. With the option off none of this exists and the world
//! draws as before.

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
pub fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
    adapter.features() & wgpu::Features::FLOAT32_BLENDABLE
}

pub fn supported(device: &wgpu::Device) -> bool {
    device
        .features()
        .contains(wgpu::Features::FLOAT32_BLENDABLE)
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
    cull: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    targets: Option<Targets>,
}

impl Oit {
    /// `terrain`: the terrain bind group layout that `vs_terrain` reads,
    /// `sprites` its override constants, `format` the world target's.
    pub fn new(
        device: &wgpu::Device,
        terrain: &wgpu::BindGroupLayout,
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
                (include_str!("../terrain.wgsl").to_owned() + include_str!("../oit.wgsl")).into(),
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
        let constants = [("SPRITE_ARRAYS", sprites[0]), ("SPRITE_SPLIT", sprites[1])];
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
                    cull_mode: (vertex == "vs_terrain").then_some(wgpu::Face::Back),
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
        let layout_for = |group| pipeline_layout(Some(terrain), group);
        let terrain_pipeline = |label, fragment, group, targets: &[_]| {
            pipeline(
                label,
                &layout_for(group),
                "vs_terrain",
                fragment,
                buffers,
                targets,
                depth(false),
            )
        };
        let terrain_pipelines = [
            terrain_pipeline(
                "OIT depth bounds",
                "fs_oit_bounds",
                0,
                &[target(BOUNDS_FORMAT, wgpu::BlendOperation::Max)],
            ),
            terrain_pipeline(
                "OIT transmittance",
                "fs_oit_transmittance",
                1,
                &[
                    target(COEFFICIENT_FORMAT, add),
                    target(COEFFICIENT_FORMAT, add),
                ],
            ),
            terrain_pipeline(
                "OIT accumulate",
                "fs_oit_accumulate",
                2,
                &[target(COEFFICIENT_FORMAT, add)],
            ),
        ];
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
        Self {
            uniform,
            layouts,
            terrain: terrain_pipelines,
            cull,
            composite,
            targets: None,
        }
    }

    /// The pipeline the terrain's translucent layer draws with in `stage`.
    pub fn terrain(&self, stage: Stage) -> &wgpu::RenderPipeline {
        &self.terrain[stage as usize]
    }

    /// Sizes the targets to the world's and writes this frame's projection.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        projection: &Mat4,
    ) {
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::cast_slice(&[projection_terms(projection), [0.0; 2]].concat()),
        );
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let make = |label, format, usage| {
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
        self.targets = Some(Targets {
            size,
            alone: group(&self.layouts[0], &[]),
            sampled: group(&self.layouts[1], &[&views.culled]),
            accumulate: group(&self.layouts[2], &[&views.culled, c0, c1]),
            cull: group(&self.layouts[1], &[&views.bounds]),
            composite: group(
                &self.layouts[3],
                &[&views.bounds, c0, c1, &views.accumulated],
            ),
            bounds,
            culled,
            views,
        });
    }

    /// `LevelRenderer.executeOit` for the terrain: `draw` records the
    /// translucent layer into each stage's pass with that stage's pipeline,
    /// then the composite goes over `color`. `depth` is the opaque world's:
    /// the stages only read it; the cull and the composite write it.
    pub fn run(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        draw: &dyn Fn(&mut wgpu::RenderPass<'_>, &wgpu::RenderPipeline),
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
        let read_depth = Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: None,
            stencil_ops: None,
        });
        let [c0, c1] = &t.views.coefficients;
        for (stage, label, group, targets) in [
            (
                Stage::DepthBounds,
                "OIT depth bounds",
                &t.alone,
                vec![attachment(
                    &t.views.bounds,
                    wgpu::LoadOp::Clear(BOUNDS_CLEAR),
                )],
            ),
            (
                Stage::Transmittance,
                "OIT transmittance",
                &t.sampled,
                vec![attachment(c0, clear), attachment(c1, clear)],
            ),
            (
                Stage::Accumulate,
                "OIT accumulate",
                &t.accumulate,
                vec![attachment(&t.views.accumulated, clear)],
            ),
        ] {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &targets,
                    depth_stencil_attachment: read_depth.clone(),
                    ..Default::default()
                });
                pass.set_bind_group(GROUP, group, &[]);
                draw(&mut pass, self.terrain(stage));
            }
            if stage == Stage::DepthBounds {
                self.execute_depth_bounds_cull(encoder, t, depth);
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OIT composite"),
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

    /// `LevelRenderer.executeDepthBoundsCull`: the bounds are copied, then where
    /// the nearest fully opaque see-through fragment is in front of the world,
    /// the far bound stops there and its depth replaces the world's.
    fn execute_depth_bounds_cull(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        depth: &wgpu::TextureView,
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

    #[test]
    fn the_shader_validates() {
        let source = include_str!("../terrain.wgsl").to_owned() + include_str!("../oit.wgsl");
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        for entry in [
            "fs_oit_bounds",
            "fs_oit_transmittance",
            "fs_oit_accumulate",
            "fs_oit_cull",
            "fs_oit_composite",
        ] {
            assert!(
                module.entry_points.iter().any(|e| e.name == entry),
                "{entry}"
            );
        }
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
}
