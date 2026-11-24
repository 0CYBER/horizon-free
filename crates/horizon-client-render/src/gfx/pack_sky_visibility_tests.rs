//! Original synthetic geometry visibility controls; no pack shader code or assets.
use super::*;
use glam::Mat4;
use wgpu::TextureFormat as T;

#[test]
#[ignore = "requires a Vulkan/Metal/DX12 adapter"]
fn pack_sun_and_moon_properties_control_independent_pixels() {
    let _gpu = crate::gfx::test_gpu::lock();
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("offscreen adapter");
    eprintln!("Sky visibility adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("offscreen device");
    let dir = std::env::temp_dir().join(format!("hz-sky-visibility-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("shaders")).unwrap();
    std::fs::write(
        dir.join("shaders/gbuffers_skytextured.vsh"),
        "#version 120\nvoid main(){gl_Position=gl_Vertex;gl_FrontColor=gl_Color;}",
    )
    .unwrap();
    std::fs::write(
        dir.join("shaders/gbuffers_skytextured.fsh"),
        "#version 120\nvoid main(){gl_FragData[0]=gl_Color;}",
    )
    .unwrap();
    let pack = horizon_shaders::Pack::open(&dir).unwrap();
    let inputs = horizon_shaders::GameInputs {
        celestial: Some(super::super::pack_sky::celestial_inputs()),
        ..Default::default()
    };
    let hud = crate::hud::HudBatch::new(
        [8, 1],
        crate::hud::GuiScale {
            requested: 0,
            unicode: false,
        },
    );
    let vertices: Vec<_> = [(-1.0, 0.0, 0xff0000ff), (0.0, 1.0, 0xff00ff00)]
        .into_iter()
        .flat_map(|(left, right, color)| {
            [
                [left, -1.0, 0.0],
                [right, -1.0, 0.0],
                [right, 1.0, 0.0],
                [left, 1.0, 0.0],
            ]
            .map(|pos| super::super::Vertex {
                pos,
                color,
                uv: [0.0; 2],
                layer: 0,
                light: 0,
                normal: [0; 4],
                glint_uv: [0.0; 2],
            })
        })
        .collect();
    let frame = frame(&hud, &vertices);
    let red = [255, 0, 0];
    let green = [0, 255, 0];
    let black = [0, 0, 0];
    for (properties, expected) in [
        ("", [red, green]),
        ("sun=false", [black, green]),
        ("moon=false", [red, black]),
        ("sun=false\nmoon=false", [black, black]),
        ("sun=true\nmoon=true", [red, green]),
    ] {
        let data = render_pack_sky(
            &device, &queue, &adapter, &pack, &inputs, &frame, properties,
        );
        for (pixel, color) in [(2, expected[0]), (6, expected[1])] {
            assert_eq!(data[pixel], color, "properties={properties}, pixel={pixel}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn render_pack_sky(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    adapter: &wgpu::Adapter,
    pack: &horizon_shaders::Pack,
    inputs: &horizon_shaders::GameInputs,
    frame: &super::super::Frame<'_>,
    properties: &str,
) -> [[u8; 3]; 8] {
    let macros = horizon_shaders::Macros::standard();
    let properties = horizon_shaders::Properties::parse(properties, &macros);
    let loaded = horizon_shaders::load_post(pack, "", &properties, &macros, inputs);
    assert!(
        loaded.sky.is_some() || loaded.sky_textured.is_some(),
        "{:?}",
        loaded.notes
    );
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: T::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let world = World {
        atlas: None,
        sprite_rects: &[],
        material_maps: None,
        lightmap: texture.create_view(&Default::default()),
        terrain: None,
        block_ids: &[],
        celestial: None,
        gui_size: [8, 1],
        clouds: None,
        weather: false,
        hand: false,
    };
    let mut post = PackPost::new(
        device,
        queue,
        adapter,
        loaded,
        T::Rgba8Unorm,
        world,
        &Default::default(),
    )
    .unwrap_or_else(|e| panic!("pack refused: {:?}", e.notes));
    post.copied |= 1;
    post.resize(device, queue, [8, 1]);
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let targets = post.targets.as_ref().unwrap();
        let view = &targets.color[0].as_ref().unwrap()[0];
        let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[attach(
                view,
                wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            )],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
    }
    post.draw_sky(device, queue, &mut encoder, [8, 1], frame);
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        post.targets.as_ref().unwrap().color[0].as_ref().unwrap()[0]
            .texture()
            .as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                bytes_per_row: Some(256),
                ..Default::default()
            },
        },
        wgpu::Extent3d {
            width: 8,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = read.slice(..).get_mapped_range().unwrap();
    std::array::from_fn(|pixel| data[pixel * 4..pixel * 4 + 3].try_into().unwrap())
}

#[test]
#[ignore = "requires a Vulkan/Metal/DX12 adapter"]
fn pack_sky_property_hides_disc_horizon_sunset_and_void_pixels() {
    let _gpu = crate::gfx::test_gpu::lock();
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("offscreen adapter");
    eprintln!("Sky group adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("offscreen device");
    let dir = std::env::temp_dir().join(format!("hz-sky-group-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("shaders")).unwrap();
    // Project the actual sky geometry onto three disjoint strips. The independent
    // stage colours make a retained sunset/void visible even when SKY was removed.
    std::fs::write(
        dir.join("shaders/gbuffers_skybasic.vsh"),
        r#"#version 120
uniform int renderStage;
void main() {
    vec2 p = gl_Vertex.xz / 512.0;
    float offset = -0.66;
    gl_FrontColor = vec4(1.0, 0.0, 0.0, 1.0);
    if (renderStage == MC_RENDER_STAGE_SUNSET) {
        p = gl_Vertex.xz / 120.0;
        offset = 0.0;
        gl_FrontColor = vec4(0.0, 1.0, 0.0, 1.0);
    } else if (renderStage == MC_RENDER_STAGE_VOID) {
        offset = 0.66;
        gl_FrontColor = vec4(0.0, 0.0, 1.0, 1.0);
    }
    gl_Position = vec4(p * vec2(0.28, 1.0) + vec2(offset, 0.0), 0.0, 1.0);
}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("shaders/gbuffers_skybasic.fsh"),
        "#version 120\nvoid main(){gl_FragData[0]=gl_Color;}",
    )
    .unwrap();
    let pack = horizon_shaders::Pack::open(&dir).unwrap();
    let inputs = horizon_shaders::GameInputs {
        sky: Some(super::super::pack_sky::inputs()),
        ..Default::default()
    };
    let hud = crate::hud::HudBatch::new(
        [8, 1],
        crate::hud::GuiScale {
            requested: 0,
            unicode: false,
        },
    );
    let mut frame = frame(&hud, &[]);
    frame.sky.sunrise_color = [1.0; 4];
    frame.dark_disc = true;
    for (properties, visible) in [
        ("", true),
        ("sky=false", false),
        ("sky=true", true),
        ("stars=false\nsun=false\nmoon=false", true),
    ] {
        let data = render_pack_sky(
            &device, &queue, &adapter, &pack, &inputs, &frame, properties,
        );
        for (pixel, color) in [(1, [255, 0, 0]), (4, [0, 255, 0]), (6, [0, 0, 255])] {
            assert_eq!(
                data[pixel],
                if visible { color } else { [0; 3] },
                "properties={properties}, pixel={pixel}"
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn frame<'a>(
    hud: &'a crate::hud::HudBatch,
    celestial: &'a [super::super::Vertex],
) -> super::super::Frame<'a> {
    super::super::Frame {
        camera: Default::default(),
        yaw: Default::default(),
        pitch: Default::default(),
        fov_degrees: Default::default(),
        eye: Default::default(),
        eye_rotation: Default::default(),
        first_person: Default::default(),
        bob: Mat4::IDENTITY,
        spin: Mat4::IDENTITY,
        sky: Default::default(),
        shadow_scene_revision: Default::default(),
        shadow_cache: Default::default(),
        distance: Default::default(),
        detail_distance: 128.0,
        depth_far: 1024.0,
        underwater: Default::default(),
        water_plane: Default::default(),
        cave_culling: Default::default(),
        title: Default::default(),
        gui_only: false,
        sky_blocked: Default::default(),
        skybox: true,
        end_sky: Default::default(),
        end_sky_sprite: Default::default(),
        boss_world_fog: Default::default(),
        environment_fog: Default::default(),
        water_fog: Default::default(),
        fog: Default::default(),
        dark_disc: Default::default(),
        outline: Default::default(),
        outline_after_terrain: Default::default(),
        outline_high_contrast: Default::default(),
        pvp: Default::default(),
        debug_lines: Default::default(),
        debug_lines_on_top: Default::default(),
        debug_crosshair: Default::default(),
        hud,
        glint_alpha: Default::default(),
        glint_ticks: Default::default(),
        nether_lighting: Default::default(),
        entities: Default::default(),
        static_block_entities: Default::default(),
        visible_static_block_entity_sections: Default::default(),
        static_block_entity_stats: Default::default(),
        entity_motion: Default::default(),
        entity_motion_posed: Default::default(),
        glowing: Default::default(),
        entity_spans: Default::default(),
        skin: Default::default(),
        weather: Default::default(),
        weather_time: crate::weather::Time::new(0, 0.0),
        dynamic_lights: Default::default(),
        colored_lights: Default::default(),
        emissive_layers: Default::default(),
        clouds: Default::default(),
        lightning: Default::default(),
        additive: Default::default(),
        see_through: Default::default(),
        world_primitives: Default::default(),
        shadow_only: Default::default(),
        celestial,
        stars: Default::default(),
        hand: Default::default(),
        hand_items: Default::default(),
        hand_fov_degrees: Default::default(),
        screen: Default::default(),
        pack: Default::default(),
        border: Default::default(),
        game_time: Default::default(),
        shader_game_time: Default::default(),
        end_portal_sprites: Default::default(),
        crumbling_sprite: Default::default(),
        ticks: Default::default(),
        darkness: Default::default(),
        night_vision: Default::default(),
        block_flicker: Default::default(),
        boss_darken: Default::default(),
        menu_blur: Default::default(),
        chunk_fade: Default::default(),
        post_effects: crate::post_chain::Request {
            effects: &[],
            root: crate::post_chain::Request::none().root,
            menu_blur_radius: 0.0,
            globals: Default::default(),
            extras: None,
        },
    }
}

#[test]
fn pack_visibility_selects_geometry_without_changing_order_or_unrelated_parts() {
    let hud = crate::hud::HudBatch::new(
        [8, 1],
        crate::hud::GuiScale {
            requested: 0,
            unicode: false,
        },
    );
    let mut frame = frame(&hud, &[]);
    frame.sky.sunrise_color[3] = 1.0;
    frame.stars = Some((Mat4::IDENTITY, 1.0));
    frame.dark_disc = true;
    for mask in 0u8..16 {
        let visibility = horizon_shaders::SkyVisibility {
            sky: mask & 1 != 0,
            stars: mask & 2 != 0,
            sun: mask & 4 != 0,
            moon: mask & 8 != 0,
        };
        let selected = super::super::pack_sky::parts(
            [20..30, 30..40, 40..50, 50..60, 60..70],
            &frame,
            2,
            visibility,
        );
        let mut expected = Vec::new();
        if mask & 1 != 0 {
            expected.push(("SKY", false, 20..30));
        }
        if mask & 1 != 0 {
            expected.push(("SUNSET", false, 30..40));
        }
        if mask & 4 != 0 {
            expected.push(("SUN", true, 0..6));
        }
        if mask & 8 != 0 {
            expected.push(("MOON", true, 6..12));
        }
        if mask & 2 != 0 {
            expected.push(("STARS", false, 40..50));
        }
        if mask & 1 != 0 {
            expected.push(("VOID", false, 50..60));
        }
        assert_eq!(
            selected
                .into_iter()
                .map(|p| (p.stage, p.textured, p.range))
                .collect::<Vec<_>>(),
            expected,
            "mask={mask}"
        );
        frame.end_sky = true;
        frame.boss_world_fog = true;
        let fan = super::super::pack_sky::parts(
            [20..30, 30..40, 40..50, 50..60, 60..70],
            &frame,
            2,
            visibility,
        );
        assert_eq!(fan.len(), 1);
        assert_eq!(fan[0].stage, "SKY");
        assert_eq!(fan[0].range, 60..70);
        frame.end_sky = false;
        frame.boss_world_fog = false;
    }
    frame.sky.sunrise_color[3] = 0.0;
    frame.stars = None;
    frame.dark_disc = false;
    let parts = super::super::pack_sky::parts(
        [20..30, 30..40, 40..50, 50..60, 60..70],
        &frame,
        0,
        Default::default(),
    );
    assert_eq!(
        parts.into_iter().map(|p| p.stage).collect::<Vec<_>>(),
        ["SKY"]
    );
}

#[test]
fn a_pack_switch_and_a_pack_sky_each_hide_their_own_body() {
    use super::super::pack_visibility::visible_bodies;
    use crate::mesher::{LOW, VERY_BRIGHT, Vertex};
    let quad = |tier: u8| {
        let mut v: Vertex = bytemuck::Zeroable::zeroed();
        v.layer = 2 | u32::from(tier) << 16;
        [v; 4]
    };
    // The sun's quad first, the moon's last (`Game::celestial_geometry`).
    let sun_and_moon = [quad(VERY_BRIGHT), quad(LOW)].concat();
    let switches = |sun, moon| horizon_shaders::SkyVisibility {
        sun,
        moon,
        ..Default::default()
    };
    let drawn =
        |own: (bool, bool), visibility| visible_bodies(&sun_and_moon, false, own, visibility);
    // Nothing hides anything.
    assert_eq!(drawn((false, false), switches(true, true)), 0..12);
    // The switch hides one body, the pack's own sky the other.
    assert!(drawn((true, false), switches(true, false)).is_empty());
    assert!(drawn((false, true), switches(false, true)).is_empty());
    // Each alone leaves the other.
    assert_eq!(drawn((true, false), switches(true, true)), 6..12);
    assert_eq!(drawn((false, true), switches(true, true)), 0..6);
    assert_eq!(drawn((false, false), switches(false, true)), 6..12);
    assert_eq!(drawn((false, false), switches(true, false)), 0..6);
    // The End's flashes are neither a sun nor a moon: they keep their whole range.
    let flashes = [quad(VERY_BRIGHT), quad(VERY_BRIGHT)].concat();
    assert_eq!(
        visible_bodies(&flashes, true, (true, true), switches(false, false)),
        0..12
    );
    assert!(visible_bodies(&[], false, (false, false), switches(true, true)).is_empty());
}

#[test]
#[ignore = "requires a Vulkan/Metal/DX12 adapter"]
fn pack_sky_property_gates_native_fallback_submissions() {
    let _gpu = crate::gfx::test_gpu::lock();
    use wgpu::util::DeviceExt;
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("offscreen adapter");
    eprintln!("Native sky gate adapter: {:?}", adapter.get_info());
    let (device, queue) =
        pollster::block_on(adapter.request_device(&Default::default())).expect("offscreen device");
    // Original marker pipelines exercise the production helper used by the native
    // disc, sunrise vertex fan and void. This does not construct a full Renderer.
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(
            r#"
@vertex fn generated(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = array<vec2f, 3>(vec2f(-1., -1.), vec2f(3., -1.), vec2f(-1., 3.));
    return vec4f(p[i], 0., 1.);
}
@vertex fn buffered(@location(0) p: vec2f) -> @builtin(position) vec4f {
    return vec4f(p, 0., 1.);
}
@fragment fn colour() -> @location(0) vec4f { return vec4f(0., 1., 0., 1.); }
"#
            .into(),
        ),
    });
    let attrs = wgpu::vertex_attr_array![0 => Float32x2];
    let layout = [Some(wgpu::VertexBufferLayout {
        array_stride: 8,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attrs,
    })];
    let pipelines = [false, true].map(|buffered| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(if buffered { "buffered" } else { "generated" }),
                compilation_options: Default::default(),
                buffers: if buffered { &layout } else { &[] },
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("colour"),
                compilation_options: Default::default(),
                targets: &[Some(T::Rgba8Unorm.into())],
            }),
            multiview_mask: None,
            cache: None,
        })
    });
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[[-1f32, -1.], [3., -1.], [-1., 3.]]),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1,
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
    let view = texture.create_view(&Default::default());
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    for (properties, visible) in [
        ("", true),
        ("sky=false", false),
        ("sky=true", true),
        ("sun=false\nmoon=false\nstars=false", true),
    ] {
        let visibility = horizon_shaders::SkyVisibility::parse(
            &horizon_shaders::Properties::parse(properties, &horizon_shaders::Macros::standard()),
        );
        for (part, buffered) in [("disc", false), ("sunrise", true), ("void", false)] {
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[attach(
                        &view,
                        wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    )],
                    ..Default::default()
                });
                super::super::pack_visibility::draw_sky_part(
                    &mut pass,
                    visibility,
                    &pipelines[usize::from(buffered)],
                    buffered.then_some(&vertices),
                    3,
                );
            }
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        bytes_per_row: Some(256),
                        ..Default::default()
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            {
                let bytes = read.slice(..).get_mapped_range().unwrap();
                assert_eq!(
                    &bytes[..3],
                    if visible { &[0, 255, 0] } else { &[0, 0, 0] },
                    "properties={properties}, part={part}"
                );
            }
            read.unmap();
        }
    }
}
