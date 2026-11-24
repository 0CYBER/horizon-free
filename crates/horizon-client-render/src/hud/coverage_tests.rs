use super::*;
use crate::gfx::hud_coverage;

#[test]
fn standalone_picture_draw_order_and_screen_exit_release() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(mut gpu) = Gpu::new() else { return };
    let image = Arc::new(pictures::Picture {
        id: u64::MAX,
        width: 1,
        height: 1,
        pixels: vec![255, 0, 0, 255],
        flags: TextureFlags::default(),
    });
    let weak = Arc::downgrade(&image);
    let mut batch = HudBatch::scratch([4, 2]);
    batch.pictures.push(image.clone());
    let rect = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 1,
        h: 1,
    };
    batch.rect_uv(0.0, 0.0, 4.0, 2.0, rect, u32::MAX);
    batch.blend_since(0, Blend::Picture(image.id));
    batch.rect_uv(
        2.0,
        0.0,
        2.0,
        2.0,
        Rect {
            u0: 0.0,
            u1: 0.5,
            ..rect
        },
        rgba(0, 255, 0, 255),
    );
    let mut uploads = crate::gfx::upload::Uploads::new(&gpu.device);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    gpu.hud
        .prepare_pictures(&gpu.device, &mut uploads, &mut encoder, &batch);
    uploads.finish(&encoder);
    gpu.queue.submit([encoder.finish()]);
    let (pixels, mask) = gpu.draw(&batch, 0.0, 0.0, true);
    for row in pixels.as_chunks::<16>().0 {
        assert_eq!(&row[..8], &[255, 0, 0, 255, 255, 0, 0, 255]);
        assert_eq!(&row[8..], &[0, 255, 0, 255, 0, 255, 0, 255]);
    }
    assert!(mask.iter().all(|v| *v == 255));
    drop(image);
    drop(batch);
    assert!(weak.upgrade().is_some());
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    gpu.hud.prepare_pictures(
        &gpu.device,
        &mut uploads,
        &mut encoder,
        &HudBatch::scratch([4, 2]),
    );
    assert!(gpu.hud.pictures.is_empty());
    assert!(
        weak.upgrade().is_none(),
        "closing the screen releases pixels and the GPU bind group"
    );
    eprintln!("standalone picture GPU rendering and release exercised");
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    hud: HudRenderer,
}

impl Gpu {
    fn new() -> Option<Self> {
        Self::with_pixels(2, 1, &[255, 255, 255, 255, 255, 255, 255, 0])
    }

