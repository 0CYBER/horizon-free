//! The uniforms a shader pack's gbuffers programs read per draw (`HzDraw` at set 2, the
//! translator's `DRAW_UNIFORMS`): one slot per render stage of a uniform buffer, picked with a
//! dynamic offset at each draw.

use horizon_shaders::{RENDER_STAGES, render_stage};

/// `HzDraw`'s size: `renderStage`, `entityId`, `blockEntityId`, a pad, `entityColor`.
const SIZE: u32 = 32;

pub(super) struct DrawUniforms {
    pub layout: wgpu::BindGroupLayout,
    group: wgpu::BindGroup,
    stride: u32,
}

impl DrawUniforms {
    pub fn new(device: &wgpu::Device) -> Self {
        let stride = SIZE.max(device.limits().min_uniform_buffer_offset_alignment);
        let mut contents = vec![0u8; stride as usize * RENDER_STAGES.len()];
        for (i, slot) in contents.chunks_mut(stride as usize).enumerate() {
            slot[..4].copy_from_slice(&(i as i32).to_le_bytes());
        }
        let buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("Shader pack draws"),
                contents: &contents,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shader pack draws"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(u64::from(SIZE)),
                },
                count: None,
            }],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader pack draws"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(u64::from(SIZE)),
                }),
            }],
        });
        Self {
            layout,
            group,
            stride,
        }
    }

    /// Draws that follow read `renderStage` = `MC_RENDER_STAGE_<stage>`, other values zero.
    pub fn bind(&self, pass: &mut wgpu::RenderPass<'_>, stage: &str) {
        pass.set_bind_group(2, &self.group, &[self.stride * render_stage(stage)]);
    }
}
