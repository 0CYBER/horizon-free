//! A Horizon shader pack on the GPU (docs/shader-packs.md): each stage
//! (`shadow`, `gbuffer`, `deferred`, `composite`, `final`) is a full-screen
//! pass that reads the world's colour, depth, opaque depth, G-buffer normals
//! and materials, shadow map, noise texture and the previous stage's output,
//! and writes its own target; the last target goes to the screen through
//! `world_post` (copy or FXAA) under the hand and HUD. Only exists while a
//! pack is selected.
use crate::shader_pack::{Rgba, ShaderPack, VERTEX_ENTRY};
use horizon_shaderpack::uniforms::SmoothFloat;

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

/// The API 2 `FrameExt` uniform of the prelude (`shader_pack_prelude.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FrameExtUniform {
    pub previous_view: [[f32; 4]; 4],
    pub previous_projection: [[f32; 4]; 4],
    pub shadow_view: [[f32; 4]; 4],
    pub shadow_projection: [[f32; 4]; 4],
    pub shadow_inverse_view: [[f32; 4]; 4],
    pub shadow_inverse_projection: [[f32; 4]; 4],
    pub previous_camera: [f32; 4],
    pub shadow_params: [f32; 4],
    pub environment: [f32; 4],
}

/// Iris's `noisetex` without a `texture.noise`: 256² random RGB, opaque.
/// Horizon packs get the same one unless `[textures] noise` replaces it.
pub(super) fn noise() -> Rgba {
    const SIZE: u32 = 256;
    let mut seed = 0x2545_f491_u32;
    let pixels = (0..SIZE * SIZE * 4)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            if i % 4 == 3 { 255 } else { (seed >> 24) as u8 }
        })
        .collect();
    Rgba {
        width: SIZE,
        height: SIZE,
        pixels,
    }
}

/// `image` as a sampled RGBA8 texture.
pub(super) fn rgba_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    image: &Rgba,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        &image.pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width * 4),
            rows_per_image: None,
        },
        texture.size(),
    );
    texture.create_view(&Default::default())
}

struct Pass {
    pipeline: wgpu::RenderPipeline,
    scale: f32,
    target_format: wgpu::TextureFormat,
    uniform: wgpu::Buffer,
    uniform_ext: wgpu::Buffer,
}
/// A stage's output and the bindings that read the world into it.
struct Target {
    size: [u32; 2],
    view: wgpu::TextureView,
    group: wgpu::BindGroup,
}

pub struct PackStages {
    pub id: String,
    pub needs_shadow: bool,
    /// The stages read the G-buffer (`build_gbuffer`) or the depth before
    /// translucent geometry (`capture_opaque_depth`).
    pub needs_gbuffer: bool,
    pub needs_opaque_depth: bool,
    pub shadow_res: u32,
    pub shadow_dist: f32,
    layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    repeat: wgpu::Sampler,
    shadow_sampler: wgpu::Sampler,
    noise_view: wgpu::TextureView,
    passes: Vec<Pass>,
    /// Built for this world target size and `world_post` generation.
    targets: Vec<Target>,
    /// `world_post`'s bindings of the last stage's output.
    output: Option<wgpu::BindGroup>,
    world_size: [u32; 2],
    post_generation: u64,
    started: std::time::Instant,
    last: Option<std::time::Instant>,
    frames: u32,
    previous: Option<([[f32; 4]; 4], [[f32; 4]; 4], [f32; 3])>,
    wetness: SmoothFloat,
}

fn stage_format(name: Option<&str>, default: wgpu::TextureFormat) -> wgpu::TextureFormat {
    match name.map(str::to_lowercase).as_deref() {
        Some("rgba16float" | "rgba16f") => wgpu::TextureFormat::Rgba16Float,
        Some("rgba8unorm" | "rgba8" | "rgba") => wgpu::TextureFormat::Rgba8Unorm,
        Some("rgb10a2unorm" | "rgb10_a2") => wgpu::TextureFormat::Rgb10a2Unorm,
        _ => default,
    }
}

