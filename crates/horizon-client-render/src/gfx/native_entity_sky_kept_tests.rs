//! `[entity] reads` and `[sky] reads` (#16011 item 4, after `[material] reads`). Each pack runs
//! through the renderer's own calls (`NativePack::new`, `install`, `write`, `before_world`,
//! `bind_terrain`); a test's own entry points then draw each stage file's `shown` with the
//! group 3 the stage draws with (`NativePack::entities`, `NativeSky::group`, `terrain`), as the
//! material's tests do. The sky's is drawn right after `before_world`, where its timed pass draws
//! (`draw_sky_pass`), the others' after `bind_terrain`.
use super::super::super::native_pack::NativePack;
use super::super::trace::Traced;
use super::material_tests::{COUNTER, COUNTS, PROBE, SHOWS_COUNT, built, device, shown, texel};
use super::trace_tests::{Scene, ray_device};
use horizon_shader_api::StageKind;
use horizon_shader_api::kept::Keep;

const ENTITY: &str =
    "fn entity(input: EntityInput) -> Surface { return Surface(shown() * input.albedo); }\n";
const SKY: &str = "fn sky(input: SkyInput) -> vec4<f32> { return shown() * input.color; }\n";
const MATERIAL: &str =
    "fn material(input: MaterialInput) -> Surface { return Surface(shown() * input.albedo); }\n";

