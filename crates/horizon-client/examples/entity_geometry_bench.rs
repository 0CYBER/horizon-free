//! Isolated production mob-model builder benchmark; no assets, Game, GPU or full-frame work.
//! CARGO_TARGET_DIR=/home/bob/Documents/horizon/target CARGO_INCREMENTAL=0 \
//! cargo run --profile fast -p horizon-client --example entity_geometry_bench
use horizon_client::{
    mesher::Vertex,
    mobs::{Part, Polygon, anim::Poses, render},
};
use std::{hint::black_box, time::Instant};

fn cube(name: String, position: f32) -> Part {
    let polygons = (0..6)
        .map(|face| {
            let axis = face / 2;
            let side = (face % 2) as f32;
            let mut normal = [0.0; 3];
            normal[axis] = side * 2.0 - 1.0;
            Polygon {
                normal,
                vertices: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|uv| {
                    let mut vertex = [0.0; 5];
                    vertex[axis] = side * 8.0;
                    vertex[(axis + 1) % 3] = uv[0] * 8.0;
                    vertex[(axis + 2) % 3] = uv[1] * 8.0;
                    vertex[3] = uv[0];
                    vertex[4] = uv[1];
                    vertex
                }),
            }
        })
        .collect();
    Part {
        name,
        pose: [position, 0.0, 0.0, 0.1, 0.2, 0.0, 1.0, 1.0, 1.0],
        polygons,
        children: Vec::new(),
    }
}

fn build(model: &Part, poses: &Poses, count: usize, out: &mut Vec<Vertex>) {
    for i in 0..count {
        render::draw(
            black_box(model),
            glam::Mat4::from_translation(glam::Vec3::new((i % 32) as f32, 0.0, (i / 32) as f32)),
            black_box(poses),
            [0.125, 0.25, 0.5, 0.75],
            0xffc0_d0e0,
            0xf0f0,
            0,
            out,
        );
    }
    black_box(out.as_slice());
}

fn measure(model: &Part, poses: &Poses, count: usize, iterations: usize, reuse: bool) -> f64 {
    let mut retained = Vec::new();
    build(model, poses, count, &mut retained); // Untimed warmup.
    let start = Instant::now();
    for _ in 0..iterations {
        let mut out = if reuse {
            std::mem::take(&mut retained)
        } else {
            Vec::new()
        };
        out.clear();
        build(model, poses, count, &mut out);
        if reuse {
            retained = out;
        }
    }
    start.elapsed().as_secs_f64() * 1e6 / iterations as f64
}

fn main() {
    let mut model = cube("body".into(), 0.0);
    model.children = (1..6).map(|i| cube(format!("part{i}"), i as f32)).collect();
    let poses = Poses::new(&model);
    for count in [32, 256, 1024, 4096, 8192] {
        let mut expected = Vec::new();
        build(&model, &poses, count, &mut expected);
        let mut retained = Vec::new();
        build(&model, &poses, count, &mut retained);
        let (pointer, capacity) = (retained.as_ptr(), retained.capacity());
        // Populated -> empty/reset -> smaller -> populated must never leave stale vertices.
        for n in [0, count / 2, count, count] {
            retained.clear();
            build(&model, &poses, n, &mut retained);
            let mut fresh = Vec::new();
            build(&model, &poses, n, &mut fresh);
            assert_eq!(
                bytemuck::cast_slice::<Vertex, u8>(&retained),
                bytemuck::cast_slice::<Vertex, u8>(&fresh)
            );
            assert_eq!(
                (retained.as_ptr(), retained.capacity()),
                (pointer, capacity)
            );
        }
        assert_eq!(
            bytemuck::cast_slice::<Vertex, u8>(&retained),
            bytemuck::cast_slice::<Vertex, u8>(&expected)
        );
        let iterations = (100_000 / count).clamp(40, 1000);
        let mut fresh = Vec::new();
        let mut reused = Vec::new();
        let mut paired_speedups = Vec::new();
        // Alternate order, reporting medians over eleven warmed batches.
        for batch in 0..11 {
            let mut pair = [0.0; 2];
            for reuse in if batch % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let time = measure(&model, &poses, count, iterations, reuse);
                pair[usize::from(reuse)] = time;
                if reuse {
                    reused.push(time);
                } else {
                    fresh.push(time);
                }
            }
            paired_speedups.push(pair[0] / pair[1]);
        }
        paired_speedups.sort_by(f64::total_cmp);
        fresh.sort_by(f64::total_cmp);
        reused.sort_by(f64::total_cmp);
        println!(
            "entities={count} vertices={} capacity={capacity} retained_bytes={} fresh_us={:.3} reused_us={:.3} speedup={:.3}x paired_speedup_median={:.3}x paired_speedup_range={:.3}..{:.3}x iterations={iterations} batches=11 byte_equivalence=pass pointer_capacity_reuse=pass",
            retained.len(),
            capacity * size_of::<Vertex>(),
            fresh[5],
            reused[5],
            fresh[5] / reused[5],
            paired_speedups[5],
            paired_speedups[0],
            paired_speedups[10]
        );
    }
}
