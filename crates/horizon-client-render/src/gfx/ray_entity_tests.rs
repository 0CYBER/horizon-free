//! The entity layer (`RAY_ENTITY`): its binding's cap, and on a GPU with ray queries the layer's
//! BLAS traced through the scene's own WGSL, as a pack that names the layer composes it.

use super::*;
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
    let ranges = [(0, 5000), (5000, 900)];
    assert_eq!(within_binding(ranges, most), (ranges, false));
    // The second geometry runs past it, and one past its end starts there with none.
    assert_eq!(
        within_binding([(0, 5000), (5000, 1000)], most),
        ([(0, 5000), (5000, 957)], true)
    );
    assert_eq!(
        within_binding([(0, 6000), (6000, 10)], most),
        ([(0, 5957), (5957, 0)], true)
    );
    // An empty geometry past the end lost nothing.
    assert_eq!(
        within_binding([(0, 10), (7000, 0)], most),
        ([(0, 10), (5957, 0)], false)
    );
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
@group(0) @binding(0) var<storage, read_write> probe_out: array<vec4<f32>, 6>;
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

/// Two geometries apart in the stream, and the quads before and between them: a ray reads the
/// quad its geometry's own base says, passes the layer's quads that aren't marked, reaches the
/// layer only when its mask names it, and finds the vertices' colour and light. The alpha test
/// keeps what entity.wgsl draws.
#[test]
fn rays_reach_the_entity_layer_through_its_two_bases() {
    let Some((device, queue)) = ray_device() else {
        eprintln!("no GPU with ray queries: skipped");
        return;
    };
    let (red, green, blue) = (0xff00_00ff, 0xff00_ff00, 0xffff_0000);
    // Geometry 0: an unmarked quad, then a marked red one behind it. Then a blue quad in no
    // geometry, and geometry 1, a green quad.
    let stream: Vec<Vertex> = [
        quad(0.0, 2.0, 0, blue),
        quad(0.0, 4.0, RAY_ENTITY_QUAD, red),
        quad(10.0, 1.0, RAY_ENTITY_QUAD, blue),
        quad(10.0, 3.0, RAY_ENTITY_QUAD, green),
    ]
    .concat();
    use wgpu::util::DeviceExt;
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&stream),
        usage: wgpu::BufferUsages::VERTEX | ENTITY_INPUT,
    });
    let white = device.create_texture_with_data(
        &queue,
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
    let stream = Stream {
        buffer: &buffer,
        atlas: &atlas,
        ranges: [(0, 2), (3, 1)],
    };
    let mut entities = Entities::new(&device, &stream);
    assert!(!entities.fit(&device, &stream));
    let indices = super::super::quad_indices(&device, 4);
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
    let empty = |size| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    };
    let (store, layers) = (empty(24), empty(8));
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
        }),
        generation: 0,
        camera: [0.0; 3],
        distance: 100.0,
    };
    let space = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&space_words(1, &traced)),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 6 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl((scene_source() + PROBE).into()),
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
    ];
    scene.extend((52..60).map(|b| (b, wgpu::BindingResource::TextureView(&sprites))));
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
        size: 6 * 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(&out, 0, &read, 0, 6 * 16);
    queue.submit([encoder.finish()]);
    read.slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let got: Vec<[f32; 4]> =
        bytemuck::cast_slice(&read.slice(..).get_mapped_range().unwrap()[..]).to_vec();
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
}
