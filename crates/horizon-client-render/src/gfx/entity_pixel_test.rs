//! Offscreen entity pipeline readback for cross-crate rendering tests.
use super::*;
use bytemuck::Zeroable;

const SIZE: u32 = 64;

struct Frame {
    device: wgpu::Device,
    queue: wgpu::Queue,
    color: wgpu::Texture,
    depth: wgpu::Texture,
    group: wgpu::BindGroup,
    bindings: wgpu::BindGroupLayout,
    atlas: wgpu::TextureView,
    atlas_texture: wgpu::Texture,
}

/// The software Vulkan adapter (lavapipe, llvmpipe) the pixel tests draw on.
fn software_adapter() -> std::result::Result<wgpu::Adapter, String> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(desc);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    }))
    .map_err(|error| format!("no software Vulkan adapter: {error}"))?;
    if adapter.get_info().device_type != wgpu::DeviceType::Cpu {
        return Err("no software Vulkan adapter".into());
    }
    Ok(adapter)
}

fn software_frame(
    atlas_size: [u32; 2],
    atlas: &[u8],
    view_proj: Mat4,
    sky: f32,
    size: [u32; 2],
) -> std::result::Result<Frame, String> {
    let adapter = software_adapter()?;
    let mut required_limits = wgpu::Limits::default();
    required_limits.max_texture_dimension_2d = required_limits
        .max_texture_dimension_2d
        .max(atlas_size[0])
        .max(atlas_size[1]);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits,
        ..Default::default()
    }))
    .map_err(|error| format!("software Vulkan device request failed: {error}"))?;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = |width, height, format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let color = texture(
        size[0],
        size[1],
        format,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(
        size[0],
        size[1],
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let atlas_texture = texture(
        atlas_size[0],
        atlas_size[1],
        format,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    queue.write_texture(
        atlas_texture.as_image_copy(),
        atlas,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(atlas_size[0] * 4),
            rows_per_image: None,
        },
        atlas_texture.size(),
    );
    let lightmap = texture(
        16,
        16,
        format,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    queue.write_texture(
        lightmap.as_image_copy(),
        &[128, 192, 255, 255].repeat(16 * 16),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(64),
            rows_per_image: None,
        },
        lightmap.size(),
    );
    let sampler = device.create_sampler(&Default::default());
    let mut globals = Globals::zeroed();
    globals.view_proj = view_proj.to_cols_array_2d();
    globals.fog = [1000.0, 2000.0, 1000.0, 2000.0];
    // Sky factor for `hdr_scene`. Zero keeps the older pixel tests on colour only.
    globals.hdr_light = [sky, sky, 1.0, 0.0];
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::bytes_of(&globals),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &entity_binding_entries(),
    });
    let atlas_view = atlas_texture.create_view(&Default::default());
    let light_view = lightmap.create_view(&Default::default());
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bindings,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&light_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
        ],
    });
    Ok(Frame {
        device,
        queue,
        color,
        depth,
        group,
        bindings,
        atlas: atlas_view,
        atlas_texture,
    })
}

fn readback(frame: &Frame, encoder: wgpu::CommandEncoder) -> std::result::Result<Vec<u8>, String> {
    let (width, height) = (frame.color.width(), frame.color.height());
    let row_bytes = width * 4;
    let pitch = row_bytes.div_ceil(256) * 256;
    let readback = frame.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(pitch) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = encoder;
    encoder.copy_texture_to_buffer(
        frame.color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pitch),
                rows_per_image: None,
            },
        },
        frame.color.size(),
    );
    frame.queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    frame
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| error.to_string())?;
    let mapped = readback
        .slice(..)
        .get_mapped_range()
        .map_err(|error| error.to_string())?;
    Ok(mapped
        .chunks_exact(pitch as usize)
        .flat_map(|row| row[..row_bytes as usize].iter().copied())
        .collect())
}

