//! The entity layer (`RAY_ENTITY`): its binding's cap, its reach, where the batch puts the
//! entities the camera doesn't draw, and on a GPU with ray queries the layer's BLAS traced
//! through the scene's own WGSL, as a pack that names the layer composes it.

use super::*;
use crate::block_entity_render::StaticSectionSnapshot;
use crate::gfx::native_post::trace::space_words;
use horizon_shader_api::stages::RAY_ENTITY_QUAD;

#[test]
fn the_layer_s_quads_are_cut_to_what_its_binding_holds() {
    let limits = wgpu::Limits {
        max_storage_buffer_binding_size: 1 << 20,
        ..Default::default()
    };
    // 176 bytes a quad of 44-byte vertices.
    assert_eq!(ENTITY_STRIDE, 44);
    let most = largest_entity_binding(&limits);
    assert_eq!(most, 5957);
    // Within it: as they are.
    let ranges = [(0, 5000), (5000, 900), (5900, 50)];
    assert_eq!(within_binding(ranges, most), (ranges, false));
    // The ray-only geometry, last in the stream, runs past it; one past its end starts there
    // with none.
    assert_eq!(
        within_binding([(0, 5000), (5000, 900), (5950, 100)], most),
        ([(0, 5000), (5000, 900), (5950, 7)], true)
    );
    assert_eq!(
        within_binding([(0, 6000), (6000, 10), (6020, 5)], most),
        ([(0, 5957), (5957, 0), (5957, 0)], true)
    );
    // An empty geometry past the end lost nothing.
    assert_eq!(
        within_binding([(0, 10), (20, 0), (7000, 0)], most),
        ([(0, 10), (20, 0), (5957, 0)], false)
    );
}

/// The layer reaches the columns the TLAS spans around the camera's (`within`), camera-relative,
/// over the world's height.
#[test]
fn the_layer_reaches_the_scene_s_columns() {
    // In column (-1, 2) at 8 chunks: x from column -9 to the end of column 7, z from -6 to 10.
    let camera = DVec3::new(-3.5, 70.0, 40.25);
    let (lo, hi) = reach_box(camera, 8);
    assert_eq!(lo, glam::Vec3::new(-144.0 + 3.5, -1.0e6, -96.0 - 40.25));
    assert_eq!(hi, glam::Vec3::new(128.0 + 3.5, 1.0e6, 176.0 - 40.25));
    let reach = crate::gfx::Frustum::boxed(lo, hi);
    let at = |x: f32, z: f32| {
        reach.intersects(
            glam::Vec3::new(x, -2.0, z),
            glam::Vec3::new(x + 1.0, 0.0, z + 1.0),
        )
    };
    // Behind the camera and beside it, near and at the edge; not a block past the last column.
    assert!(at(0.0, -5.0) && at(-40.0, 3.0) && at(130.5, 0.0));
    assert!(!at(132.0, 0.0) && !at(0.0, -137.5) && !at(0.0, 136.0));
    // Straight up and down: the columns' whole height.
    assert!(reach.intersects(
        glam::Vec3::new(0.0, 300.0, 0.0),
        glam::Vec3::new(1.0, 301.0, 1.0)
    ));
}

