use super::{
    ShapeError, ShapeRepr, VoxelShape, compute_single_cell_coords, find_index_pair,
    is_cube_like_pair, volume,
};
use crate::geometry::{Aabb, Axis, COLLISION_EPSILON, Direction, Vec3};
use std::sync::Arc;

impl VoxelShape {
    /// Native face selection samples just inside the unit-block boundary. The
    /// result preserves identity on empty/cube-like axes and caches other faces.
    pub fn face_shape(&self, direction: Direction) -> Result<Self, ShapeError> {
        let (axis, _) = face_axis(direction);
        if self.is_empty() || self.cube_like_along(axis) {
            return Ok(self.clone());
        }
        match &self.repr {
            ShapeRepr::SingleBox { .. } => self.calculate_face(direction),
            ShapeRepr::Complex(inner) => inner
                .faces
                .get_or_init(|| Box::new(std::array::from_fn(|_| std::sync::OnceLock::new())))
                [direction as usize]
                .get_or_init(|| self.calculate_face(direction))
                .clone(),
        }
    }
    /// [`Self::face_shape`] that keeps no face it had to calculate on this
    /// shape: for a one-off pass over a whole table at load, whose faces
    /// nothing asks for again (kept, they held tens of MB).
    pub fn face_shape_transient(&self, direction: Direction) -> Result<Self, ShapeError> {
        let (axis, _) = face_axis(direction);
        if self.is_empty() || self.cube_like_along(axis) {
            return Ok(self.clone());
        }
        match &self.repr {
            ShapeRepr::SingleBox { .. } => self.calculate_face(direction),
            ShapeRepr::Complex(inner) => match inner
                .faces
                .get()
                .and_then(|faces| faces[direction as usize].get())
            {
                Some(face) => face.clone(),
                None => self.calculate_face(direction),
            },
        }
    }
    fn cube_like_along(&self, axis: Axis) -> bool {
        let coords = self.coordinates(axis);
        coords.len() == 2 && is_cube_like_pair([coords[0], coords[1]])
    }
    fn calculate_face(&self, direction: Direction) -> Result<Self, ShapeError> {
        let (axis, positive) = face_axis(direction);
        let a = axis as usize;
        let probe = if positive {
            0.9999999
        } else {
            COLLISION_EPSILON
        };
        match &self.repr {
            ShapeRepr::SingleBox { coords, .. } => {
                if find_index_pair(coords[a], probe) != 0 {
                    return Ok(Self::empty());
                }
                let p = (a + 1) % 3;
                let q = (a + 2) % 3;
                if is_cube_like_pair(coords[p]) && is_cube_like_pair(coords[q]) {
                    return Ok(Self::block());
                }
                debug_assert!(
                    [coords[p], coords[q]]
                        .iter()
                        .any(|c| c[0].is_nan() || c[1].is_nan() || c[0] >= c[1]),
                    "SingleBox invariant guarantees no noncanonical faces"
                );
                Err(ShapeError::InvalidCoordinates)
            }
            ShapeRepr::Complex(inner) => {
                let index = inner.find_index(axis, probe);
                let mut starts = [0; 3];
                starts[a] = index;
                let mut ends = inner.sizes.map(|v| v as i32);
                ends[a] = index + 1;
                // SubShape extrema clamp the parent bounds; they are not recomputed
                // from the selected occupancy. Sparse slices can therefore be nonempty
                // by native metadata even when their decomposition contains no boxes.
                let min = std::array::from_fn(|i| {
                    inner.discrete_min[i].clamp(starts[i], ends[i]) - starts[i]
                });
                let max = std::array::from_fn(|i| {
                    inner.discrete_max[i].clamp(starts[i], ends[i]) - starts[i]
                });
                if (0..3).any(|i| min[i] >= max[i]) {
                    return Ok(Self::empty());
                }
                let mut coords = inner.coords.to_vecs();
                coords[a] = vec![0.0, 1.0];
                if (0..3).all(|i| {
                    coords[i].len() == 2
                        && coords[i][0].abs() <= COLLISION_EPSILON
                        && (coords[i][1] - 1.0).abs() <= COLLISION_EPSILON
                }) {
                    return Ok(Self::block());
                }
                let sizes = std::array::from_fn(|i| coords[i].len() - 1);
                let mut occupied = Vec::with_capacity(volume(sizes)?);
                for x in 0..sizes[0] {
                    for y in 0..sizes[1] {
                        for z in 0..sizes[2] {
                            let mut p = [x as i32, y as i32, z as i32];
                            p[a] = index;
                            occupied.push(inner.is_full(p[0], p[1], p[2]));
                        }
                    }
                }
                let mut cubes = inner.coordinate_cubes;
                cubes[a] = true;
                let mut data = Self::build_complex(coords, &occupied, false, cubes)?;
                data.discrete_min = min;
                data.discrete_max = max;
                data.bounds = Some(Aabb::new(
                    Vec3::new(data.min(Axis::X), data.min(Axis::Y), data.min(Axis::Z)),
                    Vec3::new(data.max(Axis::X), data.max(Axis::Y), data.max(Axis::Z)),
                ));
                data.single_cell_coords = compute_single_cell_coords(
                    data.cube,
                    &data.coords,
                    data.sizes,
                    &data.occupancy,
                    data.discrete_min,
                    data.discrete_max,
                    &data.boxes,
                );
                Ok(Self {
                    repr: ShapeRepr::Complex(Arc::new(data)),
                })
            }
        }
    }
}

