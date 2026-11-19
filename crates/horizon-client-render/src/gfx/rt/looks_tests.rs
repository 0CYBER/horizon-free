//! A terrain hit's looks on a GPU with ray queries: blocks meshed with their quads' looks, written
//! into the store as `ray_terrain_tests` writes them (either layout of the arena) with the look
//! store beside it, and every quad traced on both its triangles through the scene's WGSL. The
//! hit's tint, shade and AO match the material stage. The hit's emission comes from the vertex
//! regardless of the look binding. Needs prepared resources, like the mesher's geometry tests.

use std::collections::BTreeSet;

use glam::Vec3;

use super::entity_tests::{empty, ray_device_of, run_scene, white};
use super::*;
use crate::mesher::{SHADES, SectionMesh, look_tests::meshed, packed};

/// How far in front of its quad each ray starts: less than the 0.019 between a comparator
/// torch's nested elements.
const EPS: f32 = 0.01;
/// How far a hit's value may be from the material's.
const NEAR: f32 = 2e-3;

/// Blocks four apart, a section each: a plain cube, a grass block (its biome's tint, its side's
/// overlay), an inner corner of stone (ambient occlusion), stained glass and water
/// (translucent), and lava, whose quads have no record (their tint unknown), in a section that
/// has records for the others; then a torch (shade-less), a comparator (shaded and shade-less
/// elements), and one and four lit candles (emission 3/15 and 12/15), too many quads without a
/// record for that.
type Blocks = &'static [(
    &'static str,
    &'static [(&'static str, &'static str)],
    [i32; 3],
)];
const SECTIONS: [Blocks; 2] = [
    &[
        ("minecraft:stone", &[], [2, 2, 2]),
        ("minecraft:grass_block", &[], [6, 2, 2]),
        ("minecraft:stone", &[], [2, 2, 6]),
        ("minecraft:stone", &[], [2, 3, 6]),
        ("minecraft:stone", &[], [3, 2, 6]),
        ("minecraft:red_stained_glass", &[], [6, 2, 6]),
        ("minecraft:water", &[], [10, 2, 6]),
        ("minecraft:lava", &[], [14, 2, 6]),
    ],
    &[
        ("minecraft:torch", &[], [10, 2, 2]),
        ("minecraft:comparator", &[("powered", "true")], [14, 2, 2]),
        ("minecraft:candle", &[("lit", "true")], [2, 2, 10]),
        (
            "minecraft:candle",
            &[("lit", "true"), ("candles", "4")],
            [6, 2, 10],
        ),
    ],
];

/// The points traced on a quad of corners 0..3, as each corner's weight: one inside the
/// triangle (0, 1, 2), one inside (0, 2, 3).
const POINTS: [[f32; 4]; 2] = [[0.2, 0.5, 0.3, 0.0], [0.25, 0.0, 0.35, 0.4]];

fn decode(c: f32) -> f32 {
    c.max(0.0).powf(2.2)
}

fn byte(word: u32, i: usize) -> f32 {
    ((word >> (8 * i)) & 255) as f32 / 255.0
}

/// What the material stage's `MaterialInput` has at the point of quad `corners` weighted `w`,
/// whose look is `look`: `[tint.rgb, shade]`, `[shaded_tint.rgb, ao]`, `[emission, ..]`.
pub(in crate::gfx) fn material(
    corners: &[TerrainVertex; 4],
    look: QuadLook,
    w: [f32; 4],
) -> [[f32; 4]; 3] {
    let [x, y] = look.0;
    let code = (x >> 24) & 7;
    let blend = |f: &dyn Fn(usize) -> f32| (0..4).map(|i| f(i) * w[i]).sum::<f32>();
    let color = [0, 1, 2].map(|c| blend(&|i| byte(corners[i].color, c)));
    let tint = match code {
        0 => color,
        _ => [0, 1, 2].map(|c| byte(x, c)),
    };
    let shade = match code {
        1..=7 => SHADES[code as usize - 1],
        _ => 1.0,
    };
    let ao = match code {
        0 => 1.0,
        _ => blend(&|i| byte(y, i)),
    };
    let [r, g, b] = tint.map(decode);
    let [sr, sg, sb] = color.map(decode);
    let emission = (15 - (corners[0].color >> 28)) as f32 / 15.0;
    [
        [r, g, b, decode(shade)],
        [sr, sg, sb, decode(ao)],
        [emission, 0.0, 0.0, 0.0],
    ]
}

