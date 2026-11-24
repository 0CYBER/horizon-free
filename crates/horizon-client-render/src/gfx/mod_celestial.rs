//! Native celestial triangles use the entity atlas and the world frame's colour encoding.

use super::upload::Uploads;
use crate::render_slots::celestial_sky::{
    CelestialSky, CelestialVertex, MAX_CELESTIAL_TRIANGLES, listener,
};
use glam::Mat4;

// The shader binds the entity uniform but reads only its leading matrix.
const _: () = assert!(std::mem::offset_of!(super::Globals, view_proj) == 0);

pub(super) struct Layer {
    material: Material,
    vertices: Vec<CelestialVertex>,
}

enum Material {
    Pending {
        layout: wgpu::PipelineLayout,
        frame: (wgpu::TextureFormat, bool),
    },
    Ready {
        pipeline: wgpu::RenderPipeline,
        buffer: wgpu::Buffer,
    },
}

impl Layer {
    pub(super) fn listened(
        layout: &wgpu::PipelineLayout,
        frame: (wgpu::TextureFormat, bool),
    ) -> Option<Self> {
        listener().map(|_| Self {
            material: Material::Pending {
                layout: layout.clone(),
                frame,
            },
            vertices: Vec::new(),
        })
    }

    /// Called only when the main pass will draw the engine's celestial sky. Uploads belong
    /// to this frame's encoder, so an earlier in-flight frame never reads newer geometry.
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        view_proj: Mat4,
        angle: f32,
    ) -> u32 {
        self.vertices.clear();
        let Some(draw) = listener() else {
            return 0;
        };
        draw(&mut CelestialSky::new(view_proj, angle, &mut self.vertices));
        if self.vertices.is_empty() {
            return 0;
        }
        if let Material::Pending { layout, frame } = &self.material {
            // A failed build stays pending, draws nothing, and a later frame tries again.
            let Some(ready) = Material::ready(device, layout, *frame) else {
                return 0;
            };
            self.material = ready;
        }
        let Material::Ready { buffer, .. } = &self.material else {
            unreachable!()
        };
        uploads.buffer(encoder, buffer, 0, bytemuck::cast_slice(&self.vertices));
        self.vertices.len() as u32
    }

    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        atlas: &wgpu::BindGroup,
        vertices: u32,
    ) {
        let Material::Ready { pipeline, buffer } = &self.material else {
            unreachable!()
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, atlas, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..vertices, 0..1);
    }
}

impl Material {
    fn ready(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        (format, linear): (wgpu::TextureFormat, bool),
    ) -> Option<Self> {
        const STRIDE: u64 = std::mem::size_of::<CelestialVertex>() as u64;
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Native celestial triangles"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}{}",
                    super::wgsl_prefix::HDR_LIGHT,
                    include_str!("mod_celestial.wgsl")
                )
                .into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Native celestial triangles"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_celestial_layer"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Float32x4],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_celestial_layer"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &super::linear_blend::frame_constants(format, linear),
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Native celestial triangles"),
            size: MAX_CELESTIAL_TRIANGLES as u64 * 3 * STRIDE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let memory = pollster::block_on(memory.pop());
        if let Some(e) = memory.or(pollster::block_on(validation.pop())) {
            log::warn!("native celestial triangles unavailable this frame: {e}");
            return None;
        }
        Some(Self::Ready { pipeline, buffer })
    }
}