/// The entities the camera doesn't draw join the stream past everything a pass draws (the
/// materials, which the passes count from the front, the additive, hand and see-through quads)
/// and before the shadow-only quads, which the native shadow pass counts back from the end.
/// They are the third geometry, marked as the drawn ones are: only the runs' quads of the
/// ordinary material and improved transparency's, so a translucent item a mob holds, and a quad
/// in no run, stay out. Without them the stream is what it was.
#[test]
fn ray_only_quads_lie_past_every_draw_and_before_the_shadow_only_ones() {
    use crate::gfx::entity_batch::{EntityBatch, OIT_ENTITY};
    use crate::gfx::native_entity::{EntityKind, EntityRuns};
    let tagged = |layer, x| quad(x, 0.0, layer, 0xffff_ffff);
    let stream: Vec<Vertex> = [tagged(0, 0.0), tagged(32, 1.0), tagged(OIT_ENTITY, 2.0)].concat();
    let (additive, hand, see_through) = (tagged(2, 3.0), tagged(0, 4.0), tagged(0, 5.0));
    let shadow_only = tagged(0, 6.0);
    // Off camera: a mob (its body, a translucent item it holds, a layer of improved
    // transparency's), then a quad no run names.
    let ray_only: Vec<Vertex> = [
        tagged(0, 10.0),
        tagged(32, 11.0),
        tagged(OIT_ENTITY, 12.0),
        tagged(0, 13.0),
    ]
    .concat();
    let batch = |ray: bool| {
        let mut batch = EntityBatch::default();
        let mut runs = EntityRuns::default();
        if ray {
            batch.ray_only = ray_only.clone();
            runs.push(0..12, EntityKind::Entity);
        }
        batch.ray_kinds = runs;
        let counts = batch.update(&stream, &additive, &hand, &see_through, true);
        batch.append_shadow_only(&shadow_only);
        (batch, counts)
    };
    let (off, counts_off) = batch(false);
    let (on, counts_on) = batch(true);
    assert_eq!(counts_on, counts_off);
    // Drawn: the three materials' quads, the additive, hand and see-through ones.
    let drawn = 6 * 4;
    assert_eq!(off.combined.len(), drawn + 4);
    assert_eq!(on.combined.len(), drawn + 2 * 4 + 4);
    let bytes = |v: &[Vertex]| bytemuck::cast_slice::<Vertex, u8>(v).to_vec();
    assert_eq!(bytes(&on.combined[..drawn]), bytes(&off.combined[..drawn]));
    let tail = &on.combined[on.combined.len() - 4..];
    assert_eq!(bytes(tail), bytes(&shadow_only));
    assert_eq!(bytes(&off.combined[drawn..]), bytes(&shadow_only));
    // The ray-only ones: the body and improved transparency's layer, marked as an entity's.
    assert_eq!(on.ray_ranges[2], (6, 2));
    let xs: Vec<f32> = on.combined[drawn..drawn + 8]
        .iter()
        .step_by(4)
        .map(|v| v.pos[0])
        .collect();
    assert_eq!(xs, [10.0, 12.0]);
    let entity = crate::gfx::native_entity::kind_bits(EntityKind::Entity) | RAY_ENTITY_QUAD;
    assert!(
        on.combined[drawn..drawn + 8]
            .iter()
            .all(|v| v.layer & entity == entity)
    );
    // The drawn geometries are where they were, and nothing is left for the next frame.
    assert_eq!(on.ray_ranges[..2], off.ray_ranges[..2]);
    assert_eq!(off.ray_ranges[2], (6, 0));
    assert!(on.ray_only.is_empty() && on.ray_kinds.is_empty());
    // Improved transparency off: its layer is the ordinary material's, and still the layer's.
    let mut plain = EntityBatch::default();
    plain.ray_only = ray_only.clone();
    plain.ray_kinds.push(0..12, EntityKind::Entity);
    plain.update(&stream, &[], &[], &[], false);
    plain.append_shadow_only(&[]);
    assert_eq!(plain.ray_ranges[2], (3, 2));
}

/// A device with ray queries, if this machine has one.
fn ray_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    let features = adapter.features() & wgpu::Features::EXPERIMENTAL_RAY_QUERY;
    if features.is_empty() {
        return None;
    }
    let descriptor = wgpu::DeviceDescriptor {
        required_features: features,
        required_limits: adapter.limits(),
        experimental_features: super::super::experimental(features),
        ..Default::default()
    };
    pollster::block_on(adapter.request_device(&descriptor)).ok()
}

/// A quad of the stream facing +z at depth `z`, from `x` to `x + 1` and 0 to 1 high.
fn quad(x: f32, z: f32, layer: u32, color: u32) -> [Vertex; 4] {
    [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|[dx, dy]| Vertex {
        pos: [x + dx, dy, z],
        uv: [0.5, 0.5],
        layer,
        color,
        light: 240 | 120 << 8,
        normal: [0, 0, 127, 0],
        glint_uv: [0.0, 0.0],
    })
}

