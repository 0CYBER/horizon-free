//! Entity geometry helpers operating on an already selected collider list.
use super::{Aabb, Axis, Vec3, VoxelShape};

/// BlockGetter's floor-height decision for caller-selected collision shapes.
/// The lower block is queried lazily only if the current shape is empty.
pub fn block_floor_height<'s>(shape: &VoxelShape, below: impl FnOnce() -> &'s VoxelShape) -> f64 {
    if !shape.is_empty() {
        return shape.max(Axis::Y);
    }
    let height = below().max(Axis::Y);
    if height >= 1.0 {
        height - 1.0
    } else {
        f64::NEG_INFINITY
    }
}

/// Bounds used once by Entity.collide to select entity collisions. The same
/// entity list is reused for both subsequent block/world-border queries.
pub fn entity_collision_bounds(movement: Vec3, aabb: Aabb, max_step: f32) -> Aabb {
    aabb.expand_towards(movement)
        .expand_towards(Vec3::new(0.0, max_step as f64, 0.0))
}

/// Resolve Entity.collide's initial movement and complete step-up decision.
///
/// `collect` supplies vanilla-equivalent colliders for each requested bound,
/// including the previously selected entity shapes, applicable world border,
/// and context-dependent block shapes in vanilla order. It is called for the
/// initial movement (unless its squared length is zero), then at most once
/// more if stepping is eligible. Entity collision selection and movement's
/// grounding/velocity/inside-block side effects belong to the caller.
pub fn resolve_step_movement<'s>(
    movement: Vec3,
    aabb: Aabb,
    max_step: f32,
    on_ground: bool,
    mut collect: impl FnMut(Aabb) -> Vec<&'s VoxelShape>,
) -> Vec3 {
    try_resolve_step_movement(movement, aabb, max_step, on_ground, |bounds| {
        Ok::<_, std::convert::Infallible>(collect(bounds))
    })
    .unwrap_or_else(|never| match never {})
}

/// The same movement/step decision with fallible, owned or borrowed collider
/// collections. A provider failure stops before the next query and returns no
/// movement result. Existing world-query side effects are not rolled back.
pub fn try_resolve_step_movement<S: std::borrow::Borrow<VoxelShape>, E>(
    movement: Vec3,
    aabb: Aabb,
    max_step: f32,
    on_ground: bool,
    mut collect: impl FnMut(Aabb) -> Result<Vec<S>, E>,
) -> Result<Vec3, E> {
    let resolved = if movement.length_squared() == 0.0 {
        movement
    } else {
        let shapes = collect(aabb.expand_towards(movement))?;
        resolve_borrowed_shape_movement(movement, aabb, &shapes)
    };
    let landed = movement.y != resolved.y && movement.y < 0.0;
    if max_step > 0.0
        && (landed || on_ground)
        && (movement.x != resolved.x || movement.z != resolved.z)
    {
        let step_box = if landed {
            aabb.move_by(Vec3::new(0.0, resolved.y, 0.0))
        } else {
            aabb
        };
        let mut query = step_box.expand_towards(Vec3::new(movement.x, max_step as f64, movement.z));
        if !landed {
            query = query.expand_towards(Vec3::new(0.0, -(1.0e-5_f32 as f64), 0.0));
        }
        let shapes = collect(query)?;
        for height in borrowed_step_heights(step_box, &shapes, max_step, resolved.y as f32) {
            let candidate = resolve_borrowed_shape_movement(
                Vec3::new(movement.x, height as f64, movement.z),
                step_box,
                &shapes,
            );
            if candidate.horizontal_distance_squared() > resolved.horizontal_distance_squared() {
                return Ok(candidate.subtract(Vec3::new(
                    0.0,
                    aabb.min(Axis::Y) - step_box.min(Axis::Y),
                    0.0,
                )));
            }
        }
    }
    Ok(resolved)
}

/// Resolve vanilla's Y-first, magnitude-dependent horizontal axis ordering.
///
/// The caller must supply vanilla-equivalent block/entity/world-border shapes.
/// This helper does not collect shapes, step up, set grounded flags, modify
/// velocity, or apply inside-block effects.
pub fn resolve_shape_movement(movement: Vec3, aabb: Aabb, shapes: &[&VoxelShape]) -> Vec3 {
    resolve_borrowed_shape_movement(movement, aabb, shapes)
}
fn resolve_borrowed_shape_movement<S: std::borrow::Borrow<VoxelShape>>(
    movement: Vec3,
    aabb: Aabb,
    shapes: &[S],
) -> Vec3 {
    if shapes.is_empty() {
        return movement;
    }
    let mut result = [0.0; 3];
    for axis in Axis::step_order(movement) {
        let amount = movement.component(axis);
        if amount == 0.0 {
            continue;
        }
        let moved = aabb.move_by(Vec3::new(result[0], result[1], result[2]));
        result[axis as usize] = VoxelShape::collide_all(
            axis,
            moved,
            shapes.iter().map(std::borrow::Borrow::borrow),
            amount,
        );
    }
    Vec3::new(result[0], result[1], result[2])
}

/// Candidate heights from Entity.collectCandidateStepUpHeights, including its
/// float narrowing, signed-zero set identity, current-Y exclusion and sorting.
/// Used by the step-up decision after collecting the appropriate world shapes.
pub fn candidate_step_heights(
    aabb: Aabb,
    shapes: &[&VoxelShape],
    max_step: f32,
    current_y: f32,
) -> Vec<f32> {
    borrowed_step_heights(aabb, shapes, max_step, current_y)
}