/// What a stage's `shown` gives drawn with its group 3 now, if `pack` has the stage `kind`.
fn drawn(
    gpu: (&wgpu::Device, &wgpu::Queue),
    pack: &NativePack,
    (kind, source): (StageKind, &str),
) -> Option<[f32; 4]> {
    let (device, _) = gpu;
    let frame = &pack.frame;
    let (text, layout, group) = match kind {
        StageKind::Entity => {
            let (_, group) = pack.entities()?;
            let kept = pack.entity.as_ref()?.kept.as_ref();
            (
                source.to_owned(),
                kept.map_or(&frame.layout, |k| &k.layout),
                group,
            )
        }
        StageKind::Sky => {
            let sky = pack.sky.as_ref()?;
            let kept = sky.kept.as_ref();
            let group = sky.group(frame, pack.post.parity());
            (
                source.to_owned(),
                kept.map_or(&frame.layout, |k| &k.layout),
                group,
            )
        }
        StageKind::Material => {
            let material = pack.material.as_ref()?;
            let text = crate::gfx::native_material::test_shader(source);
            (text, material.drawn_layout(), pack.terrain()?.1)
        }
        _ => return None,
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    let target = shown(device, &mut encoder, &(text + PROBE), layout, group);
    Some(texel(gpu, encoder, &target))
}

/// Frame `n` of `pack` at R `size` as the renderer starts it: its inputs, its `[[before_world]]`
/// passes and its kept resources' frame start, then the sky as its own pass draws it; with
/// `mirror`, the water mirror's view bound (the frame's group made again); the terrain's groups
/// and the entity and sky stages' again (`bind_scene`), then the entities and the terrain. The
/// texels each stage of `sources` drew, sky first.
fn frame(
    gpu: (&wgpu::Device, &wgpu::Queue),
    pack: &mut NativePack,
    sources: &[(StageKind, String, Option<String>)],
    (n, size): (u32, [u32; 2]),
    mirror: Option<&wgpu::TextureView>,
) -> Vec<[f32; 4]> {
    let (device, queue) = gpu;
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
    pack.write(queue, &inputs, size, display, None);
    let mut encoder = device.create_command_encoder(&Default::default());
    pack.before_world(device, &mut encoder, None);
    queue.submit([encoder.finish()]);
    let source = |kind| sources.iter().find(|(k, ..)| *k == kind).map(|(_, s, _)| s);
    let mut got = Vec::new();
    got.extend(source(StageKind::Sky).and_then(|s| drawn(gpu, pack, (StageKind::Sky, s))));
    if let Some(view) = mirror {
        pack.frame.set_reflection(device, Some(view));
    }
    pack.bind_terrain(device, None);
    for kind in [StageKind::Entity, StageKind::Material] {
        got.extend(source(kind).and_then(|s| drawn(gpu, pack, (kind, s))));
    }
    got
}

/// The entity and sky stages read in the same frame what a `[[before_world]]` pass wrote, a
/// texture's this frame's version and its buffer, and last frame's version as `_previous`, the
/// versions swapping each frame; after a reset, zeros moved on by this frame's pass. With no
/// validation error.
#[test]
fn the_entity_and_sky_stages_read_what_before_world_wrote_this_frame() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((_, device, queue)) = device() else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let reads = "reads = [\"tex\", \"tex_previous\", \"count\"]\n";
    let manifest =
        format!("{COUNTER}[entity]\nfile = \"e.wgsl\"\n{reads}[sky]\nfile = \"s.wgsl\"\n{reads}");
    let (entity, sky) = (
        SHOWS_COUNT.to_owned() + ENTITY,
        SHOWS_COUNT.to_owned() + SKY,
    );
    let files = [("w.wgsl", COUNTS), ("e.wgsl", &entity), ("s.wgsl", &sky)];
    let (mut pack, sources) = built(&device, &queue, None, &manifest, &files).unwrap();
    let gpu = (&device, &queue);
    for n in 1..=4 {
        let got = frame(gpu, &mut pack, &sources, (n, [4, 4]), None);
        let n = n as f32;
        assert_eq!(got, [[n, n - 1.0, n, 1.0]; 2], "frame {n}");
    }
    // An option that resets the world (`resets = "world"`): both kinds clear before the world.
    (pack.frame.epochs.as_mut().unwrap()).option(Keep::World);
    let got = frame(gpu, &mut pack, &sources, (5, [4, 4]), None);
    assert_eq!(got, [[1.0, 0.0, 1.0, 1.0]; 2], "the reset frame");
    let got = frame(gpu, &mut pack, &sources, (6, [4, 4]), None);
    assert_eq!(got, [[2.0, 1.0, 2.0, 1.0]; 2], "after the reset");
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

/// The pack's eight kept resources, four textures (both versions) and four buffers, read in
/// its material, its entities and its sky together, within the device's limits; on a device of
/// wgpu's default limits (16 sampled textures a stage), an entity or sky stage that reads them
/// is refused at load, saying why. The counts each stage's fragment shader binds are printed.
#[test]
fn a_pack_reads_eight_resources_in_its_material_entities_and_sky() {
    let _gpu = crate::gfx::test_gpu::lock();
    let mut kept = String::new();
    for i in 0..4 {
        kept += &format!(
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
    kept += &format!(
        "[[before_world]]\nfile = \"w.wgsl\"\ndispatch = [1, 1, 1]\nworkgroup = [1, 1, 1]\n\
         writes = [{}]\n",
        names.join(", ")
    );
    let reads = format!("reads = [{}]\n", reads.join(", "));
    let stage = |name: &str, file: &str| format!("[{name}]\nfile = \"{file}\"\n{reads}");
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
    let shown = format!(
        "fn shown() -> vec4<f32> {{\n    return vec4<f32>(0.0{}, 0.0{}, 0.0{}, 1.0);\n}}\n",
        sums[0], sums[1], sums[2]
    );
    let files = |body: &str| shown.clone() + body;
    let (m, e, s) = (files(MATERIAL), files(ENTITY), files(SKY));
    let all = [
        ("w.wgsl", writes.as_str()),
        ("m.wgsl", &m),
        ("e.wgsl", &e),
        ("s.wgsl", &s),
    ];
    let manifest = format!(
        "{kept}{}{}{}",
        stage("material", "m.wgsl"),
        stage("entity", "e.wgsl"),
        stage("sky", "s.wgsl")
    );
    if let Some((_, device, queue)) = device() {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (mut pack, sources) = built(&device, &queue, None, &manifest, &all).unwrap();
        let gpu = (&device, &queue);
        let got = frame(gpu, &mut pack, &sources, (1, [4, 4]), None);
        assert_eq!(got, [[10.0, 0.0, 100.0, 1.0]; 3]);
        let got = frame(gpu, &mut pack, &sources, (2, [4, 4]), None);
        assert_eq!(got, [[10.0, 10.0, 100.0, 1.0]; 3]);
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
        let fragment = |entries: &[&[wgpu::BindGroupLayoutEntry]]| {
            let seen = || {
                (entries.iter().flat_map(|e| e.iter()))
                    .filter(|e| e.visibility.contains(wgpu::ShaderStages::FRAGMENT))
            };
            let count = |f: fn(&wgpu::BindingType) -> bool| seen().filter(|e| f(&e.ty)).count();
            use wgpu::BindingType as T;
            [
                count(|t| matches!(t, T::Texture { .. })),
                count(|t| matches!(t, T::Sampler(_))),
                count(|t| {
                    matches!(
                        t,
                        T::Buffer {
                            ty: wgpu::BufferBindingType::Storage { .. },
                            ..
                        }
                    )
                }),
                count(|t| {
                    matches!(
                        t,
                        T::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            ..
                        }
                    )
                }),
            ]
        };
        let frame = pack.frame.entries();
        let kept = &pack.sky.as_ref().unwrap().kept.as_ref().unwrap().entries;
        let entities = crate::gfx::native_entity::entity_entries();
        let terrain = crate::gfx::native_material::terrain_entries();
        eprintln!(
            "fragment stage [sampled textures, samplers, storage buffers, uniform buffers]: \
             frame {:?}, kept {:?}; entities {:?} (group 0 {:?}), sky {:?} (group 0 {:?}), \
             material {:?}; this device takes {:?}",
            fragment(&[frame]),
            fragment(&[kept]),
            fragment(&[frame, kept, &entities]),
            fragment(&[&entities]),
            fragment(&[frame, kept, &terrain]),
            fragment(&[&terrain]),
            fragment(&[frame, kept, &terrain]),
            {
                let l = device.limits();
                [
                    l.max_sampled_textures_per_shader_stage,
                    l.max_samplers_per_shader_stage,
                    l.max_storage_buffers_per_shader_stage,
                    l.max_uniform_buffers_per_shader_stage,
                ]
            }
        );
    }
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    for (name, file, body) in [("entity", "e.wgsl", &e), ("sky", "s.wgsl", &s)] {
        let manifest = format!("{kept}{}", stage(name, file));
        let files = [("w.wgsl", writes.as_str()), (file, body)];
        let refused = built(&device, &queue, None, &manifest, &files).err();
        let why = refused.expect("over the default limits");
        assert!(why.starts_with(&format!("{file}: ")), "{why}");
        assert!(why.contains("sampled textures"), "{why}");
    }
}

/// What the entity and sky stages' kept groups make, counted with `Instance::generate_report`
/// over eight frames, every frame's groups held: two each (a parity each) on the first frame,
/// nothing on the next ones, two each again after a resize remakes the R-relative textures, and
/// after the water mirror's view changes the frame's group. Stages without `reads` in the same
/// pack make none: they draw with the frame's group, as before.
#[test]
fn the_entity_and_sky_stages_make_their_kept_groups_once() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((instance, device, queue)) = device() else {
        return;
    };
    let manifest = |reads: &str| {
        format!(
            "{COUNTER}[[resource]]\nname = \"screen\"\nformat = \"rgba16float\"\nscale = 1\n\
             keep = \"view\"\n[entity]\nfile = \"e.wgsl\"\n{reads}[sky]\nfile = \"s.wgsl\"\n{reads}"
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
    let mirror = (device.create_texture(&wgpu::TextureDescriptor {
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
    }))
    .create_view(&Default::default());
    let mut held = Vec::new();
    let mut made = |reads: &str, (entity, sky): (&str, &str)| {
        let files = [("w.wgsl", COUNTS), ("e.wgsl", entity), ("s.wgsl", sky)];
        let mut counts = vec![count()?];
        let (mut pack, _) = built(&device, &queue, None, &manifest(reads), &files).unwrap();
        counts.push(count()?);
        // Whether both stages drew with the frame's own group in every frame.
        let mut plain = true;
        for n in 1..=8 {
            let size = if n < 5 { [4, 4] } else { [8, 8] };
            let mirror = (n == 7).then_some(&mirror);
            frame((&device, &queue), &mut pack, &[], (n, size), mirror);
            let sky = pack
                .sky
                .as_ref()
                .unwrap()
                .group(&pack.frame, pack.post.parity());
            let entity = pack.entities().unwrap().1;
            plain &= *entity == pack.frame.group && *sky == pack.frame.group;
            held.extend([entity.clone(), sky.clone(), pack.frame.group.clone()]);
            held.extend(pack.post.kept.iter().flat_map(|k| k.worlds.iter().cloned()));
            counts.push(count()?);
        }
        let delta = |(a, b): (&[usize; 3], &[usize; 3])| [0, 1, 2].map(|i| b[i] - a[i]);
        let deltas = (counts.iter().zip(&counts[1..]).map(delta)).collect::<Vec<_>>();
        Some((deltas, plain))
    };
    let reads = "reads = [\"tex\", \"tex_previous\", \"count\"]\n";
    let (entity, sky) = (
        SHOWS_COUNT.to_owned() + ENTITY,
        SHOWS_COUNT.to_owned() + SKY,
    );
    // Without a report (a backend outside wgpu-core) there is nothing to count.
    let Some((with, with_frame_s)) = made(reads, (&entity, &sky)) else {
        return;
    };
    assert!(!with_frame_s);
    let plain = "fn shown() -> vec4<f32> { return vec4<f32>(1.0); }\n".to_owned();
    let (entity, sky) = (plain.clone() + ENTITY, plain + SKY);
    let (without, frame_s) = made("", (&entity, &sky)).unwrap();
    assert!(
        frame_s,
        "without `reads`, the stages draw with the frame's group"
    );
    eprintln!("with reads: {with:?}\nwithout: {without:?}");
    // Groups a frame: the `[[before_world]]` pass's two (a parity each) and, with `reads`, the
    // entities' two and the sky's two; again after the resize, and after the mirror's view
    // changes the frame's group (its own one, and with `reads` the stages' four).
    let steady = [0, 0, 0];
    assert_eq!(
        with[1..],
        [
            [0, 6, 0],
            steady,
            steady,
            steady,
            [0, 6, 0],
            steady,
            [0, 5, 0],
            steady
        ]
    );
    assert_eq!(
        without[1..],
        [
            [0, 2, 0],
            steady,
            steady,
            steady,
            [0, 2, 0],
            steady,
            [0, 1, 0],
            steady
        ]
    );
    // Building: the entities' and the sky's kept layouts are the two layouts more, their
    // pipelines as many.
    assert_eq!(with[0][0], without[0][0] + 2);
    assert_eq!(with[0][2], without[0][2]);
}

/// The entity stage draws with the bound scene and a kept buffer, then falls back to the stub
/// and the same kept resource when the next frame has no scene.
#[test]
fn the_entity_stage_draws_with_ray_hits_and_kept_reads() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((_, device, queue)) = ray_device() else {
        return;
    };
    let scene = Scene::new(&device, &queue, 4.0, 1);
    let traced = scene.traced();
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let manifest =
        format!("{COUNTER}[ray_tracing]\n[entity]\nfile = \"e.wgsl\"\nreads = [\"count\"]\n");
    let entity = "fn shown() -> vec4<f32> {
        let hit = trace(vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0), 64.0);
        return vec4<f32>(hit.distance, f32(count[0]), f32(frame.has_ray_tracing), 1.0);
    }";
    let entity = entity.to_owned() + ENTITY;
    let (mut pack, sources) = built(
        &device,
        &queue,
        Some(&scene),
        &manifest,
        &[("w.wgsl", COUNTS), ("e.wgsl", &entity)],
    )
    .unwrap();
    let (_, stub, with_scene) = sources
        .iter()
        .find(|(kind, ..)| *kind == StageKind::Entity)
        .unwrap();
    let mut draw = |n: u32, scene: Option<&Traced<'_>>| {
        let hud = crate::hud::HudBatch::scratch([4, 4]);
        let mut inputs = crate::gfx::pack_uniforms::initial_history_tests::frame(&hud);
        inputs.pack.frame_timing = Some(crate::gfx::pack_uniforms::FrameTiming::nth(n, 50.0));
        pack.write(
            &queue,
            &inputs,
            [4, 4],
            crate::gfx::output::DisplayValues::SDR,
            scene,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        pack.before_world(&device, &mut encoder, scene);
        pack.bind_terrain(&device, scene);
        let entity = pack.entity.as_ref().unwrap();
        let trace = entity.scene.as_ref().unwrap();
        let (source, layout) = if trace.drawn(pack.post.parity()).is_some() {
            (with_scene.as_ref().unwrap(), trace.layout())
        } else {
            (stub, &entity.kept.as_ref().unwrap().layout)
        };
        let group = pack.entity_layers().unwrap().frame;
        let target = shown(
            &device,
            &mut encoder,
            &(source.to_owned() + PROBE),
            layout,
            group,
        );
        texel((&device, &queue), encoder, &target)
    };
    assert_eq!(draw(1, Some(&traced)), [4.0, 1.0, 1.0, 1.0]);
    assert_eq!(draw(2, None), [-1.0, 2.0, 0.0, 1.0]);
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}