    fn with_pixels(width: u32, height: u32, pixels: &[u8]) -> Option<Self> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        if !hud_coverage::supported(
            adapter.limits().max_color_attachments,
            adapter.get_downlevel_capabilities().flags,
            adapter.get_texture_format_features(hud_coverage::FORMAT),
        ) {
            eprintln!("skipping HUD coverage: adapter lacks independent blending or R8 rendering");
            return None;
        }
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).ok()?;
        let atlas = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            pixels,
        );
        let view = atlas.create_view(&Default::default());
        let hud = HudRenderer::new(
            &device,
            hud_coverage::HUD_FORMAT,
            &view,
            &view,
            &view,
            &device.create_sampler(&Default::default()),
            true,
        )
        .unwrap();
        Some(Self { device, queue, hud })
    }

    fn texture(&self, format: wgpu::TextureFormat) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 4,
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Exercise the production HUD upload path, including standalone pictures and core uniforms.
    fn prepare(&mut self, batch: &HudBatch) {
        let mut uploads = crate::gfx::upload::Uploads::new(&self.device);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.hud
            .prepare(&self.device, &mut uploads, &mut encoder, batch, [4, 2]);
        uploads.finish(&encoder);
        self.queue.submit([encoder.finish()]);
    }

    /// `batch` drawn on the HUD's target cleared to `backdrop`, with coverage cleared to
    /// `initial`: the target's and the coverage's 8-bit texels.
    fn draw(
        &self,
        batch: &HudBatch,
        backdrop: f64,
        initial: f64,
        coverage: bool,
    ) -> (Vec<u8>, Vec<u8>) {
        let frame = self.texture(hud_coverage::HUD_FORMAT);
        let mask = self.texture(hud_coverage::FORMAT);
        let depth = self.texture(wgpu::TextureFormat::Depth32Float);
        let [frame_view, mask_view, depth_view] =
            [&frame, &mask, &depth].map(|t| t.create_view(&Default::default()));
        self.queue
            .write_buffer(&self.hud.buffer, 0, bytemuck::cast_slice(&batch.vertices));
        self.queue.write_buffer(
            &self.hud.uniform,
            0,
            bytemuck::bytes_of(&HudUniform::new([4.0, 2.0, 1.0, 0.75], batch.end_portal, 0)),
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for (view, value) in [(&frame_view, backdrop), (&mask_view, initial)] {
            let mut attachment = hud_coverage::attachment(view);
            attachment.ops.load = wgpu::LoadOp::Clear(wgpu::Color {
                r: value,
                g: value,
                b: value,
                a: value,
            });
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(attachment)],
                ..Default::default()
            });
        }
        self.hud.draw_range(
            &mut encoder,
            &frame_view,
            coverage.then_some(&mask_view),
            &depth_view,
            batch,
            [4, 2],
            0..batch.vertices.len() as u32,
            None,
        );
        let buffers = [&frame, &mask].map(|t| {
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 512,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                t.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: None,
                    },
                },
                t.size(),
            );
            buffer
        });
        self.queue.submit([encoder.finish()]);
        let [color, mask] = buffers.map(|buffer| {
            buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
            self.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            buffer.get_mapped_range(..).unwrap().to_vec()
        });
        (
            color
                .chunks(256)
                .flat_map(|r| r[..16].iter().copied())
                .collect(),
            mask.chunks(256)
                .flat_map(|r| r[..4].iter().copied())
                .collect(),
        )
    }

    fn check(&self, batch: &HudBatch, initial: f64, expected: [u8; 4]) {
        let (plain, _) = self.draw(batch, 0.3, initial, false);
        let (color, mask) = self.draw(batch, 0.3, initial, true);
        assert_eq!(color, plain, "coverage changed the primary colour or alpha");
        for (got, want) in mask.iter().zip(expected.repeat(2)) {
            assert!(
                got.abs_diff(want) <= 1,
                "coverage {mask:?}, expected {expected:?}"
            );
        }
    }
}

fn quad(batch: &mut HudBatch, blend: Blend, color: u32, x: f32, uv: f32) {
    let start = batch.vertices.len();
    batch.quad(
        [[x, 0.0], [x + 2.0, 0.0], [x + 2.0, 2.0], [x, 2.0]],
        [[uv, 0.5]; 4],
        color,
    );
    batch.blend_since(start, blend);
}

#[test]
fn coverage_follows_hud_blends_discard_scissor_and_depth_without_changing_colour() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    for (blend, color, initial, count, expected) in [
        (Blend::Alpha, rgba(255, 255, 255, 128), 0.0, 2, 192),
        (Blend::Additive, rgba(255, 255, 255, 128), 0.3, 1, 77),
        (Blend::Invert, rgba(255, 255, 255, 32), 0.0, 1, 255),
        (Blend::EndPortal, rgba(255, 255, 255, 255), 0.0, 1, 255),
        (Blend::Vignette, rgba(102, 102, 102, 255), 0.0, 1, 102),
    ] {
        let mut batch = HudBatch::new(
            [4, 2],
            crate::hud::GuiScale {
                requested: 1,
                unicode: false,
            },
        );
        for _ in 0..count {
            quad(&mut batch, blend, color, 0.0, 0.25);
        }
        // Crosshair fragments with zero texture alpha discard; EndPortal has no discard.
        if blend == Blend::Invert {
            quad(&mut batch, blend, color, 2.0, 0.75);
        }
        gpu.check(
            &batch,
            initial,
            [
                expected,
                expected,
                (initial * 255.0).round() as u8,
                (initial * 255.0).round() as u8,
            ],
        );
    }
    let mut batch = HudBatch::new(
        [4, 2],
        crate::hud::GuiScale {
            requested: 1,
            unicode: false,
        },
    );
    for depth in [0.25, 0.75] {
        quad(
            &mut batch,
            Blend::Alpha,
            rgba(255, 255, 255, 128),
            0.0,
            0.25,
        );
        let start = batch.vertices.len() - 6;
        for vertex in &mut batch.vertices[start..] {
            vertex.depth = depth;
        }
    }
    batch.blends.push((0..12, Blend::Depth([1, 0, 2, 2])));
    gpu.check(&batch, 0.0, [0, 128, 0, 0]);
}

