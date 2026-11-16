//! A texture-to-texture copy keeps a depth texture's depth, on one layer and on each layer of an
//! array: the copies the pack's depth snapshots (`depthtex1`, `depthtex2`) and `shadowtex1` are
//! made with, here right after a multisampled pass with two colour targets. On OpenGL the copy
//! read nothing until arismoko/horizon#2400. Original fixture, no pack files.
use super::depth_test::Gpu;
use wgpu::{TextureFormat as T, util::DeviceExt};

const SIDE: u32 = 4;

/// Loads texel (1, 1) of a layer of `depth` into red. The depth is bound as unfilterable floats,
/// since naga's GLSL writer can't `textureLoad` a depth texture; `array` picks the array form.
fn sampler_shader(array: bool) -> String {
    let (ty, load) = if array {
        (
            "texture_2d_array<f32>",
            "textureLoad(depth, vec2<i32>(1, 1), layer.index, 0)",
        )
    } else {
        ("texture_2d<f32>", "textureLoad(depth, vec2<i32>(1, 1), 0)")
    };
    format!(
        "@group(0) @binding(0) var depth: {ty};
struct Layer {{ index: i32, pad0: i32, pad1: i32, pad2: i32 }}
@group(0) @binding(1) var<uniform> layer: Layer;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}}
@fragment fn fs() -> @location(0) vec4<f32> {{
    return vec4<f32>({load}.r, 0.0, 0.0, 1.0);
}}"
    )
}

/// The depth of each layer of `texture` (`Depth32Float`, bindable), in 1/255 steps.
fn read_layers(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let device = &gpu.device;
    let layers = texture.depth_or_array_layers();
    let array = layers > 1;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(sampler_shader(array).into()),
    });
    let dimension = if array {
        wgpu::TextureViewDimension::D2Array
    } else {
        wgpu::TextureViewDimension::D2
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: dimension,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
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
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(T::Rgba8Unorm.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(dimension),
        aspect: wgpu::TextureAspect::DepthOnly,
        ..Default::default()
    });
    // One bind group per layer, each with that layer's index.
    let groups: Vec<_> = (0..layers as i32)
        .map(|index| {
            let index = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&[index, 0, 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: index.as_entire_binding(),
                    },
                ],
            })
        })
        .collect();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: layers,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: T::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        // One pixel per layer.
        for (layer, group) in groups.iter().enumerate() {
            pass.set_viewport(layer as f32, 0.0, 1.0, 1.0, 0.0, 1.0);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    gpu.queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = read.slice(..).get_mapped_range().unwrap();
    (0..layers as usize).map(|i| bytes[i * 4]).collect()
}

/// A pass that clears and stores two 4x-multisampled `Rgba8Unorm` targets, with no resolve.
fn multisampled_targets_pass(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
    let views: Vec<_> = (0..2)
        .map(|_| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: SIDE,
                        height: SIDE,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 4,
                    dimension: wgpu::TextureDimension::D2,
                    format: T::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        })
        .collect();
    let attachments: Vec<_> = views
        .iter()
        .map(|view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })
        })
        .collect();
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &attachments,
        ..Default::default()
    });
}

/// Each layer of a `Depth32Float` texture cleared to its own depth, then copied whole into
/// another after a multisampled two-target pass: the copy reads the same depths as the source.
#[test]
fn a_copied_depth_texture_keeps_its_depth() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else { return };
    let device = &gpu.device;
    for layers in [1, 2] {
        let texture = |usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: SIDE,
                    height: SIDE,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: T::Depth32Float,
                usage: usage | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let source =
            texture(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        let copy = texture(wgpu::TextureUsages::COPY_DST);
        let depths: Vec<f32> = (0..layers).map(|l| 0.25 + 0.5 * l as f32).collect();
        let mut encoder = device.create_command_encoder(&Default::default());
        for (layer, &depth) in depths.iter().enumerate() {
            let view = source.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer as u32,
                array_layer_count: Some(1),
                ..Default::default()
            });
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(depth),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
        }
        // A multisampled pass with two colour targets just before: on OpenGL its images stay
        // on the framebuffer the copy borrows until the next pass resets it.
        multisampled_targets_pass(device, &mut encoder);
        encoder.copy_texture_to_texture(
            source.as_image_copy(),
            copy.as_image_copy(),
            source.size(),
        );
        gpu.queue.submit([encoder.finish()]);
        let expected: Vec<u8> = depths.iter().map(|d| (d * 255.0).round() as u8).collect();
        let near = |got: &[u8]| got.iter().zip(&expected).all(|(&g, &e)| g.abs_diff(e) <= 1);
        let direct = read_layers(&gpu, &source);
        assert!(
            near(&direct),
            "{layers} layers: source {direct:?}, expected {expected:?}"
        );
        let copied = read_layers(&gpu, &copy);
        assert!(
            near(&copied),
            "{layers} layers: copy {copied:?}, expected {expected:?}"
        );
    }
}