/// What the scene's WGSL makes of the layer, run on the GPU: rays over each layer mask, and
/// `horizon_ray_entity_keeps` on cases of entity.wgsl's alpha test.
const PROBE: &str = "
@group(0) @binding(0) var<storage, read_write> probe_out: array<vec4<f32>, 7>;
@compute @workgroup_size(1) fn probe() {
    let z = vec3<f32>(0.0, 0.0, 1.0);
    let a = trace_layers(vec3<f32>(0.3, 0.6, 0.0), z, 100.0, RAY_ENTITY);
    let b = trace_layers(vec3<f32>(10.3, 0.6, 0.0), z, 100.0, RAY_SOLID | RAY_ENTITY);
    probe_out[0] = vec4<f32>(a.distance, f32(a.entity), a.tint.r, a.tint.g);
    probe_out[1] = vec4<f32>(b.distance, f32(b.entity), b.tint.r, b.tint.g);
    let terrain = trace_any_layers(vec3<f32>(0.3, 0.6, 0.0), z, 100.0, RAY_SOLID | RAY_TRANSLUCENT);
    probe_out[2] = vec4<f32>(f32(terrain), a.light, f32(a.material + a.tags) + a.emission);
    let q = 0x10000000u;
    probe_out[3] = vec4<f32>(f32(horizon_ray_entity_keeps(q, 1.0, 0.1, 0.0)),
        f32(horizon_ray_entity_keeps(q, 0.5, 0.15, 0.0)), f32(horizon_ray_entity_keeps(0u, 1.0, 1.0, 0.0)),
        f32(horizon_ray_entity_keeps(q | 6u, 1.0, 1.0, 0.0)));
    probe_out[4] = vec4<f32>(f32(horizon_ray_entity_keeps(q | 38u, 0.5, 0.3, 0.2)),
        f32(horizon_ray_entity_keeps(q | 38u, 1.0, 0.1, 0.2)), f32(horizon_ray_entity_keeps(q | 38u, 0.05, 1.0, 0.0)),
        f32(horizon_ray_entity_keeps(q | 9u, 1.0, 1.0, 0.0)));
    probe_out[5] = vec4<f32>(f32(horizon_ray_entity_keeps(q | 2u, 1.0, 1.0, 0.0)),
        f32(horizon_ray_entity_keeps(q | 4u, 1.0, 1.0, 0.0)), f32(horizon_ray_entity_keeps(q | 10u, 1.0, 1.0, 0.0)),
        f32(horizon_ray_entity_keeps(q | 0x2000000u | 7u, 1.0, 1.0, 0.0)));
    let c = trace_layers(vec3<f32>(20.3, 0.6, 0.0), z, 100.0, RAY_ENTITY);
    probe_out[6] = vec4<f32>(c.distance, f32(c.entity), c.tint.r, c.tint.b);
}
";

/// The scene's WGSL for a pack whose post pass names `RAY_ENTITY`, as the check builds it.
fn scene_source() -> String {
    let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                    [[post]]\nfile = \"post.wgsl\"\n[ray_tracing]\n";
    let post = "fn post(input: PostInput) -> vec4<f32> {\n    \
                return input.color * f32(trace_any_layers(vec3<f32>(0.0), vec3<f32>(1.0, 0.0, 0.0), 1.0, RAY_ENTITY));\n}\n";
    let files = [
        (horizon_shader_api::MANIFEST, manifest),
        ("post.wgsl", post),
    ]
    .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
    .into();
    let pack = horizon_shader_api::Pack::from_files(files).unwrap();
    let report = horizon_shader_api::check(&pack).unwrap();
    let post = &report.stages[0];
    assert!(post.traces_entities());
    post.scene_source.clone().unwrap()
}

/// A 1×1 white texture: the atlas as a 2D view, the sprite arrays as an array view.
fn white(device: &wgpu::Device, queue: &wgpu::Queue) -> (wgpu::TextureView, wgpu::TextureView) {
    use wgpu::util::DeviceExt;
    let white = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
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
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &[255; 4],
    );
    let atlas = white.create_view(&Default::default());
    let sprites = white.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    (atlas, sprites)
}

/// A storage buffer of `size` bytes, zeros.
fn empty(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    })
}

