use super::*;
use horizon_core::geometry::Axis;
use std::f64::consts::{FRAC_PI_2, PI};

const FIVE_DEGREES: f64 = 5.0 * PI / 180.0;
const PLUS_Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
const EPSILON: f64 = 1.0e-9;

fn two_full_cells() -> Vec<CarrierCell> {
    vec![
        CarrierCell::new(BlockPos::new(0, 0, 0), VoxelShape::block()),
        CarrierCell::new(BlockPos::new(1, 0, 0), VoxelShape::block()),
    ]
}

fn scene_placement() -> RigidTransform {
    RigidTransform::identity_at(Vec3::new(10.0, 64.0, 10.0)).unwrap()
}

/// Static probe: does the placement touch or overlap any obstacle? A
/// zero-length translation step reports exactly that.
fn touches(geometry: &CarrierGeometry, placement: &RigidTransform, obstacles: &[Obstacle]) -> bool {
    !geometry
        .translate_step(placement, Vec3::ZERO, obstacles, &ContactBudget::default())
        .unwrap()
        .contacts
        .is_empty()
}

#[test]
fn transform_maps_local_points_with_pivot_and_rotation() {
    let placement = scene_placement();
    assert_eq!(
        placement.to_world(Vec3::from_block(BlockPos::new(0, 0, 0))),
        Vec3::new(10.0, 64.0, 10.0)
    );
    assert_eq!(
        placement.to_world(Vec3::from_block(BlockPos::new(1, 0, 0))),
        Vec3::new(11.0, 64.0, 10.0)
    );
    // Round trip through a rotated placement with a nonzero pivot.
    let quarter = RigidTransform::new(
        Vec3::new(5.0, 5.0, 5.0),
        Vec3::new(1.0, 1.0, 1.0),
        Rotation::from_axis_angle(PLUS_Y, FRAC_PI_2).unwrap(),
    )
    .unwrap();
    let point = Vec3::new(2.0, 3.0, 4.0);
    let world = quarter.to_world(point);
    let round = quarter.to_local(world);
    assert!(
        (round.x - point.x).abs() < EPSILON
            && (round.y - point.y).abs() < EPSILON
            && (round.z - point.z).abs() < EPSILON,
        "round trip {round:?}"
    );
    // t + c + R(q)(p - c) with a +90° Y turn mapping +X to -Z:
    // (5,5,5) + (1,1,1) + R((1,2,3)) = (6,6,6) + (3,2,-1).
    assert!(
        (world.x - 9.0).abs() < EPSILON
            && (world.y - 8.0).abs() < EPSILON
            && (world.z - 5.0).abs() < EPSILON,
        "got {world:?}"
    );
    // The pivot's world point stays fixed under rotation about it.
    assert_eq!(
        quarter.to_world(Vec3::new(1.0, 1.0, 1.0)),
        Vec3::new(6.0, 6.0, 6.0)
    );
    // Rotating a placement matches composing the rotation directly.
    let rotated = placement.rotated(PLUS_Y, FRAC_PI_2).unwrap();
    assert_eq!(
        rotated.rotation(),
        Rotation::from_axis_angle(PLUS_Y, FRAC_PI_2).unwrap()
    );
}