/// The HUD's target is 8-bit as vanilla's frame is, so each draw clips at white before the
/// next blends over it: a text highlight added over a light backdrop, then a 50% black panel.
#[test]
fn each_hud_draw_clips_at_white_as_on_an_8_bit_frame() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let mut batch = HudBatch::scratch([4, 2]);
    quad(
        &mut batch,
        Blend::Additive,
        rgba(128, 128, 128, 255),
        0.0,
        0.25,
    );
    quad(&mut batch, Blend::Alpha, rgba(0, 0, 0, 128), 0.0, 0.25);
    // Vanilla's frame in integers: 204 + 128 clips at 255, then the panel keeps 127/255 of it.
    let (backdrop, highlight, panel) = (204u32, 128u32, 128u32);
    let clipped = (backdrop + highlight).min(255);
    let want = ((clipped * (255 - panel)) as f64 / 255.0).round() as u8;
    assert_eq!(want, 127);
    // The HUD layer's target, and the 8-bit frame a 0.2 pack finishes on in SDR, where the
    // HUD draws without coverage.
    for coverage in [true, false] {
        let (color, mask) = gpu.draw(&batch, f64::from(backdrop) / 255.0, 0.0, coverage);
        // Unclipped, the panel would have kept 127/255 of 332: 165.
        for pixel in [0, 1, 4, 5] {
            assert_eq!(
                color[pixel * 4..pixel * 4 + 3],
                [want; 3],
                "{coverage}: {color:?}"
            );
        }
        // The highlight hides nothing; the panel hides half of what is under it.
        if coverage {
            assert_eq!(mask[0], 128, "{mask:?}");
        }
    }
}

#[test]
fn coverage_preserves_each_cit_blend_and_its_destination_alpha() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(mut gpu) = Gpu::new() else {
        return;
    };
    use horizon_optifine::cit::parse_blend;
    for (name, expected) in [
        ("alpha", 128),
        ("add", 0),
        ("dodge", 0),
        ("glint", 0),
        ("burn", 102),
        ("multiply", 26),
        ("overlay", 51),
        ("screen", 255),
        ("subtract", 255),
        ("replace", 255),
        ("0 768", 153),
        ("774 0", 153),
        ("770 771 772 773", 128),
    ] {
        let textures = cit_glint::GlintTextures {
            width: 1,
            height: 1,
            pixels: vec![102, 102, 102, 128],
            slots: vec![cit_glint::Slot {
                rgba: [1.0; 4],
                blur: false,
                blend: 0,
            }],
            blends: vec![parse_blend(name).unwrap()],
            ..Default::default()
        };
        gpu.hud
            .set_cit_glints(&gpu.device, &gpu.queue, Some(&textures));
        let mut batch = HudBatch::new(
            [4, 2],
            crate::hud::GuiScale {
                requested: 1,
                unicode: false,
            },
        );
        quad(
            &mut batch,
            Blend::CitGlint(0),
            rgba(0, 0, 0, 255),
            0.0,
            0.25,
        );
        quad(
            &mut batch,
            Blend::CitGlint(0),
            rgba(0, 0, 0, 255),
            2.0,
            0.75,
        );
        gpu.check(&batch, 0.0, [expected, expected, 0, 0]);
    }
}