/// Runs `probe` (writing `outputs` vec4s to `probe_out`) after the scene's WGSL against
/// `traced`, whose terrain store and sprite tables are empty (`store`, `layers`, `sprites`), once
/// `encoder`'s builds are done. What it wrote.
fn run_probe(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    mut encoder: wgpu::CommandEncoder,
    traced: &Traced<'_>,
    (store, layers, sprites): (&wgpu::Buffer, &wgpu::Buffer, &wgpu::TextureView),
    probe: &str,
    outputs: u64,
) -> Vec<[f32; 4]> {
    use wgpu::util::DeviceExt;
    let space = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&space_words(1, traced)),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: outputs * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl((scene_source() + probe).into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("probe"),
        compilation_options: Default::default(),
        cache: None,
    });
    let e = traced.entities.as_ref().unwrap();
    let mut scene = vec![
        (48, traced.tlas.as_binding()),
        (49, store.as_entire_binding()),
        (50, space.as_entire_binding()),
        (51, layers.as_entire_binding()),
        (60, wgpu::BindingResource::Buffer(e.vertices.clone())),
        (61, wgpu::BindingResource::TextureView(e.atlas)),
        (62, wgpu::BindingResource::Sampler(e.sampler)),
        (63, e.retained.as_entire_binding()),
    ];
    scene.extend((52..60).map(|b| (b, wgpu::BindingResource::TextureView(sprites))));
    let group = |index: u32, entries: Vec<(u32, wgpu::BindingResource<'_>)>| {
        let entries: Vec<_> = entries
            .into_iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
            .collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(index),
            entries: &entries,
        })
    };
    let groups = [
        group(0, vec![(0, out.as_entire_binding())]),
        group(1, Vec::new()),
        group(2, Vec::new()),
        group(3, scene),
    ];
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        for (i, g) in groups.iter().enumerate() {
            pass.set_bind_group(i as u32, g, &[]);
        }
        pass.dispatch_workgroups(1, 1, 1);
    }
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: outputs * 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(&out, 0, &read, 0, outputs * 16);
    queue.submit([encoder.finish()]);
    read.slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    bytemuck::cast_slice(&read.slice(..).get_mapped_range().unwrap()[..]).to_vec()
}

