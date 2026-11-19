use super::*;
use crate::gfx::{reuse::Reuse, wgsl_prefix};

/// Turning it on in a renderer made without its pipelines asks once for a renderer made again;
/// an adapter that can't draw it never asks, and turning it off stops it drawing.
#[test]
fn switching_on_asks_for_its_pipelines_once() {
    let msaa = |supported, requested| Msaa {
        supported,
        requested,
        rebuild: false,
        rebuildable: true,
        active: false,
        passes: None,
    };
    let mut off = msaa(true, false);
    off.set(true);
    assert!(std::mem::take(&mut off.rebuild));
    off.set(true);
    assert!(!off.rebuild, "asked once");
    off.set(false);
    off.set(true);
    assert!(
        off.rebuild,
        "asked again after a switch off and on without the pipelines"
    );
    off.set(false);
    assert!(
        !off.rebuild,
        "off again before the renderer was made: no rebuild"
    );

    let mut unsupported = msaa(false, false);
    unsupported.set(true);
    assert!(!unsupported.rebuild);
    assert!(!unsupported.wanted());

    let mut drawing = msaa(true, true);
    drawing.active = true;
    drawing.set(false);
    assert!(!drawing.active && drawing.passes().is_none());
}

/// The button offers it where a renderer made again would bring its pipelines; split-screen's
/// isn't made again, so a renderer there made without them doesn't offer it.
#[test]
fn split_screen_offers_it_only_with_its_pipelines() {
    let msaa = |supported, rebuildable| Msaa {
        supported,
        requested: false,
        rebuild: false,
        rebuildable,
        active: false,
        passes: None,
    };
    assert!(msaa(true, true).offered());
    assert!(!msaa(true, false).offered(), "split-screen, no pipelines");
    assert!(
        !msaa(false, true).offered(),
        "an adapter that can't draw it"
    );
}