#[test]
fn coverage_uses_pack_texts_final_alpha_and_discard() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(mut gpu) = Gpu::new() else {
        return;
    };
    // A translated-style struct result and early return; no dependency on game resources.
    let program = crate::core_shader::CoreProgram {
        vertex: naga::front::wgsl::parse_str("@vertex fn main(@location(0) p: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4(p.x / 2.0 - 1.0, 1.0 - p.y, 0.0, 1.0); }").unwrap(),
        fragment: naga::front::wgsl::parse_str("struct Result { @location(0) color: vec4<f32> } @fragment fn main(@builtin(position) p: vec4<f32>) -> Result { if p.x > 2.0 { discard; } return Result(vec4(0.4, 0.2, 0.1, 0.5)); }").unwrap(),
        other: Vec::new(),
    };
    gpu.hud.set_text_shader(&gpu.device, Some(&program));
    assert!(gpu.hud.pack_text.is_some());
    let mut batch = HudBatch::new(
        [4, 2],
        crate::hud::GuiScale {
            requested: 1,
            unicode: false,
        },
    );
    for x in [0.0, 2.0, 0.0, 2.0] {
        quad(&mut batch, Blend::Alpha, rgba(255, 255, 255, 255), x, 0.25);
    }
    for v in &mut batch.vertices {
        v.text = 1;
    }
    gpu.check(&batch, 0.0, [191, 191, 0, 0]);
}

#[test]
fn a_pack_text_shader_without_a_colour_output_is_dropped_not_wrapped() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(mut gpu) = Gpu::new() else {
        return;
    };
    let program = crate::core_shader::CoreProgram {
        vertex: naga::front::wgsl::parse_str("@vertex fn main(@location(0) p: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4(p, 0.0, 1.0); }").unwrap(),
        fragment: naga::front::wgsl::parse_str("@fragment fn main() -> @location(1) vec4<f32> { return vec4(1.0); }").unwrap(),
        other: Vec::new(),
    };
    gpu.hud.set_text_shader(&gpu.device, Some(&program));
    assert!(gpu.hud.pack_text.is_none());
}

/// A pack's `position_tex_color` draws a `PackBlit` with its picture as `Sampler0`, in place of
/// the native tiles before it (where it discards, nothing shows); without the shader the tiles
/// draw and the quad doesn't.
#[test]
fn a_pack_blit_replaces_its_native_tiles_only_with_the_packs_shader() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(mut gpu) = Gpu::new() else { return };
    let image = Arc::new(pictures::Picture {
        id: u64::MAX - 1,
        width: 1,
        height: 1,
        pixels: vec![255, 0, 0, 255],
        flags: TextureFlags::default(),
    });
    let mut batch = HudBatch::scratch([4, 2]);
    batch.pictures.push(image.clone());
    let rect = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 0.5,
        v1: 1.0,
        w: 1,
        h: 1,
    };
    batch.rect_uv(0.0, 0.0, 4.0, 2.0, rect, rgba(0, 255, 0, 255));
    batch.blend_since(0, Blend::PackBlitNative(image.id));
    let start = batch.vertices.len();
    batch.core_quad(
        [[0.0, 0.0], [0.0, 2.0], [4.0, 2.0], [4.0, 0.0]],
        [[0.0, 0.0], [0.0, 1.0], [0.5, 1.0], [0.5, 0.0]],
        [u32::MAX; 4],
    );
    batch.blend_since(start, Blend::PackBlit(image.id));
    let draw = |gpu: &mut Gpu| {
        let mut uploads = crate::gfx::upload::Uploads::new(&gpu.device);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        gpu.hud
            .prepare_pictures(&gpu.device, &mut uploads, &mut encoder, &batch);
        uploads.finish(&encoder);
        gpu.queue.submit([encoder.finish()]);
        gpu.draw(&batch, 0.0, 0.0, true).0
    };
    assert!(
        draw(&mut gpu)
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0, 255, 0, 255])
    );
    let program = crate::core_shader::CoreProgram {
        vertex: naga::front::wgsl::parse_str("@vertex fn main(@location(0) p: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4(p.x / 2.0 - 1.0, 1.0 - p.y, 0.0, 1.0); }").unwrap(),
        fragment: naga::front::wgsl::parse_str("@group(0) @binding(4) var t: texture_2d<f32>; @group(0) @binding(5) var s: sampler; @fragment fn main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> { if p.x > 2.0 { discard; } return textureSample(t, s, vec2(0.5)); }").unwrap(),
        other: Vec::new(),
    };
    gpu.hud.set_blit_shader(&gpu.device, Some(&program));
    for row in draw(&mut gpu).as_chunks::<16>().0 {
        assert_eq!(&row[..8], &[255, 0, 0, 255, 255, 0, 0, 255]);
        assert_eq!(&row[8..], &[0; 8]);
    }
}

