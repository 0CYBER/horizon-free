use super::{ShapeError, VoxelShape, volume};
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
        let only_first = operation.apply(true, false);
        let only_second = operation.apply(false, true);
        for axis in Axis::ALL {
            if self.max(axis) < other.min(axis) - COLLISION_EPSILON
                || other.max(axis) < self.min(axis) - COLLISION_EPSILON
            {
                return Ok(only_first || only_second);
            }
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
    fn mergers(
        &self,
        other: &Self,
        only_first: bool,
        only_second: bool,
    ) -> Result<[Merger; 3], ShapeError> {
        let x = merge(
            1,
            &self.coords[0],
            &other.coords[0],
            self.coordinate_cubes[0],
            other.coordinate_cubes[0],
            only_first,
            only_second,
        )?;
        let y = merge(
            x.coords.len() - 1,
            &self.coords[1],
            &other.coords[1],
            self.coordinate_cubes[1],
            other.coordinate_cubes[1],
            only_first,
            only_second,
        )?;
        let multiplier = (x.coords.len() - 1)
            .checked_mul(y.coords.len() - 1)
            .ok_or(ShapeError::TooLarge)?;
        let z = merge(
            multiplier,
            &self.coords[2],
            &other.coords[2],
            self.coordinate_cubes[2],
            other.coordinate_cubes[2],
            only_first,
            only_second,
        )?;
        Ok([x, y, z])
    }
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
    use crate::geometry::Vec3;
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
}
