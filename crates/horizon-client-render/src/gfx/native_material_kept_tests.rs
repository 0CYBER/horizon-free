//! Step 7 of #16011 item 4: `[material] reads`. Each pack runs through the renderer's own calls
//! (`NativePack::new`, `install`, `write`, `before_world`, `bind_terrain`, `terrain`, the
//! chain); a test's own entry points then draw the material file's `shown` with the group 3 the
//! terrain draws with, over one texel, so what the terrain reads comes back as floats.
use super::super::super::native_pack::NativePack;
use super::super::trace::Traced;
use super::install;
use super::trace_tests::{Scene, World, ray_device};
use horizon_shader_api::kept::Keep;
use horizon_shader_api::{MANIFEST, Pack, StageKind, check};

const HEAD: &str = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n";
const IDENTITY: &str = "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color; }\n";
/// A full-screen triangle that shows a stage file's `shown`.
pub(super) const PROBE: &str = "
@vertex fn test_vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4(vec2(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn test_fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return shown();
}
";

/// The material file's own `material`, which draws what `shown` gives.
const MATERIAL: &str =
    "fn material(input: MaterialInput) -> Surface { return Surface(shown() * input.albedo); }\n";

/// A device with its adapter's limits, as the game opens one.
pub(super) fn device() -> Option<(wgpu::Instance, wgpu::Device, wgpu::Queue)> {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    let desc = wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&desc)).ok()?;
    Some((instance, device, queue))
}

/// A pack of `manifest` (with the identity tone curve) and `files`, built by `NativePack::new`
/// against 1×1 sprites, or `scene`'s, and installed; and its material's composed sources, without
/// the scene and with it.
fn build(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: Option<&Scene>,
    manifest: &str,
    files: &[(&str, &str)],
) -> Result<(NativePack, String, Option<String>), String> {
    let (pack, mut sources) = built(device, queue, scene, manifest, files)?;
    let (_, source, scene_source) = (sources.drain(..))
        .find(|(kind, ..)| *kind == StageKind::Material)
        .unwrap();
    Ok((pack, source, scene_source))
}

/// `build`, with every stage's composed sources: its kind, without the scene and with it.
pub(super) fn built(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: Option<&Scene>,
    manifest: &str,
    files: &[(&str, &str)],
) -> Result<(NativePack, Vec<(StageKind, String, Option<String>)>), String> {
    let manifest = format!("{HEAD}{manifest}[tonemap]\nfile = \"t.wgsl\"\n");
    let mut all: std::collections::BTreeMap<String, Vec<u8>> = (files.iter())
        .chain(&[("t.wgsl", IDENTITY)])
        .map(|(n, t)| ((*n).to_owned(), t.as_bytes().to_vec()))
        .collect();
    all.insert(MANIFEST.into(), manifest.into_bytes());
    let report = check(&Pack::from_files(all).unwrap()).unwrap();
    let sources = (report.stages.iter())
        .map(|s| (s.kind, s.source.clone(), s.scene_source.clone()))
        .collect();
    let sprite = || {
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
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        })
    };
    let own: Vec<_> = (0..8).map(|_| sprite()).collect();
    // LabPBR maps, so `[displacement]` builds its grid (it needs heights).
    let map = || {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    let pbr = [map(), map()];
    let (globals, empty) = crate::gfx::native_pack::tests::terrain_layouts(device);
    // The entities' group 0, which the entity stage and the shadow pass's entity draw read, and
    // the retained block entities' transform.
    let entities = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &crate::gfx::native_entity::entity_entries(),
    });
    let retained = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let terrain = crate::gfx::native_material::TerrainShape {
        sprite_views: scene.map_or(&own, |s| &s.sprites),
        pbr: Some(&pbr),
        entities: &entities,
        retained_entities: &retained,
        ..crate::gfx::native_pack::tests::terrain_shape(&globals, &empty)
    };
    let (mut pack, _) = NativePack::new(device, queue, &terrain, report, &[], Default::default())?;
    install(device, Some(&mut pack.post), None, [1, 1])?;
    Ok((pack, sources))
}