fn borrowed_step_heights<S: std::borrow::Borrow<VoxelShape>>(
    aabb: Aabb,
    shapes: &[S],
    max_step: f32,
    current_y: f32,
) -> Vec<f32> {
    let mut heights = Vec::<f32>::with_capacity(4);
    for shape in shapes {
        let shape = shape.borrow();
        for coordinate in shape.coordinates(Axis::Y) {
            let height = (*coordinate - aabb.min(Axis::Y)) as f32;
            if height < 0.0 || height == current_y {
                continue;
            }
            if height > max_step {
                break;
            }
            // FloatArraySet uses Float.floatToIntBits, so NaNs compare equal
            // while positive and negative zero remain separate members.
            let height = if height.is_nan() { f32::NAN } else { height };
            if !heights.iter().any(|old| old.to_bits() == height.to_bits()) {
                heights.push(height);
            }
        }
    }
    heights.sort_unstable_by(f32::total_cmp);
    heights
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::BlockPos;
    #[test]
    fn movement_slides_after_a_wall_collision() {
        let wall = VoxelShape::block();
        let b = Aabb::new(Vec3::new(-0.6, 0.0, 0.2), Vec3::new(-0.2, 1.8, 0.8));
        let requested = Vec3::new(1.0, 0.0, 0.1);
        assert_eq!(
            resolve_shape_movement(requested, b, &[&wall]),
            Vec3::new(0.2, 0.0, 0.1)
        );
        assert_eq!(
            resolve_shape_movement(Vec3::new(-0.0, -0.0, -0.0), b, &[]),
            Vec3::new(-0.0, -0.0, -0.0)
        );
        assert_eq!(
            resolve_shape_movement(Vec3::new(-0.0, -0.0, -0.0), b, &[&wall]),
            Vec3::ZERO
        );
    }
    #[test]
    fn step_heights_deduplicate_and_exclude_current_y() {
        let shape = VoxelShape::from_cube([1, 2, 1], &[true, true]).unwrap();
        let b = Aabb::block(BlockPos::default());
        assert_eq!(
            candidate_step_heights(b, &[&shape, &shape], 0.6, 0.0),
            vec![0.5]
        );
        assert_eq!(
            candidate_step_heights(b, &[&shape], 1.0, 0.5),
            vec![0.0, 1.0]
        );
    }

    #[test]
    fn stepping_requeries_and_clears_a_half_block() {
        let ledge = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0))).unwrap();
        let b = Aabb::new(Vec3::new(-0.6, 0.0, 0.2), Vec3::new(-0.2, 1.8, 0.8));
        let requested = Vec3::new(1.0, 0.0, 0.0);
        let mut queries = Vec::new();
        let result = resolve_step_movement(requested, b, 0.6, true, |query| {
            queries.push(query);
            vec![&ledge]
        });
        assert_eq!(result, Vec3::new(1.0, 0.5, 0.0));
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0], b.expand_towards(requested));
        assert_eq!(queries[1].min(Axis::Y), -(1.0e-5_f32 as f64));
        assert_eq!(
            resolve_step_movement(requested, b, 0.6, false, |_| vec![&ledge]),
            Vec3::new(0.2, 0.0, 0.0)
        );
    }

    #[test]
    fn squared_length_underflow_skips_collection_and_keeps_signed_zero() {
        let b = Aabb::block(BlockPos::default());
        let requested = Vec3::new(-0.0, 1e-200, -0.0);
        assert_eq!(
            resolve_step_movement(requested, b, 0.6, true, |_| panic!("unexpected query")),
            requested
        );
    }
    #[test]
    fn floor_height_defers_the_lower_lookup_until_empty() {
        let empty = VoxelShape::empty();
        let block = VoxelShape::block();
        assert_eq!(
            block_floor_height(&block, || panic!("unnecessary lower lookup")),
            1.0
        );
        assert_eq!(block_floor_height(&empty, || &block), 0.0);
        assert_eq!(block_floor_height(&empty, || &empty), f64::NEG_INFINITY);
    }
}

#[cfg(test)]
mod fallible_tests {
    use super::*;
    #[test]
    fn owned_collision_queries_stop_at_first_failure() {
        let bounds = Aabb::new(Vec3::new(-0.6, 0.0, 0.2), Vec3::new(-0.2, 1.8, 0.8));
        let ledge = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0))).unwrap();
        for fail in [0, 1] {
            let mut queries = 0;
            let result =
                try_resolve_step_movement(Vec3::new(1.0, 0.0, 0.0), bounds, 0.6, true, |_| {
                    let call = queries;
                    queries += 1;
                    if call == fail {
                        Err(call)
                    } else {
                        Ok(vec![ledge.clone()])
                    }
                });
            assert_eq!(result, Err(fail));
            assert_eq!(queries, fail + 1);
        }
        let result = try_resolve_step_movement(
            Vec3::new(-0.0, -0.0, -0.0),
            bounds,
            0.6,
            true,
            |_| -> Result<Vec<VoxelShape>, ()> {
                panic!("zero movement must skip shape collection")
            },
        )
        .unwrap();
        assert_eq!(
            [result.x.to_bits(), result.y.to_bits(), result.z.to_bits()],
            [(-0.0f64).to_bits(); 3]
        );
    }
}