/// A ray at a point of a quad: its layer's mask, the quad's point, and the origin `EPS` in
/// front of it along the quad's normal.
struct Ray {
    layer: u32,
    point: Vec3,
    origin: Vec3,
    direction: Vec3,
}

fn corners(q: &[TerrainVertex; 4]) -> [Vec3; 4] {
    [0, 1, 2, 3].map(|i| Vec3::from(q[i].pos))
}

/// Each quad of the solid then the translucent layer at both `POINTS`.
fn rays(mesh: &SectionMesh) -> Vec<Ray> {
    let layers = [
        (&mesh.solid, RAY_SOLID),
        (&mesh.translucent, RAY_TRANSLUCENT),
    ];
    let mut rays = Vec::new();
    for (quads, layer) in layers {
        for q in quads.as_chunks::<4>().0 {
            let p = corners(q);
            let normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            if normal == Vec3::ZERO {
                continue;
            }
            for w in POINTS {
                let point = p[0] * w[0] + p[1] * w[1] + p[2] * w[2] + p[3] * w[3];
                rays.push(Ray {
                    layer,
                    point,
                    origin: point + normal * EPS,
                    direction: -normal,
                });
            }
        }
    }
    rays
}

/// Each corner's weight at `point` of quad `q`, when the point is on it.
fn weights(q: &[TerrainVertex; 4], point: Vec3) -> Option<[f32; 4]> {
    let p = corners(q);
    [[0, 1, 2], [0, 2, 3]].into_iter().find_map(|[a, b, c]| {
        let (e1, e2, d) = (p[b] - p[a], p[c] - p[a], point - p[a]);
        let normal = e1.cross(e2);
        if normal.length_squared() < 1e-12 || d.dot(normal.normalize()).abs() > 1e-4 {
            return None;
        }
        let (d00, d01, d11) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
        let (d20, d21) = (d.dot(e1), d.dot(e2));
        let det = d00 * d11 - d01 * d01;
        let v = (d11 * d20 - d01 * d21) / det;
        let u = (d00 * d21 - d01 * d20) / det;
        let mut w = [0.0; 4];
        (w[a], w[b], w[c]) = (1.0 - v - u, v, u);
        w.iter().all(|&x| x >= -1e-4).then_some(w)
    })
}

/// The scene's WGSL of a pack whose post pass reads a hit's `shaded_tint`, and its `tint` too
/// where `looks`: then it has the scene's looks.
fn scene(looks: bool) -> String {
    let manifest = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n\
                    [[post]]\nfile = \"post.wgsl\"\n[ray_tracing]\n";
    let read = if looks { " * h.tint.r" } else { "" };
    let post = format!(
        "fn post(input: PostInput) -> vec4<f32> {{\n    \
         let h = trace(vec3<f32>(0.0), vec3<f32>(1.0, 0.0, 0.0), 1.0);\n    \
         return input.color * h.shaded_tint.r{read};\n}}\n"
    );
    let files = [
        (horizon_shader_api::MANIFEST, manifest),
        ("post.wgsl", post.as_str()),
    ]
    .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
    .into();
    let pack = horizon_shader_api::Pack::from_files(files).unwrap();
    let report = horizon_shader_api::check(&pack).unwrap();
    assert_eq!(report.stages[0].traces_looks(), looks);
    report.stages[0].scene_source.clone().unwrap()
}