/// Renders CPU entity quads. A missing software Vulkan adapter fails the test.
pub fn render(
    vertices: &[Vertex],
    atlas_size: [u32; 2],
    atlas: &[u8],
    view_proj: Mat4,
    hdr: bool,
) -> std::result::Result<Vec<u8>, String> {
    // The HDR mark pass is gone (the entity shader has no `fs_hdr_mark`): `hdr` changes nothing.
    let _ = hdr;
    let _gpu = crate::gfx::test_gpu::lock();
    let frame = software_frame(atlas_size, atlas, view_proj, 0.0, [SIZE, SIZE])?;
    let shader = frame
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(entity_shader().into()),
        });
    let layout = frame
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&frame.bindings)],
            immediate_size: 0,
        });
    let buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ENTITY_ATTRIBUTES,
    })];
    let reuse = reuse::Reuse::default();
    let kept = reuse.on(&frame.device);
    let make = |fragment, depth_write, bias, blend, write_mask| {
        entity_pipeline_factory(
            &kept,
            &layout,
            &layout,
            &shader,
            &buffers,
            wgpu::TextureFormat::Rgba8Unorm,
            false,
        )(EntityPipelineOptions {
            label: "Entities",
            cull_mode: None,
            depth_write,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias,
            blend,
            clouds: false,
            vertex: "vs_entity",
            fragment,
            write_mask,
        })
    };
    let pipeline = make(
        "fs_entity",
        true,
        wgpu::DepthBiasState {
            constant: 128,
            slope_scale: 0.0,
            clamp: 0.0,
        },
        wgpu::BlendState::ALPHA_BLENDING,
        wgpu::ColorWrites::ALL,
    );
    let vertex = frame
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let indices: Vec<u32> = (0..vertices.len() as u32 / 4)
        .flat_map(|i| [4 * i, 4 * i + 1, 4 * i + 2, 4 * i, 4 * i + 2, 4 * i + 3])
        .collect();
    let index = frame
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
    let mut encoder = frame.device.create_command_encoder(&Default::default());
    {
        let color_view = frame.color.create_view(&Default::default());
        let depth_view = frame.depth.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &frame.group, &[]);
        pass.set_vertex_buffer(0, vertex.slice(..));
        pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
    }
    readback(&frame, encoder)
}

/// Inputs for one offscreen prepared instance. Fields stay named so the draw is not a long argument list.
pub struct PreparedInstanceDraw<'a> {
    pub vertices: &'a [super::prepared_model::PreparedModelVertex],
    pub ranges: Vec<Option<(u32, u32)>>,
    pub uploaded: u64,
    pub requested: u64,
    pub slot: u32,
    pub instances: &'a [super::prepared_model::PreparedModelInstance],
    /// When set, each copy is its own draw. Otherwise they are one run.
    pub per_copy: bool,
    /// Ignored: the HDR mark pass no longer exists, so every draw is the colour pass only.
    pub hdr: bool,
    pub atlas_size: [u32; 2],
    pub atlas: &'a [u8],
    pub view_proj: Mat4,
}

/// One prepared instance. The geometry is dropped before submit.
pub fn render_instance(draw: PreparedInstanceDraw<'_>) -> std::result::Result<Vec<u8>, String> {
    let _gpu = crate::gfx::test_gpu::lock();
    let PreparedInstanceDraw {
        vertices,
        ranges,
        uploaded,
        requested,
        slot,
        instances,
        per_copy,
        hdr: _,
        atlas_size,
        atlas,
        view_proj,
    } = draw;
    let frame = software_frame(atlas_size, atlas, view_proj, 0.0, [SIZE, SIZE])?;
    let models = super::prepared_model::PreparedModels::create(
        &frame.device,
        &reuse::Reuse::default(),
        &frame.bindings,
        wgpu::TextureFormat::Rgba8Unorm,
        uploaded,
        vertices,
        ranges,
    )
    .map_err(|error| error.to_string())?;
    let range = models
        .indices(requested, slot)
        .map_err(|error| error.to_string())?;
    let instance = frame
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(instances),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let mut encoder = frame.device.create_command_encoder(&Default::default());
    {
        let color_view = frame.color.create_view(&Default::default());
        let depth_view = frame.depth.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &frame.group, &[]);
        let runs = if per_copy {
            (0..instances.len() as u32)
                .map(|first| super::prepared_model::InstanceRun { first, count: 1 })
                .collect::<Vec<_>>()
        } else {
            vec![super::prepared_model::InstanceRun {
                first: 0,
                count: instances.len() as u32,
            }]
        };
        for run in runs {
            models.draw_run(&mut pass, &instance, range, run);
        }
    }
    drop(models);
    readback(&frame, encoder)
}

