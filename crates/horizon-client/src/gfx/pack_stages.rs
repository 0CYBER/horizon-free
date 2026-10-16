//! A Horizon shader pack on the GPU (docs/shader-packs.md): each `composite`
//! stage is one full-screen pass that reads the world's colour and depth and
//! the previous stage's output and writes its own target; the last target
//! goes to the screen through `world_post` (copy or FXAA) under the hand and
//! HUD. Only exists while a pack is selected.
use crate::shader_pack::{ShaderPack, VERTEX_ENTRY};

/// The `Frame` uniform of the prelude (`shader_pack_prelude.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FrameUniform {
    pub view: [[f32; 4]; 4],
    pub projection: [[f32; 4]; 4],
    pub inverse_view: [[f32; 4]; 4],
    pub inverse_projection: [[f32; 4]; 4],
    pub camera_position: [f32; 4],
    pub resolution: [f32; 4],
    pub time: [f32; 4],
    pub sun: [f32; 4],
    pub moon: [f32; 4],
    pub sky_color: [f32; 4],
    pub fog_color: [f32; 4],
    pub fog: [f32; 4],
    pub weather: [f32; 4],
    pub camera_state: [f32; 4],
    pub view_params: [f32; 4],
    pub dimension: [f32; 4],
}

struct Pass {
    pipeline: wgpu::RenderPipeline,
    scale: f32,
    uniform: wgpu::Buffer,
}
/// A stage's output and the bindings that read the world into it.
struct Target {
    size: [u32; 2],
    view: wgpu::TextureView,
    group: wgpu::BindGroup,
}

pub struct PackStages {
    pub id: String,
    layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    passes: Vec<Pass>,
    format: wgpu::TextureFormat,
    /// Built for this world target size.
    targets: Vec<Target>,
    /// `world_post`'s bindings of the last stage's output.
    output: Option<wgpu::BindGroup>,
    world_size: [u32; 2],
    started: std::time::Instant,
    frames: u32,
}

impl PackStages {
    /// The pipelines of a loaded (already validated) pack, or wgpu's error.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pack: &ShaderPack,
    ) -> Result<Self, String> {
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
        let sampler = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(ty),
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shader pack stage"),
            entries: &[
                texture(0, float),
                texture(1, wgpu::TextureSampleType::Depth),
                texture(2, float),
                sampler(3, wgpu::SamplerBindingType::Filtering),
                sampler(4, wgpu::SamplerBindingType::NonFiltering),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shader pack stage"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let constants = pack.constants();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let passes = pack
            .stages
            .iter()
            .map(|stage| {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(&stage.file),
                    source: wgpu::ShaderSource::Wgsl(stage.source.as_str().into()),
                });
                // Only the overrides a stage declares may be passed to it.
                let constants: Vec<(&str, f64)> = constants
                    .iter()
                    .copied()
                    .filter(|(key, _)| declares_override(&stage.source, key))
                    .collect();
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(&stage.file),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some(VERTEX_ENTRY),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(&stage.entry),
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
                });
                Pass {
                    pipeline,
                    scale: stage.scale,
                    uniform: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Shader pack frame"),
                        size: std::mem::size_of::<FrameUniform>() as u64,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                }
            })
            .collect();
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(error.to_string());
        }
        let sampler = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Shader pack"),
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        Ok(Self {
            id: pack.id.clone(),
            layout,
            linear: sampler(wgpu::FilterMode::Linear),
            nearest: sampler(wgpu::FilterMode::Nearest),
            passes,
            format,
            targets: Vec::new(),
            output: None,
            world_size: [0, 0],
            started: std::time::Instant::now(),
            frames: 0,
        })
    }

    /// Run every stage over the world's `color` and `depth` (`size`) and
    /// return `post`'s bindings of the last stage's output.
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
        mut uniform: FrameUniform,
    ) -> &wgpu::BindGroup {
        if self.world_size != size || self.targets.len() != self.passes.len() {
            self.world_size = size;
            self.targets.clear();
            for pass in &self.passes {
                let target_size = size.map(|v| ((v as f32 * pass.scale).round() as u32).max(1));
                let view = device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("Shader pack stage"),
                        size: wgpu::Extent3d {
                            width: target_size[0],
                            height: target_size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: self.format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&Default::default());
                let previous = self.targets.last().map_or(color, |t: &Target| &t.view);
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Shader pack stage"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(color),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(depth),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(previous),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&self.linear),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::Sampler(&self.nearest),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: pass.uniform.as_entire_binding(),
                        },
                    ],
                });
                self.targets.push(Target {
                    size: target_size,
                    view,
                    group,
                });
            }
            let last = &self.targets.last().expect("a pack has a stage").view;
            self.output = Some(post.bind(device, last));
        }
        uniform.time[0] = self.started.elapsed().as_secs_f32();
        uniform.time[1] = self.frames as f32;
        self.frames = self.frames.wrapping_add(1);
        for (pass, target) in self.passes.iter().zip(&self.targets) {
            let [w, h] = target.size.map(|v| v as f32);
            uniform.resolution = [w, h, 1.0 / w, 1.0 / h];
            queue.write_buffer(&pass.uniform, 0, bytemuck::bytes_of(&uniform));
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack stage"),
                color_attachments: &attachments,
                ..Default::default()
            });
            render.set_pipeline(&pass.pipeline);
            render.set_bind_group(0, &target.group, &[]);
            render.draw(0..3, 0..1);
        }
        self.output.as_ref().expect("bound with the targets")
    }
}

/// `override <key>` appears in the source (a declaration, not just a use).
fn declares_override(source: &str, key: &str) -> bool {
    source.lines().any(|line| {
        line.trim_start()
            .strip_prefix("override")
            .and_then(|rest| rest.trim_start().strip_prefix(key))
            .is_some_and(|rest| rest.trim_start().starts_with([':', '=']))
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_frame_uniform_matches_the_prelude() {
        // 4 matrices and 12 vectors, as `struct Frame` in the prelude.
        assert_eq!(std::mem::size_of::<super::FrameUniform>(), 4 * 64 + 12 * 16);
        let prelude = crate::shader_pack::PRELUDE;
        let body = prelude.split("struct Frame {").nth(1).unwrap();
        let body = &body[..body.find('}').unwrap()];
        assert_eq!(body.matches("mat4x4<f32>").count(), 4);
        assert_eq!(body.matches("vec4<f32>").count(), 12);
    }
    #[test]
    fn only_declared_overrides_are_passed() {
        let source =
            "override saturation: f32 = 1.0;\noverride  vignette = true;\nlet x = saturation_2;";
        assert!(super::declares_override(source, "saturation"));
        assert!(super::declares_override(source, "vignette"));
        assert!(!super::declares_override(source, "saturation_2"));
        assert!(!super::declares_override(source, "haze"));
    }
}