/// `rays` traced through the scene of `mesh`, its solid quads unpacked from their compact
/// `records` (`packed`) or its vertices, with the look store `looks` (none: the scene of a pack
/// that reads no look). Each ray's `[tint.rgb, shade]`, `[shaded_tint.rgb, ao]`,
/// `[emission, distance, ..]`.
fn trace(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    (mesh, records, packed): (&SectionMesh, &[[u32; 6]], bool),
    looks: Option<&[QuadLook]>,
    rays: &[Ray],
) -> Vec<[[f32; 4]; 3]> {
    use wgpu::util::DeviceExt;
    let solid = (mesh.solid.len() / 4) as u32;
    let translucent = (mesh.translucent.len() / 4) as u32;
    let (page, base) = match packed {
        true => (bytemuck::cast_slice(records), packed::PACKED_BASE),
        false => (bytemuck::cast_slice::<_, u8>(&mesh.solid), 0),
    };
    let store = store_buffer(device, u64::from(solid + translucent) * 4).unwrap();
    let at = u64::from(solid) * 4 * STRIDE;
    queue.write_buffer(&store, at, bytemuck::cast_slice(&mesh.translucent));
    let init = |contents: &[u8]| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents,
            usage: wgpu::BufferUsages::STORAGE,
        })
    };
    let page = init(page);
    let looks = looks.map(|l| init(bytemuck::cast_slice(l)));
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        let job = RtJob {
            base,
            quads: solid,
            out: 0,
            pad: 0,
        };
        let unpack = unpack_pipeline(device);
        unpack_page(device, &mut pass, &unpack, &page, &[job], &store);
    }
    let indices = super::super::query::quad_indices(device, solid.max(translucent));
    let (solid_size, solid_blas) = new_blas(device, solid).unwrap();
    let (glass_size, glass_blas) = new_blas(device, translucent).unwrap();
    let builds = [
        (&solid_blas, &solid_size, 0),
        (&glass_blas, &glass_size, solid),
    ]
    .map(|(blas, size, first)| wgpu::BlasBuildEntry {
        blas,
        geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
            size,
            vertex_buffer: &store,
            first_vertex: first * 4,
            vertex_stride: STRIDE,
            index_buffer: Some(&indices),
            first_index: Some(0),
            transform_buffer: None,
            transform_buffer_offset: None,
        }]),
    });
    let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let mut tlas = create_tlas(device, 2);
    tlas[0] = Some(wgpu::TlasInstance::new(
        &solid_blas,
        identity,
        0,
        RAY_SOLID as u8,
    ));
    let glass = RAY_TRANSLUCENT as u8;
    tlas[1] = Some(wgpu::TlasInstance::new(&glass_blas, identity, solid, glass));
    encoder.build_acceleration_structures(builds.iter(), std::iter::once(&tlas));
    let traced = Traced {
        tlas: &tlas,
        solid_instances: 1,
        store: &store,
        looks: looks.as_ref(),
        translucent_instances: 2,
        entities: None,
        generation: 0,
        camera: [0.0; 3],
        nether: false,
        distance: 100.0,
    };
    let v = |v: Vec3, w: f32| format!("vec4<f32>({:?}, {:?}, {:?}, {w:?})", v.x, v.y, v.z);
    let words: Vec<String> = (rays.iter())
        .flat_map(|r| [v(r.origin, r.layer as f32), v(r.direction, 0.0)])
        .collect();
    let n = rays.len();
    let probe = format!(
        "var<private> probe_rays: array<vec4<f32>, {}> = array<vec4<f32>, {}>({});\n\
         @group(0) @binding(0) var<storage, read_write> probe_out: array<vec4<f32>, {}>;\n\
         @compute @workgroup_size(1) fn probe() {{\n    \
         for (var i = 0u; i < {n}u; i++) {{\n        \
         let o = probe_rays[2u * i];\n        \
         let h = trace_layers(o.xyz, probe_rays[2u * i + 1u].xyz, {:?}, u32(o.w));\n        \
         probe_out[3u * i] = vec4<f32>(h.tint.rgb, h.shade);\n        \
         probe_out[3u * i + 1u] = vec4<f32>(h.shaded_tint.rgb, h.ao);\n        \
         probe_out[3u * i + 2u] = vec4<f32>(h.emission, h.distance, 0.0, 0.0);\n    \
         }}\n}}\n",
        2 * n,
        2 * n,
        words.join(", "),
        3 * n,
        2.0 * EPS
    );
    let (_, sprites) = white(device, queue);
    let layers = empty(device, 8);
    let source = scene(traced.looks.is_some()) + &probe;
    let got = run_scene(
        (device, queue),
        encoder,
        &traced,
        (&store, &layers, &sprites),
        &source,
        3 * n as u64,
    );
    got.as_chunks::<3>().0.to_vec()
}