/// `vertices`, in the runs `spans` gives them (as the game marks them), drawn on software Vulkan
/// by a probe pack of our own: each of its gbuffers programs writes one flat colour of
/// [`PACK_COLOURS`] and puts a vertex at half its camera-relative x and y. Runs the pack leaves
/// to Horizon stay black.
pub fn render_pack(
    vertices: &[Vertex],
    spans: &[pack_draw::EntitySpan],
) -> std::result::Result<Vec<[u8; 4]>, String> {
    let _gpu = crate::gfx::test_gpu::lock();
    use std::io::Write;
    let colour = |[r, g, b, _]: [u8; 4]| format!("vec4({r}.0, {g}.0, {b}.0, 255.0) / 255.0");
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut add = |name: &str, source: &str| -> std::io::Result<()> {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(source.as_bytes())
    };
    let programs = ["gbuffers_entities", "gbuffers_block", "gbuffers_textured"];
    for (program, rgba) in programs.into_iter().zip(PACK_COLOURS) {
        add(
            &format!("{program}.vsh"),
            "#version 120\nvoid main() { gl_Position = vec4(gl_Vertex.xy * 0.5, 0.5, 1.0); }\n",
        )
        .and_then(|()| {
            add(
                &format!("{program}.fsh"),
                &format!(
                    "#version 120\nvoid main() {{ gl_FragData[0] = {}; }}\n",
                    colour(rgba)
                ),
            )
        })
        .map_err(|error| error.to_string())?;
    }
    add(
        "final.vsh",
        "#version 120\nvarying vec2 uv;\n\
         void main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }\n",
    )
    .and_then(|()| {
        add(
            "final.fsh",
            "#version 120\nuniform sampler2D colortex0;\nvarying vec2 uv;\n\
             void main() { gl_FragColor = texture2D(colortex0, uv); }\n",
        )
    })
    .map_err(|error| error.to_string())?;
    let bytes = zip.finish().map_err(|error| error.to_string())?;
    let pack = horizon_shaders::Pack::from_zip_bytes("original-owner-probe", bytes.into_inner())
        .map_err(|error| format!("{error:?}"))?;
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    let inputs = horizon_shaders::GameInputs {
        entities: Some(pack_entities::inputs()),
        entity_geometry: Some(pack_entities::geometry_inputs()),
        ..Default::default()
    };
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &inputs);
    let adapter = software_adapter()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .map_err(|error| error.to_string())?;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = |size: u32, format| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Original owner probe"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let mut post = pack_post::PackPost::new(
        &device,
        &queue,
        &adapter,
        post,
        format,
        pack_post::World {
            atlas: None,
            material_maps: None,
            lightmap: texture(1, format),
            terrain: None,
            block_ids: &[],
            sprite_rects: &[],
            celestial: None,
            gui_size: [SIZE; 2],
            clouds: None,
            weather: false,
            hand: false,
        },
        &Default::default(),
    )
    .map_err(|refused| refused.reason)?;
    let mut rest = Vec::new();
    post.prepare_entities(
        &device,
        &queue,
        vertices,
        spans,
        [0.0; 4],
        [0.0; 4],
        &mut rest,
        None,
        (&[], &[]),
    );
    let (world, depth, screen) = (
        texture(SIZE, format),
        texture(SIZE, wgpu::TextureFormat::Depth32Float),
        texture(SIZE, format),
    );
    let hud = HudBatch::scratch([SIZE; 2]);
    let mut encoder = device.create_command_encoder(&Default::default());
    post.run(
        &device,
        &queue,
        &mut encoder,
        &world,
        &depth,
        &screen,
        &probe_frame(&hud),
        &Default::default(),
        [SIZE; 2],
        [SIZE; 2],
        &[],
        &mut |_, _, _, _| {},
    );
    let (color, read) = (screen.texture(), SIZE * 4);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(read * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(read),
                rows_per_image: None,
            },
        },
        color.size(),
    );
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| error.to_string())?;
    let mapped = readback
        .slice(..)
        .get_mapped_range()
        .map_err(|error| error.to_string())?;
    Ok(mapped.as_chunks::<4>().0.to_vec())
}