fn face_axis(direction: Direction) -> (Axis, bool) {
    match direction {
        Direction::Down => (Axis::Y, false),
        Direction::Up => (Axis::Y, true),
        Direction::North => (Axis::Z, false),
        Direction::South => (Axis::Z, true),
        Direction::West => (Axis::X, false),
        Direction::East => (Axis::X, true),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ShapeIdentity, ShapeOp};
    use super::*;

    fn complex_inner(shape: &VoxelShape) -> &Arc<ShapeIdentity> {
        match &shape.repr {
            ShapeRepr::Complex(inner) => inner,
            ShapeRepr::SingleBox { .. } => panic!("expected Complex shape"),
        }
    }

    #[test]
    fn face_cache_is_lazy_shared_and_does_not_retain_parent() {
        let shape = VoxelShape::from_cube(
            [2, 2, 2],
            &[true, false, false, false, false, false, false, false],
        )
        .unwrap();
        assert!(complex_inner(&shape).faces.get().is_none());
        let weak = Arc::downgrade(complex_inner(&shape));
        let face = shape.face_shape(Direction::Down).unwrap();
        assert!(!face.same_identity(&shape));
        assert!(face.same_identity(&shape.clone().face_shape(Direction::Down).unwrap()));
        assert!(complex_inner(&shape.fresh_identity()).faces.get().is_none());
        assert!(
            complex_inner(&shape.moved(Vec3::ZERO))
                .faces
                .get()
                .is_none()
        );
        drop(shape);
        assert!(weak.upgrade().is_none());
        assert!(!face.is_empty());
    }

    /// A transient face is the cached one's geometry, keeps nothing, and
    /// returns the cached face once there is one.
    #[test]
    fn transient_faces_are_not_kept() {
        let shape = VoxelShape::from_cube(
            [2, 2, 2],
            &[true, false, false, false, false, false, false, false],
        )
        .unwrap();
        for direction in Direction::ALL {
            let kept = |shape: &VoxelShape| {
                complex_inner(shape)
                    .faces
                    .get()
                    .is_some_and(|faces| faces[direction as usize].get().is_some())
            };
            let transient = shape.face_shape_transient(direction).unwrap();
            assert!(!kept(&shape), "{direction:?}");
            let cached = shape.face_shape(direction).unwrap();
            assert!(kept(&shape));
            assert_eq!(transient.boxes(), cached.boxes(), "{direction:?}");
            let again = shape.face_shape_transient(direction).unwrap();
            assert!(again.same_identity(&cached));
        }
    }

    #[test]
    fn noncanonical_box_faces_preserve_uncached_cached_identity_and_xor() {
        // Box extending outside the unit cube can produce noncanonical faces, so it
        // keeps the shared lazy FaceCache rather than inline storage.
        let aabb = Aabb::new(Vec3::new(-0.5, -0.5, -0.5), Vec3::new(1.5, 1.5, 1.5));
        let shape = VoxelShape::from_box(aabb).unwrap();
        assert!(complex_inner(&shape).faces.get().is_none());
        let alias = shape.clone();

        // Two uncached face_shape_transient(Down) calls yield distinct identities and
        // do not populate the shared cache.
        let t1 = shape.face_shape_transient(Direction::Down).unwrap();
        let t2 = shape.face_shape_transient(Direction::Down).unwrap();
        assert!(complex_inner(&shape).faces.get().is_none());
        assert!(!t1.same_identity(&shape));
        assert!(!t2.same_identity(&shape));
        assert!(!t1.same_identity(&t2));

        // Because t1 and t2 have distinct identities, join_unoptimized(XOR) cannot
        // take the identity shortcut to canonical empty and preserves merged
        // coordinates and empty-join sentinel discrete bounds.
        let uncached_xor = t1.join_unoptimized(&t2, ShapeOp::XOR).unwrap();
        assert!(uncached_xor.is_empty());
        assert!(!uncached_xor.is_empty_identity());
        assert_eq!(uncached_xor.coordinates(Axis::X), &[-0.5, 1.5]);
        assert_eq!(uncached_xor.coordinates(Axis::Y), &[0.0, 1.0]);
        assert_eq!(uncached_xor.coordinates(Axis::Z), &[-0.5, 1.5]);
        assert_eq!(uncached_xor.cube_coordinate_axes(), [false, true, false]);
        assert_eq!(
            uncached_xor.discrete_bounds(),
            ([i32::MAX; 3], [i32::MIN + 1; 3])
        );

        // Cached face_shape(Down) is distinct from prior transient faces, shared
        // across pre-existing and new clones, and returned by subsequent transient
        // calls; XORing two cached calls takes the identity shortcut to empty().
        let c1 = shape.face_shape(Direction::Down).unwrap();
        let c2 = shape.face_shape(Direction::Down).unwrap();
        assert!(!c1.same_identity(&t1));
        assert!(!c1.same_identity(&t2));
        assert!(c1.same_identity(&c2));
        assert!(c1.same_identity(&alias.face_shape(Direction::Down).unwrap()));
        assert!(c1.same_identity(&shape.clone().face_shape(Direction::Down).unwrap()));
        assert!(c1.same_identity(&shape.face_shape_transient(Direction::Down).unwrap()));
        assert!(c1.same_identity(&alias.face_shape_transient(Direction::Down).unwrap()));
        assert!(!c1.same_identity(&shape.fresh_identity().face_shape(Direction::Down).unwrap()));
        assert!(!c1.same_identity(&shape.moved(Vec3::ZERO).face_shape(Direction::Down).unwrap()));
        assert!(!c1.same_identity(&shape.face_shape(Direction::Up).unwrap()));

        let cached_xor = c1.join_unoptimized(&c2, ShapeOp::XOR).unwrap();
        assert!(cached_xor.is_empty_identity());
        assert_eq!(cached_xor.coordinates(Axis::X), &[0.0]);
        assert_eq!(cached_xor.discrete_bounds(), ([0; 3], [0; 3]));

        // Depth-2 noncanonical faces also preserve uncached vs cached identity, and
        // depth-3 collapses to canonical block().
        let d2_t1 = c1.face_shape_transient(Direction::West).unwrap();
        let d2_t2 = c1.face_shape_transient(Direction::West).unwrap();
        assert!(!d2_t1.same_identity(&d2_t2));
        let d2_xor = d2_t1.join_unoptimized(&d2_t2, ShapeOp::XOR).unwrap();
        assert!(d2_xor.is_empty());
        assert!(!d2_xor.is_empty_identity());
        assert_eq!(d2_xor.coordinates(Axis::X), &[0.0, 1.0]);
        assert_eq!(d2_xor.coordinates(Axis::Y), &[0.0, 1.0]);
        assert_eq!(d2_xor.coordinates(Axis::Z), &[-0.5, 1.5]);
        assert_eq!(d2_xor.discrete_bounds(), ([i32::MAX; 3], [i32::MIN + 1; 3]));
        let d2_cached = c1.face_shape(Direction::West).unwrap();
        assert!(!d2_cached.same_identity(&d2_t1));
        assert!(d2_cached.same_identity(&c1.face_shape_transient(Direction::West).unwrap()));
        assert!(d2_cached.face_shape(Direction::North).unwrap().is_block());
        assert!(
            d2_cached
                .face_shape_transient(Direction::North)
                .unwrap()
                .is_block()
        );

        // In-unit non-quantized partial box touching a face also keeps the shared cache.
        let partial = VoxelShape::from_box(Aabb::new(
            Vec3::new(0.1, 0.0, 0.1),
            Vec3::new(0.9, 0.5, 0.9),
        ))
        .unwrap();
        assert!(complex_inner(&partial).faces.get().is_none());
        let p_t1 = partial.face_shape_transient(Direction::Down).unwrap();
        let p_t2 = partial.face_shape_transient(Direction::Down).unwrap();
        assert!(!p_t1.same_identity(&p_t2));
        let p_xor = p_t1.join_unoptimized(&p_t2, ShapeOp::XOR).unwrap();
        assert!(p_xor.is_empty());
        assert!(!p_xor.is_empty_identity());
        assert_eq!(p_xor.coordinates(Axis::X), &[0.1, 0.9]);
        assert_eq!(p_xor.coordinates(Axis::Y), &[0.0, 1.0]);
        assert_eq!(p_xor.coordinates(Axis::Z), &[0.1, 0.9]);
        assert_eq!(p_xor.discrete_bounds(), ([i32::MAX; 3], [i32::MIN + 1; 3]));
        let p_cached = partial.face_shape(Direction::Down).unwrap();
        assert!(!p_cached.same_identity(&p_t1));
        assert!(p_cached.same_identity(&partial.face_shape_transient(Direction::Down).unwrap()));

        // A moved unit block shifted on two axes can also produce a noncanonical face.
        let shifted = VoxelShape::block().moved(Vec3::new(0.5, 0.5, 0.0));
        assert!(complex_inner(&shifted).faces.get().is_none());
        let s_t1 = shifted.face_shape_transient(Direction::East).unwrap();
        let s_t2 = shifted.face_shape_transient(Direction::East).unwrap();
        assert!(!s_t1.same_identity(&s_t2));
        let s_xor = s_t1.join_unoptimized(&s_t2, ShapeOp::XOR).unwrap();
        assert!(s_xor.is_empty());
        assert!(!s_xor.is_empty_identity());
        assert_eq!(s_xor.coordinates(Axis::X), &[0.0, 1.0]);
        assert_eq!(s_xor.coordinates(Axis::Y), &[0.5, 1.5]);
        assert_eq!(s_xor.coordinates(Axis::Z), &[0.0, 1.0]);
        assert_eq!(s_xor.discrete_bounds(), ([i32::MAX; 3], [i32::MIN + 1; 3]));
        let s_cached = shifted.face_shape(Direction::East).unwrap();
        assert!(!s_cached.same_identity(&s_t1));
        assert!(s_cached.same_identity(&shifted.face_shape_transient(Direction::East).unwrap()));
    }

    #[test]
    fn inline_single_box_canonical_faces_match_complex() {
        let cases = [
            // Out-of-unit-cube box: every face returns canonical empty().
            VoxelShape::from_box(Aabb::new(
                Vec3::new(2.0, 2.0, 2.0),
                Vec3::new(3.0, 3.0, 3.0),
            ))
            .unwrap(),
            // Interior non-quantized box not touching any face: canonical empty().
            VoxelShape::from_box(Aabb::new(
                Vec3::new(0.1, 0.1, 0.1),
                Vec3::new(0.9, 0.9, 0.9),
            ))
            .unwrap(),
            // Zero-moved block: every face preserves self identity.
            VoxelShape::block().moved(Vec3::ZERO),
            // Integer-moved block on one axis: empty() on X, self on Y/Z.
            VoxelShape::block().moved(Vec3::new(1.0, 0.0, 0.0)),
            // Half-shifted block on one axis: block() on East, empty() on West, self on Y/Z.
            VoxelShape::block().moved(Vec3::new(0.5, 0.0, 0.0)),
            // Extending on one axis only: block() on North/South, self on X/Y.
            VoxelShape::from_box(Aabb::new(
                Vec3::new(0.0, 0.0, -0.5),
                Vec3::new(1.0, 1.0, 1.5),
            ))
            .unwrap(),
        ];

        for inline in cases {
            assert!(matches!(inline.repr, ShapeRepr::SingleBox { .. }));
            let complex = VoxelShape {
                repr: ShapeRepr::Complex(Arc::new(
                    VoxelShape::build_complex(
                        Axis::ALL.map(|axis| inline.coordinates(axis).to_vec()),
                        &[true],
                        false,
                        [false; 3],
                    )
                    .unwrap(),
                )),
            };
            for d in Direction::ALL {
                let it1 = inline.face_shape_transient(d).unwrap();
                let it2 = inline.face_shape_transient(d).unwrap();
                let ct1 = complex.face_shape_transient(d).unwrap();
                let ct2 = complex.face_shape_transient(d).unwrap();
                assert_eq!(it1.same_identity(&it2), ct1.same_identity(&ct2), "{d:?}");
                assert!(it1.same_identity(&it2), "{d:?}");
                assert_eq!(
                    it1.same_identity(&inline),
                    ct1.same_identity(&complex),
                    "{d:?}"
                );
                assert_eq!(it1.is_empty_identity(), ct1.is_empty_identity(), "{d:?}");
                assert_eq!(it1.is_block(), ct1.is_block(), "{d:?}");
                assert_eq!(it1.boxes(), ct1.boxes(), "{d:?}");

                let ic = inline.face_shape(d).unwrap();
                let cc = complex.face_shape(d).unwrap();
                assert!(ic.same_identity(&it1), "{d:?}");
                assert!(cc.same_identity(&ct1), "{d:?}");
                assert!(ic.same_identity(&inline.clone().face_shape(d).unwrap()));
                assert!(
                    it1.join_unoptimized(&it2, ShapeOp::XOR)
                        .unwrap()
                        .is_empty_identity(),
                    "{d:?}"
                );
            }
        }

        // Collapsed moved SingleBox reports InvalidCoordinates on hitting non-cube-like
        // axes and empty() on missed axes for both cached and transient queries.
        let collapsed = VoxelShape::from_box(Aabb::new(
            Vec3::new(-0.5, -0.5, -0.5),
            Vec3::new(1.5, 1.5, 1.5),
        ))
        .unwrap()
        .moved(Vec3::new(1.0e20, 0.0, 0.0));
        assert!(matches!(collapsed.repr, ShapeRepr::SingleBox { .. }));
        for d in [Direction::West, Direction::East] {
            assert!(collapsed.face_shape(d).unwrap().is_empty_identity());
            assert!(
                collapsed
                    .face_shape_transient(d)
                    .unwrap()
                    .is_empty_identity()
            );
        }
        for d in [
            Direction::Down,
            Direction::Up,
            Direction::North,
            Direction::South,
        ] {
            assert_eq!(
                collapsed.face_shape(d).unwrap_err(),
                ShapeError::InvalidCoordinates
            );
            assert_eq!(
                collapsed.face_shape_transient(d).unwrap_err(),
                ShapeError::InvalidCoordinates
            );
        }
    }
}
