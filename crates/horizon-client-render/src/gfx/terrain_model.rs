//! Opaque/cutout terrain-sheet native models, with ordinary culling and depth writes.
pub(super) fn pipelines(
    device: &wgpu::Device,
    layout: Option<&wgpu::PipelineLayout>,
    shader: &wgpu::ShaderModule,
    (format, linear): (wgpu::TextureFormat, bool),
    sprite_constants: &[(&str, f64)],
) -> [wgpu::RenderPipeline; 3] {
    std::array::from_fn(|i| {
        let mut constants = sprite_constants.to_vec();
        constants.extend(super::linear_blend::frame_constants(format, linear));
        constants.push(("NATIVE_MODEL_ALPHA_CUTOFF", [0.0, 0.1, 0.5][i]));
        // The ordinary Vertex ABI is unchanged. This material's private payload uses
        // normal's four bytes for a float z, and glint UV for float normal x/y.
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Uint32,6=>Float32x2];
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(
                [
                    "Native model solid",
                    "Native model cutout",
                    "Native model cutout mipmapped",
                ][i],
            ),
            layout,
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_native_model"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &constants,
                    ..Default::default()
                },
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<super::Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_native_model"),
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
        })
    })
}

#[cfg(test)]
mod tests {
    use super::super as gfx;

    #[test]
    fn terrain_model_and_fluid_match_float_frame_attachments() {
        let _gpu = gfx::test_gpu::lock();
        let Some(adapter) = gfx::test_gpu::adapter() else {
            return;
        };
        let (device, queue) = gfx::test_gpu::device(&adapter);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Native terrain materials"),
            source: wgpu::ShaderSource::Wgsl(gfx::terrain_shader().into()),
        });
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &gfx::native_material::terrain_entries(),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bindings)],
            ..Default::default()
        });
        let attachment = |format| {
            device
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
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let color = attachment(gfx::LINEAR_FRAME);
        let depth = attachment(wgpu::TextureFormat::Depth32Float);
        for linear in [false, true] {
            let fluid = gfx::fluid::pipeline(
                &device,
                Some(&layout),
                &shader,
                (gfx::LINEAR_FRAME, linear),
                &[],
            );
            let models = super::pipelines(
                &device,
                Some(&layout),
                &shader,
                (gfx::LINEAR_FRAME, linear),
                &[],
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_pipeline(&fluid);
                for model in &models {
                    pass.set_pipeline(model);
                }
            }
            queue.submit([encoder.finish()]);
        }
    }
}
