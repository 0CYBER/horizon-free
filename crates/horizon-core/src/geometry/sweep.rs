//! BlockGetter.forEachBlockIntersectedBetween's ordered movement-path visits.
use super::{Aabb, TraversalError, Vec3, clamp, floor_i32};
use crate::{fast_hash::FxHashSet, position::BlockPos};

const INLINE_VISITED_CAP: usize = 24;

/// Inline stack set for the common short-movement case (typically 2-12 blocks per
/// entity tick), spilling to a heap `FxHashSet` only on long sweeps.
enum VisitedSet {
    Inline {
        items: [i64; INLINE_VISITED_CAP],
        len: usize,
    },
    Hashed(FxHashSet<i64>),
}

impl VisitedSet {
    #[inline]
    fn new() -> Self {
        Self::Inline {
            items: [0; INLINE_VISITED_CAP],
            len: 0,
        }
    }

    #[inline]
    fn insert(&mut self, value: i64) -> bool {
        match self {
            Self::Inline { items, len } => {
                for &existing in &items[..*len] {
                    if existing == value {
                        return false;
                    }
                }
                if *len < INLINE_VISITED_CAP {
                    items[*len] = value;
                    *len += 1;
                    return true;
                }
                let mut set =
                    FxHashSet::with_capacity_and_hasher(INLINE_VISITED_CAP * 2, Default::default());
                for &existing in &items[..*len] {
                    set.insert(existing);
                }
                set.insert(value);
                *self = Self::Hashed(set);
                true
            }
            Self::Hashed(set) => set.insert(value),
        }
    }
}

struct Sweep<'v, V> {
    visitor: &'v mut V,
    visited: VisitedSet,
    work: usize,
    limit: usize,
}
impl<V: FnMut(BlockPos, i32) -> bool> Sweep<'_, V> {
    fn spend(&mut self) -> Result<(), TraversalError> {
        if self.work == self.limit {
            return Err(TraversalError::VisitLimit);
        }
        self.work += 1;
        Ok(())
    }
    fn visit(
        &mut self,
        positions: impl Iterator<Item = BlockPos>,
        step: i32,
        deduplicate: bool,
    ) -> Result<bool, TraversalError> {
        for pos in positions {
            self.spend()?;
            if deduplicate && !self.visited.insert(pos.pack()) {
                continue;
            }
            if !(self.visitor)(pos, step) {
                return Ok(false);
            }
            if !deduplicate {
                self.visited.insert(pos.pack());
            }
        }
        Ok(true)
    }
}