/// What the rays met, against what the material has at their points with the looks `looks`.
#[derive(Default)]
struct Tally {
    /// Rays whose hit is the material's at a quad there, and those whose isn't.
    agree: usize,
    disagree: Vec<String>,
    /// Rays that met another surface first.
    elsewhere: usize,
    /// Of the agreeing ones: the shade codes, whether a look's tint wasn't white, the points
    /// (by `POINTS`) with ambient occlusion, and the translucent quads.
    codes: BTreeSet<u32>,
    tinted: bool,
    shadowed: [bool; 2],
    translucent: usize,
}

fn tally(mesh: &SectionMesh, looks: &[QuadLook], rays: &[Ray], got: &[[[f32; 4]; 3]]) -> Tally {
    let solid = mesh.solid.len() / 4;
    let mut t = Tally::default();
    for (i, (ray, got)) in rays.iter().zip(got).enumerate() {
        if (got[2][1] - EPS).abs() > 2e-4 {
            t.elsewhere += 1;
            continue;
        }
        let (quads, first) = match ray.layer == RAY_SOLID {
            true => (&mesh.solid, 0),
            false => (&mesh.translucent, solid),
        };
        let near = |want: &[[f32; 4]; 3]| {
            (got[2][0] - want[2][0]).abs() <= NEAR
                && (0..2).all(|r| (0..4).all(|c| (got[r][c] - want[r][c]).abs() <= NEAR))
        };
        let found = (quads.as_chunks::<4>().0.iter().enumerate()).find_map(|(q, corners)| {
            let w = weights(corners, ray.point)?;
            let look = looks[first + q];
            near(&material(corners, look, w)).then_some((look, w))
        });
        let Some((look, w)) = found else {
            t.disagree
                .push(format!("ray {i} at {:?}: {got:?}", ray.point));
            continue;
        };
        t.agree += 1;
        let [x, y] = look.0;
        let code = (x >> 24) & 7;
        t.codes.insert(code);
        t.tinted |= code != 0 && x & 0xff_ffff != 0xff_ffff;
        let point = usize::from(w[1] == 0.0 && w[3] > 0.0);
        t.shadowed[point] |= code != 0 && (0..4).any(|c| w[c] > 0.0 && byte(y, c) < 1.0);
        t.translucent += usize::from(ray.layer != RAY_SOLID);
    }
    t
}