#[test]
fn rotation_input_is_normalized_canonical_or_refused() {
    assert_eq!(
        Rotation::from_components(2.0, 0.0, 0.0, 0.0)
            .unwrap()
            .components(),
        [1.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        Rotation::from_components(-1.0, 0.0, 0.0, 0.0)
            .unwrap()
            .components(),
        [1.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        Rotation::from_components(0.0, -2.0, 0.0, 0.0)
            .unwrap()
            .components(),
        [0.0, 1.0, 0.0, 0.0]
    );
    // A subnormal quaternion still normalizes instead of underflowing.
    assert_eq!(
        Rotation::from_components(1.0e-320, 0.0, 0.0, 0.0)
            .unwrap()
            .components(),
        [1.0, 0.0, 0.0, 0.0]
    );
    // A quarter turn about +Y maps +X to -Z (design: positive 90° Y).
    let turned = Rotation::from_axis_angle(PLUS_Y, FRAC_PI_2).unwrap();
    let mapped = turned.rotate(Vec3::new(1.0, 0.0, 0.0));
    assert!(
        (mapped.x).abs() < EPSILON
            && (mapped.y).abs() < EPSILON
            && (mapped.z + 1.0).abs() < EPSILON
    );
    assert_eq!(
        Rotation::from_components(f64::NAN, 0.0, 0.0, 0.0),
        Err(Refused::NonFinite)
    );
    assert_eq!(
        Rotation::from_components(0.0, f64::INFINITY, 0.0, 0.0),
        Err(Refused::NonFinite)
    );
    assert_eq!(
        Rotation::from_components(0.0, 0.0, 0.0, 0.0),
        Err(Refused::ZeroQuaternion)
    );
    assert_eq!(
        Rotation::from_axis_angle(Vec3::ZERO, 1.0),
        Err(Refused::ZeroAxis)
    );
    assert_eq!(
        Rotation::from_axis_angle(PLUS_Y, f64::NAN),
        Err(Refused::NonFinite)
    );
}

#[test]
fn eighteen_five_degree_steps_match_the_acceptance_scene() {
    let geometry = CarrierGeometry::new(7, two_full_cells()).unwrap();
    let mut placement = scene_placement();
    for _ in 0..18 {
        let step = geometry
            .rotate_step(
                &placement,
                PLUS_Y,
                FIVE_DEGREES,
                &[],
                &ContactBudget::default(),
            )
            .unwrap();
        assert_eq!(step.fraction, 1.0);
        assert!(step.contacts.is_empty());
        placement = placement
            .rotated(PLUS_Y, FIVE_DEGREES * step.fraction)
            .unwrap();
    }
    let first = placement.to_world(Vec3::from_block(BlockPos::new(0, 0, 0)));
    let second = placement.to_world(Vec3::from_block(BlockPos::new(1, 0, 0)));
    let expected_first = Vec3::new(10.0, 64.0, 10.0);
    let expected_second = Vec3::new(10.0, 64.0, 9.0);
    assert!(
        (first.x - expected_first.x).abs() < EPSILON
            && (first.y - expected_first.y).abs() < EPSILON
            && (first.z - expected_first.z).abs() < EPSILON,
        "first origin {first:?}"
    );
    assert!(
        (second.x - expected_second.x).abs() < EPSILON
            && (second.y - expected_second.y).abs() < EPSILON
            && (second.z - expected_second.z).abs() < EPSILON,
        "second origin {second:?}"
    );
}

#[test]
fn wall_translation_reaches_thirteen_then_blocks_without_penetration() {
    let geometry = CarrierGeometry::new(7, two_full_cells()).unwrap();
    let wall = [Obstacle::world_block(
        BlockPos::new(13, 64, 10),
        VoxelShape::block(),
    )];
    let mut placement = scene_placement();
    // Initial maximum x is 12 (broad phase = union of transformed cell
    // bounds).
    let bounds = geometry.world_bounds(&placement).unwrap();
    assert_eq!(bounds.max(Axis::X), 12.0);
    // First step: one block towards the wall reaches 13 exactly.
    let first = geometry
        .translate_step(
            &placement,
            Vec3::new(1.0, 0.0, 0.0),
            &wall,
            &ContactBudget::default(),
        )
        .unwrap();
    assert_eq!(first.fraction, 1.0);
    assert_eq!(first.contacts.len(), 1);
    placement = placement.translated(Vec3::new(1.0, 0.0, 0.0).scale(first.fraction));
    assert!((geometry.world_bounds(&placement).unwrap().max(Axis::X) - 13.0).abs() < EPSILON);
    // Second step: blocked with zero penetration.
    let second = geometry
        .translate_step(
            &placement,
            Vec3::new(1.0, 0.0, 0.0),
            &wall,
            &ContactBudget::default(),
        )
        .unwrap();
    assert_eq!(second.fraction, 0.0);
    assert!(!second.contacts.is_empty());
    placement = placement.translated(Vec3::new(1.0, 0.0, 0.0).scale(second.fraction));
    assert!(
        geometry.world_bounds(&placement).unwrap().max(Axis::X) <= 13.0 + EPSILON,
        "no penetration into the wall"
    );
}

#[test]
fn translation_stops_touching_from_half_a_block() {
    let geometry = CarrierGeometry::new(7, two_full_cells()).unwrap();
    let wall = [Obstacle::world_block(
        BlockPos::new(13, 64, 10),
        VoxelShape::block(),
    )];
    // A 0.5 gap: cells at x 10.5..12.5, wall face at 13.
    let placement = RigidTransform::identity_at(Vec3::new(10.5, 64.0, 10.0)).unwrap();
    let step = geometry
        .translate_step(
            &placement,
            Vec3::new(1.0, 0.0, 0.0),
            &wall,
            &ContactBudget::default(),
        )
        .unwrap();
    assert!(
        (step.fraction - 0.5).abs() < EPSILON,
        "fraction {}",
        step.fraction
    );
    assert_eq!(step.contacts.len(), 1);
    let moved = placement.translated(Vec3::new(1.0, 0.0, 0.0).scale(step.fraction));
    assert!((geometry.world_bounds(&moved).unwrap().max(Axis::X) - 13.0).abs() < EPSILON);
}

#[test]
fn sparse_gap_column_is_traversable_for_rays() {
    // A three-cell span holding only the x=0 and x=2 cells: the gap
    // column at local x=1 must stay open for rays.
    let geometry = CarrierGeometry::new(
        7,
        vec![
            CarrierCell::new(BlockPos::new(0, 0, 0), VoxelShape::block()),
            CarrierCell::new(BlockPos::new(2, 0, 0), VoxelShape::block()),
        ],
    )
    .unwrap();
    let placement = scene_placement();
    // The outer bounds span the gap; only the sparse query respects it.
    let bounds = geometry.world_bounds(&placement).unwrap();
    assert_eq!(bounds.min(Axis::X), 10.0);
    assert_eq!(bounds.max(Axis::X), 13.0);
    let through_gap = geometry.clip(
        &placement,
        Vec3::new(11.5, 66.0, 10.5),
        Vec3::new(11.5, 63.5, 10.5),
    );
    assert_eq!(through_gap, None, "ray down local (1.5, 2, 0.5) must miss");
    let control = geometry
        .clip(
            &placement,
            Vec3::new(10.5, 66.0, 10.5),
            Vec3::new(10.5, 63.5, 10.5),
        )
        .expect("ray through the x=0 cell hits");
    assert_eq!(control.local_cell, BlockPos::new(0, 0, 0));
    assert_eq!(control.direction, Direction::Up);
    assert!(
        (control.location.x - 10.5).abs() < EPSILON
            && (control.location.y - 65.0).abs() < EPSILON
            && (control.location.z - 10.5).abs() < EPSILON,
        "hit {control:?}"
    );
}

#[test]
fn mid_sweep_obstacle_stops_rotation_with_clear_endpoint_poses() {
    let geometry = CarrierGeometry::new(7, two_full_cells()).unwrap();
    // A block in the sweep quadrant of cell (1,0,0) that neither the
    // initial (z >= 0) nor the final (x <= 1) pose can reach.
    let obstacle = Obstacle::with_identity(
        0,
        BlockPos::new(1, 0, -1),
        Vec3::ZERO,
        VoxelShape::from_box(Aabb::new(
            Vec3::new(1.05, 0.0, -1.55),
            Vec3::new(1.35, 1.0, -1.25),
        ))
        .unwrap(),
    )
    .unwrap();
    let obstacles = [obstacle];
    let mut placement = RigidTransform::identity_at(Vec3::ZERO).unwrap();
    let start_clear = !touches(&geometry, &placement, &obstacles);
    let final_pose = placement.rotated(PLUS_Y, FRAC_PI_2).unwrap();
    let end_clear = !touches(&geometry, &final_pose, &obstacles);
    assert!(
        start_clear && end_clear,
        "both endpoint poses must be clear"
    );
    let mut stopped = None;
    for step in 0..18 {
        let outcome = geometry
            .rotate_step(
                &placement,
                PLUS_Y,
                FIVE_DEGREES,
                &obstacles,
                &ContactBudget::default(),
            )
            .unwrap();
        placement = placement
            .rotated(PLUS_Y, FIVE_DEGREES * outcome.fraction)
            .unwrap();
        if outcome.fraction < 1.0 {
            stopped = Some(step);
            assert!(!outcome.contacts.is_empty());
            assert!(
                outcome
                    .contacts
                    .iter()
                    .all(|contact| contact.local_cell == BlockPos::new(1, 0, 0)),
                "only the sweeping cell contacts: {:?}",
                outcome.contacts
            );
            break;
        }
    }
    let step = stopped.expect("rotation must stop mid-sweep");
    assert!(
        step < 17,
        "contact happens inside the sweep, not at the end"
    );
    // Further rotation stays blocked at the touching configuration.
    for _ in 0..3 {
        let outcome = geometry
            .rotate_step(
                &placement,
                PLUS_Y,
                FIVE_DEGREES,
                &obstacles,
                &ContactBudget::default(),
            )
            .unwrap();
        assert_eq!(outcome.fraction, 0.0);
        assert!(!outcome.contacts.is_empty());
    }
    // The carrier never reached the 90° pose.
    let second = placement.to_world(Vec3::from_block(BlockPos::new(1, 0, 0)));
    assert!(
        second.z > -0.9 && second.x > 0.5,
        "stopped mid-sweep at {second:?}"
    );
}

#[test]
fn exhausted_budgets_refuse_instead_of_clamping() {
    let geometry = CarrierGeometry::new(7, two_full_cells()).unwrap();
    let placement = scene_placement();
    let wall = [Obstacle::world_block(
        BlockPos::new(13, 64, 10),
        VoxelShape::block(),
    )];
    // Two transformed boxes against a one-box budget.
    assert_eq!(
        geometry.translate_step(
            &placement,
            Vec3::new(1.0, 0.0, 0.0),
            &wall,
            &ContactBudget {
                boxes: 1,
                iterations: 100
            }
        ),
        Err(Refused::BoxBudget)
    );
    // Two narrow-phase pair tests against a one-iteration budget.
    assert_eq!(
        geometry.translate_step(
            &placement,
            Vec3::new(1.0, 0.0, 0.0),
            &wall,
            &ContactBudget {
                boxes: 64,
                iterations: 1
            }
        ),
        Err(Refused::IterationBudget)
    );
    // Conservative advancement cannot even probe once: an obstacle the
    // rotation's bounding sphere reaches (a block above the sweep).
    let overhead = [Obstacle::world_block(
        BlockPos::new(12, 66, 10),
        VoxelShape::block(),
    )];
    assert_eq!(
        geometry.rotate_step(
            &placement,
            PLUS_Y,
            FIVE_DEGREES,
            &overhead,
            &ContactBudget {
                boxes: 64,
                iterations: 0
            }
        ),
        Err(Refused::IterationBudget)
    );
    // The refusal left the placement untouched.
    assert_eq!(
        geometry.world_bounds(&placement).unwrap().max(Axis::X),
        12.0
    );
}

#[test]
fn structure_resting_on_floor_slides_and_rotates() {
    let geometry = CarrierGeometry::new(7, two_full_cells()).unwrap();
    // Floor blocks at y=63 under both cells and one ahead of the sweep,
    // so sliding rides onto it while resting on the rest.
    let floor = [
        Obstacle::world_block(BlockPos::new(10, 63, 10), VoxelShape::block()),
        Obstacle::world_block(BlockPos::new(11, 63, 10), VoxelShape::block()),
        Obstacle::world_block(BlockPos::new(12, 63, 10), VoxelShape::block()),
    ];
    let placement = scene_placement();
    let slide = geometry
        .translate_step(
            &placement,
            Vec3::new(1.0, 0.0, 0.0),
            &floor,
            &ContactBudget::default(),
        )
        .unwrap();
    assert_eq!(slide.fraction, 1.0);
    assert!(slide.contacts.is_empty());
    let turn = geometry
        .rotate_step(
            &placement,
            PLUS_Y,
            FIVE_DEGREES,
            &floor,
            &ContactBudget::default(),
        )
        .unwrap();
    assert_eq!(turn.fraction, 1.0);
    assert!(turn.contacts.is_empty());
}

#[test]
fn pressed_against_wall_backs_off_and_turns_away() {
    let geometry = CarrierGeometry::new(7, two_full_cells()).unwrap();
    let wall = [Obstacle::world_block(
        BlockPos::new(13, 64, 10),
        VoxelShape::block(),
    )];
    // Cells at x 11..13: the eastern face touches the wall at x=13.
    let placement = RigidTransform::identity_at(Vec3::new(11.0, 64.0, 10.0)).unwrap();
    // Backing away from the touch: full step, no contact.
    let back = geometry
        .translate_step(
            &placement,
            Vec3::new(-1.0, 0.0, 0.0),
            &wall,
            &ContactBudget::default(),
        )
        .unwrap();
    assert_eq!(back.fraction, 1.0);
    assert!(back.contacts.is_empty());
    // Turning the touching corner away from the wall: full step.
    let away = geometry
        .rotate_step(
            &placement,
            PLUS_Y,
            -FIVE_DEGREES,
            &wall,
            &ContactBudget::default(),
        )
        .unwrap();
    assert_eq!(away.fraction, 1.0);
    assert!(away.contacts.is_empty());
    // Turning it into the wall stays blocked.
    let into = geometry
        .rotate_step(
            &placement,
            PLUS_Y,
            FIVE_DEGREES,
            &wall,
            &ContactBudget::default(),
        )
        .unwrap();
    assert_eq!(into.fraction, 0.0);
    assert!(!into.contacts.is_empty());
}

#[test]
fn tall_column_contacts_reach_obstacles_along_the_rotation_axis() {
    // An 11-cell vertical column over the pivot's height. The literal
    // scene cells (0,0,0)..(0,10,0) with pivot (0,0,0) lie ON the +Y
    // rotation axis and sweep nothing, so the column stands one block
    // east; the obstacle hangs at the top cell's sweep quadrant and
    // nothing but that cell can reach it.
    let cells: Vec<CarrierCell> = (0..=10)
        .map(|y| CarrierCell::new(BlockPos::new(1, y, 0), VoxelShape::block()))
        .collect();
    let geometry = CarrierGeometry::new(9, cells).unwrap();
    let obstacle = Obstacle::with_identity(
        0,
        BlockPos::new(1, 0, -1),
        Vec3::ZERO,
        VoxelShape::from_box(Aabb::new(
            Vec3::new(1.05, 10.0, -1.55),
            Vec3::new(1.35, 11.0, -1.25),
        ))
        .unwrap(),
    )
    .unwrap();
    let obstacles = [obstacle];
    let mut placement = RigidTransform::identity_at(Vec3::ZERO).unwrap();
    assert!(!touches(&geometry, &placement, &obstacles));
    assert!(!touches(
        &geometry,
        &placement.rotated(PLUS_Y, FRAC_PI_2).unwrap(),
        &obstacles
    ));
    let mut stopped = false;
    for _ in 0..18 {
        let outcome = geometry
            .rotate_step(
                &placement,
                PLUS_Y,
                FIVE_DEGREES,
                &obstacles,
                &ContactBudget::default(),
            )
            .unwrap();
        placement = placement
            .rotated(PLUS_Y, FIVE_DEGREES * outcome.fraction)
            .unwrap();
        if outcome.fraction < 1.0 {
            stopped = true;
            assert!(
                outcome
                    .contacts
                    .iter()
                    .all(|contact| contact.local_cell == BlockPos::new(1, 10, 0)),
                "only the top cell contacts: {:?}",
                outcome.contacts
            );
            break;
        }
    }
    assert!(stopped, "the top cell must meet the obstacle mid-sweep");
    // Further rotation stays blocked at the touching configuration.
    let outcome = geometry
        .rotate_step(
            &placement,
            PLUS_Y,
            FIVE_DEGREES,
            &obstacles,
            &ContactBudget::default(),
        )
        .unwrap();
    assert_eq!(outcome.fraction, 0.0);
    assert!(!outcome.contacts.is_empty());
}

#[test]
fn huge_and_subnormal_axes_rotate_like_unit_axes() {
    let reference = Rotation::from_axis_angle(PLUS_Y, FIVE_DEGREES).unwrap();
    for axis in [
        Vec3::new(0.0, 1.0e200, 0.0),
        Vec3::new(0.0, 1.0e-200, 0.0),
        Vec3::new(0.0, 1.0e-320, 0.0),
        PLUS_Y.scale(1.0e8),
    ] {
        assert_eq!(
            Rotation::from_axis_angle(axis, FIVE_DEGREES)
                .unwrap()
                .components(),
            reference.components(),
            "axis {axis:?}"
        );
    }
    // The step path normalizes through the same helper.
    let geometry = CarrierGeometry::new(7, two_full_cells()).unwrap();
    let placement = scene_placement();
    let step = geometry
        .rotate_step(
            &placement,
            Vec3::new(0.0, 1.0e200, 0.0),
            FIVE_DEGREES,
            &[],
            &ContactBudget::default(),
        )
        .unwrap();
    assert_eq!(step.fraction, 1.0);
}

#[test]
fn earliest_impact_is_order_independent() {
    // Two obstacles 5e-8 apart: their impacts differ by less than
    // COLLISION_EPSILON, so an epsilon-guarded running minimum would
    // keep whichever impact came first. The committed fraction must be
    // exactly the earlier impact in either order.
    let geometry = CarrierGeometry::new(
        7,
        vec![CarrierCell::new(
            BlockPos::new(0, 0, 0),
            VoxelShape::block(),
        )],
    )
    .unwrap();
    let placement = RigidTransform::identity_at(Vec3::ZERO).unwrap();
    let slab = |min_x: f64| {
        VoxelShape::from_box(Aabb::new(
            Vec3::new(min_x, 0.0, 0.0),
            Vec3::new(min_x + 1.0, 1.0, 1.0),
        ))
        .unwrap()
    };
    let near =
        Obstacle::with_identity(1, BlockPos::new(1, 0, 0), Vec3::ZERO, slab(1.5 - 5.0e-8)).unwrap();
    let far = Obstacle::with_identity(2, BlockPos::new(2, 0, 0), Vec3::ZERO, slab(1.5)).unwrap();
    let forward = geometry
        .translate_step(
            &placement,
            Vec3::new(1.0, 0.0, 0.0),
            &[near.clone(), far.clone()],
            &ContactBudget::default(),
        )
        .unwrap();
    let reverse = geometry
        .translate_step(
            &placement,
            Vec3::new(1.0, 0.0, 0.0),
            &[far, near.clone()],
            &ContactBudget::default(),
        )
        .unwrap();
    assert_eq!(forward.fraction.to_bits(), reverse.fraction.to_bits());
    let only_near = geometry
        .translate_step(
            &placement,
            Vec3::new(1.0, 0.0, 0.0),
            &[near],
            &ContactBudget::default(),
        )
        .unwrap();
    assert_eq!(forward.fraction.to_bits(), only_near.fraction.to_bits());
    assert!(
        forward.fraction < 0.5,
        "the earlier impact wins: {}",
        forward.fraction
    );
    // Both impacts fall inside the epsilon band, so both report.
    assert_eq!(forward.contacts.len(), 2);
    assert_eq!(forward.contacts, reverse.contacts);
}

#[test]
fn edge_cross_axis_separates_tilted_box() {
    // A unit cube turned 90° about the (1,1,0)/sqrt(2) diagonal, with
    // axes (Rodrigues, theta = 90°):
    //   u_x = (1/2, 1/2, -sqrt(2)/2)
    //   u_y = (1/2, 1/2,  sqrt(2)/2)
    //   u_z = (sqrt(2)/2, -sqrt(2)/2, 0)
    // and cube center C = (u_x + u_y + u_z)/2 = ((1+r)/2, (1-r)/2, 0)
    // with r = sqrt(2)/2. An axis-aligned unit cube sits 1.6*n away
    // along the edge-cross axis
    //   n = normalize(u_y x X) = (0, sqrt(2/3), -1/sqrt(3)).
    // Projections on n: |u_x.n| = sqrt(2/3), |u_y.n| = 0,
    // |u_z.n| = 1/sqrt(3), and the world cube projects
    // 0 + sqrt(2/3) + 1/sqrt(3) over three axes of half-sides 1/2, so
    // each cube spans (sqrt(2/3) + 1/sqrt(3))/2 = 0.69692 from its
    // center and the gap on n is
    //   1.6 - (sqrt(2/3) + 1/sqrt(3)) = 0.20615.
    // Every face axis overlaps instead of separating — offset
    // d = 1.6*n = (0, 1.30639, -0.92376) against reach sums:
    //   X: |0| < 0.85355 + 0.5          (cube on X: (0.5+0.5+r)/2)
    //   Y: 1.30639 < 0.85355 + 0.5
    //   Z: 0.92376 < 0.70711 + 0.5      (cube on Z: (r+r+0)/2)
    //   u_x: |d.u_x| = 1.30639 < 0.5 + 0.85355
    //   u_y: |d.u_y| = 0
    //   u_z: |d.u_z| = 0.92376 < 0.5 + 0.70711
    // so only the edge-cross axis sees the separation; disabling the
    // cross axes makes the pair look penetrating.
    let r = std::f64::consts::FRAC_1_SQRT_2;
    let n = Vec3::new(0.0, (2.0f64 / 3.0).sqrt(), -(1.0f64 / 3.0).sqrt());
    let center = Vec3::new((1.0 + r) / 2.0, (1.0 - r) / 2.0, 0.0);
    let obstacle_center = center.add_vector(n.scale(1.6));
    let obstacle = Obstacle::with_identity(
        0,
        BlockPos::new(0, 1, -1),
        Vec3::ZERO,
        VoxelShape::from_box(Aabb::new(
            obstacle_center.subtract(Vec3::new(0.5, 0.5, 0.5)),
            obstacle_center.add_vector(Vec3::new(0.5, 0.5, 0.5)),
        ))
        .unwrap(),
    )
    .unwrap();
    let obstacles = [obstacle];
    let placement = RigidTransform::new(
        Vec3::ZERO,
        Vec3::ZERO,
        Rotation::from_axis_angle(Vec3::new(1.0, 1.0, 0.0), FRAC_PI_2).unwrap(),
    )
    .unwrap();
    let geometry = CarrierGeometry::new(
        7,
        vec![CarrierCell::new(
            BlockPos::new(0, 0, 0),
            VoxelShape::block(),
        )],
    )
    .unwrap();
    // Static pose: separated, and only the edge axis can tell.
    assert!(!touches(&geometry, &placement, &obstacles));
    // Swept towards the obstacle along n at 0.7 blocks per step: the
    // step stops at gap / speed = 0.20615 / 0.7 = 0.29450.
    let reach = (2.0f64 / 3.0).sqrt() + (1.0f64 / 3.0).sqrt();
    let step = geometry
        .translate_step(
            &placement,
            n.scale(0.7),
            &obstacles,
            &ContactBudget::default(),
        )
        .unwrap();
    assert!(
        (step.fraction - (1.6 - reach) / 0.7).abs() < EPSILON,
        "fraction {}",
        step.fraction
    );
    assert_eq!(step.contacts.len(), 1);
}

#[test]
fn invalid_motion_values_are_refused() {
    let geometry = CarrierGeometry::new(7, two_full_cells()).unwrap();
    let placement = scene_placement();
    let budget = ContactBudget::default();
    let obstacles = [Obstacle::world_block(
        BlockPos::new(13, 64, 10),
        VoxelShape::block(),
    )];
    assert_eq!(
        geometry.translate_step(
            &placement,
            Vec3::new(f64::NAN, 0.0, 0.0),
            &obstacles,
            &budget
        ),
        Err(Refused::NonFinite)
    );
    assert_eq!(
        geometry.translate_step(
            &placement,
            Vec3::new(0.0, f64::INFINITY, 0.0),
            &obstacles,
            &budget
        ),
        Err(Refused::NonFinite)
    );
    assert_eq!(
        geometry.translate_step(&placement, Vec3::new(1.5, 0.0, 0.0), &obstacles, &budget),
        Err(Refused::TranslationTooFar)
    );
    assert_eq!(
        geometry.translate_step(&placement, Vec3::new(0.0, -2.0, 0.0), &obstacles, &budget),
        Err(Refused::TranslationTooFar)
    );
    assert_eq!(
        geometry.rotate_step(&placement, PLUS_Y, 10.0 * PI / 180.0, &obstacles, &budget),
        Err(Refused::RotationTooFar)
    );
    assert_eq!(
        geometry.rotate_step(&placement, PLUS_Y, f64::NAN, &obstacles, &budget),
        Err(Refused::NonFinite)
    );
    assert_eq!(
        geometry.rotate_step(&placement, Vec3::ZERO, FIVE_DEGREES, &obstacles, &budget),
        Err(Refused::ZeroAxis)
    );
    assert_eq!(
        RigidTransform::new(
            Vec3::new(f64::NAN, 0.0, 0.0),
            Vec3::ZERO,
            Rotation::IDENTITY
        ),
        Err(Refused::NonFinite)
    );
    // Exactly at the limits is accepted.
    assert!(
        geometry
            .translate_step(&placement, Vec3::new(1.0, -1.0, 1.0), &obstacles, &budget)
            .is_ok()
    );
    assert!(
        geometry
            .rotate_step(&placement, PLUS_Y, FIVE_DEGREES, &obstacles, &budget)
            .is_ok()
    );
}

#[test]
fn simultaneous_contacts_stay_in_uuid_and_cell_order() {
    let geometry = CarrierGeometry::new(7, two_full_cells()).unwrap();
    let placement = scene_placement();
    // Three wall slabs meeting the carrier at the same fraction, with
    // identities that only stable (uuid, cell) ordering sorts.
    let slab = |min_z: f64, max_z: f64| {
        VoxelShape::from_box(Aabb::new(
            Vec3::new(13.0, 64.0, min_z),
            Vec3::new(14.0, 65.0, max_z),
        ))
        .unwrap()
    };
    let obstacles = [
        Obstacle::with_identity(9, BlockPos::new(1, 0, 0), Vec3::ZERO, slab(10.0, 10.4)).unwrap(),
        Obstacle::with_identity(0, BlockPos::new(5, 0, 0), Vec3::ZERO, slab(10.4, 10.7)).unwrap(),
        Obstacle::with_identity(0, BlockPos::new(0, 0, 0), Vec3::ZERO, slab(10.7, 11.0)).unwrap(),
    ];
    let step = geometry
        .translate_step(
            &placement,
            Vec3::new(1.0, 0.0, 0.0),
            &obstacles,
            &ContactBudget::default(),
        )
        .unwrap();
    assert_eq!(step.fraction, 1.0);
    assert_eq!(step.contacts.len(), 3);
    assert_eq!(
        step.contacts,
        vec![
            Contact {
                uuid: 0,
                obstacle_cell: BlockPos::new(0, 0, 0),
                local_cell: BlockPos::new(1, 0, 0)
            },
            Contact {
                uuid: 0,
                obstacle_cell: BlockPos::new(5, 0, 0),
                local_cell: BlockPos::new(1, 0, 0)
            },
            Contact {
                uuid: 9,
                obstacle_cell: BlockPos::new(1, 0, 0),
                local_cell: BlockPos::new(1, 0, 0)
            },
        ]
    );
}

#[test]
fn duplicate_cells_are_refused_and_cells_sorted() {
    assert_eq!(
        CarrierGeometry::new(
            7,
            vec![
                CarrierCell::new(BlockPos::new(1, 0, 0), VoxelShape::block()),
                CarrierCell::new(BlockPos::new(0, 0, 0), VoxelShape::block()),
                CarrierCell::new(BlockPos::new(1, 0, 0), VoxelShape::block()),
            ]
        )
        .unwrap_err(),
        Refused::DuplicateCells
    );
    let geometry = CarrierGeometry::new(
        7,
        vec![
            CarrierCell::new(BlockPos::new(1, 0, 0), VoxelShape::block()),
            CarrierCell::new(BlockPos::new(-2, 0, 5), VoxelShape::block()),
        ],
    )
    .unwrap();
    let locals: Vec<BlockPos> = geometry.cells().iter().map(|cell| cell.local()).collect();
    assert_eq!(
        locals,
        vec![BlockPos::new(-2, 0, 5), BlockPos::new(1, 0, 0)]
    );
    assert_eq!(geometry.uuid(), 7);
}
