//! The sun's passes on the float frame, through the actual entity shader and pipelines: a row
//! of sun texels across the target, over a lifted sky.
use super::Celestial;
use crate::gfx::hdr::{SCENE_PEAK as S, SUN_CAP, SUNLIT_GAIN, TIERS};
use crate::mesher::{LOW, VERY_BRIGHT, Vertex};
use wgpu::util::DeviceExt;

/// Vanilla's sky in gamma, and HDR lighting's `hdr_light.y` for it: the zenith at midday, the
/// sunrise's glow under the sun, night.
const SKIES: [([f32; 3], f32); 3] = [
    ([0.533, 0.683, 1.0], 1.0),
    ([0.828, 0.393, 0.215], 0.42),
    ([0.02, 0.02, 0.04], 0.0),
];
/// sun.png's disc: the outer, middle and centre rings.
const RINGS: [[u8; 3]; 3] = [[255, 213, 74], [255, 255, 170], [255, 255, 217]];

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    pollster::block_on(adapter.request_device(&Default::default())).ok()
}

/// The sky's lift in the frame's gamma (`sky_finished`: HDR lighting at its 8-bit mark).
fn sky_gain(light: f32) -> f32 {
    let mark = ((1.0 - 0.24 * light) * 255.0).round() / 255.0;
    (1.0 + (SUNLIT_GAIN - 1.0) * (1.0 - mark) / 0.24).powf(1.0 / 2.2)
}

/// The brightest channel of stored colour `c` as light, past S held at S as the tone curve does.
fn top(c: [f32; 3]) -> f32 {
    c.iter().fold(0.0f32, |m, v| m.max(v.powf(2.2))).min(S)
}

/// The float frame's stored colour of each texel of `row` drawn by the bodies' passes at `tier`
/// onto vanilla's `sky` lifted by HDR lighting's `light`, the sun's disc's peak at `sun_peak`.
fn draw(
    lifted: bool,
    row: &[[u8; 3]],
    tier: u8,
    (sky, light): ([f32; 3], f32),
    sun_peak: f32,
) -> Option<Vec<[f32; 3]>> {
    let (device, queue) = gpu()?;
    let width = row.len() as u32;
    let size = |width| wgpu::Extent3d {
        width,
        height: 1,
        depth_or_array_layers: 1,
    };
    let texture = |width, format, usage, data: Option<&[u8]>| {
        let descriptor = wgpu::TextureDescriptor {
            label: None,
            size: size(width),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        };
        match data {
            Some(data) => {
                device.create_texture_with_data(&queue, &descriptor, Default::default(), data)
            }
            None => device.create_texture(&descriptor),
        }
    };
    let sampled = |width, data: &[u8]| {
        let usage = wgpu::TextureUsages::TEXTURE_BINDING;
        texture(width, wgpu::TextureFormat::Rgba8Unorm, usage, Some(data))
            .create_view(&Default::default())
    };
    let texels: Vec<u8> = row.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect();
    let (atlas, white) = (sampled(width, &texels), sampled(1, &[255; 4]));
    let entries: Vec<_> = (0..8)
        .map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: match binding {
                0 => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                2 | 4 | 6 => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                _ => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
            },
            count: None,
        })
        .collect();
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &entries,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(super::super::entity_shader().into()),
    });
    let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
    let reuse = super::super::reuse::Reuse::default();
    let celestial = Celestial::new(
        &reuse.on(&device),
        &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        }),
        &shader,
        &[Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })],
        (super::super::LINEAR_FRAME, false),
    );
    let globals = super::super::Globals {
        view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
        hdr_windows: [TIERS.map(|t| t.start), TIERS.map(|t| t.full)],
        hdr_light: [light, light, 1.0, 1.0],
        hdr_settings: [sun_peak, SUNLIT_GAIN, 1.0, 0.0],
        hdr_gains: TIERS.map(|t| t.gain),
        ..bytemuck::Zeroable::zeroed()
    };
    let buffer = |contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents,
            usage,
        })
    };
    let uniform = buffer(bytemuck::bytes_of(&globals), wgpu::BufferUsages::UNIFORM);
    let sampler = device.create_sampler(&Default::default());
    let resources: Vec<_> = (0..8)
        .map(|binding| wgpu::BindGroupEntry {
            binding,
            resource: match binding {
                0 => uniform.as_entire_binding(),
                1 => wgpu::BindingResource::TextureView(&atlas),
                2 | 4 | 6 => wgpu::BindingResource::Sampler(&sampler),
                _ => wgpu::BindingResource::TextureView(&white),
            },
        })
        .collect();
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &resources,
    });
    // One quad over the target, the row's texels left to right, as the sun's (layer 2).
    let corners = [(0., 0.), (1., 0.), (1., 1.), (0., 1.)].map(|(x, y): (f32, f32)| Vertex {
        pos: [x * 2.0 - 1.0, y * 2.0 - 1.0, 0.5],
        uv: [x, 0.5],
        layer: 2 | u32::from(tier) << 16,
        color: u32::MAX,
        light: 0,
        normal: [0; 4],
        glint_uv: [0.0; 2],
    });
    let vertices = buffer(bytemuck::cast_slice(&corners), wgpu::BufferUsages::VERTEX);
    let indices = buffer(
        bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
        wgpu::BufferUsages::INDEX,
    );
    let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
    let frame = texture(
        width,
        super::super::LINEAR_FRAME,
        attachment | wgpu::TextureUsages::COPY_SRC,
        None,
    );
    let depth = texture(width, wgpu::TextureFormat::Depth32Float, attachment, None);
    let (frame_view, depth_view) = (
        frame.create_view(&Default::default()),
        depth.create_view(&Default::default()),
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let [r, g, b] = sky.map(|c| f64::from(c * sky_gain(light)));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &frame_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&celestial.added);
        pass.set_bind_group(0, &group, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        celestial.draw_bodies(&mut pass, lifted, 0..6);
    }
    let bytes_per_row = (width * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(bytes_per_row),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        frame.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
        },
        size(width),
    );
    queue.submit([encoder.finish()]);
    read.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = read.get_mapped_range(..).unwrap();
    let half = |p: &[u8]| half::f16::from_le_bytes([p[0], p[1]]).to_f32();
    Some(
        data[..width as usize * 8]
            .chunks(8)
            .map(|p| [half(p), half(&p[2..]), half(&p[4..])])
            .collect(),
    )
}