/// Every quad's points, traced on each adapter with ray queries (Vulkan, and DirectX 12 through
/// DXC: FXC compiles no ray queries), in the overworld and under the Nether's shading, from
/// either layout: each hit that met its point has the material's looks there, and among them are
/// every shade code each world has (0.9 is the Nether's, 6 and 7 its fluid sides'), a biome's
/// tint, ambient occlusion on
/// both triangles, and translucent quads. With the look store
/// zeroed each has a quad's without a look; that, and the scene without looks, disagree.
#[test]
#[ignore = "daily"]
fn a_terrain_hit_has_the_material_s_looks_at_its_point() {
    let _gpu = crate::gfx::test_gpu::lock();
    let meshes = [false, true].map(|nether| (nether, SECTIONS.map(|s| meshed(s, true, nether))));
    // The first section's solid quads have records, some not (lava); the second has none.
    for (_, [(cubes, records), (_, none)]) in &meshes {
        let solid = cubes.solid.len() / 4;
        let packed = packed::packed_quads(records, solid);
        assert!(
            packed > 0 && packed < solid && none.is_empty(),
            "{packed} of {solid}"
        );
    }
    let mut traced = 0;
    for (adapter, name) in crate::gfx::test_gpu::each_adapter() {
        if name.contains("FXC") {
            continue;
        }
        let Some((device, queue)) = ray_device_of(&adapter) else {
            eprintln!("{name}: no ray queries: skipped");
            continue;
        };
        let info = adapter.get_info();
        eprintln!(
            "{name}: {} ({:?}, {})",
            info.name, info.backend, info.driver
        );
        traced += 1;
        for (nether, sections) in &meshes {
            let mut seen = Tally::default();
            for (s, (mesh, records)) in sections.iter().enumerate() {
                let looks: Vec<QuadLook> = (mesh.solid_looks.iter())
                    .chain(&mesh.translucent_looks)
                    .copied()
                    .collect();
                assert_eq!(looks.len(), (mesh.solid.len() + mesh.translucent.len()) / 4);
                let zero = vec![QuadLook::default(); looks.len()];
                let rays = rays(mesh);
                for packed in [false, true]
                    .into_iter()
                    .filter(|&p| !p || !records.is_empty())
                {
                    let case = format!("{name}, Nether {nether}, section {s}, records {packed}");
                    let at = |l: Option<&[QuadLook]>| {
                        trace((&device, &queue), (mesh, records, packed), l, &rays)
                    };
                    let looked = at(Some(&looks));
                    let t = tally(mesh, &looks, &rays, &looked);
                    eprintln!(
                        "{case}: {} rays, {} agree, {} met another surface first",
                        rays.len(),
                        t.agree,
                        t.elsewhere
                    );
                    assert!(t.disagree.is_empty(), "{case}: {:#?}", t.disagree);
                    assert!(
                        t.agree * 10 >= rays.len() * 9,
                        "{case}: too few rays met their point"
                    );
                    // Zeroed, each is a quad's without a look; the material's looks it isn't.
                    let zeroed = at(Some(&zero));
                    let z = tally(mesh, &zero, &rays, &zeroed);
                    assert!(z.disagree.is_empty(), "{case}, zeroed: {:#?}", z.disagree);
                    let no_store = at(None);
                    let fallback = tally(mesh, &zero, &rays, &no_store);
                    assert!(
                        fallback.disagree.is_empty(),
                        "{case}, no store: {:#?}",
                        fallback.disagree
                    );
                    assert_eq!(fallback.agree, z.agree, "{case}, no store versus zeroed");
                    for ((actual, zeroed), plain) in looked.iter().zip(&zeroed).zip(&no_store) {
                        assert_eq!(actual[2], plain[2], "{case}, emission and hit distance");
                        assert_eq!(zeroed[2], plain[2], "{case}, emission with a zeroed store");
                    }
                    let without = tally(mesh, &looks, &rays, &zeroed).disagree.len();
                    let plain = tally(mesh, &looks, &rays, &no_store).disagree.len();
                    eprintln!(
                        "{case}: against the material, {without} disagree zeroed, {plain} without looks"
                    );
                    assert!(without * 2 > t.agree && plain * 2 > t.agree, "{case}");
                    seen.codes.extend(&t.codes);
                    seen.tinted |= t.tinted;
                    seen.shadowed = [0, 1].map(|p| seen.shadowed[p] | t.shadowed[p]);
                    seen.translucent += t.translucent;
                }
            }
            let codes: BTreeSet<u32> = match nether {
                false => [1, 2, 3, 5].into(),
                true => [2, 3, 4, 6, 7].into(),
            };
            assert!(
                seen.codes.is_superset(&codes),
                "{name}, Nether {nether}: {:?}",
                seen.codes
            );
            assert!(seen.tinted && seen.shadowed == [true; 2] && seen.translucent > 0);
        }
    }
    if traced == 0 {
        eprintln!("no GPU with ray queries: skipped");
    }
}