impl PackStages {
    /// The pipelines of a loaded (already validated) pack, or wgpu's error.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
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
        let uniform_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
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
                uniform_entry(5),
                uniform_entry(6),
                texture(7, float),
                texture(8, float),
                texture(9, wgpu::TextureSampleType::Depth),
                texture(10, wgpu::TextureSampleType::Depth),
                sampler(11, wgpu::SamplerBindingType::Comparison),
                texture(12, float),
                sampler(13, wgpu::SamplerBindingType::Filtering),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shader pack stage"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let constants = pack.constants();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let stage_count = pack.stages.len();
        let passes = pack
            .stages
            .iter()
            .enumerate()
            .map(|(idx, stage)| {
                let is_last = idx + 1 == stage_count;
                let target_format = if is_last {
                    format
                } else {
                    stage_format(stage.format.as_deref(), format)
                };
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
                            format: target_format,
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
                    target_format,
                    uniform: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Shader pack frame"),
                        size: std::mem::size_of::<FrameUniform>() as u64,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    uniform_ext: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Shader pack frame ext"),
                        size: std::mem::size_of::<FrameExtUniform>() as u64,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                }
            })
            .collect();
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(error.to_string());
        }
        let make_sampler = |filter, address, compare| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Shader pack"),
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                mag_filter: filter,
                min_filter: filter,
                compare,
                ..Default::default()
            })
        };
        let noise_view = match &pack.custom_noise {
            Some(image) => rgba_view(device, queue, "Shader pack noise", image),
            None => rgba_view(device, queue, "Shader pack noise", &noise()),
        };
        let (shadow_res, shadow_dist) = pack.shadow_config();
        Ok(Self {
            id: pack.id.clone(),
            needs_shadow: pack.needs_shadow(),
            needs_gbuffer: pack.needs_gbuffer(),
            needs_opaque_depth: pack.needs_opaque_depth(),
            shadow_res,
            shadow_dist,
            layout,
            linear: make_sampler(
                wgpu::FilterMode::Linear,
                wgpu::AddressMode::ClampToEdge,
                None,
            ),
            nearest: make_sampler(
                wgpu::FilterMode::Nearest,
                wgpu::AddressMode::ClampToEdge,
                None,
            ),
            repeat: make_sampler(wgpu::FilterMode::Linear, wgpu::AddressMode::Repeat, None),
            shadow_sampler: make_sampler(
                wgpu::FilterMode::Linear,
                wgpu::AddressMode::ClampToEdge,
                Some(wgpu::CompareFunction::GreaterEqual),
            ),
            noise_view,
            passes,
            targets: Vec::new(),
            output: None,
            world_size: [0, 0],
            post_generation: u64::MAX,
            started: std::time::Instant::now(),
            last: None,
            frames: 0,
            previous: None,
            wetness: SmoothFloat::default(),
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
        eye_light: [u8; 2],
    ) -> &wgpu::BindGroup {
        if self.world_size != size
            || self.post_generation != post.generation()
            || self.targets.len() != self.passes.len()
        {
            self.world_size = size;
            self.post_generation = post.generation();
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
                        format: pass.target_format,
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
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: pass.uniform_ext.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: wgpu::BindingResource::TextureView(post.normal()),
                        },
                        wgpu::BindGroupEntry {
                            binding: 8,
                            resource: wgpu::BindingResource::TextureView(post.material()),
                        },
                        wgpu::BindGroupEntry {
                            binding: 9,
                            resource: wgpu::BindingResource::TextureView(post.opaque_depth()),
                        },
                        wgpu::BindGroupEntry {
                            binding: 10,
                            resource: wgpu::BindingResource::TextureView(post.shadow_depth()),
                        },
                        wgpu::BindGroupEntry {
                            binding: 11,
                            resource: wgpu::BindingResource::Sampler(&self.shadow_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 12,
                            resource: wgpu::BindingResource::TextureView(&self.noise_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 13,
                            resource: wgpu::BindingResource::Sampler(&self.repeat),
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
        let now = std::time::Instant::now();
        let dt = self.last.map_or(0.0, |t| (now - t).as_secs_f32());
        self.last = Some(now);
        let wet = self.wetness.update(uniform.weather[0], 60.0, 120.0, dt);
        let cam = [
            uniform.camera_position[0],
            uniform.camera_position[1],
            uniform.camera_position[2],
        ];
        let (prev_view, prev_proj, prev_cam) =
            self.previous
                .unwrap_or((uniform.view, uniform.projection, cam));
        self.previous = Some((uniform.view, uniform.projection, cam));
        let shadow_view = post.shadow_view;
        let shadow_proj = post.shadow_proj;
        let ext = FrameExtUniform {
            previous_view: prev_view,
            previous_projection: prev_proj,
            shadow_view: shadow_view.to_cols_array_2d(),
            shadow_projection: shadow_proj.to_cols_array_2d(),
            shadow_inverse_view: shadow_view.inverse().to_cols_array_2d(),
            shadow_inverse_projection: shadow_proj.inverse().to_cols_array_2d(),
            previous_camera: [prev_cam[0], prev_cam[1], prev_cam[2], dt],
            shadow_params: [
                post.shadow_resolution as f32,
                post.shadow_distance,
                0.0,
                if post.shadow_active { 1.0 } else { 0.0 },
            ],
            environment: [
                f32::from(eye_light[0]) * 16.0,
                f32::from(eye_light[1]) * 16.0,
                wet,
                0.0,
            ],
        };
        uniform.time[0] = self.started.elapsed().as_secs_f32();
        uniform.time[1] = self.frames as f32;
        self.frames = self.frames.wrapping_add(1);
        for (pass, target) in self.passes.iter().zip(&self.targets) {
            let [w, h] = target.size.map(|v| v as f32);
            uniform.resolution = [w, h, 1.0 / w, 1.0 / h];
            queue.write_buffer(&pass.uniform, 0, bytemuck::bytes_of(&uniform));
            queue.write_buffer(&pass.uniform_ext, 0, bytemuck::bytes_of(&ext));
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