/// Visit blocks and step indices for a moving box, stopping if the callback
/// returns false. `final_box` is the box at `end`, not at `start`.
///
/// Only finite inputs/computed geometry and positive Java-int-sized block boxes
/// are supported. `max_work` limits iterator candidates and ray steps, including
/// suppressed duplicate visits. Prior callbacks remain applied on an error.
/// Block/entity effects and their state transitions are separate from visitation.
pub fn sweep_intersected_blocks(
    start: Vec3,
    end: Vec3,
    final_box: Aabb,
    max_work: usize,
    mut visitor: impl FnMut(BlockPos, i32) -> bool,
) -> Result<bool, TraversalError> {
    if !start.is_finite()
        || !end.is_finite()
        || !final_box.min_position().is_finite()
        || !final_box.max_position().is_finite()
    {
        return Err(TraversalError::NonFinite);
    }
    let movement = end.subtract(start);
    if !movement.is_finite() {
        return Err(TraversalError::NonFinite);
    }
    if movement.length_squared() < (1.0e-5_f32 * 1.0e-5_f32) as f64 {
        let blocks =
            BlockPos::between_closed_box(final_box).map_err(|_| TraversalError::InvalidBounds)?;
        // This branch has no deduplication in vanilla. Avoid allocating a set
        // or copying visited positions for stationary/tiny movements.
        for (work, pos) in blocks.enumerate() {
            if work == max_work {
                return Err(TraversalError::VisitLimit);
            }
            if !visitor(pos, 0) {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    let mut sweep = Sweep {
        visitor: &mut visitor,
        visited: VisitedSet::new(),
        work: 0,
        limit: max_work,
    };
    let initial_box = final_box.move_by(movement.scale(-1.0));
    if !initial_box.min_position().is_finite() || !initial_box.max_position().is_finite() {
        return Err(TraversalError::NonFinite);
    }
    let initial = BlockPos::between_box_corners_in_direction(initial_box, movement)
        .map_err(|_| TraversalError::InvalidBounds)?;
    if !sweep.visit(initial, 0, false)? {
        return Ok(false);
    }
    let size = final_box.size().components();
    let corner = furthest_corner(movement);
    let center = final_box.center().components();
    let finish = Vec3::new(
        center[0] + size[0] * 0.5 * corner[0] as f64,
        center[1] + size[1] * 0.5 * corner[1] as f64,
        center[2] + size[2] * 0.5 * corner[2] as f64,
    );
    let origin = finish.subtract(movement);
    if !origin.is_finite() || !finish.is_finite() || size.iter().any(|v| !v.is_finite()) {
        return Err(TraversalError::NonFinite);
    }
    let mut position = origin.components().map(floor_i32);
    let delta = movement.components();
    let sign = delta.map(|v| {
        if v == 0.0 {
            0i32
        } else if v > 0.0 {
            1
        } else {
            -1
        }
    });
    let increment = std::array::from_fn::<_, 3, _>(|i| {
        if sign[i] == 0 {
            f64::MAX
        } else {
            sign[i] as f64 / delta[i]
        }
    });
    let mut boundary = std::array::from_fn::<_, 3, _>(|i| {
        let value = origin.components()[i];
        let fraction = value - (value.floor() as i64) as f64;
        increment[i]
            * if sign[i] > 0 {
                1.0 - fraction
            } else {
                fraction
            }
    });
    let mut step = 0i32;
    while boundary.iter().any(|v| *v <= 1.0) {
        sweep.spend()?;
        let axis = if boundary[0] < boundary[1] {
            if boundary[0] < boundary[2] { 0 } else { 2 }
        } else if boundary[1] < boundary[2] {
            1
        } else {
            2
        };
        position[axis] = position[axis].wrapping_add(sign[axis]);
        boundary[axis] += increment[axis];
        let pos = BlockPos::new(position[0], position[1], position[2]);
        // The static Java clip takes raw endpoints, including wrapped +1 ints.
        let block_box = Aabb {
            min: Vec3::from_block(pos),
            max: Vec3::from_block(pos.offset(1, 1, 1)),
        };
        let Some(point) = block_box.clip(origin, finish) else {
            continue;
        };
        step = step.wrapping_add(1);
        let clamped = std::array::from_fn::<_, 3, _>(|i| {
            clamp(
                point.components()[i],
                position[i] as f64 + 1.0e-5_f32 as f64,
                (position[i] as f64 + 1.0) - 1.0e-5_f32 as f64,
            )
        });
        let opposite =
            std::array::from_fn::<_, 3, _>(|i| floor_i32(clamped[i] - size[i] * corner[i] as f64));
        let blocks = BlockPos::between_corners_in_direction(
            pos,
            BlockPos::new(opposite[0], opposite[1], opposite[2]),
            movement,
        )
        .map_err(|_| TraversalError::InvalidBounds)?;
        if !sweep.visit(blocks, step, true)? {
            return Ok(false);
        }
    }
    let final_blocks = BlockPos::between_box_corners_in_direction(final_box, movement)
        .map_err(|_| TraversalError::InvalidBounds)?;
    sweep.visit(final_blocks, step.wrapping_add(1), true)
}

fn furthest_corner(movement: Vec3) -> [i32; 3] {
    let abs = [
        Vec3::new(1.0, 0.0, 0.0).dot(movement).abs(),
        Vec3::new(0.0, 1.0, 0.0).dot(movement).abs(),
        Vec3::new(0.0, 0.0, 1.0).dot(movement).abs(),
    ];
    let [x, y, z] = movement.components().map(|v| if v >= 0.0 { 1 } else { -1 });
    if abs[0] <= abs[1] && abs[0] <= abs[2] {
        [-x, -z, y]
    } else if abs[1] <= abs[2] {
        [z, -y, -x]
    } else {
        [-y, x, -z]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiny_movement_includes_maximum_faces_and_stops_immediately() {
        let box_ = Aabb::unit_cube(Vec3::ZERO);
        let mut visits = Vec::new();
        assert_eq!(
            sweep_intersected_blocks(Vec3::ZERO, Vec3::ZERO, box_, 20, |p, step| {
                visits.push((p, step));
                true
            }),
            Ok(true)
        );
        assert_eq!(visits.len(), 8);
        assert!(visits.iter().all(|(_, step)| *step == 0));
        assert_eq!(
            sweep_intersected_blocks(Vec3::ZERO, Vec3::ZERO, box_, 20, |_, _| false),
            Ok(false)
        );
        assert_eq!(
            sweep_intersected_blocks(Vec3::ZERO, Vec3::ZERO, box_, 1, |_, _| true),
            Err(TraversalError::VisitLimit)
        );
    }

    #[test]
    fn inline_visited_set_matches_hash_set_across_spill_boundary() {
        let box_short = Aabb::new(Vec3::new(0.2, 0.0, 0.2), Vec3::new(0.8, 1.8, 0.8));
        let mut short_visits = Vec::new();
        assert_eq!(
            sweep_intersected_blocks(
                Vec3::new(0.5, 0.0, 0.5),
                Vec3::new(0.65, 0.0, 0.55),
                box_short.move_by(Vec3::new(0.15, 0.0, 0.05)),
                256,
                |p, step| {
                    short_visits.push((p, step));
                    true
                }
            ),
            Ok(true)
        );
        // Ensure no duplicate positions were visited
        let mut seen = FxHashSet::default();
        for &(p, _) in &short_visits {
            assert!(seen.insert(p), "duplicate visit {p:?}");
        }
        assert!(!short_visits.is_empty());

        // Long diagonal move that spills well past INLINE_VISITED_CAP (24)
        let mut long_visits = Vec::new();
        assert_eq!(
            sweep_intersected_blocks(
                Vec3::new(0.5, 0.0, 0.5),
                Vec3::new(12.5, 4.0, 9.5),
                box_short.move_by(Vec3::new(12.0, 4.0, 9.0)),
                4096,
                |p, step| {
                    long_visits.push((p, step));
                    true
                }
            ),
            Ok(true)
        );
        assert!(long_visits.len() > INLINE_VISITED_CAP * 2);
        let mut seen_long = FxHashSet::default();
        for &(p, _) in &long_visits {
            assert!(seen_long.insert(p), "duplicate long visit {p:?}");
        }
    }
}