/// A world frame with nothing in it but the pack's entities (`render_pack`).
fn probe_frame(hud: &HudBatch) -> super::Frame<'_> {
    super::Frame {
        eye: glam::DVec3::ZERO,
        eye_rotation: [0.0; 2],
        first_person: true,
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        water_plane: None,
        weather: &[],
        weather_time: crate::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 0.0,
        fov_degrees: 90.0,
        bob: Mat4::IDENTITY,
        spin: Mat4::IDENTITY,
        sky: crate::environment::Sky::default(),
        distance: 12.0,
        detail_distance: 12.0,
        depth_far: 1000.0,
        underwater: false,
        cave_culling: false,
        title: false,
        gui_only: false,
        sky_blocked: false,
        skybox: false,
        end_sky: false,
        end_sky_sprite: None,
        boss_world_fog: false,
        environment_fog: None,
        water_fog: [0.0; 3],
        fog: None,
        dark_disc: false,
        outline: Vec::new(),
        outline_after_terrain: false,
        outline_high_contrast: false,
        pvp: Default::default(),
        debug_lines: &[],
        debug_lines_on_top: &[],
        debug_crosshair: &[],
        hud,
        glint_alpha: 0.75,
        glint_ticks: 0.0,
        nether_lighting: false,
        entities: &[],
        static_block_entities: &[],
        visible_static_block_entity_sections: &[],
        static_block_entity_stats: crate::block_entity_render::StaticCacheFrameStats::default(),
        entity_motion: &[],
        entity_motion_posed: &[],
        colored_lights: &[],
        emissive_layers: &[],
        shadow_only: &[],
        entity_spans: &[],
        skin: None,
        glowing: &[],
        dynamic_lights: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
        world_primitives: Default::default(),
        celestial: &[],
        stars: None,
        hand: &[],
        hand_items: &[],
        hand_fov_degrees: 70.0,
        screen: &[],
        pack: Default::default(),
        border: None,
        game_time: 0.0,
        shader_game_time: 0.0,
        end_portal_sprites: [[0.0; 4]; 2],
        crumbling_sprite: [0.0; 4],
        ticks: 0,
        darkness: (0.0, 0.0),
        night_vision: 0.0,
        block_flicker: 0.0,
        boss_darken: 0.0,
        menu_blur: None,
        chunk_fade: 0.0,
        post_effects: crate::post_chain::Request::none(),
    }
}

