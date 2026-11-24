use super::{ShapeError, VoxelShape, not_less, volume};
use crate::geometry::{Axis, COLLISION_EPSILON};

/// A four-bit truth table, indexed as (first << 1) | second.
/// True for two empty cells would describe an unbounded volume, which vanilla
/// rejects for shape joins. Typed errors preserve that rejection here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapeOp(u8);
impl ShapeOp {
    pub const FALSE: Self = Self(0);
    pub const AND: Self = Self(8);
    pub const OR: Self = Self(14);
    pub const XOR: Self = Self(6);
    pub const ONLY_FIRST: Self = Self(4);
    pub const ONLY_SECOND: Self = Self(2);
    pub const fn from_truth_table(bits: u8) -> Self {
        Self(bits & 15)
    }
    pub const fn apply(self, first: bool, second: bool) -> bool {
        self.0 & (1 << ((first as u8) * 2 + second as u8)) != 0
    }
    fn validate(self) -> Result<(), ShapeError> {
        if self.apply(false, false) {
            Err(ShapeError::UnboundedBoolean)
        } else {
            Ok(())
        }
    }
}

const INLINE_MERGER_CAP: usize = 16;

struct InlineMerger {
    len: usize,
    first: [i8; INLINE_MERGER_CAP],
    second: [i8; INLINE_MERGER_CAP],
}

/// Stack-allocated index-only equivalent of [`merge`] for [`VoxelShape::join_is_not_empty`].
/// Returns `Ok(None)` when the merged axis exceeds `INLINE_MERGER_CAP` cells so the caller
/// falls back to the general heap-backed [`merge`].
#[allow(clippy::too_many_arguments)]
fn merge_inline(
    multiplier: usize,
    a: &[f64],
    b: &[f64],
    a_cube: bool,
    b_cube: bool,
    only_first: bool,
    only_second: bool,
) -> Result<Option<InlineMerger>, ShapeError> {
    let na = a.len() - 1;
    let nb = b.len() - 1;
    if a_cube && b_cube {
        let g = gcd(na, nb);
        let lcm = (na / g).checked_mul(nb).ok_or(ShapeError::TooLarge)?;
        if multiplier
            .checked_mul(lcm)
            .is_some_and(|volume| volume <= 256)
        {
            if lcm > INLINE_MERGER_CAP {
                return Ok(None);
            }
            let mut first = [0i8; INLINE_MERGER_CAP];
            let mut second = [0i8; INLINE_MERGER_CAP];
            let step_a = nb / g;
            let step_b = na / g;
            for i in 0..lcm {
                first[i] = (i / step_a) as i8;
                second[i] = (i / step_b) as i8;
            }
            return Ok(Some(InlineMerger {
                len: lcm,
                first,
                second,
            }));
        }
    }
    if a[na] < b[0] - COLLISION_EPSILON {
        return Ok(disjoint_inline(a.len(), b.len(), false));
    }
    if b[nb] < a[0] - COLLISION_EPSILON {
        return Ok(disjoint_inline(b.len(), a.len(), true));
    }
    if a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y) {
        if na > INLINE_MERGER_CAP {
            return Ok(None);
        }
        let mut first = [0i8; INLINE_MERGER_CAP];
        let mut second = [0i8; INLINE_MERGER_CAP];
        for i in 0..na {
            first[i] = i as i8;
            second[i] = i as i8;
        }
        return Ok(Some(InlineMerger {
            len: na,
            first,
            second,
        }));
    }
    if a.len() + b.len() > INLINE_MERGER_CAP {
        return Ok(None);
    }
    let mut first = [0i8; INLINE_MERGER_CAP];
    let mut second = [0i8; INLINE_MERGER_CAP];
    let mut count = 0usize;
    let (mut i, mut j) = (0usize, 0usize);
    let mut last = f64::NAN;
    while i < a.len() || j < b.len() {
        let a_end = i == a.len();
        let b_end = j == b.len();
        let choose_a = !a_end && (b_end || a[i] < b[j] + COLLISION_EPSILON);
        if choose_a {
            i += 1;
            if !only_first && (j == 0 || b_end) {
                continue;
            }
        } else {
            j += 1;
            if !only_second && (i == 0 || a_end) {
                continue;
            }
        }
        let ai = i as i8 - 1;
        let bj = j as i8 - 1;
        let value = if choose_a { a[i - 1] } else { b[j - 1] };
        if !matches!(
            last.partial_cmp(&(value - COLLISION_EPSILON)),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ) {
            first[count] = ai;
            second[count] = bj;
            count += 1;
            last = value;
        } else {
            let n = count - 1;
            first[n] = ai;
            second[n] = bj;
        }
    }
    let len = count.saturating_sub(1);
    Ok(Some(InlineMerger { len, first, second }))
}

