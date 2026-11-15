//! Which retained sections the entity layer builds a BLAS for, and when it builds one again or
//! drops it; which of their quads it takes; and its store's cap. The hit lookup on the GPU is in
//! ray_entity_tests.rs.

use super::*;

/// Section `key`'s snapshot at `revision`, of `quads` quads of overlay 0.
fn snapshot(key: Key, revision: u64, quads: usize) -> StaticSectionSnapshot {
    StaticSectionSnapshot {
        section: key,
        origin: [key[0] * 16, -64 + key[1] * 16, key[2] * 16],
        revision,
        entities: 1,
        vertices: vec![<Vertex as bytemuck::Zeroable>::zeroed(); quads * 4].into(),
    }
}

/// `plan` over sections kept at `kept` revisions, the camera in column (0, 0), a reach of 2.
fn plan_of(kept: &[(Key, u64)], snapshots: &[StaticSectionSnapshot]) -> Plan {
    plan(kept.iter().copied(), snapshots, (0, 0), 2)
}

const A: Key = [0, 4, 0];
const B: Key = [1, 4, -2];
/// Past the reach of 2, within its 2 more chunks.
const EDGE: Key = [3, 4, 0];
/// Past both.
const FAR: Key = [5, 4, 0];

#[test]
fn a_section_within_reach_gets_a_blas_once() {
    let snapshots = [snapshot(A, 7, 3), snapshot(B, 8, 1), snapshot(EDGE, 9, 2)];
    // The first frame builds the two within reach, not the one past it.
    let first = plan_of(&[], &snapshots);
    assert_eq!(
        first,
        Plan {
            drop: vec![],
            build: vec![0, 1]
        }
    );
    // Nothing changed: nothing is built or dropped, however many frames go by.
    assert_eq!(plan_of(&[(A, 7), (B, 8)], &snapshots), Plan::default());
}

#[test]
fn a_new_revision_builds_its_section_alone_again() {
    // A chest in B opened: B's snapshot is another revision (the chest moved to the stream), and
    // B alone is built again. It closes: another revision again.
    let opened = [snapshot(A, 7, 3), snapshot(B, 10, 0)];
    let plan = plan_of(&[(A, 7), (B, 8)], &opened);
    assert_eq!(
        plan,
        Plan {
            drop: vec![],
            build: vec![1]
        }
    );
    let closed = [snapshot(A, 7, 3), snapshot(B, 11, 1)];
    assert_eq!(plan_of(&[(A, 7), (B, 10)], &closed).build, [1]);
}

#[test]
fn a_section_whose_block_entities_went_drops_its_blas() {
    // B has no snapshot: its block entities were broken, or all of them are opening.
    let snapshots = [snapshot(A, 7, 3)];
    let plan = plan_of(&[(A, 7), (B, 8)], &snapshots);
    assert_eq!(
        plan,
        Plan {
            drop: vec![B],
            build: vec![]
        }
    );
    // Every section gone (another world): all of them go.
    let plan = plan_of(&[(A, 7), (B, 8)], &[]);
    assert_eq!(plan.drop, [A, B]);
}

#[test]
fn a_section_leaving_reach_drops_its_blas_past_two_more_chunks() {
    let snapshots = [
        snapshot(A, 7, 3),
        snapshot(EDGE, 9, 2),
        snapshot(FAR, 12, 1),
    ];
    // EDGE was built while the camera was nearer: it stays, as terrain's do, so walking along a
    // chunk border doesn't build it over and over; FAR goes.
    let kept = plan_of(&[(A, 7), (EDGE, 9), (FAR, 12)], &snapshots);
    assert_eq!(
        kept,
        Plan {
            drop: vec![FAR],
            build: vec![]
        }
    );
    // Changed while past the reach, it goes too: it wouldn't be built there.
    let changed = [snapshot(A, 7, 3), snapshot(EDGE, 13, 2)];
    assert_eq!(plan_of(&[(A, 7), (EDGE, 9)], &changed).drop, [EDGE]);
    // The camera walks up to it: built again.
    let near = plan(std::iter::empty(), &changed, (1, 0), 2);
    assert_eq!(near.build, [0, 1]);
}

#[test]
fn a_frame_builds_at_most_its_share() {
    let snapshots: Vec<_> = (0..BUILDS_PER_FRAME as i32 + 10)
        .map(|i| snapshot([0, i, 0], 1, 1))
        .collect();
    let first = plan_of(&[], &snapshots);
    assert_eq!(first.build.len(), BUILDS_PER_FRAME);
    // The rest on the next frame.
    let kept: Vec<_> = first
        .build
        .iter()
        .map(|&i| (snapshots[i].section, 1))
        .collect();
    assert_eq!(plan_of(&kept, &snapshots).build.len(), 10);
}

#[test]
fn the_layer_takes_the_ordinary_quads_marked() {
    use crate::gfx::entity_batch::{OIT_ENTITY, TRANSLUCENT_ITEM_OVERLAY, WATER_MASK_OVERLAY};
    let quad = |layer, x| {
        [Vertex {
            layer,
            pos: [x, 0.0, 0.0],
            ..<Vertex as bytemuck::Zeroable>::zeroed()
        }; 4]
    };
    // A chest's lit quad, one with HDR's tier in the high bits, and an unlit overlay (6), which
    // the shader passes as it does the stream's; a translucent item and a water mask, which are
    // other materials; and one tagged for improved transparency, which the retained draw lights
    // as the ordinary material.
    let vertices: Vec<Vertex> = [
        quad(0, 0.0),
        quad(3 << 16, 1.0),
        quad(6, 2.0),
        quad(TRANSLUCENT_ITEM_OVERLAY, 3.0),
        quad(WATER_MASK_OVERLAY, 4.0),
        quad(OIT_ENTITY, 5.0),
    ]
    .concat();
    let taken = layer_quads(&vertices);
    let xs: Vec<f32> = taken.iter().step_by(4).map(|v| v.pos[0]).collect();
    assert_eq!(xs, [0.0, 1.0, 2.0, 5.0]);
    let layers: Vec<u32> = taken.iter().step_by(4).map(|v| v.layer).collect();
    let q = RAY_ENTITY_QUAD;
    assert_eq!(layers, [q, 3 << 16 | q, 6 | q, OIT_ENTITY | q]);
    // Nothing else of a vertex changes.
    assert!(taken.iter().all(|v| v.layer & q != 0 && v.uv == [0.0, 0.0]));
}

#[test]
fn the_store_s_cap_is_a_binding_and_the_custom_data_s_24_bits() {
    let mut limits = wgpu::Limits {
        max_buffer_size: 1 << 40,
        max_storage_buffer_binding_size: 1 << 30,
        ..Default::default()
    };
    // A gibibyte of 176-byte quads.
    assert_eq!(largest_store(&limits), 6_100_805 * 4);
    limits.max_storage_buffer_binding_size = 1 << 40;
    assert_eq!(largest_store(&limits), MOST_QUADS * 4);
    limits.max_buffer_size = 1 << 20;
    assert_eq!(largest_store(&limits), 5957 * 4);
}

#[test]
fn the_most_quads_of_a_section_size_the_shared_indices() {
    let snapshots = [snapshot(A, 1, 3), snapshot(B, 2, 40), snapshot(FAR, 3, 7)];
    assert_eq!(Retained::most_quads(&snapshots), 40);
    assert_eq!(Retained::most_quads(&[]), 0);
}