/// Draws a real screen batch through the engine HUD pipeline on software Vulkan.
/// Uses the same adapter, atlas upload and readback as the entity pixel fixtures.
pub fn render_hud(
    batch: &HudBatch,
    atlas: &crate::hud::HudAtlas,
    size: [u32; 2],
) -> std::result::Result<Vec<u8>, String> {
    let _gpu = crate::gfx::test_gpu::lock();
    let frame = software_frame(
        [atlas.width, atlas.height],
        &atlas.pixels(),
        Mat4::IDENTITY,
        0.0,
        size,
    )?;
    let sampler = frame.device.create_sampler(&Default::default());
    let mut hud = crate::hud::HudRenderer::new(
        &frame.device,
        wgpu::TextureFormat::Rgba8Unorm,
        &frame.atlas,
        &frame.atlas,
        &frame.atlas,
        &sampler,
        false,
    )
    .map_err(|error| error.to_string())?;
    // As the game's renderer has it (gfx.rs): the batch's atlas patches, a map's or a mod's
    // image, upload before the draw.
    hud.atlas_texture = Some(frame.atlas_texture.clone());
    let mut uploads = super::upload::Uploads::new(&frame.device);
    let mut encoder = frame.device.create_command_encoder(&Default::default());
    let color = frame.color.create_view(&Default::default());
    let depth = frame.depth.create_view(&Default::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    hud.prepare(&frame.device, &mut uploads, &mut encoder, batch, size);
    uploads.finish(&encoder);
    hud.draw(&mut encoder, &color, &depth, batch, size);
    readback(&frame, encoder)
}

/// What our probe pack's `gbuffers_entities`, `gbuffers_block` and `gbuffers_textured` each write
/// ([`render_pack`]), in that order.
pub const PACK_COLOURS: [[u8; 4]; 3] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];

#[cfg(test)]
#[rustfmt::skip]
mod instance_run {
    use super::super::prepared_model::{PreparedModelInstance, PreparedModelVertex};
    use super::{PreparedInstanceDraw, SIZE};
    use glam::{Mat4, Vec3};

    fn corner(x: f32, y: f32) -> PreparedModelVertex {
        PreparedModelVertex { pos: [x, y, 0.5, 1.0], uv: [0.5, 0.5, 0.0, 0.0], normal: [0.0, 0.0, 1.0, 0.0], flags: [1, 0, 0, 0] }
    }

    fn instance_at(x: f32, tint: [f32; 4]) -> PreparedModelInstance {
        let mut normal = [[0.0; 4]; 3];
        normal[0][0] = 1.0;
        normal[1][1] = 1.0;
        normal[2][2] = 1.0;
        PreparedModelInstance { model: Mat4::from_translation(Vec3::new(x, 0.0, 0.0)).to_cols_array_2d(), normal, tint, light: 0xff00, _pad: [0; 3] }
    }

    fn render(per_copy: bool, hdr: bool) -> Result<Vec<u8>, String> {
        let instances = [instance_at(-0.45, [1.0, 0.0, 0.0, 1.0]), instance_at(0.45, [0.0, 0.0, 1.0, 1.0])];
        let vertices = [corner(-0.2, -0.2), corner(0.2, -0.2), corner(0.2, 0.2), corner(-0.2, 0.2)];
        super::render_instance(PreparedInstanceDraw {
            vertices: &vertices, ranges: vec![Some((0, 6))], uploaded: 1, requested: 1, slot: 0,
            instances: &instances, per_copy, hdr, atlas_size: [4, 4],
            atlas: &[255, 255, 255, 255].repeat(16), view_proj: Mat4::IDENTITY,
        })
    }

    #[test]
    #[ignore = "daily"]
    fn an_instanced_run_matches_per_copy_pixels_with_hdr() {
        let per_copy = render(true, true).expect("per-copy software Vulkan draw");
        let instanced = render(false, true).expect("instanced software Vulkan draw");
        assert_eq!(per_copy, instanced);
        assert_eq!(per_copy.len(), (SIZE * SIZE * 4) as usize);
        let covered = per_copy.as_chunks::<4>().0.iter().filter(|p| p[0] != 0 || p[2] != 0).count();
        assert!(covered > 8, "the run covered {covered} pixels");
        let plain = render(false, false).expect("color-only software Vulkan draw");
        let alpha = |pixels: &[u8]| pixels.iter().skip(3).step_by(4).copied().collect::<Vec<_>>();
        assert_eq!(alpha(&per_copy), alpha(&plain), "a bias-0 mark must not change alpha");
    }
}