fn disjoint_inline(nl: usize, nu: usize, swap: bool) -> Option<InlineMerger> {
    let len = nl + nu - 1;
    if len > INLINE_MERGER_CAP {
        return None;
    }
    let mut first = [0i8; INLINE_MERGER_CAP];
    let mut second = [0i8; INLINE_MERGER_CAP];
    for i in 0..nl {
        first[i] = i as i8;
        second[i] = -1;
    }
    for i in 0..nu - 1 {
        first[nl + i] = nl as i8 - 1;
        second[nl + i] = i as i8;
    }
    if swap {
        std::mem::swap(&mut first, &mut second);
    }
    Some(InlineMerger { len, first, second })
}

struct Merger {
    coords: Vec<f64>,
    first: Vec<i32>,
    second: Vec<i32>,
    cube_range: bool,
    discrete_cube: bool,
}

impl VoxelShape {
    /// Shapes.joinUnoptimized, preserving merged coordinates and cube metadata.
    pub fn join_unoptimized(&self, other: &Self, operation: ShapeOp) -> Result<Self, ShapeError> {
        operation.validate()?;
        if self.same_identity(other) {
            return Ok(if operation.apply(true, true) {
                self.clone()
            } else {
                Self::empty()
            });
        }
        let only_first = operation.apply(true, false);
        let only_second = operation.apply(false, true);
        if self.is_empty() {
            return Ok(if only_second {
                other.clone()
            } else {
                Self::empty()
            });
        }
        if other.is_empty() {
            return Ok(if only_first {
                self.clone()
            } else {
                Self::empty()
            });
        }
        let mergers = self.mergers(other, only_first, only_second)?;
        let sizes = std::array::from_fn(|i| mergers[i].coords.len() - 1);
        let count = volume(sizes)?;
        let mut occupied = Vec::with_capacity(count);
        for x in 0..sizes[0] {
            for y in 0..sizes[1] {
                for z in 0..sizes[2] {
                    let a = self.is_full(
                        mergers[0].first[x],
                        mergers[1].first[y],
                        mergers[2].first[z],
                    );
                    let b = other.is_full(
                        mergers[0].second[x],
                        mergers[1].second[y],
                        mergers[2].second[z],
                    );
                    occupied.push(operation.apply(a, b));
                }
            }
        }
        let [x, y, z] = mergers;
        let coordinate_cubes = [x.cube_range, y.cube_range, z.cube_range];
        let cube = x.discrete_cube && y.discrete_cube && z.discrete_cube;
        let shape = Self::build(
            [x.coords, y.coords, z.coords],
            &occupied,
            cube,
            coordinate_cubes,
        )?;
        if shape.is_empty() {
            shape.with_discrete_bounds([i32::MAX; 3], [i32::MIN + 1; 3])
        } else {
            Ok(shape)
        }
    }
    /// Shapes.joinIsNotEmpty without allocating the result occupancy or boxes.
    pub fn join_is_not_empty(&self, other: &Self, operation: ShapeOp) -> Result<bool, ShapeError> {
        operation.validate()?;
        let a_empty = self.is_empty();
        let b_empty = other.is_empty();
        if a_empty || b_empty {
            return Ok(operation.apply(!a_empty, !b_empty));
        }
        if self.same_identity(other) {
            return Ok(operation.apply(true, true));
        }
        if operation == ShapeOp::AND
            && let (Some((ca, ac)), Some((cb, bc))) =
                (self.single_box_coords(), other.single_box_coords())
        {
            return Ok(single_boxes_and_non_empty(ca, cb, ac, bc));
        }
        let only_first = operation.apply(true, false);
        let only_second = operation.apply(false, true);
        for axis in Axis::ALL {
            if self.max(axis) < other.min(axis) - COLLISION_EPSILON
                || other.max(axis) < self.min(axis) - COLLISION_EPSILON
            {
                return Ok(only_first || only_second);
            }
        }
        if let Some(mergers) = self.inline_mergers(other, only_first, only_second)? {
            let sizes = std::array::from_fn(|i| mergers[i].len);
            volume(sizes)?;
            for x in 0..sizes[0] {
                let ax = i32::from(mergers[0].first[x]);
                let bx = i32::from(mergers[0].second[x]);
                for y in 0..sizes[1] {
                    let ay = i32::from(mergers[1].first[y]);
                    let by = i32::from(mergers[1].second[y]);
                    for z in 0..sizes[2] {
                        if operation.apply(
                            self.is_full(ax, ay, i32::from(mergers[2].first[z])),
                            other.is_full(bx, by, i32::from(mergers[2].second[z])),
                        ) {
                            return Ok(true);
                        }
                    }
                }
            }
            return Ok(false);
        }
        let mergers = self.mergers(other, only_first, only_second)?;
        let sizes = std::array::from_fn(|i| mergers[i].coords.len() - 1);
        volume(sizes)?;
        for x in 0..sizes[0] {
            for y in 0..sizes[1] {
                for z in 0..sizes[2] {
                    if operation.apply(
                        self.is_full(
                            mergers[0].first[x],
                            mergers[1].first[y],
                            mergers[2].first[z],
                        ),
                        other.is_full(
                            mergers[0].second[x],
                            mergers[1].second[y],
                            mergers[2].second[z],
                        ),
                    ) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
    fn inline_mergers(
        &self,
        other: &Self,
        only_first: bool,
        only_second: bool,
    ) -> Result<Option<[InlineMerger; 3]>, ShapeError> {
        let a_cubes = self.cube_coordinate_axes();
        let b_cubes = other.cube_coordinate_axes();
        let Some(x) = merge_inline(
            1,
            self.coordinates(Axis::X),
            other.coordinates(Axis::X),
            a_cubes[0],
            b_cubes[0],
            only_first,
            only_second,
        )?
        else {
            return Ok(None);
        };
        let Some(y) = merge_inline(
            x.len,
            self.coordinates(Axis::Y),
            other.coordinates(Axis::Y),
            a_cubes[1],
            b_cubes[1],
            only_first,
            only_second,
        )?
        else {
            return Ok(None);
        };
        let multiplier = x.len.checked_mul(y.len).ok_or(ShapeError::TooLarge)?;
        let Some(z) = merge_inline(
            multiplier,
            self.coordinates(Axis::Z),
            other.coordinates(Axis::Z),
            a_cubes[2],
            b_cubes[2],
            only_first,
            only_second,
        )?
        else {
            return Ok(None);
        };
        Ok(Some([x, y, z]))
    }
    fn mergers(
        &self,
        other: &Self,
        only_first: bool,
        only_second: bool,
    ) -> Result<[Merger; 3], ShapeError> {
        let a_cubes = self.cube_coordinate_axes();
        let b_cubes = other.cube_coordinate_axes();
        let x = merge(
            1,
            self.coordinates(Axis::X),
            other.coordinates(Axis::X),
            a_cubes[0],
            b_cubes[0],
            only_first,
            only_second,
        )?;
        let y = merge(
            x.coords.len() - 1,
            self.coordinates(Axis::Y),
            other.coordinates(Axis::Y),
            a_cubes[1],
            b_cubes[1],
            only_first,
            only_second,
        )?;
        let multiplier = (x.coords.len() - 1)
            .checked_mul(y.coords.len() - 1)
            .ok_or(ShapeError::TooLarge)?;
        let z = merge(
            multiplier,
            self.coordinates(Axis::Z),
            other.coordinates(Axis::Z),
            a_cubes[2],
            b_cubes[2],
            only_first,
            only_second,
        )?;
        Ok([x, y, z])
    }
}

/// Exact zero-allocation evaluation of `join_is_not_empty(..., ShapeOp::AND)` between
/// two `1×1×1` occupied boxes, matching `merge` on all `COLLISION_EPSILON`, signed-zero,
/// infinite, collapsed, and `NaN` boundaries.
#[inline]
fn single_boxes_and_non_empty(
    a: [[f64; 2]; 3],
    b: [[f64; 2]; 3],
    a_cubes: [bool; 3],
    b_cubes: [bool; 3],
) -> bool {
    for k in 0..3 {
        if !single_axis_and_overlaps(a[k], b[k], a_cubes[k], b_cubes[k]) {
            return false;
        }
    }
    true
}

#[inline]
fn single_axis_and_overlaps(a: [f64; 2], b: [f64; 2], a_cube: bool, b_cube: bool) -> bool {
    let [a0, a1] = a;
    let [b0, b1] = b;
    if a1 < b0 - COLLISION_EPSILON || b1 < a0 - COLLISION_EPSILON {
        return false;
    }
    if (a_cube && b_cube) || (a0 == b0 && a1 == b1) {
        return true;
    }
    let v0 = if a0 < b0 + COLLISION_EPSILON {
        if a1 < b0 + COLLISION_EPSILON {
            return false;
        }
        b0
    } else {
        if not_less(a0, b1 + COLLISION_EPSILON) {
            return false;
        }
        a0
    };
    let v1 = if a1 < b1 + COLLISION_EPSILON { a1 } else { b1 };
    !matches!(
        v0.partial_cmp(&(v1 - COLLISION_EPSILON)),
        Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
    )
}

#[allow(clippy::too_many_arguments)] // Mirrors the coordinate-merger selection inputs.
fn merge(
    multiplier: usize,
    a: &[f64],
    b: &[f64],
    a_cube: bool,
    b_cube: bool,
    only_first: bool,
    only_second: bool,
) -> Result<Merger, ShapeError> {
    let na = a.len() - 1;
    let nb = b.len() - 1;
    if a_cube && b_cube {
        let gcd = gcd(na, nb);
        let lcm = (na / gcd).checked_mul(nb).ok_or(ShapeError::TooLarge)?;
        if multiplier
            .checked_mul(lcm)
            .is_some_and(|volume| volume <= 256)
        {
            return Ok(Merger {
                coords: (0..=lcm).map(|i| i as f64 / lcm as f64).collect(),
                first: (0..lcm).map(|i| (i / (nb / gcd)) as i32).collect(),
                second: (0..lcm).map(|i| (i / (na / gcd)) as i32).collect(),
                cube_range: true,
                discrete_cube: true,
            });
        }
    }
    if a[na] < b[0] - COLLISION_EPSILON {
        return Ok(disjoint(a, b, false));
    }
    if b[nb] < a[0] - COLLISION_EPSILON {
        return Ok(disjoint(b, a, true));
    }
    // Primitive DoubleArrayList equality treats the two signed zeros equally.
    if a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a == b) {
        return Ok(Merger {
            coords: a.to_vec(),
            first: (0..na).map(|i| i as i32).collect(),
            second: (0..na).map(|i| i as i32).collect(),
            cube_range: a_cube,
            discrete_cube: false,
        });
    }
    let mut coords = Vec::with_capacity(a.len() + b.len());
    let mut first = Vec::with_capacity(coords.capacity());
    let mut second = Vec::with_capacity(coords.capacity());
    let (mut i, mut j) = (0usize, 0usize);
    let mut last = f64::NAN;
    while i < a.len() || j < b.len() {
        let a_end = i == a.len();
        let b_end = j == b.len();
        let choose_a = !a_end && (b_end || a[i] < b[j] + COLLISION_EPSILON);
        if choose_a {
            i += 1;
            if !only_first && (j == 0 || b_end) {
                continue;
            }
        } else {
            j += 1;
            if !only_second && (i == 0 || a_end) {
                continue;
            }
        }
        let ai = i as i32 - 1;
        let bj = j as i32 - 1;
        let value = if choose_a { a[i - 1] } else { b[j - 1] };
        if !matches!(
            last.partial_cmp(&(value - COLLISION_EPSILON)),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ) {
            coords.push(value);
            first.push(ai);
            second.push(bj);
            last = value;
        } else {
            let n = coords.len() - 1;
            first[n] = ai;
            second[n] = bj;
        }
    }
    if coords.len() <= 1 {
        coords = vec![0.0];
        first.clear();
        second.clear();
    } else {
        first.pop();
        second.pop();
    }
    Ok(Merger {
        coords,
        first,
        second,
        cube_range: false,
        discrete_cube: false,
    })
}
fn disjoint(lower: &[f64], upper: &[f64], swap: bool) -> Merger {
    let nl = lower.len();
    let nu = upper.len();
    let mut first = Vec::with_capacity(nl + nu - 1);
    let mut second = Vec::with_capacity(nl + nu - 1);
    for i in 0..nl {
        first.push(i as i32);
        second.push(-1);
    }
    for i in 0..nu - 1 {
        first.push(nl as i32 - 1);
        second.push(i as i32);
    }
    if swap {
        std::mem::swap(&mut first, &mut second);
    }
    Merger {
        coords: lower.iter().chain(upper).copied().collect(),
        first,
        second,
        cube_range: false,
        discrete_cube: false,
    }
}
fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Aabb, Vec3};
    #[test]
    fn signed_zero_list_equality_preserves_first_coordinates() {
        let a = VoxelShape::from_grid([vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]], &[true])
            .unwrap();
        let b = VoxelShape::from_grid([vec![-0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]], &[true])
            .unwrap();
        let joined = a.join_unoptimized(&b, ShapeOp::FALSE).unwrap();
        assert!(joined.is_empty());
        assert_eq!(joined.coordinates(Axis::X)[0].to_bits(), 0.0f64.to_bits());
        assert_eq!(joined.coordinates(Axis::X).len(), 2);
    }
    #[test]
    fn returned_shapes_preserve_alias_identity() {
        let a = VoxelShape::block().moved(Vec3::new(2.0, 0.0, 0.0));
        let alias = VoxelShape::empty()
            .join_unoptimized(&a, ShapeOp::OR)
            .unwrap();
        assert!(a.same_identity(&alias));
        let result = a.join_unoptimized(&alias, ShapeOp::FALSE).unwrap();
        assert!(result.same_identity(&VoxelShape::empty()));
        assert_eq!(result.coordinates(Axis::X), &[0.0]);
        assert!(!a.same_identity(&a.moved(Vec3::ZERO)));
    }
    #[test]
    fn unions_intersections_difference_and_rejection() {
        let a = VoxelShape::block();
        let b = a.moved(Vec3::new(0.5, 0.0, 0.0));
        let union = a.join_unoptimized(&b, ShapeOp::OR).unwrap();
        assert_eq!(union.max(Axis::X), 1.5);
        let intersection = a.join_unoptimized(&b, ShapeOp::AND).unwrap();
        assert_eq!(intersection.min(Axis::X), 0.5);
        assert_eq!(intersection.max(Axis::X), 1.0);
        let difference = a.join_unoptimized(&b, ShapeOp::ONLY_FIRST).unwrap();
        assert_eq!(difference.max(Axis::X), 0.5);
        assert!(!a.join_is_not_empty(&a, ShapeOp::XOR).unwrap());
        assert!(
            a.join_unoptimized(&b, ShapeOp::from_truth_table(1))
                .is_err()
        );
    }
    #[test]
    fn single_box_and_fast_path_matches_general_merger() {
        let shapes = [
            VoxelShape::block(),
            VoxelShape::block().moved(Vec3::ZERO),
            VoxelShape::block().moved(Vec3::new(1.0, 0.0, 0.0)),
            VoxelShape::block().moved(Vec3::new(1.0 - 0.5 * COLLISION_EPSILON, 0.0, 0.0)),
            VoxelShape::block().moved(Vec3::new(1.0 - COLLISION_EPSILON, 0.0, 0.0)),
            VoxelShape::block().moved(Vec3::new(1.0 - 2.0 * COLLISION_EPSILON, 0.0, 0.0)),
            VoxelShape::block().moved(Vec3::new(1.0 + COLLISION_EPSILON, 0.0, 0.0)),
            VoxelShape::block().moved(Vec3::new(-1.0 + 0.5 * COLLISION_EPSILON, 0.0, 0.0)),
            VoxelShape::block().moved(Vec3::new(-1.0 + 2.0 * COLLISION_EPSILON, 0.0, 0.0)),
            VoxelShape::from_grid([vec![-0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]], &[true])
                .unwrap(),
            VoxelShape::from_box(Aabb::new(
                Vec3::new(0.25, -0.5, 0.1),
                Vec3::new(0.75, 1.5, 0.9),
            ))
            .unwrap(),
            VoxelShape::from_box(Aabb::new(
                Vec3::new(f64::NEG_INFINITY, -10.0, f64::NEG_INFINITY),
                Vec3::new(f64::INFINITY, 10.0, f64::INFINITY),
            ))
            .unwrap(),
            VoxelShape::from_box(Aabb::new(
                Vec3::new(10.0, 64.0, -5.0),
                Vec3::new(10.6, 65.8, -4.4),
            ))
            .unwrap(),
            VoxelShape::block().moved(Vec3::new(1.0e20, 0.0, 0.0)),
        ];

        for (i, a) in shapes.iter().enumerate() {
            for (j, b) in shapes.iter().enumerate() {
                let fast = a.join_is_not_empty(b, ShapeOp::AND).unwrap();
                // General merger reference via `mergers()` directly.
                let expected = if a.same_identity(b) {
                    true
                } else {
                    let mergers = a.mergers(b, false, false).unwrap();
                    let sizes: [usize; 3] = std::array::from_fn(|k| mergers[k].coords.len() - 1);
                    let mut found = false;
                    for x in 0..sizes[0] {
                        for y in 0..sizes[1] {
                            for z in 0..sizes[2] {
                                if a.is_full(
                                    mergers[0].first[x],
                                    mergers[1].first[y],
                                    mergers[2].first[z],
                                ) && b.is_full(
                                    mergers[0].second[x],
                                    mergers[1].second[y],
                                    mergers[2].second[z],
                                ) {
                                    found = true;
                                }
                            }
                        }
                    }
                    found
                };
                assert_eq!(fast, expected, "pair ({i}, {j})");
            }
        }
    }