/// Frame `n` of `pack` as the renderer starts it, with `scene` or without: its inputs, its
/// `[[before_world]]` passes and its kept resources' frame start, the terrain's groups; then,
/// in `encoder`, before the chain, the material's `shown` (of `source`, or `scene_source` where
/// the terrain traces) drawn with the terrain's group 3. The texel it drew.
fn frame(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    pack: &mut NativePack,
    (source, scene_source): (&str, Option<&str>),
    n: u32,
    scene: Option<&Traced<'_>>,
    world: Option<&World>,
) -> [f32; 4] {
    let hud = crate::hud::HudBatch::new(
        [4, 4],
        crate::hud::GuiScale {
            requested: 0,
            unicode: false,
        },
    );
    let mut inputs = crate::gfx::pack_uniforms::initial_history_tests::frame(&hud);
    inputs.pack.frame_timing = Some(crate::gfx::pack_uniforms::FrameTiming::nth(n, 50.0));
    let display = crate::gfx::output::DisplayValues::SDR;
    pack.write(queue, &inputs, [4, 4], display, scene);
    let mut encoder = device.create_command_encoder(&Default::default());
    pack.before_world(device, &mut encoder, scene);
    pack.bind_terrain(device, scene);
    let material = pack.material.as_ref().unwrap();
    let traced = material
        .scene
        .as_ref()
        .is_some_and(|t| t.drawn(0).is_some());
    let source = if traced {
        scene_source.unwrap()
    } else {
        source
    };
    let text = crate::gfx::native_material::test_shader(source) + PROBE;
    let (_, group) = pack.terrain().unwrap();
    let target = shown(device, &mut encoder, &text, material.drawn_layout(), group);
    if let Some(world) = world {
        let view = world.screen.create_view(&Default::default());
        let targets = (&world.color, &world.depth, &pack.frame.surface_view);
        (pack.post).run(device, &mut encoder, targets, &view, [4, 4], None, scene);
    }
    texel((device, queue), encoder, &target)
}