#[test]
fn the_suns_disc_shows_vanillas_colour_with_white_at_the_peak() {
    for sky in SKIES {
        let Some(out) = draw(true, &RINGS, VERY_BRIGHT, sky, S) else {
            eprintln!("celestial GPU unavailable");
            return;
        };
        for (ring, stored) in RINGS.iter().zip(out) {
            for c in 0..3 {
                // Vanilla: the gamma sum clipped at white; here white is S.
                let vanilla = (sky.0[c] + f32::from(ring[c]) / 255.0).min(1.0);
                let (got, want) = (stored[c].powf(2.2), S * vanilla.powf(2.2));
                assert!(
                    (got - want).abs() < 0.005 * S,
                    "sky {sky:?}, ring {ring:?}: {stored:?}"
                );
            }
        }
    }
}

#[test]
fn the_halo_and_the_moon_add_as_before_below_the_peak() {
    // The disc's edge, then a halo like sun.png's falling away from it.
    let halo = [40u8, 36, 32, 27, 23, 19, 14, 10, 7, 4, 2, 1].map(|v| [v, v, v / 2]);
    let row: Vec<_> = std::iter::once(RINGS[0]).chain(halo).collect();
    for sky in SKIES {
        let (Some(sun), Some(added)) = (
            draw(true, &row, VERY_BRIGHT, sky, S),
            draw(false, &row, VERY_BRIGHT, sky, S),
        ) else {
            eprintln!("celestial GPU unavailable");
            return;
        };
        assert_eq!(
            sun[1..],
            added[1..],
            "sky {sky:?}: the halo isn't added as before"
        );
        let light: Vec<f32> = sun.into_iter().map(top).collect();
        assert!((light[0] - S).abs() < 0.005 * S, "sky {sky:?}: {light:?}");
        assert!(
            light[1..].windows(2).all(|p| p[0] < S && p[1] < p[0]),
            "sky {sky:?}: {light:?}"
        );
        let moon = [[217, 228, 255], [122, 130, 150], [40, 40, 50]];
        assert_eq!(
            draw(true, &moon, LOW, sky, S),
            draw(false, &moon, LOW, sky, S),
            "sky {sky:?}: the moon"
        );
    }
}

#[test]
fn a_soft_edged_sun_fades_into_the_sky_without_a_ring() {
    // A resource pack's sun: warm white falling smoothly to black, through the whole of the
    // very bright tier's window.
    let row: Vec<[u8; 3]> = (0..96u16)
        .map(|i| {
            let v = 255.0 * (1.0 - f32::from(i) / 95.0);
            [v, v, v * 0.85].map(|c| c.round() as u8)
        })
        .collect();
    for sky in SKIES {
        let Some(out) = draw(true, &row, VERY_BRIGHT, sky, S) else {
            eprintln!("celestial GPU unavailable");
            return;
        };
        let light: Vec<f32> = out.into_iter().map(top).collect();
        assert!((light[0] - S).abs() < 0.005 * S, "sky {sky:?}: {light:?}");
        let smooth = |p: &[f32]| p[1] <= p[0] * 1.002 && p[0] - p[1] < 0.1 * S;
        assert!(
            light.windows(2).all(smooth),
            "sky {sky:?}: a ring or a step in {light:?}"
        );
    }
}

/// A display above the scene's peak (`Hdr::sun_peak`: 1500 nits, and the cap, at a paper white
/// of 203): the disc's white is the display's, and what isn't the disc is drawn exactly as on a
/// display at the scene's peak.
#[test]
fn on_a_brighter_display_only_the_disc_follows_its_peak() {
    let halo = [40u8, 36, 27, 19, 10, 4, 1].map(|v| [v, v, v / 2]);
    let row: Vec<_> = RINGS.into_iter().chain(halo).collect();
    let moon = [[217, 228, 255], [122, 130, 150], [40, 40, 50]];
    for peak in [1500.0 / 203.0, SUN_CAP] {
        for sky in SKIES {
            let (Some(bright), Some(mastered)) = (
                draw(true, &row, VERY_BRIGHT, sky, peak),
                draw(true, &row, VERY_BRIGHT, sky, S),
            ) else {
                eprintln!("celestial GPU unavailable");
                return;
            };
            for (ring, stored) in RINGS.iter().zip(&bright) {
                for c in 0..3 {
                    let vanilla = (sky.0[c] + f32::from(ring[c]) / 255.0).min(1.0);
                    let (got, want) = (stored[c].powf(2.2), peak * vanilla.powf(2.2));
                    assert!(
                        (got - want).abs() < 0.005 * peak,
                        "peak {peak}, sky {sky:?}, ring {ring:?}: {stored:?}"
                    );
                }
            }
            assert_eq!(
                bright[RINGS.len()..],
                mastered[RINGS.len()..],
                "peak {peak}, sky {sky:?}: the halo"
            );
            assert_eq!(
                draw(true, &moon, LOW, sky, peak),
                draw(true, &moon, LOW, sky, S),
                "peak {peak}, sky {sky:?}: the moon"
            );
        }
    }
}