/// Real raster coverage through terrain.wgsl's `cutout_coverage` and the seed and resolve
/// passes: HDR light above white, its alpha codes, reverse-Z, a native pack's integer surface,
/// targets made again at a new size and surface, the tone-weighted resolve of linear light, and
/// the sample an edge pixel keeps.
/// Without an adapter that multisamples these formats (CI's runners) it returns early.
#[test]
fn coverage_resolves_hdr_depth_and_native_surfaces() {
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba16Float;
    if !supported(&adapter, format) {
        eprintln!("skipped: {:?} can't draw 4x MSAA", adapter.get_info());
        return;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features()
            & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        ..Default::default()
    }))
    .unwrap();
    // The function as the native materials copy it (`native_material.rs`).
    let coverage = wgsl_prefix::section(
        wgsl_prefix::TERRAIN,
        "fn cutout_coverage(",
        "struct TerrainMsaa",
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("MSAA coverage regression"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "{coverage}\n{}",
                r#"
struct Vertex {
    @builtin(position) p: vec4<f32>,
    @location(0) @interpolate(flat) geometric: u32,
}
@vertex fn vertex(@builtin(vertex_index) i: u32) -> Vertex {
    let p = array(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0),
                  vec2(0.0, -1.0), vec2(1.0, -1.0), vec2(0.0, 1.0));
    return Vertex(vec4(p[i], select(0.75, 0.5, i >= 3u), 1.0), u32(i >= 3u));
}
struct Surface {
    @location(0) color: vec4<f32>,
    @location(1) metadata: vec4<u32>,
    @builtin(sample_mask) coverage: u32,
}
fn shade(in: Vertex) -> Surface {
    let x = min(u32(in.p.x), 3u);
    let alpha = array(0.0, 0.5, 1.0, 0.0)[x];
    let mask = cutout_coverage(alpha, x == 3u);
    if in.geometric == 0u && in.p.x >= 4.0 { discard; }
    // Deliberately stale surface depth: the resolve must replace it with the chosen sample's.
    return Surface(vec4(8.0, 8.0, 8.0, 0.25), vec4(101u, 202u, 303u, bitcast<u32>(0.125)),
                   select(mask, 15u, in.geometric != 0u));
}
@fragment fn surface(in: Vertex) -> Surface { return shade(in); }
struct Color {
    @location(0) color: vec4<f32>,
    @builtin(sample_mask) coverage: u32,
}
@fragment fn color(in: Vertex) -> Color {
    let result = shade(in);
    return Color(result.color, result.coverage);
}
"#
            )
            .into(),
        ),
    });
    let reuse = Reuse::default();
    let kept = reuse.on(&device);
    let mut both = [false, true].map(|linear| Passes::new(&kept, &device, format, linear));
    // Edge pixels whose resolve kept the sky's sample, and those that kept the geometry's.
    let mut kept_by = [0; 2];
    // The gamma passes keep their targets from one case to the next: a new size or surface
    // makes them again. Sky 2.0 and terrain 8.0 average to 5.0 as stored, and to 3.5 weighted
    // (2/3 and 8/9, whose mean 7/9 turns back into 3.5).
    for (linear, surface, height, half) in [
        (false, false, 4, 5.0),
        (false, true, 6, 5.0),
        (false, false, 4, 5.0),
        (true, false, 4, 3.5),
    ] {
        let passes = &mut both[usize::from(linear)];
        let size = wgpu::Extent3d {
            width: 8,
            height,
            depth_or_array_layers: 1,
        };
        let texture = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let color = texture(format);
        let depth = texture(wgpu::TextureFormat::Depth32Float);
        let metadata = texture(SURFACE_FORMAT);
        let (color_view, depth_view, metadata_view) = (
            color.create_view(&Default::default()),
            depth.create_view(&Default::default()),
            metadata.create_view(&Default::default()),
        );
        let targets = [
            Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            }),
            Some(wgpu::ColorTargetState {
                format: SURFACE_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            }),
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("MSAA regression terrain"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: state(SAMPLES),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(if surface { "surface" } else { "color" }),
                compilation_options: Default::default(),
                targets: &targets[..if surface { 2 } else { 1 }],
            }),
            multiview_mask: None,
            cache: None,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            // Sky's light, above paper white, must survive both seeding and resolution.
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(attachment(
                    &color_view,
                    wgpu::LoadOp::Clear(wgpu::Color {
                        r: 2.0,
                        g: 2.0,
                        b: 2.0,
                        a: 1.0,
                    }),
                ))],
                ..Default::default()
            });
        }
        assert!(passes.fit(&device, [8, height], surface));
        passes.seed(&device, &mut encoder, &color_view);
        {
            let mut pass = passes.begin(&mut encoder);
            pass.set_pipeline(&pipeline);
            pass.draw(0..6, 0..1);
        }
        passes.resolve(
            &mut encoder,
            &color_view,
            &depth_view,
            surface.then_some(&metadata_view),
        );
        let stride = 256;
        let plane = u64::from(stride * height);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: plane * 3,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        for (i, texture) in [&color, &depth, &metadata].into_iter().enumerate() {
            if i == 2 && !surface {
                continue;
            }
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: if i == 1 {
                        wgpu::TextureAspect::DepthOnly
                    } else {
                        wgpu::TextureAspect::All
                    },
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: plane * i as u64,
                        bytes_per_row: Some(stride),
                        rows_per_image: Some(height),
                    },
                },
                size,
            );
        }
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read, .., move |r| tx.send(r).unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = readback.get_mapped_range(..).unwrap();
        let word =
            |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let rgba = |x, y| {
            std::array::from_fn::<_, 4, _>(|c| {
                let offset = (y * stride + x * 8) as usize + c * 2;
                half::f16::from_bits(u16::from_le_bytes(
                    bytes[offset..offset + 2].try_into().unwrap(),
                ))
                .to_f32()
            })
        };
        let mut geometric_edges = 0;
        for y in 0..height {
            for x in 0..8 {
                let color = rgba(x, y);
                let depth = f32::from_bits(word(plane as usize + (y * stride + x * 4) as usize));
                if x < 4 {
                    let expected = [2.0, half, 8.0, 8.0][x as usize];
                    assert!(
                        color[0..3].iter().all(|c| (c - expected).abs() < 1e-3),
                        "column {x}: {color:?}, not {expected}"
                    );
                    assert_eq!(
                        color[3],
                        if x == 0 { 1.0 } else { 0.25 },
                        "HDR light code must not be averaged"
                    );
                    assert_eq!(depth, if x == 0 { 0.0 } else { 0.75 });
                } else if color[0] > 2.0 && color[0] < 8.0 {
                    geometric_edges += 1;
                    // Geometry covering half the pixel or more (its colour at `half` or past
                    // it) owns its depth and light code, as the pixel centre would; less, the sky
                    // keeps them, so what draws after the resolve isn't hidden behind the edge.
                    let geometry = color[0] >= half - 1e-3;
                    kept_by[usize::from(geometry)] += 1;
                    assert_eq!(
                        (color[3], depth),
                        if geometry { (0.25, 0.5) } else { (1.0, 0.0) },
                        "edge {x},{y}: {color:?}"
                    );
                }
                if surface {
                    let offset = plane as usize * 2 + (y * stride + x * 16) as usize;
                    let actual = std::array::from_fn::<_, 4, _>(|i| word(offset + i * 4));
                    assert_eq!(
                        actual,
                        if depth == 0.0 {
                            [0; 4]
                        } else {
                            [101, 202, 303, depth.to_bits()]
                        }
                    );
                }
            }
        }
        assert!(
            geometric_edges > 0,
            "diagonal geometry needs subpixel coverage too"
        );
    }
    assert!(
        kept_by.iter().all(|&n| n > 0),
        "edges kept by each: {kept_by:?}"
    );
}