#[test]
fn core_item_draws_the_baked_tint_and_stitched_item_sheet() {
    use crate::gfx::core_models::{Gui, ModelShaders, texture};
    use horizon_assets::resource_source::{Resource, ResourceSource};

    let _gpu = crate::gfx::test_gpu::lock();
    let Some(mut gpu) = Gpu::new() else { return };
    let vertex = r#"
layout(std140) uniform Projection {
    mat4 ProjMat;
};
in vec3 Position;
in vec4 Color;
in vec2 UV0;
in vec3 Normal;
out vec2 uv;
out vec4 tint;
void main() {
    gl_Position = ProjMat * vec4(Position + vec3(Normal.z, 0.0, 0.0), 1.0);
    uv = UV0;
    tint = Color;
}
"#;
    let fragment = r#"
uniform sampler2D Sampler0;
uniform sampler2D Sampler1;
uniform sampler2D Sampler2;
in vec2 uv;
in vec4 tint;
out vec4 fragColor;
void main() {
    if (textureSize(Sampler0, 0) != ivec2(2, 1)) discard;
    vec4 material = texture(Sampler0, uv);
    float overlay = texelFetch(Sampler1, ivec2(0, 10), 0).a;
    float light = texelFetch(Sampler2, ivec2(15), 0).b;
    fragColor = vec4(tint.r, material.g, overlay * light, material.a);
}
"#;
    let pack = ResourceSource::indexed(
        [
            (
                "assets/minecraft/shaders/core/item.vsh".into(),
                Resource::bytes(vertex.as_bytes().to_vec()),
            ),
            (
                "assets/minecraft/shaders/core/item.fsh".into(),
                Resource::bytes(fragment.as_bytes().to_vec()),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let mut atlas = HudAtlas::boot();
    atlas.model_shaders = ModelShaders::load(&pack, &ResourceSource::indexed(Default::default()));
    atlas.width = 4;
    atlas.height = 1;
    let rect = |u0, u1| Rect {
        u0,
        u1,
        v0: 0.0,
        v1: 1.0,
        w: 1,
        h: 1,
    };
    atlas.sprites = [
        (MISSING_SPRITE.to_string(), rect(0.0, 0.25)),
        ("item/test".into(), rect(0.75, 1.0)),
    ]
    .into_iter()
    .collect();
    let layouts = [
        ("missingno".into(), rect(0.0, 0.5)),
        ("item/test".into(), rect(0.5, 1.0)),
    ]
    .into_iter()
    .collect();
    let image = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 4,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let light = texture(&gpu.device, "synthetic lightmap", [16, 16]);
    let write = |texture: &wgpu::Texture, pixels: &[u8]| {
        gpu.queue.write_texture(
            texture.as_image_copy(),
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(texture.width() * 4),
                rows_per_image: None,
            },
            texture.size(),
        );
    };
    write(
        &image,
        &[
            255, 0, 255, 255, 40, 80, 160, 255, 5, 10, 15, 255, 60, 120, 250, 255,
        ],
    );
    write(&light, &[255, 255, 96, 255].repeat(16 * 16));
    let [view, light_view] = [&image, &light].map(|t| t.create_view(&Default::default()));
    let sampler = gpu.device.create_sampler(&Default::default());
    gpu.hud = HudRenderer::new(
        &gpu.device,
        hud_coverage::HUD_FORMAT,
        &view,
        &view,
        &view,
        &sampler,
        true,
    )
    .unwrap();
    let tint = rgba(32, 200, 20, 255);
    let points = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
    let mut stock = HudBatch::scratch([4, 2]);
    stock.quad(points, [[0.875, 0.5]; 4], tint);
    let (control, _) = gpu.draw(&stock, 0.0, 0.0, true);
    assert_eq!(&control[0..4], &[8, 94, 20, 255]);
    assert_eq!(&control[8..12], &[0; 4]);

    let (gui, installed) = Gui::new(
        &gpu.device,
        &gpu.queue,
        hud_coverage::HUD_FORMAT,
        true,
        &image,
        &atlas,
        &layouts,
        [[glam::Vec3::Z; 2]; 2],
        (&light_view, &sampler),
    )
    .unwrap();
    gpu.hud.pack_item = Some(gui);
    let mut packed = HudBatch::scratch([4, 2]);
    let r = installed["item/test"];
    packed.item_model_quad(
        points.map(|[x, y]| [x, y, 0.0]),
        [[(r.u0 + r.u1) * 0.5, 0.5]; 4],
        [0.0, 0.0, 1.0],
        tint,
        false,
    );
    let mut uploads = crate::gfx::upload::Uploads::new(&gpu.device);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    gpu.hud
        .prepare(&gpu.device, &mut uploads, &mut encoder, &packed, [4, 2]);
    uploads.finish(&encoder);
    gpu.queue.submit([encoder.finish()]);
    let (colors, mask) = gpu.draw(&packed, 0.0, 0.0, true);
    assert_eq!(
        &colors[0..4],
        &[0; 4],
        "the real normal shifts the icon one pixel"
    );
    assert_eq!(
        &colors[8..12],
        &[32, 120, 96, 255],
        "raw tint, item material, overlay and lightmap"
    );
    assert_eq!(mask, [0, 255, 255, 0].repeat(2));
}

fn write_gui_fixture_shader(root: &Path, name: &str, vertex: &str, fragment: &str) {
    let dir = root.join("assets/minecraft/shaders/core");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{name}.vsh")), vertex).unwrap();
    fs::write(dir.join(format!("{name}.fsh")), fragment).unwrap();
}

#[test]
fn loaded_core_gui_preserves_gradient_scissor_and_hdr_coverage() {
    let _gpu = crate::gfx::test_gpu::lock();
    let stock = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    let stock_src = crate::resource_source::ResourceSource::directory(stock.path().to_owned());
    let pack_src = crate::resource_source::ResourceSource::directory(pack.path().to_owned());
    write_gui_fixture_shader(
        pack.path(),
        "gui",
        "#version 330\n\
         layout(std140) uniform Projection { mat4 ProjMat; };\n\
         layout(std140) uniform DynamicTransforms { mat4 ModelViewMat; };\n\
         in vec3 Position; in vec4 Color; out vec4 shade;\n\
         void main() { shade = Color; gl_Position = ProjMat * ModelViewMat * vec4(Position.xy + vec2(1.0, 0.0), 1.0, 1.0); }\n",
        "#version 330\n\
         in vec4 shade; out vec4 frag;\n\
         void main() { frag = vec4(shade.bgr, 0.5); }\n",
    );
    let stock = HudAtlas::build(&stock_src).unwrap();
    let atlas = HudAtlas::build_over(&pack_src, &stock_src).unwrap();
    assert!(stock.gui_shader.is_none());
    let shader = atlas
        .gui_shader
        .as_deref()
        .expect("core/gui in the pack is loaded");
    let Some(mut gpu) = Gpu::with_pixels(atlas.width, atlas.height, &atlas.pixels()) else {
        return;
    };
    let draw = |atlas: &HudAtlas, gpu: &mut Gpu| {
        let mut batch = HudBatch::scratch([4, 2]);
        batch.fill_gradient(
            atlas,
            (0.0, 0.0, 3.0, 2.0),
            rgba(255, 0, 0, 255),
            rgba(0, 0, 255, 255),
        );
        batch.clip_since(0, 1.0, 0.0, 3.0, 2.0);
        // An ordinary atlas quad immediately after the clipped draw must not inherit its scissor
        // or be sent to the fill program merely because its UVs sample white.
        batch.rect_uv(3.0, 0.0, 1.0, 2.0, atlas.white, rgba(0, 255, 0, 255));
        gpu.prepare(&batch);
        let (plain, _) = gpu.draw(&batch, 0.0, 0.0, false);
        let (color, mask) = gpu.draw(&batch, 0.0, 0.0, true);
        assert_eq!(color, plain);
        (color, mask)
    };
    let check = |(pixels, mask): (Vec<u8>, Vec<u8>), top: [u8; 4], bottom: [u8; 4]| {
        let px = |c: [u8; 4]| [[0; 4], c, c, [0, 255, 0, 255]];
        let expected = [px(top), px(bottom)].into_iter().flatten().flatten();
        assert_eq!(pixels.len(), 32);
        assert!(
            pixels
                .iter()
                .copied()
                .zip(expected)
                .all(|(p, w)| p.abs_diff(w) <= 1),
            "{pixels:?}"
        );
        let expected = [0, top[3], top[3], 255, 0, bottom[3], bottom[3], 255];
        assert_eq!(mask.len(), 8);
        assert!(
            mask.iter()
                .copied()
                .zip(expected)
                .all(|(p, w)| p.abs_diff(w) <= 1),
            "{mask:?}"
        );
    };
    check(draw(&stock, &mut gpu), [191, 0, 64, 255], [64, 0, 191, 255]);
    gpu.hud.set_gui_shader(&gpu.device, Some(shader));
    assert!(gpu.hud.pack_gui.is_some());
    // At the row centres the original gradient is 3/4 red or 3/4 blue. The shader reverses
    // RGB and halves alpha; alpha blending over clear then halves each channel.
    check(draw(&atlas, &mut gpu), [32, 0, 96, 128], [96, 0, 32, 128]);
}

#[test]
fn special_blend_fills_record_one_run_with_core_gui() {
    let stock = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    write_gui_fixture_shader(
        pack.path(),
        "gui",
        "#version 330\n\
         layout(std140) uniform Projection { mat4 ProjMat; };\n\
         layout(std140) uniform DynamicTransforms { mat4 ModelViewMat; };\n\
         in vec3 Position; in vec4 Color; out vec4 shade;\n\
         void main() { shade = Color; gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0); }\n",
        "#version 330\n\
         in vec4 shade; out vec4 frag;\n\
         void main() { frag = shade; }\n",
    );
    let atlas = HudAtlas::build_over(
        &crate::resource_source::ResourceSource::directory(pack.path().to_owned()),
        &crate::resource_source::ResourceSource::directory(stock.path().to_owned()),
    )
    .unwrap();
    assert!(atlas.gui_shader.is_some());
    let mut batch = HudBatch::scratch([4, 2]);
    batch.fill(&atlas, 0.0, 0.0, 4.0, 2.0, u32::MAX);
    // The End poem's portal background, then an inverting crosshair run.
    batch.fill_native(&atlas, [0.0, 0.0, 4.0, 2.0], u32::MAX, Blend::EndPortal);
    batch.fill_native(&atlas, [1.0, 0.0, 2.0, 1.0], u32::MAX, Blend::Invert);
    assert_eq!(
        batch.runs(),
        vec![
            (0..6, Blend::GuiFill(GuiFill::Alpha, None)),
            (6..12, Blend::EndPortal),
            (12..18, Blend::Invert),
        ]
    );
    assert_eq!(batch.blends.len(), 3, "no vertex is under two blends");
}

#[test]
fn loaded_gui_textured_blit_samples_and_rescales_the_original_inventory_texture() {
    let _gpu = crate::gfx::test_gpu::lock();
    let root = tempfile::tempdir().unwrap();
    let source = crate::resource_source::ResourceSource::directory(root.path().to_owned());
    let texture = "gui/container/inventory";
    let image = root
        .path()
        .join(format!("assets/minecraft/textures/{texture}.png"));
    fs::create_dir_all(image.parent().unwrap()).unwrap();
    let mut png = png::Encoder::new(fs::File::create(image).unwrap(), 8, 2);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    let colors = [
        [255, 0, 255, 255],
        [0, 255, 255, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
    ];
    let row = [
        colors[0], colors[0], colors[1], colors[1], colors[2], colors[2], colors[3], colors[3],
    ];
    png.write_header()
        .unwrap()
        .write_image_data([row, row].as_flattened().as_flattened())
        .unwrap();
    let stock = HudAtlas::build(&source).unwrap();
    let pack = tempfile::tempdir().unwrap();
    write_gui_fixture_shader(
        pack.path(),
        "position_tex_color",
        "#version 330\n\
         layout(std140) uniform Projection { mat4 ProjMat; };\n\
         layout(std140) uniform DynamicTransforms { mat4 ModelViewMat; };\n\
         uniform sampler2D Sampler0;\n\
         in vec3 Position; in vec4 Color; in vec2 UV0; out vec2 uv;\n\
         void main() {\n\
             uv = UV0 * 2.0; vec2 p = Position.xy * 2.0;\n\
             if (textureSize(Sampler0, 0) != ivec2(8, 2) || texelFetch(Sampler0, ivec2(0, 1), 0).rb != vec2(1.0)) { p.x += 16.0; }\n\
             gl_Position = ProjMat * ModelViewMat * vec4(p, 1.0, 1.0);\n\
         }\n",
        "#version 330\n\
         uniform sampler2D Sampler0; in vec2 uv; out vec4 frag;\n\
         void main() { frag = texture(Sampler0, uv); }\n",
    );
    let atlas = HudAtlas::build_over(
        &crate::resource_source::ResourceSource::directory(pack.path().to_owned()),
        &source,
    )
    .unwrap();
    let shader = atlas
        .blit_shader
        .as_deref()
        .expect("core/position_tex_color in the pack is loaded");
    let Some(mut gpu) = Gpu::with_pixels(stock.width, stock.height, &stock.pixels()) else {
        return;
    };
    let batch = |atlas: &HudAtlas| {
        let mut batch = HudBatch::scratch([4, 2]);
        // Same extraction as InventoryScreen and AbstractMountInventoryScreen: the source
        // uses nominal GUI units (here 4x2), not the PNG's physical dimensions (8x2).
        batch.texture_blit(atlas, texture, [0.0, 0.0, 2.0, 1.0], [0.0; 2], [4.0, 2.0]);
        batch
    };
    let native = batch(&stock);
    assert!(native.blends.is_empty() && native.pictures.is_empty());
    gpu.prepare(&native);
    assert_eq!(
        gpu.draw(&native, 0.0, 0.0, true),
        (
            [
                colors[0], colors[1], [0; 4], [0; 4], [0; 4], [0; 4], [0; 4], [0; 4]
            ]
            .concat(),
            vec![255, 255, 0, 0, 0, 0, 0, 0]
        ),
    );
    gpu.hud.set_blit_shader(&gpu.device, Some(shader));
    assert!(gpu.hud.pack_blit.is_some());
    let replaced = batch(&atlas);
    assert_eq!(replaced.pictures.len(), 1);
    gpu.prepare(&replaced);
    // The two-pixel native crop is replaced, not painted over another time: the vertex stage
    // expands it to four pixels; the source's UVs now read all four colors on both rows.
    assert_eq!(
        gpu.draw(&replaced, 0.0, 0.0, true),
        ([colors, colors].as_flattened().concat(), vec![255; 8]),
    );

    // The draw-list path also supports a source extent independent of the output rectangle.
    // Source x = 1..2 in a nominal width of 4; the shader doubles those UVs to 0.5..1.
    let mut stretched = HudBatch::scratch([4, 2]);
    crate::screen_draw::paint(
        &mut stretched,
        &atlas,
        &[crate::screen_draw::Draw::Blit {
            texture: format!("textures/{texture}.png"),
            x: 0,
            y: 0,
            w: 2,
            h: 1,
            u: 1.0,
            v: 0.0,
            src: [1, 1],
            tex: [4, 2],
            color: -1,
        }],
    );
    gpu.prepare(&stretched);
    assert_eq!(
        gpu.draw(&stretched, 0.0, 0.0, true),
        (
            [colors[2], colors[2], colors[3], colors[3]]
                .repeat(2)
                .concat(),
            vec![255; 8]
        ),
    );
}
