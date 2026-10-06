//! Ordered block visits for a finite ray, as in BlockGetter.traverseBlocks.
use super::{Vec3, floor_i32};
use crate::position::BlockPos;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraversalError {
    NonFinite,
    VisitLimit,
    InvalidBounds,
}
impl fmt::Display for TraversalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "block traversal failed: {self:?}")
    }
}
impl std::error::Error for TraversalError {}

/// Visit blocks in vanilla ray order until the visitor returns Some, or the
/// segment is exhausted. `None` corresponds to vanilla's miss callback.
///
/// Non-finite geometry and exceeding the caller's visit budget return typed
/// errors. Previously invoked callbacks are not rolled back on a budget error.
pub fn traverse_blocks<T>(
    start: Vec3,
    end: Vec3,
    max_visits: usize,
    mut visitor: impl FnMut(BlockPos) -> Option<T>,
) -> Result<Option<T>, TraversalError> {
    if !start.is_finite() || !end.is_finite() {
        return Err(TraversalError::NonFinite);
    }
    if start == end {
        return Ok(None);
    }
    let finish = end.lerp(start, -1e-7);
    let origin = start.lerp(end, -1e-7);
    let delta = origin.vector_to(finish).components();
    if !origin.is_finite() || !finish.is_finite() || delta.iter().any(|v| !v.is_finite()) {
        return Err(TraversalError::NonFinite);
    }
    let mut position = origin.components().map(floor_i32);
    let sign = delta.map(|v| {
        if v == 0.0 {
            0i32
        } else if v > 0.0 {
            1
        } else {
            -1
        }
    });
    let step = std::array::from_fn::<_, 3, _>(|i| {
        if sign[i] == 0 {
            f64::MAX
        } else {
            sign[i] as f64 / delta[i]
        }
    });
    let mut boundary = std::array::from_fn::<_, 3, _>(|i| {
        let value = origin.components()[i];
        let fraction = value - (value.floor() as i64) as f64;
        step[i]
            * if sign[i] > 0 {
                1.0 - fraction
            } else {
                fraction
            }
    });
    let mut visited = 0;
    loop {
        if visited == max_visits {
            return Err(TraversalError::VisitLimit);
        }
        visited += 1;
        if let Some(result) = visitor(BlockPos::new(position[0], position[1], position[2])) {
            return Ok(Some(result));
        }
        if !(boundary[0] <= 1.0 || boundary[1] <= 1.0 || boundary[2] <= 1.0) {
            return Ok(None);
        }
        // Strict comparisons give Z priority on a three-way corner tie.
        let axis = if boundary[0] < boundary[1] {
            if boundary[0] < boundary[2] { 0 } else { 2 }
        } else if boundary[1] < boundary[2] {
            1
        } else {
            2
        };
        position[axis] = position[axis].wrapping_add(sign[axis]);
        boundary[axis] += step[axis];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corner_order_and_early_exit() {
        let mut visited = Vec::new();
        assert_eq!(
            traverse_blocks::<()>(
                Vec3::new(0.5, 0.5, 0.5),
                Vec3::new(1.5, 1.5, 1.5),
                10,
                |pos| {
                    visited.push(pos);
                    None
                }
            ),
            Ok(None)
        );
        assert_eq!(
            visited,
            vec![
                BlockPos::new(0, 0, 0),
                BlockPos::new(0, 0, 1),
                BlockPos::new(0, 1, 1),
                BlockPos::new(1, 1, 1)
            ]
        );
        let result = traverse_blocks(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(1.5, 1.5, 1.5),
            10,
            |pos| (pos.z == 1).then_some(pos),
        );
        assert_eq!(result, Ok(Some(BlockPos::new(0, 0, 1))));
    }
    #[test]
    fn miss_zero_identity_and_budget_errors() {
        let mut count = 0;
        assert_eq!(
            traverse_blocks::<()>(Vec3::ZERO, Vec3::ZERO, 0, |_| panic!(
                "no visits for equal endpoints"
            )),
            Ok(None)
        );
        assert_eq!(
            traverse_blocks::<()>(Vec3::new(-0.0, 0.0, 0.0), Vec3::ZERO, 10, |_| {
                count += 1;
                None
            }),
            Ok(None)
        );
        assert_eq!(count, 4);
        assert_eq!(
            traverse_blocks::<()>(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0), 1, |_| None),
            Err(TraversalError::VisitLimit)
        );
        assert_eq!(
            traverse_blocks::<()>(Vec3::ZERO, Vec3::new(f64::NAN, 1.0, 1.0), 1, |_| None),
            Err(TraversalError::NonFinite)
        );
    }
}