/// The scene's own look store (`Unpacker`): made beside the store, a part's looks written into
/// its quads, kept when the store grows, and zeros for a part whose section kept none, in a slot
/// another part's looks were in; gone when the pack's rays stop reading them.
#[test]
fn the_look_store_grows_with_the_store_and_a_part_without_looks_reads_none() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = super::entity_tests::ray_device() else {
        eprintln!("no GPU with ray queries: skipped");
        return;
    };
    // Two quads.
    let mut unpacker = Unpacker::new(&device, 8).unwrap();
    assert!(unpacker.set_looks(&device, true) && !unpacker.set_looks(&device, true));
    let look = |n: u32| QuadLook([n, n + 1]);
    let section = |looks: &[QuadLook]| {
        let mut s = crate::gfx::tests::section(looks.len().max(1) as u32, 0, None);
        s.ray_looks[0] = looks.into();
        s
    };
    let (two, none, one) = (
        section(&[look(1), look(4)]),
        section(&[]),
        section(&[look(7)]),
    );
    let write = |s, first| LookWrite {
        part: ((0, 0, 0), Layer::Solid),
        section: s,
        quads: Layer::Solid.quads(s).max(1),
        first,
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    let looks = |u: &Unpacker| u.looks.clone().unwrap();
    let stage = |records: &[QuadLook]| stage_looks(&device, records);
    assert!(write_looks(
        &mut encoder,
        &looks(&unpacker),
        &[write(&two, 0)],
        stage
    ));
    // The larger store retains both records. A refused staging allocation must clear the
    // reused first slot, and an upload with no look must clear the reused last slot.
    assert!(unpacker.fit(&device, &mut encoder, 16, look_buffer));
    assert_eq!(looks(&unpacker).size(), 4 * LOOK);
    assert!(write_looks(
        &mut encoder,
        &looks(&unpacker),
        &[write(&one, 12)],
        stage
    ));
    assert!(!write_looks(
        &mut encoder,
        &looks(&unpacker),
        &[write(&one, 0)],
        |_| None
    ));
    assert!(write_looks(
        &mut encoder,
        &looks(&unpacker),
        &[write(&none, 12), write(&one, 8)],
        stage,
    ));
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4 * LOOK,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(&looks(&unpacker), 0, &read, 0, 4 * LOOK);
    queue.submit([encoder.finish()]);
    read.slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let got: Vec<QuadLook> =
        bytemuck::cast_slice(&read.slice(..).get_mapped_range().unwrap()[..]).to_vec();
    assert_eq!(
        got,
        [QuadLook::default(), look(4), look(7), QuadLook::default()]
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    // Refusing a bigger optional look store leaves room for more geometry.
    assert!(unpacker.fit(&device, &mut encoder, 32, |_, _| None));
    assert_eq!(unpacker.store.size(), 32 * STRIDE);
    assert!(unpacker.looks.is_none());
    queue.submit([encoder.finish()]);
    assert!(unpacker.set_looks(&device, true));
    assert_eq!(looks(&unpacker).size(), 8 * LOOK);
    assert!(unpacker.set_looks(&device, false) && unpacker.looks.is_none());
}

#[test]
fn a_refused_staging_batch_retries_smaller_parts_in_turn() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = super::entity_tests::ray_device() else {
        eprintln!("no GPU with ray queries: skipped");
        return;
    };
    let mut scene = RayScene::new(&device).unwrap();
    scene.set_looks(&device, true);
    let mut sections = Resident::default();
    let parts = [(0, 2), (1, 1), (2, 1)].map(|(x, quads)| {
        let key = (x, 0, 0);
        let part = (key, Layer::Solid);
        let mut section = crate::gfx::tests::section(quads, 0, None);
        section.ray_looks[0] = vec![QuadLook([1, 2]); quads as usize].into();
        sections.insert(key, section);
        assert!(
            scene
                .slots
                .alloc(part, u64::from(quads) * 4, |_| false)
                .is_some()
        );
        part
    });
    let key = (3, 0, 0);
    let part = (key, Layer::Solid);
    let mut section = crate::gfx::tests::section(1, 0, None);
    section.ray_looks[0] = vec![QuadLook([3, 4])].into();
    sections.insert(key, section);
    let (size, blas) = new_blas(&device, 1).unwrap();
    let builds = [Build {
        part,
        s: sections.find(key).unwrap(),
        quads: 1,
        size,
        blas,
        first: scene.slots.alloc(part, 4, |_| false).unwrap(),
    }];
    scene.look_retries.extend(parts);
    let mut sizes = Vec::new();
    for _ in 0..4 {
        let mut encoder = device.create_command_encoder(&Default::default());
        scene.upload_looks(&mut encoder, &sections, &builds, |records| {
            sizes.push(records.len());
            (records.len() == 1)
                .then(|| stage_looks(&device, records))
                .flatten()
        });
        queue.submit([encoder.finish()]);
    }
    assert_eq!(sizes, [4, 1, 2, 1, 1, 1, 1, 1]);
    assert_eq!(scene.look_retries, [parts[0]]);

    let mut encoder = device.create_command_encoder(&Default::default());
    scene.upload_looks(&mut encoder, &sections, &builds, |records| {
        stage_looks(&device, records)
    });
    queue.submit([encoder.finish()]);
    assert!(scene.look_retries.is_empty());
}