/// Three geometries apart in the stream, and the quads before and between them: a ray reads the
/// quad its geometry's own base says, the ray-only geometry's too, passes the layer's quads that
/// aren't marked, reaches the layer only when its mask names it, and finds the vertices' colour
/// and light. The alpha test keeps what entity.wgsl draws.
#[test]
fn rays_reach_the_entity_layer_through_its_three_bases() {
    let Some((device, queue)) = ray_device() else {
        eprintln!("no GPU with ray queries: skipped");
        return;
    };
    let (red, green, blue, magenta) = (0xff00_00ff, 0xff00_ff00, 0xffff_0000, 0xffff_00ff);
    // Geometry 0: an unmarked quad, then a marked red one behind it. Then a blue quad in no
    // geometry, geometry 1, a green quad, and the ray-only geometry 2, a magenta one.
    let stream: Vec<Vertex> = [
        quad(0.0, 2.0, 0, blue),
        quad(0.0, 4.0, RAY_ENTITY_QUAD, red),
        quad(10.0, 1.0, RAY_ENTITY_QUAD, blue),
        quad(10.0, 3.0, RAY_ENTITY_QUAD, green),
        quad(20.0, 5.0, RAY_ENTITY_QUAD, magenta),
    ]
    .concat();
    use wgpu::util::DeviceExt;
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&stream),
        usage: wgpu::BufferUsages::VERTEX | ENTITY_INPUT,
    });
    let (atlas, sprites) = white(&device, &queue);
    let stream = Stream {
        buffer: &buffer,
        atlas: &atlas,
        ranges: [(0, 2), (3, 1), (4, 1)],
        retained: &[],
    };
    let mut entities = Entities::new(&device, &stream);
    assert!(!entities.fit(&device, &stream));
    let indices = super::super::quad_indices(&device, 5);
    let mut tlas = create_tlas(&device, 1);
    let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    tlas[0] = Some(wgpu::TlasInstance::new(
        &entities.blas,
        identity,
        0,
        RAY_ENTITY as u8,
    ));
    let mut encoder = device.create_command_encoder(&Default::default());
    let sizes = entities.built_sizes();
    let builds = [entities.build(&sizes, &indices)];
    encoder.build_acceleration_structures(builds.iter(), std::iter::once(&tlas));
    // The terrain's store and sprite tables, empty: no terrain instance is in the TLAS.
    let (store, layers) = (empty(&device, 24), empty(&device, 8));
    let traced = Traced {
        tlas: &tlas,
        solid_instances: 0,
        store: &store,
        translucent_instances: 0,
        entities: Some(TracedEntities {
            vertices: buffer.as_entire_buffer_binding(),
            atlas: &atlas,
            sampler: &entities.sampler,
            bases: entities.ranges.map(|r| r.0),
            retained: entities.retained.store(),
            retained_instances: 1,
        }),
        generation: 0,
        camera: [0.0; 3],
        distance: 100.0,
    };
    let device_queue = (&device, &queue);
    let got = run_probe(
        device_queue,
        encoder,
        &traced,
        (&store, &layers, &sprites),
        PROBE,
        7,
    );
    // Through the unmarked quad to the red one; the green one at geometry 1's own base, not
    // the blue one before it.
    assert_eq!(got[0], [4.0, 1.0, 1.0, 0.0]);
    assert_eq!(got[1], [3.0, 1.0, 0.0, 1.0]);
    // Terrain's layers alone miss it; the hit's light is block 7.5, sky 15, nothing of a block's.
    assert_eq!(got[2], [0.0, 0.5, 1.0, 0.0]);
    // Kept at alpha 0.1; not at 0.5 * 0.15, nor unmarked, nor an unlit overlay (6, 9, 2, 4,
    // 10), whatever the kind bits; on the dissolve, kept where the tint reaches the mask and
    // the texel 0.1, the tint itself then counting as 1.
    assert_eq!(got[3], [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(got[4], [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(got[5], [0.0, 0.0, 0.0, 1.0]);
    // The ray-only quad, at the third base.
    assert_eq!(got[6], [5.0, 1.0, 1.0, 1.0]);
}

/// Rays at the stream's quad and at two retained sections' (`ray_retained.rs`).
const RETAINED_PROBE: &str = "
@group(0) @binding(0) var<storage, read_write> probe_out: array<vec4<f32>, 5>;
@compute @workgroup_size(1) fn probe() {
    let z = vec3<f32>(0.0, 0.0, 1.0);
    let a = trace_layers(vec3<f32>(0.3, 0.6, 0.0), z, 100.0, RAY_ENTITY);
    let b = trace_layers(vec3<f32>(10.3, 0.6, 0.0), z, 100.0, RAY_ENTITY);
    let c = trace_layers(vec3<f32>(20.3, 0.6, 0.0), z, 100.0, RAY_SOLID | RAY_ENTITY);
    probe_out[0] = vec4<f32>(a.distance, f32(a.entity), a.tint.r, a.tint.b);
    probe_out[1] = vec4<f32>(b.distance, f32(b.entity), b.tint.g, b.tint.b);
    probe_out[2] = vec4<f32>(c.distance, f32(c.entity), c.tint.b, c.tint.g);
    probe_out[3] = vec4<f32>(b.light, b.uv);
    probe_out[4] = vec4<f32>(b.normal.z, f32(b.material + b.tags + b.fluid) + b.emission,
        f32(trace_any_layers(vec3<f32>(10.3, 0.6, 0.0), z, 100.0, RAY_SOLID | RAY_TRANSLUCENT)),
        b.albedo.a);
}
";

/// The retained block entities in the layer: a section's quads are read in the retained store
/// at the base its instance's custom data says, apart from the stream's, which keep theirs. Only
/// the layer's quads are in it (a text quad isn't), a section is built once and again only when
/// its revision changes, and one whose block entities go leaves the TLAS.
#[test]
fn rays_reach_retained_block_entities_through_their_sections() {
    use crate::gfx::entity_batch::TEXT_POLYGON_OFFSET_OVERLAY;
    let Some((device, queue)) = ray_device() else {
        eprintln!("no GPU with ray queries: skipped");
        return;
    };
    let (red, green, blue) = (0xff00_00ff, 0xff00_ff00, 0xffff_0000);
    use wgpu::util::DeviceExt;
    // The stream: a red quad at x 0, depth 4.
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&quad(0.0, 4.0, RAY_ENTITY_QUAD, red)),
        usage: wgpu::BufferUsages::VERTEX | ENTITY_INPUT,
    });
    let (atlas, sprites) = white(&device, &queue);
    // Section a, its corner at the anchor's: a text quad in front, which isn't the layer's, and
    // a green quad at x 10, depth 2. Section b, 16 blocks east: a blue quad at its x 4 (20 in
    // the scene), depth 3. Neither is marked: the layer marks them.
    let unmarked = |x, z, layer, color| {
        let mut q = quad(x, z, layer, color);
        q.iter_mut().for_each(|v| v.uv = [0.25, 0.75]);
        q
    };
    let a = [
        unmarked(10.0, 1.0, TEXT_POLYGON_OFFSET_OVERLAY, blue),
        unmarked(10.0, 2.0, 0, green),
    ]
    .concat();
    let snapshot = |section, origin, revision, vertices: &[Vertex]| StaticSectionSnapshot {
        section,
        origin,
        revision,
        entities: 1,
        vertices: vertices.into(),
    };
    let sections = |b_revision| {
        [
            snapshot([0, 4, 0], [0, 0, 0], 1, &a),
            snapshot(
                [1, 4, 0],
                [16, 0, 0],
                b_revision,
                &unmarked(4.0, 3.0, 0, blue),
            ),
        ]
    };
    let stream = Stream {
        buffer: &buffer,
        atlas: &atlas,
        ranges: [(0, 1), (1, 0), (1, 0)],
        retained: &[],
    };
    let mut entities = Entities::new(&device, &stream);
    entities.fit(&device, &stream);
    let indices = super::super::quad_indices(&device, 4);
    let (store, layers) = (empty(&device, 24), empty(&device, 8));
    // Builds the frame's sections (`update`, `builds`), the stream's instance and theirs after
    // it, and runs the probe.
    let mut frame = |snapshots: &[StaticSectionSnapshot]| {
        let mut encoder = device.create_command_encoder(&Default::default());
        entities
            .retained
            .update(&device, &mut encoder, snapshots, (0, 0), 8);
        let built = entities.retained.builds(&indices).count();
        let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let mut tlas = create_tlas(&device, 4);
        tlas[0] = Some(wgpu::TlasInstance::new(
            &entities.blas,
            identity,
            0,
            RAY_ENTITY as u8,
        ));
        let mut n = 1;
        for instance in entities.retained.instances((0, 0), 8) {
            tlas[n] = Some(instance);
            n += 1;
        }
        let sizes = entities.built_sizes();
        let mut builds = vec![entities.build(&sizes, &indices)];
        builds.extend(entities.retained.builds(&indices));
        encoder.build_acceleration_structures(builds.iter(), std::iter::once(&tlas));
        drop(builds);
        let traced = Traced {
            tlas: &tlas,
            solid_instances: 0,
            store: &store,
            translucent_instances: 0,
            entities: Some(TracedEntities {
                vertices: buffer.as_entire_buffer_binding(),
                atlas: &atlas,
                sampler: &entities.sampler,
                bases: entities.ranges.map(|r| r.0),
                retained: entities.retained.store(),
                retained_instances: 1,
            }),
            generation: 0,
            camera: [0.0; 3],
            distance: 100.0,
        };
        let device_queue = (&device, &queue);
        let tables = (&store, &layers, &sprites);
        let got = run_probe(device_queue, encoder, &traced, tables, RETAINED_PROBE, 5);
        (built, n - 1, got)
    };
    let (built, instances, got) = frame(&sections(2));
    assert_eq!((built, instances), (2, 2));
    // The stream's red quad at its own base; section a's green one, behind the text quad that
    // isn't the layer's; section b's blue one at its base, past a's quads, with solid terrain
    // named too.
    assert_eq!(got[0], [4.0, 1.0, 1.0, 0.0]);
    assert_eq!(got[1], [2.0, 1.0, 1.0, 0.0]);
    assert_eq!(got[2], [3.0, 1.0, 1.0, 0.0]);
    // Its light is the vertices' (block 7.5, sky 15), its uv theirs, its normal against the ray,
    // nothing of a block's, and the texel opaque; terrain's layers alone don't reach it.
    assert_eq!(got[3], [0.5, 1.0, 0.25, 0.75]);
    assert_eq!(got[4], [-1.0, 0.0, 0.0, 1.0]);
    // Nothing changed: nothing is built, and the rays find the same.
    let (built, instances, again) = frame(&sections(2));
    assert_eq!((built, instances, &again), (0, 2, &got));
    // Section b at another revision (a chest opened and closed): b alone is built again.
    let (built, _, again) = frame(&sections(3));
    assert_eq!((built, &again), (1, &got));
    // Section b's block entities went: it leaves the TLAS, and the ray at it finds nothing.
    let (built, instances, gone) = frame(&sections(3)[..1]);
    assert_eq!((built, instances), (0, 1));
    assert_eq!(gone[2][0], -1.0);
    assert_eq!(gone[..2], got[..2]);
}