    #[test]
    fn inline_mergers_match_heap_mergers_across_multi_cell_shapes_and_ops() {
        let stair = VoxelShape::from_grid(
            [vec![0.0, 0.5, 1.0], vec![0.0, 0.5, 1.0], vec![0.0, 1.0]],
            &[true, false, true, true],
        )
        .unwrap();
        let fence = VoxelShape::from_grid(
            [
                vec![0.0, 0.375, 0.625, 1.0],
                vec![0.0, 1.5],
                vec![0.0, 0.375, 0.625, 1.0],
            ],
            &[false, true, false, true, true, true, false, true, false],
        )
        .unwrap();
        let cube2 = VoxelShape::from_cube(
            [2, 2, 2],
            &[true, false, false, true, true, true, false, true],
        )
        .unwrap();
        let player = VoxelShape::from_box(Aabb::new(
            Vec3::new(0.2, 0.4, 0.2),
            Vec3::new(0.8, 2.2, 0.8),
        ))
        .unwrap();
        let shapes = [
            stair.clone(),
            stair.moved(Vec3::new(0.5, 0.0, 0.0)),
            stair.moved(Vec3::new(1.0 - 0.5 * COLLISION_EPSILON, 0.0, 0.0)),
            fence.clone(),
            fence.moved(Vec3::new(0.1, -0.25, 0.3)),
            cube2.clone(),
            player.clone(),
            player.moved(Vec3::new(0.3, 0.1, -0.1)),
        ];
        let ops = [
            ShapeOp::FALSE,
            ShapeOp::AND,
            ShapeOp::OR,
            ShapeOp::XOR,
            ShapeOp::ONLY_FIRST,
            ShapeOp::ONLY_SECOND,
        ];
        for (i, a) in shapes.iter().enumerate() {
            for (j, b) in shapes.iter().enumerate() {
                for op in ops {
                    let got = a.join_is_not_empty(b, op).unwrap();
                    let expected = if a.same_identity(b) {
                        op.apply(true, true)
                    } else {
                        let only_first = op.apply(true, false);
                        let only_second = op.apply(false, true);
                        let mergers = a.mergers(b, only_first, only_second).unwrap();
                        let sizes: [usize; 3] =
                            std::array::from_fn(|k| mergers[k].coords.len() - 1);
                        let mut found = false;
                        'outer: for x in 0..sizes[0] {
                            for y in 0..sizes[1] {
                                for z in 0..sizes[2] {
                                    if op.apply(
                                        a.is_full(
                                            mergers[0].first[x],
                                            mergers[1].first[y],
                                            mergers[2].first[z],
                                        ),
                                        b.is_full(
                                            mergers[0].second[x],
                                            mergers[1].second[y],
                                            mergers[2].second[z],
                                        ),
                                    ) {
                                        found = true;
                                        break 'outer;
                                    }
                                }
                            }
                        }
                        found
                    };
                    assert_eq!(got, expected, "pair ({i}, {j}) op {op:?}");
                }
            }
        }
    }
}
