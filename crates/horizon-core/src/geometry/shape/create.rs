use super::{ShapeError, ShapeOp, VoxelShape};
use crate::geometry::{Aabb, Axis, COLLISION_EPSILON};

impl VoxelShape {
    /// Shapes.create(AABB): thin-box rejection and up to 1/8 cube quantization.
    pub fn from_box(aabb: Aabb) -> Result<Self, ShapeError> {
        if aabb.has_nan() {
            return Err(ShapeError::InvalidCoordinates);
        }
        if Axis::ALL
            .into_iter()
            .any(|axis| aabb.max(axis) - aabb.min(axis) < COLLISION_EPSILON)
        {
            return Ok(Self::empty());
        }
        let bits = Axis::ALL.map(|axis| find_bits(aabb.min(axis), aabb.max(axis)));
        if bits.iter().any(Option::is_none) {
            return Self::from_grid(
                Axis::ALL.map(|axis| vec![aabb.min(axis), aabb.max(axis)]),
                &[true],
            );
        }
        let bits = bits.map(Option::unwrap);
        if bits == [0, 0, 0] {
            return Ok(Self::block());
        }
        let sizes = bits.map(|bits| 1usize << bits);
        let lo = std::array::from_fn::<_, 3, _>(|i| {
            round_small(aabb.min(Axis::ALL[i]) * sizes[i] as f64) as usize
        });
        let hi = std::array::from_fn::<_, 3, _>(|i| {
            round_small(aabb.max(Axis::ALL[i]) * sizes[i] as f64) as usize
        });
        let mut occupied = Vec::with_capacity(sizes[0] * sizes[1] * sizes[2]);
        for x in 0..sizes[0] {
            for y in 0..sizes[1] {
                for z in 0..sizes[2] {
                    occupied.push(
                        x >= lo[0]
                            && x < hi[0]
                            && y >= lo[1]
                            && y < hi[1]
                            && z >= lo[2]
                            && z < hi[2],
                    );
                }
            }
        }
        Self::from_cube(sizes, &occupied)?
            .with_discrete_bounds(lo.map(|v| v as i32), hi.map(|v| v as i32))
    }
    /// VoxelShape.optimize, reconstructing greedy boxes in vanilla order.
    pub fn optimized(&self) -> Result<Self, ShapeError> {
        let mut result = Self::empty();
        for aabb in self.boxes.iter() {
            let part = Self::from_box(*aabb)?;
            result = result.join_unoptimized(&part, ShapeOp::OR)?;
        }
        Ok(result)
    }
    pub fn join(&self, other: &Self, operation: ShapeOp) -> Result<Self, ShapeError> {
        self.join_unoptimized(other, operation)?.optimized()
    }
}
fn find_bits(min: f64, max: f64) -> Option<u32> {
    if min < -COLLISION_EPSILON || max > 1.0000001 {
        return None;
    }
    for bits in 0..=3 {
        let size = (1usize << bits) as f64;
        let lo = min * size;
        let hi = max * size;
        if (lo - round_small(lo) as f64).abs() < COLLISION_EPSILON * size
            && (hi - round_small(hi) as f64).abs() < COLLISION_EPSILON * size
        {
            return Some(bits);
        }
    }
    None
}
// Values reach this helper only in the small range admitted by find_bits.
// Avoid floor(value + 0.5): addition can round a value just below a tie upward.
fn round_small(value: f64) -> i64 {
    let lower = value.floor();
    (if value - lower >= 0.5 {
        lower + 1.0
    } else {
        lower
    }) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Vec3;
    #[test]
    fn quantization_thin_boxes_and_shared_block_identity() {
        let full = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0))).unwrap();
        assert!(full.same_identity(&VoxelShape::block()));
        let slab = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0))).unwrap();
        assert!(slab.is_cube());
        assert_eq!(slab.sizes(), [1, 2, 1]);
        assert_eq!(slab.max(Axis::Y), 0.5);
        let thin = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1e-8, 1.0, 1.0))).unwrap();
        assert!(thin.same_identity(&VoxelShape::empty()));
        assert_eq!(round_small(f64::from_bits(0.5f64.to_bits() - 1)), 0);
    }
    #[test]
    fn optimization_restores_composite_shapes() {
        let a = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0))).unwrap();
        let b = VoxelShape::from_box(Aabb::new(
            Vec3::new(0.5, 0.5, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
        ))
        .unwrap();
        let combined = a.join(&b, ShapeOp::OR).unwrap();
        assert_eq!(combined.boxes().len(), 2);
        assert_eq!(combined.max(Axis::Y), 1.0);
        assert!(
            !combined
                .join_is_not_empty(&a.join_unoptimized(&b, ShapeOp::OR).unwrap(), ShapeOp::XOR)
                .unwrap()
        );
    }
    #[test]
    fn quantized_empty_boxes_keep_independent_axis_extrema() {
        let shape = VoxelShape::from_box(Aabb::new(
            crate::geometry::Vec3::new(1e-8, 0.875, -1e-8),
            crate::geometry::Vec3::new(0.12500001, 1.0, 9e-8),
        ))
        .unwrap();
        assert!(shape.is_empty());
        assert_eq!(shape.min(Axis::Y), 0.875);
        assert_eq!(shape.max(Axis::Y), 1.0);
        assert_eq!(shape.max(Axis::Z), f64::NEG_INFINITY);
        assert_eq!(
            crate::geometry::block_floor_height(&VoxelShape::empty(), || &shape),
            0.0
        );
        assert!(
            shape
                .optimized()
                .unwrap()
                .same_identity(&VoxelShape::empty())
        );
    }
}
