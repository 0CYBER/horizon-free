//! The menus' backdrop treatment (`ui_theme`'s Horizon style): the finished
//! world frame is blurred (dual Kawase: three halvings down, back up) and
//! colour graded before the HUD is drawn over it. Needs a swapchain that can
//! be copied from; without one the frame is left as it is.

/// How the frame under a Horizon-style menu is treated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostGrade {
    /// Colour multiplier and lift (`c * tint + lift`).
    pub tint: [f32; 3],
    pub lift: [f32; 3],
    /// Toward grey, 0..1.
    pub desaturate: f32,
    /// Darkening toward the corners, 0..1.
    pub vignette: f32,
}
impl PostGrade {
    /// The title backdrop: icy, slightly lifted shadows.
    pub const FROST: Self = Self {
        tint: [0.62, 0.78, 1.0],
        lift: [0.0, 0.01, 0.035],
        desaturate: 0.45,
        vignette: 0.5,
    };
    /// Behind the pause menu: the world stays recognisable.
    pub const PAUSE: Self = Self {
        tint: [0.8, 0.86, 0.98],
        lift: [0.0, 0.004, 0.012],
        desaturate: 0.25,
        vignette: 0.3,
    };
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    texel: [f32; 4],
    tint: [f32; 4],
    lift: [f32; 4],
    params: [f32; 4],
}
const LEVELS: usize = 3;
pub struct Post {
    layout: wgpu::BindGroupLayout,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    finish: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    /// The copied frame, then the halvings.
    levels: Vec<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
    uniforms: Vec<wgpu::Buffer>,
    size: [u32; 2],
    format: wgpu::TextureFormat,
}
impl Post {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Menu backdrop"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Menu backdrop bindings"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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
            label: Some("Menu backdrop"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Menu backdrop pass"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Menu backdrop"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let uniforms = (0..LEVELS * 2 + 1)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Menu backdrop pass"),
                    size: std::mem::size_of::<Uniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        Self {
            down: pipeline("fs_down"),
            up: pipeline("fs_up"),
            finish: pipeline("fs_final"),
            layout,
            sampler,
            levels: Vec::new(),
            uniforms,
            size: [0, 0],
            format,
        }
    }
    fn ensure(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.size == size && !self.levels.is_empty() {
            return;
        }
        self.size = size;
        self.levels = (0..=LEVELS)
            .map(|i| {
                let s = [(size[0] >> i).max(1), (size[1] >> i).max(1)];
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Menu backdrop level"),
                    size: wgpu::Extent3d {
                        width: s[0],
                        height: s[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                (texture, view, s)
            })
            .collect();
    }
    fn pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        (from, uniform): (usize, usize),
        target: &wgpu::TextureView,
        data: Uniform,
    ) {
        let (_, view, s) = &self.levels[from];
        let mut data = data;
        data.texel = [0.5 / s[0] as f32, 0.5 / s[1] as f32, 0.0, 0.0];
        queue.write_buffer(&self.uniforms[uniform], 0, bytemuck::bytes_of(&data));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Menu backdrop pass"),
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniforms[uniform].as_entire_binding(),
                },
            ],
        });
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Menu backdrop pass"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    /// Blur and grade `frame` (the swapchain texture) into `target`.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
        target: &wgpu::TextureView,
        size: [u32; 2],
        grade: &PostGrade,
    ) {
        self.ensure(device, size);
        encoder.copy_texture_to_texture(
            frame.as_image_copy(),
            self.levels[0].0.as_image_copy(),
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        let data = Uniform {
            texel: [0.0; 4],
            tint: [grade.tint[0], grade.tint[1], grade.tint[2], 1.0],
            lift: [grade.lift[0], grade.lift[1], grade.lift[2], 0.0],
            params: [grade.desaturate, grade.vignette, 0.0, 0.0],
        };
        let mut u = 0;
        for i in 1..=LEVELS {
            let target = &self.levels[i].1;
            self.pass(device, queue, encoder, &self.down, (i - 1, u), target, data);
            u += 1;
        }
        for i in (1..LEVELS).rev() {
            let target = &self.levels[i].1;
            self.pass(device, queue, encoder, &self.up, (i + 1, u), target, data);
            u += 1;
        }
        self.pass(device, queue, encoder, &self.finish, (1, u), target, data);
    }
}
