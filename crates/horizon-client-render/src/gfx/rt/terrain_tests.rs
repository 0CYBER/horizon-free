//! A terrain hit's `back` on a GPU with ray queries: blocks meshed as a shader pack's terrain is,
//! their solid quads written into the store by the scene's own unpack pass from either layout of
//! the arena (vertices, or the compact records the draws expand), the translucent ones as they
//! are, each layer built into its BLAS and traced through the scene's WGSL. Needs prepared
//! resources, like the mesher's geometry tests.

use super::entity_tests::{empty, ray_device, run_probe, white};
use super::*;
use crate::mesher::packed;

/// The middle of each block: stone, red stained glass, and an oak sapling (a cross model: two
/// planes, each drawn from both sides).
const STONE: [f32; 3] = [4.5, 4.5, 4.5];
const GLASS: [f32; 3] = [10.5, 4.5, 4.5];
const SAPLING: [f32; 3] = [4.5, 4.5, 10.5];

fn along(p: [f32; 3], d: [f32; 3], t: f32) -> [f32; 3] {
    [p[0] + d[0] * t, p[1] + d[1] * t, p[2] + d[2] * t]
}

/// Rays at a full stone block's six faces from outside read its front, the same six from inside
/// it its back, whichever layout the store was unpacked from; a ray at stained glass from outside
/// reads its front. A cross-model plant's plane is two quads in one place, one facing each way,
/// met from either side at the same distance: which one a ray takes is the GPU's, so only what
/// it read is printed.
#[test]
#[ignore = "daily"]
fn a_terrain_hit_is_front_from_outside_a_block_and_back_from_inside() {
    let Some((device, queue)) = ray_device() else {
        eprintln!("no GPU with ray queries: skipped");
        return;
    };
    let (mesh, records) = crate::mesher::look_tests::meshed_alone(&[
        ("minecraft:stone", [4, 4, 4]),
        ("minecraft:red_stained_glass", [10, 4, 4]),
        ("minecraft:oak_sapling", [4, 4, 10]),
    ]);
    let solid = (mesh.solid.len() / 4) as u32;
    let translucent = (mesh.translucent.len() / 4) as u32;
    assert_eq!(
        solid,
        6 + 4,
        "the stone's six faces and the sapling's two quads a plane"
    );
    assert!(
        packed::packed_quads(&records, solid as usize) >= 6,
        "the stone's faces have records"
    );
    // (origin, direction, layers): each face from 2 blocks out, each from the middle, the glass
    // from the north.
    let axes = [
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
    ];
    let mut rays: Vec<([f32; 3], [f32; 3], u32)> = Vec::new();
    rays.extend(axes.map(|n| (along(STONE, n, 2.0), n.map(|c: f32| -c), RAY_SOLID)));
    rays.extend(axes.map(|n| (STONE, n, RAY_SOLID)));
    rays.push((
        along(GLASS, [0.0, 0.0, -1.0], 2.0),
        [0.0, 0.0, 1.0],
        RAY_TRANSLUCENT,
    ));
    // Each of the sapling's planes along its normal both ways, 1.5 blocks out, at a point 0.2
    // from the middle so the ray runs beside the other plane.
    let s = std::f32::consts::FRAC_1_SQRT_2;
    for (normal, within) in [([s, 0.0, -s], [s, 0.0, s]), ([s, 0.0, s], [s, 0.0, -s])] {
        let at = along(SAPLING, within, 0.2);
        for d in [normal, normal.map(|c: f32| -c)] {
            rays.push((along(at, d, -1.5), d, RAY_SOLID));
        }
    }
    let mut probe = format!(
        "@group(0) @binding(0) var<storage, read_write> probe_out: array<vec4<f32>, {}>;\n\
         @compute @workgroup_size(1) fn probe() {{\n",
        rays.len()
    );
    for (i, (o, d, layers)) in rays.iter().enumerate() {
        probe += &format!(
            "    let h{i} = trace_layers(vec3<f32>({:?}, {:?}, {:?}), vec3<f32>({:?}, {:?}, {:?}), \
             100.0, {layers}u);\n    \
             probe_out[{i}] = vec4<f32>(h{i}.distance, f32(h{i}.back), f32(h{i}.translucent), 0.0);\n",
            o[0], o[1], o[2], d[0], d[1], d[2]
        );
    }
    probe += "}\n";
    let (_, sprites) = white(&device, &queue);
    let layers = empty(&device, 8);
    let indices = crate::gfx::rt::query::quad_indices(&device, solid.max(translucent));
    let unpack = unpack_pipeline(&device);
    let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let layouts = [
        ("vertices", bytemuck::cast_slice::<_, u8>(&mesh.solid), 0),
        (
            "records",
            bytemuck::cast_slice(&records),
            packed::PACKED_BASE,
        ),
    ];
    for (layout, page, base) in layouts {
        use wgpu::util::DeviceExt;
        let store = store_buffer(&device, u64::from(solid + translucent) * 4).unwrap();
        queue.write_buffer(
            &store,
            u64::from(solid) * 4 * STRIDE,
            bytemuck::cast_slice(&mesh.translucent),
        );
        let page = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: page,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            let job = RtJob {
                base,
                quads: solid,
                out: 0,
                pad: 0,
            };
            unpack_page(&device, &mut pass, &unpack, &page, &[job], &store);
        }
        let (solid_size, solid_blas) = new_blas(&device, solid).unwrap();
        let (glass_size, glass_blas) = new_blas(&device, translucent).unwrap();
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
        let mut tlas = create_tlas(&device, 2);
        tlas[0] = Some(wgpu::TlasInstance::new(
            &solid_blas,
            identity,
            0,
            RAY_SOLID as u8,
        ));
        tlas[1] = Some(wgpu::TlasInstance::new(
            &glass_blas,
            identity,
            solid,
            RAY_TRANSLUCENT as u8,
        ));
        encoder.build_acceleration_structures(builds.iter(), std::iter::once(&tlas));
        let traced = Traced {
            tlas: &tlas,
            solid_instances: 1,
            store: &store,
            translucent_instances: 2,
            entities: None,
            generation: 0,
            camera: [0.0; 3],
            nether: false,
            distance: 100.0,
        };
        let got = run_probe(
            (&device, &queue),
            encoder,
            &traced,
            (&store, &layers, &sprites),
            &probe,
            rays.len() as u64,
        );
        for (i, (got, ray)) in got.iter().zip(&rays).enumerate() {
            let case = format!("{layout}, ray {i} {ray:?}: {got:?}");
            let (distance, back, translucent) = match i {
                0..6 => (1.5, Some(false), false),
                6..12 => (0.5, Some(true), false),
                12 => (1.5, Some(false), true),
                _ => (1.5, None, false),
            };
            assert!(
                (got[0] - distance).abs() < 1e-4,
                "{case}: not {distance} away"
            );
            assert_eq!(got[2], f32::from(u8::from(translucent)), "{case}");
            if let Some(back) = back {
                assert_eq!(got[1], f32::from(u8::from(back)), "{case}");
            }
        }
        let plant: Vec<f32> = got[13..].iter().map(|g| g[1]).collect();
        eprintln!("{layout}: the sapling's planes, each both ways, read back: {plant:?}");
    }
}
