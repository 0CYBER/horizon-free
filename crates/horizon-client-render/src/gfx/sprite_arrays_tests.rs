//! The sprite arrays a render hook samples.
/// A mod samples `BlockSpriteArrays` as the renderer hands them over: a layer of the second
/// array through its view, texels sharp and repeating with the nearest sampler.
#[test]
fn a_pass_samples_a_sprite_layer_sharp() {
    let _gpu = crate::gfx::test_gpu::lock();
    let instance = super::test_gpu::instance();
    let Some((device, queue)) = pollster::block_on(instance.request_adapter(&Default::default()))
        .ok()
        .and_then(|adapter| pollster::block_on(adapter.request_device(&Default::default())).ok())
    else {
        return;
    };
    let gl = device.adapter_info().backend == wgpu::Backend::Gl;
    // two arrays of two 2 by 2 layers; array 1's layer 1 red, green / blue, white
    let layers = super::array_layers(2, gl);
    let quad = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255; 4],
    ];
    let textures: Vec<wgpu::Texture> = (0..2)
        .map(|array| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 2,
                    height: 2,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let mut texels = vec![0u8; 16 * layers as usize];
            if array == 1 {
                texels[16..32].copy_from_slice(quad.as_flattened());
            }
            queue.write_texture(
                texture.as_image_copy(),
                &texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(8),
                    rows_per_image: Some(2),
                },
                texture.size(),
            );
            texture
        })
        .collect();
    let unused = textures[0].create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let views = super::sprite_views(&textures, &unused, Some(1));
    let nearest = device.create_sampler(&super::SPRITE_NEAREST);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(
            "@group(0) @binding(0) var sprites: texture_2d_array<f32>;
             @group(0) @binding(1) var sprites_1: texture_2d_array<f32>;
             @group(0) @binding(2) var nearest: sampler;
             @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                 return vec4(f32(i & 1u) * 4.0 - 1.0, f32(i >> 1u) * 4.0 - 1.0, 0.0, 1.0);
             }
             @fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
                 // inside each texel, and past the edge (repeating); the last reads array 0
                 let uv = array(vec2(1.3, 0.2), vec2(0.7, 0.45), vec2(-0.8, 0.9), vec2(0.6, 0.6));
                 let x = u32(at.x);
                 if x == 4u {
                     return textureSampleLevel(sprites, nearest, vec2(0.3), 1, 0.0);
                 }
                 return textureSampleLevel(sprites_1, nearest, uv[x], 1, 0.0);
             }"
            .into(),
        ),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&views[0]),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&views[1]),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&nearest),
            },
        ],
    });
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 5,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let view = target.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
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
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &out,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    out.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let texels = out.slice(..).get_mapped_range().unwrap()[..20].to_vec();
    let mut expected = quad.as_flattened().to_vec();
    expected.extend([0, 0, 0, 0]);
    assert_eq!(texels, expected);
}