/// Draws `text`'s `test_vs` and `test_fs` (`PROBE`) in `encoder`, with `group` of `layout` at
/// group 3, over one `rgba32float` texel.
pub(super) fn shown(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    text: &str,
    layout: &wgpu::BindGroupLayout,
    group: &wgpu::BindGroup,
) -> wgpu::Texture {
    let parsed = naga::front::wgsl::parse_str(text).unwrap_or_else(|e| panic!("{e}"));
    // The terrain's overrides the probe doesn't reach still need values.
    let constants: Vec<(String, f64)> = (parsed.overrides.iter())
        .filter(|(_, o)| o.init.is_none())
        .filter_map(|(_, o)| Some((o.name.clone()?, 0.0)))
        .collect();
    let constants: Vec<(&str, f64)> = constants.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(parsed)),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[None, None, None, Some(layout)],
        ..Default::default()
    });
    let options = wgpu::PipelineCompilationOptions {
        constants: &constants,
        ..Default::default()
    };
    let format = wgpu::TextureFormat::Rgba32Float;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("test_vs"),
            compilation_options: options.clone(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("test_fs"),
            compilation_options: options,
            targets: &[Some(format.into())],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let target = device.create_texture(&wgpu::TextureDescriptor {
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.create_view(&Default::default()),
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
        pass.set_bind_group(horizon_shader_api::stages::GROUP, group, &[]);
        pass.draw(0..3, 0..1);
    }
    target
}

/// Submits `encoder` with a copy of `target`'s texel: the texel.
pub(super) fn texel(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    mut encoder: wgpu::CommandEncoder,
    target: &wgpu::Texture,
) -> [f32; 4] {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    (buffer.slice(..)).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = buffer.slice(..).get_mapped_range().unwrap()[..16].to_vec();
    let texel: &[f32] = bytemuck::cast_slice(&bytes);
    texel.try_into().unwrap()
}

/// A counter a `[[before_world]]` pass moves on once a frame, in a buffer and a texture.
pub(super) const COUNTER: &str = "[[resource]]\nname = \"tex\"\nformat = \"r32float\"\nsize = [1, 1]\nkeep = \"view\"\n\
                       [[resource]]\nname = \"count\"\nbuffer = 16\nelement = \"u32\"\nkeep = \"view\"\n\
                       [[before_world]]\nfile = \"w.wgsl\"\ndispatch = [1, 1, 1]\nworkgroup = [1, 1, 1]\n\
                       writes = [\"tex\", \"count\"]\n";
pub(super) const COUNTS: &str = "fn compute(input: ComputeInput) {\n    if input.in_bounds {\n        \
                      count[0] = count[0] + 1u;\n        \
                      textureStore(tex, vec2<i32>(0), vec4<f32>(f32(count[0])));\n    }\n}\n";
/// This frame's texture, last frame's, and the buffer.
pub(super) const SHOWS_COUNT: &str = "fn shown() -> vec4<f32> {\n    \
                           return vec4<f32>(textureLoad(tex, vec2<i32>(0), 0).x, \
                           textureLoad(tex_previous, vec2<i32>(0), 0).x, f32(count[0]), 1.0);\n}\n";

/// The terrain reads in the same frame what a `[[before_world]]` pass wrote, a texture's this
/// frame's version and its buffer, and last frame's version as `_previous`, the versions
/// swapping each frame; after a reset, zeros moved on by this frame's pass. With no validation
/// error.
#[test]
fn the_material_reads_what_before_world_wrote_this_frame() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((_, device, queue)) = device() else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let manifest = format!(
        "{COUNTER}[material]\nfile = \"m.wgsl\"\nreads = [\"tex\", \"tex_previous\", \"count\"]\n"
    );
    let material = SHOWS_COUNT.to_owned() + MATERIAL;
    let files = [("w.wgsl", COUNTS), ("m.wgsl", material.as_str())];
    let (mut pack, source, _) = build(&device, &queue, None, &manifest, &files).unwrap();
    let gpu = (&device, &queue);
    for n in 1..=4 {
        let got = frame(gpu, &mut pack, (&source, None), n, None, None);
        let n = n as f32;
        assert_eq!(got, [n, n - 1.0, n, 1.0], "frame {n}");
    }
    // An option that resets the world (`resets = "world"`): both kinds clear before the world.
    (pack.frame.epochs.as_mut().unwrap()).option(Keep::World);
    let got = frame(gpu, &mut pack, (&source, None), 5, None, None);
    assert_eq!(got, [1.0, 0.0, 1.0, 1.0], "the reset frame");
    let got = frame(gpu, &mut pack, (&source, None), 6, None, None);
    assert_eq!(got, [2.0, 1.0, 2.0, 1.0], "after the reset");
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

/// A pack without `[[before_world]]` passes starts its kept resources' frame before the world
/// too: the terrain reads as `_previous` what the chain's compute pass wrote last frame, not the
/// frame before.
#[test]
fn the_material_reads_last_frame_s_chain_as_previous() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((_, device, queue)) = device() else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let manifest = "[[resource]]\nname = \"tex\"\nformat = \"r32float\"\nsize = [1, 1]\nkeep = \"view\"\n\
                    [[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = [1, 1, 1]\n\
                    workgroup = [1, 1, 1]\nreads = [\"tex_previous\"]\nwrites = [\"tex\"]\n\
                    [[post]]\nfile = \"l.wgsl\"\n\
                    [material]\nfile = \"m.wgsl\"\nreads = [\"tex_previous\"]\n";
    let counts = "fn compute(input: ComputeInput) {\n    \
                  let was = textureLoad(tex_previous, vec2<i32>(0), 0).x;\n    \
                  if input.in_bounds { textureStore(tex, vec2<i32>(0), vec4<f32>(was + 1.0)); }\n}\n";
    let last = "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
    let material = "fn shown() -> vec4<f32> {\n    \
                    return vec4<f32>(textureLoad(tex_previous, vec2<i32>(0), 0).x, 0.0, 0.0, 1.0);\n}\n"
        .to_owned()
        + MATERIAL;
    let files = [("c.wgsl", counts), ("l.wgsl", last), ("m.wgsl", &material)];
    let (mut pack, source, _) = build(&device, &queue, None, manifest, &files).unwrap();
    let world = World::new(&device);
    for n in 1..=4 {
        let got = frame(
            (&device, &queue),
            &mut pack,
            (&source, None),
            n,
            None,
            Some(&world),
        );
        assert_eq!(got[0], n as f32 - 1.0, "frame {n}");
    }
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

/// A pack at the limit of eight kept resources reads all of them in its material, four textures
/// (both versions) and four buffers; on a device of wgpu's default limits (16 sampled textures a
/// stage), the same material is refused at load, saying why.
#[test]
fn a_material_reads_eight_resources_within_the_device_s_limits() {
    let _gpu = crate::gfx::test_gpu::lock();
    let mut manifest = String::new();
    for i in 0..4 {
        manifest += &format!(
            "[[resource]]\nname = \"t{i}\"\nformat = \"r32float\"\nsize = [1, 1]\nkeep = \"view\"\n\
             [[resource]]\nname = \"b{i}\"\nbuffer = 16\nelement = \"u32\"\nkeep = \"view\"\n"
        );
    }
    let names: Vec<String> = (0..4)
        .flat_map(|i| [format!("\"t{i}\""), format!("\"b{i}\"")])
        .collect();
    let reads: Vec<String> = (0..4)
        .flat_map(|i| {
            [
                format!("\"t{i}\""),
                format!("\"t{i}_previous\""),
                format!("\"b{i}\""),
            ]
        })
        .collect();
    manifest += &format!(
        "[[before_world]]\nfile = \"w.wgsl\"\ndispatch = [1, 1, 1]\nworkgroup = [1, 1, 1]\n\
         writes = [{}]\n[material]\nfile = \"m.wgsl\"\nreads = [{}]\n",
        names.join(", "),
        reads.join(", ")
    );
    let mut writes = String::from("fn compute(input: ComputeInput) {\n    if input.in_bounds {\n");
    let mut sums = [String::new(), String::new(), String::new()];
    for i in 0..4 {
        writes += &format!(
            "        textureStore(t{i}, vec2<i32>(0), vec4<f32>({}.0));\n        b{i}[0] = {}u;\n",
            i + 1,
            10 * (i + 1)
        );
        sums[0] += &format!(" + textureLoad(t{i}, vec2<i32>(0), 0).x");
        sums[1] += &format!(" + textureLoad(t{i}_previous, vec2<i32>(0), 0).x");
        sums[2] += &format!(" + f32(b{i}[0])");
    }
    writes += "    }\n}\n";
    let material = format!(
        "fn shown() -> vec4<f32> {{\n    return vec4<f32>(0.0{}, 0.0{}, 0.0{}, 1.0);\n}}\n{MATERIAL}",
        sums[0], sums[1], sums[2]
    );
    let files = [("w.wgsl", writes.as_str()), ("m.wgsl", &material)];
    if let Some((_, device, queue)) = device() {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (mut pack, source, _) = build(&device, &queue, None, &manifest, &files).unwrap();
        let gpu = (&device, &queue);
        let got = frame(gpu, &mut pack, (&source, None), 1, None, None);
        assert_eq!(got, [10.0, 0.0, 100.0, 1.0]);
        let got = frame(gpu, &mut pack, (&source, None), 2, None, None);
        assert_eq!(got, [10.0, 10.0, 100.0, 1.0]);
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
    }
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let refused = build(&device, &queue, None, &manifest, &files).err();
    let why = refused.expect("over the default limits");
    assert!(why.starts_with("m.wgsl: "), "{why}");
    assert!(why.contains("sampled textures"), "{why}");
}

/// A material that traces and reads kept resources: against the scene its group joins the
/// scene and the kept resources for each parity, with the look binding only for the tint
/// reader. The second scene has no look store. Without a scene the rays miss.
#[test]
fn a_traced_material_reads_kept_resources() {
    use wgpu::util::DeviceExt;

    let _gpu = crate::gfx::test_gpu::lock();
    let Some((_, device, queue)) = ray_device() else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let (near_scene, far_scene) = (
        Scene::new(&device, &queue, 4.0, 1),
        Scene::new(&device, &queue, 6.0, 2),
    );
    let manifest = format!(
        "[ray_tracing]\n{COUNTER}[material]\nfile = \"m.wgsl\"\nreads = [\"tex_previous\", \"count\"]\n"
    );
    let material = "fn shown() -> vec4<f32> {\n    \
                    let hit = trace(vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0), 64.0);\n    \
                    return vec4<f32>(RAY_VALUE, textureLoad(tex_previous, vec2<i32>(0), 0).x, \
                    f32(count[0]), f32(frame.has_ray_tracing));\n}\n"
        .to_owned()
        + MATERIAL;
    let shade = 1 << 24;
    let look = crate::mesher::QuadLook([shade, !0]);
    let looks = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::bytes_of(&look),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let near = Traced {
        looks: Some(&looks),
        ..near_scene.traced()
    };
    let far = far_scene.traced();
    let gpu = (&device, &queue);
    for (reads_looks, value, [near_value, far_value]) in [
        (true, "hit.distance * (1.0 + hit.tint.r)", [4.0, 12.0]),
        (
            false,
            "hit.distance + round(hit.emission * 15.0)",
            [16.0, 18.0],
        ),
    ] {
        let material = material.replace("RAY_VALUE", value);
        let files = [("w.wgsl", COUNTS), ("m.wgsl", material.as_str())];
        let (mut pack, source, scene_source) =
            build(&device, &queue, Some(&near_scene), &manifest, &files).unwrap();
        let traced = pack.material.as_ref().unwrap().scene.as_ref().unwrap();
        assert_eq!(traced.traces_looks(), reads_looks);
        let sources = (source.as_str(), scene_source.as_deref());
        for (n, scene, expected) in [
            (1, Some(&near), near_value),
            (2, Some(&near), near_value),
            (3, Some(&far), far_value),
            (4, Some(&far), far_value),
        ] {
            let got = frame(gpu, &mut pack, sources, n, scene, None);
            let n = n as f32;
            assert_eq!(got, [expected, n - 1.0, n, 1.0], "{value}, frame {n}");
        }
        let got = frame(gpu, &mut pack, sources, 5, None, None);
        assert!(got[0] < 0.0, "{got:?}");
        assert_eq!(got[1..], [4.0, 5.0, 0.0]);
    }
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

/// What the terrain's kept groups make, counted with `Instance::generate_report` over six
/// frames, every frame's group held: two groups (a parity each) on the first frame, nothing on
/// the next ones, two again after a resize remakes the R-relative textures. A material without
/// `reads` in the same pack makes none: it draws with the frame's group.
#[test]
fn the_material_makes_its_kept_groups_once() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((instance, device, queue)) = device() else {
        return;
    };
    let manifest = |reads: &str| {
        format!(
            "{COUNTER}[[resource]]\nname = \"screen\"\nformat = \"rgba16float\"\nscale = 1\n\
             keep = \"view\"\n[material]\nfile = \"m.wgsl\"\n{reads}"
        )
    };
    let count = || {
        let hub = instance.generate_report()?.hub;
        let counts = [
            hub.bind_group_layouts,
            hub.bind_groups,
            hub.render_pipelines,
        ];
        Some(counts.map(|c| c.num_allocated))
    };
    let mut held = Vec::new();
    let mut made = |reads: &str, material: &str| {
        let files = [("w.wgsl", COUNTS), ("m.wgsl", material)];
        let mut counts = vec![count()?];
        let (mut pack, _, _) = build(&device, &queue, None, &manifest(reads), &files).unwrap();
        counts.push(count()?);
        for n in 1..=6 {
            let hud = crate::hud::HudBatch::new(
                [4, 4],
                crate::hud::GuiScale {
                    requested: 0,
                    unicode: false,
                },
            );
            let mut inputs = crate::gfx::pack_uniforms::initial_history_tests::frame(&hud);
            inputs.pack.frame_timing = Some(crate::gfx::pack_uniforms::FrameTiming::nth(n, 50.0));
            let size = if n < 5 { [4, 4] } else { [8, 8] };
            let display = crate::gfx::output::DisplayValues::SDR;
            pack.write(&queue, &inputs, size, display, None);
            let mut encoder = device.create_command_encoder(&Default::default());
            pack.before_world(&device, &mut encoder, None);
            pack.bind_terrain(&device, None);
            held.push(pack.terrain().unwrap().1.clone());
            held.extend(pack.post.kept.iter().flat_map(|k| k.worlds.iter().cloned()));
            queue.submit([encoder.finish()]);
            counts.push(count()?);
        }
        let delta = |(a, b): (&[usize; 3], &[usize; 3])| [0, 1, 2].map(|i| b[i] - a[i]);
        Some(
            counts
                .iter()
                .zip(&counts[1..])
                .map(delta)
                .collect::<Vec<_>>(),
        )
    };
    let reads = "reads = [\"tex\", \"tex_previous\", \"count\"]\n";
    let material = SHOWS_COUNT.to_owned() + MATERIAL;
    // Without a report (a backend outside wgpu-core) there is nothing to count.
    let Some(with) = made(reads, &material) else {
        return;
    };
    let plain = "fn shown() -> vec4<f32> { return vec4<f32>(1.0); }\n".to_owned() + MATERIAL;
    let without = made("", &plain).unwrap();
    eprintln!("with reads: {with:?}\nwithout: {without:?}");
    // Groups a frame: the `[[before_world]]` pass's two (a parity each) and, with `reads`, the
    // terrain's two; again after the resize.
    let steady = [0, 0, 0];
    assert_eq!(
        with[1..],
        [[0, 4, 0], steady, steady, steady, [0, 4, 0], steady]
    );
    assert_eq!(
        without[1..],
        [[0, 2, 0], steady, steady, steady, [0, 2, 0], steady]
    );
    // Building: the terrain's kept layout is the one layout more, its pipelines as many.
    assert_eq!(with[0][0], without[0][0] + 1);
    assert_eq!(with[0][2], without[0][2]);
}

/// A material that reads kept resources beside `[vertex]`, `[shadow]` and `[displacement]`:
/// the shadow pass and the displaced grid compile the material's module, its kept declarations
/// included, against layouts without them: the pack loads, its shadow and grid pipelines built,
/// and its terrain reads as before, with no validation error.
#[test]
fn a_material_that_reads_loads_with_its_shadow_and_relief() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((_, device, queue)) = device() else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let manifest = format!(
        "{COUNTER}[material]\nfile = \"m.wgsl\"\nreads = [\"tex\", \"tex_previous\", \"count\"]\n\
         [vertex]\nfile = \"v.wgsl\"\n[shadow]\n[displacement]\n"
    );
    let material = SHOWS_COUNT.to_owned()
        + MATERIAL
        + "fn displacement(input: DisplacementInput) -> f32 { return (1.0 - input.height) * 0.125; }\n";
    let vertex = "fn vertex(input: VertexInput) -> vec3<f32> { return input.position; }\n";
    let files = [
        ("w.wgsl", COUNTS),
        ("m.wgsl", &material),
        ("v.wgsl", vertex),
    ];
    let (mut pack, source, _) = build(&device, &queue, None, &manifest, &files).unwrap();
    assert!(pack.shadow.is_some() && pack.displacement.is_some());
    let gpu = (&device, &queue);
    for n in 1..=2 {
        let got = frame(gpu, &mut pack, (&source, None), n, None, None);
        let n = n as f32;
        assert_eq!(got, [n, n - 1.0, n, 1.0], "frame {n}");
    }
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}
