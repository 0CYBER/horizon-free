use super::{
    Aabb, Axis, BlockHit, COLLISION_EPSILON, Direction, Vec3, clamp, floor_i32, java_max, java_min,
};
use crate::position::BlockPos;
use std::cell::Cell;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
mod create;
mod face;
mod join;
pub use join::ShapeOp;

const ID_CHUNK_SIZE: u64 = 1 << 20;
static NEXT_SINGLE_ID_CHUNK: AtomicU64 = AtomicU64::new(ID_CHUNK_SIZE);

thread_local! {
    static LOCAL_SINGLE_ID: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

#[inline]
fn next_single_id() -> u64 {
    LOCAL_SINGLE_ID.with(|cell| {
        let (next, limit) = cell.get();
        if next < limit {
            cell.set((next + 1, limit));
            return next;
        }
        let start = NEXT_SINGLE_ID_CHUNK.fetch_add(ID_CHUNK_SIZE, Ordering::Relaxed);
        assert!(
            start <= u64::MAX - ID_CHUNK_SIZE,
            "VoxelShape inline identity counter overflow"
        );
        cell.set((start + 1, start + ID_CHUNK_SIZE));
        start
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShapeError {
    InvalidCoordinates,
    InvalidOccupancy,
    TooLarge,
    UnboundedBoolean,
    InvalidBounds,
}
impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid voxel shape: {self:?}")
    }
}
impl std::error::Error for ShapeError {}

/// Immutable discrete occupancy with exact double coordinate boundaries.
///
/// Single-cell array shapes whose faces all preserve identity, return a
/// canonical singleton, or fail validation (e.g. moved full blocks and
/// world-space entity AABBs) are stored inline without heap allocations or
/// atomic refcounts. General grids and single-cell boxes that can produce a
/// noncanonical face share a single `Arc<ShapeIdentity>` with its lazy face
/// cache.
///
/// Greedy boxes are calculated once and reused for ray and proximity queries.
/// Cube shapes retain vanilla's arithmetic cell lookup instead of replacing it
/// with a binary search over coordinates (which differs at rounding boundaries).
#[derive(Clone, Debug)]
pub struct VoxelShape {
    repr: ShapeRepr,
}

#[derive(Clone, Debug)]
enum ShapeRepr {
    /// Inline single-cell array shape (`sizes = [1, 1, 1]`, cell `(0, 0, 0)`
    /// full, `cube = false`, `coordinate_cubes = [false; 3]`,
    /// `discrete_min = [0; 3]`, `discrete_max = [1; 3]`) whose faces all
    /// preserve identity, return a canonical singleton, or fail validation.
    SingleBox {
        bounds: Aabb,
        coords: [[f64; 2]; 3],
        identity: u64,
    },
    Complex(Arc<ShapeIdentity>),
}

/// The three axes' boundaries in one allocation: axis `i` is
/// `values[starts[i]..starts[i + 1]]`. A moved shape (one per block an
/// entity's movement meets) then allocates its coordinates once, not four
/// times.
#[derive(Clone, Debug)]
struct Coords {
    values: Arc<[f64]>,
    starts: [usize; 4],
}
impl Coords {
    fn new(axes: [Vec<f64>; 3]) -> Self {
        let mut starts = [0; 4];
        for i in 0..3 {
            starts[i + 1] = starts[i] + axes[i].len();
        }
        let mut values = Vec::with_capacity(starts[3]);
        for axis in axes {
            values.extend(axis);
        }
        Self {
            values: values.into(),
            starts,
        }
    }
    fn from_single_box(coords: [[f64; 2]; 3]) -> Self {
        Self {
            values: Arc::from([
                coords[0][0],
                coords[0][1],
                coords[1][0],
                coords[1][1],
                coords[2][0],
                coords[2][1],
            ]),
            starts: [0, 2, 4, 6],
        }
    }
    fn to_vecs(&self) -> [Vec<f64>; 3] {
        std::array::from_fn(|i| self[i].to_vec())
    }
    /// Every boundary plus its axis's offset, the same addition as moving
    /// each axis on its own.
    fn moved(&self, offset: Vec3) -> Self {
        let [_, y, z, _] = self.starts;
        let [dx, dy, dz] = offset.components();
        let values = self
            .values
            .iter()
            .enumerate()
            .map(|(i, value)| {
                *value
                    + if i < y {
                        dx
                    } else if i < z {
                        dy
                    } else {
                        dz
                    }
            })
            .collect();
        Self {
            values,
            starts: self.starts,
        }
    }
}
impl std::ops::Index<usize> for Coords {
    type Output = [f64];
    fn index(&self, axis: usize) -> &[f64] {
        &self.values[self.starts[axis]..self.starts[axis + 1]]
    }
}

type FaceCache = [OnceLock<Result<VoxelShape, ShapeError>>; 6];

#[derive(Debug)]
struct ShapeIdentity {
    faces: OnceLock<Box<FaceCache>>,
    coords: Coords,
    sizes: [usize; 3],
    occupancy: Arc<[u64]>,
    cube: bool,
    coordinate_cubes: [bool; 3],
    boxes: Arc<[Aabb]>,
    bounds: Option<Aabb>,
    discrete_min: [i32; 3],
    discrete_max: [i32; 3],
    /// Non-NaN axis bounds of the sole occupied cell when `!cube && boxes.len() == 1`
    /// and every axis spans exactly one discrete cell, enabling the O(1) collide fast path.
    single_cell_coords: Option<[[f64; 2]; 3]>,
}

impl ShapeIdentity {
    fn min(&self, axis: Axis) -> f64 {
        let i = axis as usize;
        let index = self.discrete_min[i];
        if index >= self.sizes[i] as i32 {
            f64::INFINITY
        } else {
            self.coords[i][index as usize]
        }
    }
    fn max(&self, axis: Axis) -> f64 {
        let i = axis as usize;
        let index = self.discrete_max[i];
        if index <= 0 {
            f64::NEG_INFINITY
        } else {
            self.coords[i][index as usize]
        }
    }
    fn is_full(&self, x: i32, y: i32, z: i32) -> bool {
        if x < 0
            || y < 0
            || z < 0
            || x as usize >= self.sizes[0]
            || y as usize >= self.sizes[1]
            || z as usize >= self.sizes[2]
        {
            return false;
        }
        full(
            &self.occupancy,
            index(self.sizes, x as usize, y as usize, z as usize),
        )
    }
    fn find_index(&self, axis: Axis, value: f64) -> i32 {
        let i = axis as usize;
        if self.cube {
            return floor_i32(clamp(
                value * self.sizes[i] as f64,
                -1.0,
                self.sizes[i] as f64,
            ));
        }
        // Upper-bound search, including the final outside cell index.
        let coords = &self.coords[i];
        let mut start = 0;
        let mut end = coords.len();
        while start < end {
            let middle = (start + end) / 2;
            if value < coords[middle] {
                end = middle;
            } else {
                start = middle + 1;
            }
        }
        start as i32 - 1
    }
}

#[inline]
fn find_index_pair(coords: [f64; 2], value: f64) -> i32 {
    if value < coords[1] {
        if value < coords[0] { -1 } else { 0 }
    } else {
        1
    }
}

#[inline]
fn is_cube_like_pair(coords: [f64; 2]) -> bool {
    coords[0].abs() <= COLLISION_EPSILON && (coords[1] - 1.0).abs() <= COLLISION_EPSILON
}

/// Returns `true` iff every `face_shape` / `face_shape_transient` call on a
/// `1×1×1` occupied box with `coords` either preserves `self`'s identity,
/// returns a canonical singleton (`empty()` / `block()`), or fails with
/// `ShapeError::InvalidCoordinates`.
///
/// Shapes satisfying this invariant can never produce a noncanonical face and
/// therefore need no shared `FaceCache`, allowing zero-allocation inline storage.
#[inline]
fn has_only_canonical_faces(coords: &[[f64; 2]; 3]) -> bool {
    let cube = [
        is_cube_like_pair(coords[0]),
        is_cube_like_pair(coords[1]),
        is_cube_like_pair(coords[2]),
    ];
    let invalid = |c: [f64; 2]| c[0].is_nan() || c[1].is_nan() || c[0] >= c[1];
    for a in 0..3 {
        if cube[a] {
            continue;
        }
        if find_index_pair(coords[a], COLLISION_EPSILON) != 0
            && find_index_pair(coords[a], 0.9999999) != 0
        {
            continue;
        }
        let p = (a + 1) % 3;
        let q = (a + 2) % 3;
        if cube[p] && cube[q] {
            continue;
        }
        if !invalid(coords[p]) && !invalid(coords[q]) {
            return false;
        }
    }
    true
}

fn unit_occupancy() -> Arc<[u64]> {
    static ONE: OnceLock<Arc<[u64]>> = OnceLock::new();
    Arc::clone(ONE.get_or_init(|| Arc::from([1u64])))
}

/// Equivalent to `!(a < b)` including `NaN` (returns `true` if either operand is
/// `NaN`), matching `find_index`'s `if value < coord` branch without triggering
/// `clippy::neg_cmp_op_on_partial_ord`.
#[inline]
fn not_less(a: f64, b: f64) -> bool {
    !matches!(a.partial_cmp(&b), Some(std::cmp::Ordering::Less))
}

fn compute_single_cell_coords(
    cube: bool,
    coords: &Coords,
    sizes: [usize; 3],
    occupancy: &[u64],
    discrete_min: [i32; 3],
    discrete_max: [i32; 3],
    boxes: &[Aabb],
) -> Option<[[f64; 2]; 3]> {
    if cube || boxes.len() != 1 {
        return None;
    }
    if (0..3).any(|i| {
        discrete_min[i] < 0
            || discrete_max[i] != discrete_min[i].saturating_add(1)
            || discrete_max[i] as usize > sizes[i]
    }) {
        return None;
    }
    let [x, y, z] = discrete_min.map(|v| v as usize);
    if !full(occupancy, index(sizes, x, y, z)) {
        return None;
    }
    if coords.values.iter().any(|v| v.is_nan()) {
        return None;
    }
    Some([
        [coords[0][x], coords[0][x + 1]],
        [coords[1][y], coords[1][y + 1]],
        [coords[2][z], coords[2][z + 1]],
    ])
}

/// Exact O(1) collision for a non-cube shape with a single occupied cell `[c0, c1]`
/// per axis, matching upper-bound `find_index` bit-for-bit (including `NaN`, `-0.0`,
/// `±inf`, and collapsed coordinates from extreme `moved` offsets).
#[inline]
fn collide_single_box(coords: &[[f64; 2]; 3], axis: Axis, aabb: Aabb, amount: f64) -> f64 {
    if amount.abs() < COLLISION_EPSILON {
        return 0.0;
    }
    let i = axis as usize;
    let p = Axis::ALL[(i + 1) % 3];
    let q = Axis::ALL[(i + 2) % 3];
    let cp = coords[p as usize];
    if not_less(aabb.min(p) + COLLISION_EPSILON, cp[1]) || aabb.max(p) - COLLISION_EPSILON < cp[0] {
        return amount;
    }
    let cq = coords[q as usize];
    if not_less(aabb.min(q) + COLLISION_EPSILON, cq[1]) || aabb.max(q) - COLLISION_EPSILON < cq[0] {
        return amount;
    }
    let ca = coords[i];
    if amount > 0.0 {
        let v_max = aabb.max(axis) - COLLISION_EPSILON;
        if v_max < ca[0] && v_max < ca[1] {
            let distance = ca[0] - aabb.max(axis);
            return if distance >= -COLLISION_EPSILON {
                java_min(amount, distance)
            } else {
                amount
            };
        }
    } else if amount < 0.0 && not_less(aabb.min(axis) + COLLISION_EPSILON, ca[1]) {
        let distance = ca[1] - aabb.min(axis);
        return if distance <= COLLISION_EPSILON {
            java_max(amount, distance)
        } else {
            amount
        };
    }
    amount
}

/// Exact O(1) collision for a `1×1×1` cube shape (`coords = [[0.0, 1.0]; 3]`,
/// `cube = true`), preserving `floor_i32(clamp(v, -1.0, 1.0))` semantics for `NaN`.
#[inline]
fn collide_unit_cube(axis: Axis, aabb: Aabb, amount: f64) -> f64 {
    if amount.abs() < COLLISION_EPSILON {
        return 0.0;
    }
    let i = axis as usize;
    let p = Axis::ALL[(i + 1) % 3];
    let q = Axis::ALL[(i + 2) % 3];
    if aabb.min(p) + COLLISION_EPSILON >= 1.0 || aabb.max(p) - COLLISION_EPSILON < 0.0 {
        return amount;
    }
    if aabb.min(q) + COLLISION_EPSILON >= 1.0 || aabb.max(q) - COLLISION_EPSILON < 0.0 {
        return amount;
    }
    if amount > 0.0 && aabb.max(axis) - COLLISION_EPSILON < 0.0 {
        let distance = 0.0 - aabb.max(axis);
        return if distance >= -COLLISION_EPSILON {
            java_min(amount, distance)
        } else {
            amount
        };
    }
    if amount < 0.0 && aabb.min(axis) + COLLISION_EPSILON >= 1.0 {
        let distance = 1.0 - aabb.min(axis);
        return if distance <= COLLISION_EPSILON {
            java_max(amount, distance)
        } else {
            amount
        };
    }
    amount
}

fn collide_complex(inner: &ShapeIdentity, axis: Axis, aabb: Aabb, amount: f64) -> f64 {
    if amount.abs() < COLLISION_EPSILON {
        return 0.0;
    }
    let i = axis as usize;
    let p = Axis::ALL[(i + 1) % 3];
    let q = Axis::ALL[(i + 2) % 3];
    let p_start = inner.find_index(p, aabb.min(p) + COLLISION_EPSILON).max(0);
    let p_end = (inner.find_index(p, aabb.max(p) - COLLISION_EPSILON) + 1)
        .min(inner.sizes[p as usize] as i32);
    let q_start = inner.find_index(q, aabb.min(q) + COLLISION_EPSILON).max(0);
    let q_end = (inner.find_index(q, aabb.max(q) - COLLISION_EPSILON) + 1)
        .min(inner.sizes[q as usize] as i32);
    let occupied_layer = |layer| {
        (p_start..p_end).any(|u| {
            (q_start..q_end).any(|v| {
                let mut cell = [0; 3];
                cell[i] = layer;
                cell[p as usize] = u;
                cell[q as usize] = v;
                inner.is_full(cell[0], cell[1], cell[2])
            })
        })
    };
    if amount > 0.0 {
        let start = inner.find_index(axis, aabb.max(axis) - COLLISION_EPSILON) + 1;
        for layer in start..inner.sizes[i] as i32 {
            if occupied_layer(layer) {
                let distance = inner.coords[i][layer as usize] - aabb.max(axis);
                return if distance >= -COLLISION_EPSILON {
                    java_min(amount, distance)
                } else {
                    amount
                };
            }
        }
    } else if amount < 0.0 {
        let start = inner.find_index(axis, aabb.min(axis) + COLLISION_EPSILON);
        for layer in (0..start).rev() {
            if occupied_layer(layer) {
                let distance = inner.coords[i][layer as usize + 1] - aabb.min(axis);
                return if distance <= COLLISION_EPSILON {
                    java_max(amount, distance)
                } else {
                    amount
                };
            }
        }
    }
    amount
}

impl VoxelShape {
    #[inline]
    fn single_box(bounds: Aabb, coords: [[f64; 2]; 3]) -> Self {
        debug_assert!(has_only_canonical_faces(&coords));
        Self {
            repr: ShapeRepr::SingleBox {
                bounds,
                coords,
                identity: next_single_id(),
            },
        }
    }
    #[inline]
    fn single_box_or_complex(bounds: Aabb, coords: [[f64; 2]; 3]) -> Self {
        if has_only_canonical_faces(&coords) {
            return Self::single_box(bounds, coords);
        }
        Self::complex_single_box(bounds, coords)
    }
    #[cold]
    fn complex_single_box(bounds: Aabb, coords: [[f64; 2]; 3]) -> Self {
        let single_cell_coords = if coords.iter().flatten().any(|v| v.is_nan()) {
            None
        } else {
            Some(coords)
        };
        Self {
            repr: ShapeRepr::Complex(Arc::new(ShapeIdentity {
                faces: OnceLock::new(),
                coords: Coords::from_single_box(coords),
                sizes: [1, 1, 1],
                occupancy: unit_occupancy(),
                cube: false,
                coordinate_cubes: [false; 3],
                boxes: Arc::from([bounds]),
                bounds: Some(bounds),
                discrete_min: [0; 3],
                discrete_max: [1; 3],
                single_cell_coords,
            })),
        }
    }
    #[inline]
    fn single_box_coords(&self) -> Option<([[f64; 2]; 3], [bool; 3])> {
        match &self.repr {
            ShapeRepr::SingleBox { coords, .. } => Some((*coords, [false; 3])),
            ShapeRepr::Complex(inner) if inner.sizes == [1, 1, 1] && inner.boxes.len() == 1 => {
                Some((
                    [
                        [inner.coords[0][0], inner.coords[0][1]],
                        [inner.coords[1][0], inner.coords[1][1]],
                        [inner.coords[2][0], inner.coords[2][1]],
                    ],
                    inner.coordinate_cubes,
                ))
            }
            ShapeRepr::Complex(_) => None,
        }
    }
    pub fn from_grid(coords: [Vec<f64>; 3], occupied: &[bool]) -> Result<Self, ShapeError> {
        Self::build(coords, occupied, false, [false; 3])
    }
    pub fn from_cube(sizes: [usize; 3], occupied: &[bool]) -> Result<Self, ShapeError> {
        if sizes.contains(&0) {
            return Err(ShapeError::InvalidCoordinates);
        }
        let volume = volume(sizes)?;
        if occupied.len() != volume {
            return Err(ShapeError::InvalidOccupancy);
        }
        let coords = sizes.map(|size| (0..=size).map(|i| i as f64 / size as f64).collect());
        Self::build(coords, occupied, true, [true; 3])
    }
    fn build(
        coords: [Vec<f64>; 3],
        occupied: &[bool],
        cube: bool,
        coordinate_cubes: [bool; 3],
    ) -> Result<Self, ShapeError> {
        Ok(Self {
            repr: ShapeRepr::Complex(Arc::new(Self::build_complex(
                coords,
                occupied,
                cube,
                coordinate_cubes,
            )?)),
        })
    }
    fn build_complex(
        coords: [Vec<f64>; 3],
        occupied: &[bool],
        cube: bool,
        coordinate_cubes: [bool; 3],
    ) -> Result<ShapeIdentity, ShapeError> {
        if coords.iter().any(|c| {
            c.is_empty()
                || c.len() > 1_048_577
                || c.iter().any(|v| v.is_nan())
                || c.windows(2).any(|p| p[0] >= p[1])
        }) {
            return Err(ShapeError::InvalidCoordinates);
        }
        let sizes = std::array::from_fn(|i| coords[i].len() - 1);
        let count = volume(sizes)?;
        if count != occupied.len() {
            return Err(ShapeError::InvalidOccupancy);
        }
        let mut occupancy = vec![0u64; count.div_ceil(64)];
        let mut discrete_min = sizes.map(|v| v as i32);
        let mut discrete_max = [0; 3];
        for (i, full) in occupied.iter().enumerate() {
            if *full {
                occupancy[i / 64] |= 1 << (i % 64);
                let position = [
                    (i / (sizes[1] * sizes[2])) as i32,
                    (i / sizes[2] % sizes[1]) as i32,
                    (i % sizes[2]) as i32,
                ];
                for axis in 0..3 {
                    discrete_min[axis] = discrete_min[axis].min(position[axis]);
                    discrete_max[axis] = discrete_max[axis].max(position[axis] + 1);
                }
            }
        }
        let coords = Coords::new(coords);
        let boxes: Arc<[Aabb]> = Self::decompose(&coords, sizes, &occupancy).into();
        let bounds = boxes.iter().copied().reduce(Aabb::union_bounds);
        let single_cell_coords = compute_single_cell_coords(
            cube,
            &coords,
            sizes,
            &occupancy,
            discrete_min,
            discrete_max,
            &boxes,
        );
        Ok(ShapeIdentity {
            faces: OnceLock::new(),
            coords,
            sizes,
            occupancy: occupancy.into(),
            cube,
            coordinate_cubes,
            boxes,
            bounds,
            discrete_min,
            discrete_max,
            single_cell_coords,
        })
    }
    pub fn empty() -> Self {
        Self::empty_ref().clone()
    }
    /// The shared empty shape, without cloning it.
    pub fn empty_ref() -> &'static Self {
        static EMPTY: OnceLock<VoxelShape> = OnceLock::new();
        EMPTY.get_or_init(|| {
            Self::from_grid([vec![0.0], vec![0.0], vec![0.0]], &[]).expect("valid empty grid")
        })
    }
    /// `shape == Shapes.empty()`: the same as `same_identity(&empty())`
    /// without cloning the shared empty shape.
    pub fn is_empty_identity(&self) -> bool {
        self.same_identity(Self::empty_ref())
    }
    pub fn block() -> Self {
        Self::block_ref().clone()
    }
    fn block_ref() -> &'static Self {
        static BLOCK: OnceLock<VoxelShape> = OnceLock::new();
        BLOCK.get_or_init(|| Self::from_cube([1, 1, 1], &[true]).expect("valid unit cube"))
    }
    /// `shape == Shapes.block()`: the same as `same_identity(&block())`
    /// without cloning the shared unit cube.
    pub fn is_block(&self) -> bool {
        self.same_identity(Self::block_ref())
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        match &self.repr {
            ShapeRepr::SingleBox { .. } => false,
            ShapeRepr::Complex(inner) => inner.bounds.is_none(),
        }
    }
    #[inline]
    pub fn bounds(&self) -> Option<Aabb> {
        match &self.repr {
            ShapeRepr::SingleBox { bounds, .. } => Some(*bounds),
            ShapeRepr::Complex(inner) => inner.bounds,
        }
    }
    #[inline]
    pub fn boxes(&self) -> &[Aabb] {
        match &self.repr {
            ShapeRepr::SingleBox { bounds, .. } => std::slice::from_ref(bounds),
            ShapeRepr::Complex(inner) => &inner.boxes,
        }
    }
    /// `DiscreteVoxelShape.forAllEdges`: merged boundary edges, in axis-cycle order.
    pub fn edges(&self) -> Vec<[f64; 6]> {
        let sizes = self.sizes();
        let mut result = Vec::new();
        for axis in [2, 1, 0] {
            let a = (axis + 1) % 3;
            let b = (axis + 2) % 3;
            for u in 0..=sizes[a] {
                for v in 0..=sizes[b] {
                    let mut start = None;
                    for t in 0..=sizes[axis] {
                        let mut count = 0;
                        let mut odd = 0;
                        for du in 0..=1 {
                            for dv in 0..=1 {
                                let mut p = [0i32; 3];
                                p[axis] = t as i32;
                                p[a] = u as i32 + du - 1;
                                p[b] = v as i32 + dv - 1;
                                if self.is_full(p[0], p[1], p[2]) {
                                    count += 1;
                                    odd ^= du ^ dv;
                                }
                            }
                        }
                        let edge = count == 1 || count == 3 || (count == 2 && odd == 0);
                        if edge {
                            if start.is_none() {
                                start = Some(t);
                            }
                        } else if let Some(first) = start.take() {
                            let mut from = [0.0; 3];
                            let mut to = [0.0; 3];
                            from[a] = self.coordinates(Axis::ALL[a])[u];
                            from[b] = self.coordinates(Axis::ALL[b])[v];
                            to[a] = from[a];
                            to[b] = from[b];
                            from[axis] = self.coordinates(Axis::ALL[axis])[first];
                            to[axis] = self.coordinates(Axis::ALL[axis])[t];
                            result.push([from[0], from[1], from[2], to[0], to[1], to[2]]);
                        }
                    }
                }
            }
        }
        result
    }

    #[inline]
    pub fn coordinates(&self, axis: Axis) -> &[f64] {
        match &self.repr {
            ShapeRepr::SingleBox { coords, .. } => &coords[axis as usize],
            ShapeRepr::Complex(inner) => &inner.coords[axis as usize],
        }
    }
    #[inline]
    pub fn sizes(&self) -> [usize; 3] {
        match &self.repr {
            ShapeRepr::SingleBox { .. } => [1, 1, 1],
            ShapeRepr::Complex(inner) => inner.sizes,
        }
    }
    #[inline]
    pub fn is_cube(&self) -> bool {
        match &self.repr {
            ShapeRepr::SingleBox { .. } => false,
            ShapeRepr::Complex(inner) => inner.cube,
        }
    }
    /// Clones and operations returning an existing immutable shape preserve its
    /// identity. Separately constructed grids remain distinct even if equal.
    #[inline]
    pub fn same_identity(&self, other: &Self) -> bool {
        match (&self.repr, &other.repr) {
            (
                ShapeRepr::SingleBox { identity: a, .. },
                ShapeRepr::SingleBox { identity: b, .. },
            ) => a == b,
            (ShapeRepr::Complex(a), ShapeRepr::Complex(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    /// A separately constructed shape with the same immutable grid. Storage is
    /// shared, while identity-sensitive vanilla join shortcuts remain distinct.
    pub fn fresh_identity(&self) -> Self {
        match &self.repr {
            ShapeRepr::SingleBox { bounds, coords, .. } => Self::single_box(*bounds, *coords),
            ShapeRepr::Complex(inner) => Self {
                repr: ShapeRepr::Complex(Arc::new(ShapeIdentity {
                    faces: OnceLock::new(),
                    coords: inner.coords.clone(),
                    sizes: inner.sizes,
                    occupancy: Arc::clone(&inner.occupancy),
                    cube: inner.cube,
                    coordinate_cubes: inner.coordinate_cubes,
                    boxes: Arc::clone(&inner.boxes),
                    bounds: inner.bounds,
                    discrete_min: inner.discrete_min,
                    discrete_max: inner.discrete_max,
                    single_cell_coords: inner.single_cell_coords,
                })),
            },
        }
    }
    #[inline]
    pub fn cube_coordinate_axes(&self) -> [bool; 3] {
        match &self.repr {
            ShapeRepr::SingleBox { .. } => [false; 3],
            ShapeRepr::Complex(inner) => inner.coordinate_cubes,
        }
    }
    /// An array shape can contain CubePointRange axes after a partial cube
    /// merge. Preserve that metadata for subsequent Boolean operations.
    pub fn from_mixed_grid(
        coords: [Vec<f64>; 3],
        occupied: &[bool],
        coordinate_cubes: [bool; 3],
    ) -> Result<Self, ShapeError> {
        for i in 0..3 {
            if coordinate_cubes[i] {
                let size = coords[i].len().saturating_sub(1);
                if size == 0
                    || coords[i]
                        .iter()
                        .enumerate()
                        .any(|(j, v)| v.to_bits() != (j as f64 / size as f64).to_bits())
                {
                    return Err(ShapeError::InvalidCoordinates);
                }
            }
        }
        Self::build(coords, occupied, false, coordinate_cubes)
    }
    #[inline]
    pub fn min(&self, axis: Axis) -> f64 {
        match &self.repr {
            ShapeRepr::SingleBox { coords, .. } => coords[axis as usize][0],
            ShapeRepr::Complex(inner) => inner.min(axis),
        }
    }
    #[inline]
    pub fn max(&self, axis: Axis) -> f64 {
        match &self.repr {
            ShapeRepr::SingleBox { coords, .. } => coords[axis as usize][1],
            ShapeRepr::Complex(inner) => inner.max(axis),
        }
    }
    /// Discrete extrema are separate from occupancy: a quantized empty shape
    /// can retain nonempty axis ranges, and empty joins retain Java sentinels.
    #[inline]
    pub fn discrete_bounds(&self) -> ([i32; 3], [i32; 3]) {
        match &self.repr {
            ShapeRepr::SingleBox { .. } => ([0; 3], [1; 3]),
            ShapeRepr::Complex(inner) => (inner.discrete_min, inner.discrete_max),
        }
    }
    /// Import reference extrema. Nonempty shapes must match their occupancy;
    /// empty shapes admit bounded ranges or the empty-join sentinel triple.
    pub fn with_discrete_bounds(self, min: [i32; 3], max: [i32; 3]) -> Result<Self, ShapeError> {
        let sizes = self.sizes();
        let (cur_min, cur_max) = self.discrete_bounds();
        let in_range = (0..3).all(|i| {
            min[i] >= 0 && min[i] <= sizes[i] as i32 && max[i] >= 0 && max[i] <= sizes[i] as i32
        });
        let join_sentinel = min == [i32::MAX; 3] && max == [i32::MIN + 1; 3];
        if (!in_range && !join_sentinel) || (!self.is_empty() && (min != cur_min || max != cur_max))
        {
            return Err(ShapeError::InvalidBounds);
        }
        if min == cur_min && max == cur_max {
            return Ok(self);
        }
        match self.repr {
            ShapeRepr::SingleBox { .. } => unreachable!("SingleBox is never empty"),
            ShapeRepr::Complex(inner) => Ok(Self {
                repr: ShapeRepr::Complex(Arc::new(ShapeIdentity {
                    faces: OnceLock::new(),
                    coords: inner.coords.clone(),
                    sizes: inner.sizes,
                    occupancy: Arc::clone(&inner.occupancy),
                    cube: inner.cube,
                    coordinate_cubes: inner.coordinate_cubes,
                    boxes: Arc::clone(&inner.boxes),
                    bounds: inner.bounds,
                    discrete_min: min,
                    discrete_max: max,
                    single_cell_coords: None,
                })),
            }),
        }
    }
    /// Minimum occupied coordinate along an axis at the two cyclic
    /// perpendicular coordinates; an empty/outside column yields +infinity.
    pub fn min_at(&self, axis: Axis, first: f64, second: f64) -> f64 {
        let a = (axis as usize + 1) % 3;
        let b = (axis as usize + 2) % 3;
        if let ShapeRepr::SingleBox { coords, .. } = &self.repr {
            return if find_index_pair(coords[a], first) == 0
                && find_index_pair(coords[b], second) == 0
            {
                coords[axis as usize][0]
            } else {
                f64::INFINITY
            };
        }
        let mut position = self.column_position(axis, first, second);
        let sizes = self.sizes();
        let axis_coords = self.coordinates(axis);
        for i in 0..sizes[axis as usize] {
            position[axis as usize] = i as i32;
            if self.is_full(position[0], position[1], position[2]) {
                return axis_coords[i];
            }
        }
        f64::INFINITY
    }
    /// Maximum occupied coordinate at the same cyclic column; an empty/outside
    /// column yields -infinity. The maximum is the cell's upper face.
    pub fn max_at(&self, axis: Axis, first: f64, second: f64) -> f64 {
        let a = (axis as usize + 1) % 3;
        let b = (axis as usize + 2) % 3;
        if let ShapeRepr::SingleBox { coords, .. } = &self.repr {
            return if find_index_pair(coords[a], first) == 0
                && find_index_pair(coords[b], second) == 0
            {
                coords[axis as usize][1]
            } else {
                f64::NEG_INFINITY
            };
        }
        let mut position = self.column_position(axis, first, second);
        let sizes = self.sizes();
        let axis_coords = self.coordinates(axis);
        for i in (0..sizes[axis as usize]).rev() {
            position[axis as usize] = i as i32;
            if self.is_full(position[0], position[1], position[2]) {
                return axis_coords[i + 1];
            }
        }
        f64::NEG_INFINITY
    }
    fn column_position(&self, axis: Axis, first: f64, second: f64) -> [i32; 3] {
        let a = (axis as usize + 1) % 3;
        let b = (axis as usize + 2) % 3;
        let mut position = [0; 3];
        position[a] = self.find_index(Axis::ALL[a], first);
        position[b] = self.find_index(Axis::ALL[b], second);
        position
    }
    /// Vanilla movement converts cube coordinates to an array shape, even for
    /// a zero offset. Keep this distinction for subsequent cell lookups.
    pub fn moved(&self, offset: Vec3) -> Self {
        match &self.repr {
            ShapeRepr::SingleBox { bounds, coords, .. } => {
                let [dx, dy, dz] = offset.components();
                Self::single_box_or_complex(
                    bounds.move_by(offset),
                    [
                        [coords[0][0] + dx, coords[0][1] + dx],
                        [coords[1][0] + dy, coords[1][1] + dy],
                        [coords[2][0] + dz, coords[2][1] + dz],
                    ],
                )
            }
            ShapeRepr::Complex(inner) => {
                if inner.bounds.is_none() {
                    return Self::empty();
                }
                if inner.sizes == [1, 1, 1] && inner.boxes.len() == 1 {
                    let [dx, dy, dz] = offset.components();
                    return Self::single_box_or_complex(
                        inner.boxes[0].move_by(offset),
                        [
                            [inner.coords[0][0] + dx, inner.coords[0][1] + dx],
                            [inner.coords[1][0] + dy, inner.coords[1][1] + dy],
                            [inner.coords[2][0] + dz, inner.coords[2][1] + dz],
                        ],
                    );
                }
                let coords = inner.coords.moved(offset);
                // The same occupancy decomposes into the same boxes, over
                // coordinates moved by the same additions.
                let boxes: Arc<[Aabb]> = inner.boxes.iter().map(|b| b.move_by(offset)).collect();
                let bounds = inner.bounds.map(|bounds| bounds.move_by(offset));
                let single_cell_coords = compute_single_cell_coords(
                    false,
                    &coords,
                    inner.sizes,
                    &inner.occupancy,
                    inner.discrete_min,
                    inner.discrete_max,
                    &boxes,
                );
                Self {
                    repr: ShapeRepr::Complex(Arc::new(ShapeIdentity {
                        faces: OnceLock::new(),
                        coords,
                        sizes: inner.sizes,
                        occupancy: Arc::clone(&inner.occupancy),
                        cube: false,
                        coordinate_cubes: [false; 3],
                        boxes,
                        bounds,
                        discrete_min: inner.discrete_min,
                        discrete_max: inner.discrete_max,
                        single_cell_coords,
                    })),
                }
            }
        }
    }
    #[inline]
    pub fn is_full(&self, x: i32, y: i32, z: i32) -> bool {
        match &self.repr {
            ShapeRepr::SingleBox { .. } => x == 0 && y == 0 && z == 0,
            ShapeRepr::Complex(inner) => inner.is_full(x, y, z),
        }
    }
    #[inline]
    pub fn find_index(&self, axis: Axis, value: f64) -> i32 {
        match &self.repr {
            ShapeRepr::SingleBox { coords, .. } => find_index_pair(coords[axis as usize], value),
            ShapeRepr::Complex(inner) => inner.find_index(axis, value),
        }
    }
    #[inline]
    pub fn collide(&self, axis: Axis, aabb: Aabb, amount: f64) -> f64 {
        match &self.repr {
            ShapeRepr::SingleBox { coords, .. } => collide_single_box(coords, axis, aabb, amount),
            ShapeRepr::Complex(inner) => {
                if inner.bounds.is_none() {
                    return amount;
                }
                if let Some(single) = &inner.single_cell_coords {
                    return collide_single_box(single, axis, aabb, amount);
                }
                if inner.cube && inner.sizes == [1, 1, 1] && inner.boxes.len() == 1 {
                    return collide_unit_cube(axis, aabb, amount);
                }
                collide_complex(inner, axis, aabb, amount)
            }
        }
    }
    pub fn collide_all<'a>(
        axis: Axis,
        aabb: Aabb,
        shapes: impl IntoIterator<Item = &'a Self>,
        mut amount: f64,
    ) -> f64 {
        for shape in shapes {
            if amount.abs() < COLLISION_EPSILON {
                return 0.0;
            }
            amount = shape.collide(axis, aabb, amount);
        }
        amount
    }
    pub fn clip(&self, start: Vec3, end: Vec3, block: BlockPos) -> Option<BlockHit> {
        if self.is_empty() {
            return None;
        }
        let delta = end.subtract(start);
        if delta.length_squared() < COLLISION_EPSILON {
            return None;
        }
        let nudged = start.add_vector(delta.scale(0.001));
        let x = self.find_index(Axis::X, nudged.x - block.x as f64);
        let y = self.find_index(Axis::Y, nudged.y - block.y as f64);
        let z = self.find_index(Axis::Z, nudged.z - block.z as f64);
        if self.is_full(x, y, z) {
            Some(BlockHit {
                location: nudged,
                direction: Direction::approximate_nearest(delta).opposite(),
                block,
                inside: true,
            })
        } else {
            Aabb::clip_boxes(self.boxes().iter().copied(), start, end, block)
        }
    }
    /// `VoxelShape.clip` over an already decomposed union, without rebuilding its grid.
    /// Uses half-open box containment for the same nudged inside test as the grid query.
    pub fn clip_boxes<I>(boxes: I, start: Vec3, end: Vec3, block: BlockPos) -> Option<BlockHit>
    where
        I: Iterator<Item = Aabb> + Clone,
    {
        let delta = end.subtract(start);
        if delta.length_squared() < COLLISION_EPSILON {
            return None;
        }
        let nudged = start.add_vector(delta.scale(0.001));
        let local = nudged.subtract(Vec3::from_block(block));
        if boxes.clone().any(|b| b.contains(local)) {
            Some(BlockHit {
                location: nudged,
                direction: Direction::approximate_nearest(delta).opposite(),
                block,
                inside: true,
            })
        } else {
            Aabb::clip_boxes(boxes, start, end, block)
        }
    }
    pub fn closest_point(&self, point: Vec3) -> Option<Vec3> {
        let mut best = None;
        for aabb in self.boxes() {
            let candidate = Vec3::new(
                clamp(point.x, aabb.min(Axis::X), aabb.max(Axis::X)),
                clamp(point.y, aabb.min(Axis::Y), aabb.max(Axis::Y)),
                clamp(point.z, aabb.min(Axis::Z), aabb.max(Axis::Z)),
            );
            if best
                .is_none_or(|old| point.distance_squared(candidate) < point.distance_squared(old))
            {
                best = Some(candidate);
            }
        }
        best
    }
    fn decompose(coords: &Coords, sizes: [usize; 3], occupancy: &[u64]) -> Vec<Aabb> {
        if occupancy.iter().all(|&word| word == 0) {
            return Vec::new();
        }
        let mut remaining = occupancy.to_vec();
        let mut boxes = Vec::new();
        let [nx, ny, nz] = sizes;
        for y in 0..ny {
            for x in 0..nx {
                let mut run = None;
                for z in 0..=nz {
                    if z < nz && full(&remaining, index(sizes, x, y, z)) {
                        if run.is_none() {
                            run = Some(z);
                        }
                        continue;
                    }
                    let Some(z_start) = run.take() else { continue };
                    clear_strip(&mut remaining, sizes, x, y, z_start, z);
                    let mut x_end = x + 1;
                    while x_end < nx && strip_full(&remaining, sizes, x_end, y, z_start, z) {
                        clear_strip(&mut remaining, sizes, x_end, y, z_start, z);
                        x_end += 1;
                    }
                    let mut y_end = y + 1;
                    while y_end < ny
                        && (x..x_end).all(|u| strip_full(&remaining, sizes, u, y_end, z_start, z))
                    {
                        for u in x..x_end {
                            clear_strip(&mut remaining, sizes, u, y_end, z_start, z);
                        }
                        y_end += 1;
                    }
                    boxes.push(Aabb::new(
                        Vec3::new(coords[0][x], coords[1][y], coords[2][z_start]),
                        Vec3::new(coords[0][x_end], coords[1][y_end], coords[2][z]),
                    ));
                }
            }
        }
        boxes
    }
}

fn volume(sizes: [usize; 3]) -> Result<usize, ShapeError> {
    let count = sizes
        .into_iter()
        .try_fold(1usize, usize::checked_mul)
        .ok_or(ShapeError::TooLarge)?;
    if count > 1_048_576 {
        Err(ShapeError::TooLarge)
    } else {
        Ok(count)
    }
}
fn index(sizes: [usize; 3], x: usize, y: usize, z: usize) -> usize {
    (x * sizes[1] + y) * sizes[2] + z
}
fn full(bits: &[u64], i: usize) -> bool {
    bits[i / 64] & (1 << (i % 64)) != 0
}
fn strip_full(bits: &[u64], sizes: [usize; 3], x: usize, y: usize, min: usize, max: usize) -> bool {
    (min..max).all(|z| full(bits, index(sizes, x, y, z)))
}
fn clear_strip(bits: &mut [u64], sizes: [usize; 3], x: usize, y: usize, min: usize, max: usize) {
    for z in min..max {
        let i = index(sizes, x, y, z);
        bits[i / 64] &= !(1 << (i % 64));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference implementation mirroring the general discrete grid `collide` walk
    /// so every fast path (`SingleBox`, unit cube, and single-cell `Complex`) can
    /// be checked bit-for-bit against the general algorithm.
    fn collide_reference(shape: &VoxelShape, axis: Axis, aabb: Aabb, amount: f64) -> f64 {
        if shape.is_empty() {
            return amount;
        }
        if amount.abs() < COLLISION_EPSILON {
            return 0.0;
        }
        let i = axis as usize;
        let p = Axis::ALL[(i + 1) % 3];
        let q = Axis::ALL[(i + 2) % 3];
        let sizes = shape.sizes();
        let p_start = shape.find_index(p, aabb.min(p) + COLLISION_EPSILON).max(0);
        let p_end = (shape.find_index(p, aabb.max(p) - COLLISION_EPSILON) + 1)
            .min(sizes[p as usize] as i32);
        let q_start = shape.find_index(q, aabb.min(q) + COLLISION_EPSILON).max(0);
        let q_end = (shape.find_index(q, aabb.max(q) - COLLISION_EPSILON) + 1)
            .min(sizes[q as usize] as i32);
        let occupied_layer = |layer| {
            (p_start..p_end).any(|u| {
                (q_start..q_end).any(|v| {
                    let mut cell = [0; 3];
                    cell[i] = layer;
                    cell[p as usize] = u;
                    cell[q as usize] = v;
                    shape.is_full(cell[0], cell[1], cell[2])
                })
            })
        };
        let axis_coords = shape.coordinates(axis);
        if amount > 0.0 {
            let start = shape.find_index(axis, aabb.max(axis) - COLLISION_EPSILON) + 1;
            for layer in start..sizes[i] as i32 {
                if occupied_layer(layer) {
                    let distance = axis_coords[layer as usize] - aabb.max(axis);
                    return if distance >= -COLLISION_EPSILON {
                        java_min(amount, distance)
                    } else {
                        amount
                    };
                }
            }
        } else if amount < 0.0 {
            let start = shape.find_index(axis, aabb.min(axis) + COLLISION_EPSILON);
            for layer in (0..start).rev() {
                if occupied_layer(layer) {
                    let distance = axis_coords[layer as usize + 1] - aabb.min(axis);
                    return if distance <= COLLISION_EPSILON {
                        java_max(amount, distance)
                    } else {
                        amount
                    };
                }
            }
        }
        amount
    }

    fn canonical_bits(v: f64) -> u64 {
        if v.is_nan() {
            0x7ff8_0000_0000_0000
        } else {
            v.to_bits()
        }
    }

    fn next_rand(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *state
    }

    #[test]
    fn stop_at_obstacle_and_reuse_boxes() {
        let shape = VoxelShape::block();
        let player = Aabb::new(Vec3::new(-0.6, 0.0, 0.2), Vec3::new(-0.2, 1.8, 0.8));
        assert_eq!(shape.collide(Axis::X, player, 1.0), 0.2);
        assert_eq!(shape.collide(Axis::Z, player, 1.0), 1.0);
        assert_eq!(shape.boxes().len(), 1);
        assert!(
            shape
                .clip(
                    Vec3::new(0.5, 0.5, 0.5),
                    Vec3::new(2.0, 0.5, 0.5),
                    BlockPos::default()
                )
                .unwrap()
                .inside
        );
        assert_eq!(
            VoxelShape::empty().collide(Axis::X, player, -0.0).to_bits(),
            (-0.0f64).to_bits()
        );
    }
    #[test]
    fn sparse_stair_decomposition() {
        let shape = VoxelShape::from_cube(
            [2, 2, 2],
            &[true, true, false, false, true, true, true, true],
        )
        .unwrap();
        assert_eq!(shape.boxes().len(), 2);
        assert_eq!(
            shape.closest_point(Vec3::new(0.1, 0.9, 0.5)),
            Some(Vec3::new(0.1, 0.5, 0.5))
        );
        assert!(VoxelShape::from_grid(std::array::from_fn(|_| vec![0.0, 1.0]), &[]).is_err());
    }
    #[test]
    fn moved_boxes_are_the_moved_shapes_decomposition() {
        let stair = VoxelShape::from_cube(
            [2, 2, 2],
            &[true, true, false, false, true, true, true, true],
        )
        .unwrap();
        let slab = VoxelShape::from_grid([vec![0.0, 1.0], vec![0.0, 0.5], vec![0.0, 1.0]], &[true])
            .unwrap();
        let offsets = [
            Vec3::new(3.0, -64.0, -7.0),
            Vec3::new(-0.0, 0.0, -0.0),
            Vec3::new(1.0e-9, 319.999, -30_000_000.0),
            Vec3::new(f64::from(i32::MAX), f64::from(i32::MIN), 0.1),
        ];
        for shape in [VoxelShape::block(), stair, slab] {
            for offset in offsets {
                let moved = shape.moved(offset);
                let [nx, ny, nz] = shape.sizes();
                let occupied: Vec<bool> = (0..nx * ny * nz)
                    .map(|i| {
                        shape.is_full(
                            (i / (ny * nz)) as i32,
                            (i / nz % ny) as i32,
                            (i % nz) as i32,
                        )
                    })
                    .collect();
                let rebuilt = VoxelShape::from_grid(
                    Axis::ALL.map(|axis| moved.coordinates(axis).to_vec()),
                    &occupied,
                )
                .unwrap();
                assert_eq!(moved.boxes(), rebuilt.boxes());
                assert_eq!(moved.bounds(), rebuilt.bounds());
            }
        }
        assert!(VoxelShape::block().is_block());
        assert!(!VoxelShape::block().moved(Vec3::ZERO).is_block());
        assert!(!VoxelShape::block().fresh_identity().is_block());
    }
    #[test]
    fn each_moved_axis_takes_only_its_own_offset() {
        // Three axes of different lengths, so a boundary given another
        // axis's offset would show.
        let coords = [
            vec![0.0, 0.25, 1.0],
            vec![0.0, 0.5],
            vec![0.0, 0.125, 0.75, 1.0],
        ];
        let shape = VoxelShape::from_grid(coords.clone(), &[true; 6]).unwrap();
        let offset = Vec3::new(3.0, -64.0, 0.1);
        let moved = shape.moved(offset);
        for (axis, (values, by)) in Axis::ALL
            .into_iter()
            .zip(coords.iter().zip([3.0, -64.0, 0.1]))
        {
            let expected: Vec<u64> = values.iter().map(|v| (v + by).to_bits()).collect();
            let got: Vec<u64> = moved
                .coordinates(axis)
                .iter()
                .map(|v| v.to_bits())
                .collect();
            assert_eq!(got, expected, "{axis:?}");
            assert_eq!(
                shape.coordinates(axis),
                &values[..],
                "{axis:?} of the original"
            );
        }
    }

    #[test]
    fn inline_representation_bounds_struct_size_and_variants() {
        assert!(std::mem::size_of::<VoxelShape>() <= 112);
        let world_box = VoxelShape::from_box(Aabb::new(
            Vec3::new(10.0, 64.0, -5.0),
            Vec3::new(10.6, 65.8, -4.4),
        ))
        .unwrap();
        assert!(matches!(world_box.repr, ShapeRepr::SingleBox { .. }));
        let moved_block = VoxelShape::block().moved(Vec3::new(4.0, 64.0, -2.0));
        assert!(matches!(moved_block.repr, ShapeRepr::SingleBox { .. }));
        assert!(matches!(
            VoxelShape::block().moved(Vec3::ZERO).repr,
            ShapeRepr::SingleBox { .. }
        ));
        assert!(matches!(
            VoxelShape::block().moved(Vec3::new(0.5, 0.0, 0.0)).repr,
            ShapeRepr::SingleBox { .. }
        ));
        let single_axis_extending = VoxelShape::from_box(Aabb::new(
            Vec3::new(0.0, 0.0, -0.5),
            Vec3::new(1.0, 1.0, 1.5),
        ))
        .unwrap();
        assert!(matches!(
            single_axis_extending.repr,
            ShapeRepr::SingleBox { .. }
        ));
        assert!(matches!(VoxelShape::empty().repr, ShapeRepr::Complex(_)));
        assert!(matches!(VoxelShape::block().repr, ShapeRepr::Complex(_)));
        // Boxes that can produce a noncanonical face stay Complex to keep the
        // shared lazy FaceCache while retaining the O(1) collide fast path.
        let noncanonical_box = VoxelShape::from_box(Aabb::new(
            Vec3::new(-0.5, -0.5, -0.5),
            Vec3::new(1.5, 1.5, 1.5),
        ))
        .unwrap();
        assert!(matches!(
            &noncanonical_box.repr,
            ShapeRepr::Complex(inner) if inner.single_cell_coords.is_some()
        ));
        let moved_noncanonical = VoxelShape::block().moved(Vec3::new(0.5, 0.5, 0.0));
        assert!(matches!(
            &moved_noncanonical.repr,
            ShapeRepr::Complex(inner) if inner.single_cell_coords.is_some()
        ));
        let moved_slab = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0)))
            .unwrap()
            .moved(Vec3::new(4.0, 64.0, -2.0));
        assert!(matches!(
            &moved_slab.repr,
            ShapeRepr::Complex(inner) if inner.single_cell_coords.is_some()
        ));
    }

    #[test]
    fn collide_fast_paths_match_general_reference_bit_for_bit() {
        let slab = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0))).unwrap();
        let top_slab = VoxelShape::from_box(Aabb::new(
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
        ))
        .unwrap();
        let snow = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.125, 1.0))).unwrap();
        // Two adjacent occupied cells on Y so an internal grid plane at y = 0.5 exists.
        let two_cell = VoxelShape::from_grid(
            [vec![0.0, 1.0], vec![0.0, 0.5, 1.0], vec![0.0, 1.0]],
            &[true, true],
        )
        .unwrap();
        let shapes = [
            VoxelShape::empty(),
            VoxelShape::block(),
            VoxelShape::block().moved(Vec3::ZERO),
            VoxelShape::block().moved(Vec3::new(10.0, -64.0, 3.5)),
            VoxelShape::block().moved(Vec3::new(1.0e20, 0.0, -1.0e20)),
            VoxelShape::block().moved(Vec3::new(f64::NAN, 0.0, 0.0)),
            VoxelShape::from_box(Aabb::new(
                Vec3::new(-0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.75, 2.0),
            ))
            .unwrap(),
            VoxelShape::from_box(Aabb::new(
                Vec3::new(f64::NEG_INFINITY, -1.0, 0.0),
                Vec3::new(f64::INFINITY, 2.0, 0.5),
            ))
            .unwrap(),
            VoxelShape::from_box(Aabb::new(
                Vec3::new(f64::NEG_INFINITY, 0.0, 0.0),
                Vec3::new(f64::INFINITY, 1.0, 1.0),
            ))
            .unwrap()
            .moved(Vec3::new(f64::INFINITY, 0.0, 0.0)),
            slab.moved(Vec3::new(5.0, 64.0, -3.0)),
            top_slab.moved(Vec3::new(-1.0, 0.0, 2.0)),
            snow.moved(Vec3::new(1.0e20, 1.0e20, 0.0)),
            two_cell,
        ];

        let probe_values = [
            f64::NEG_INFINITY,
            -1.0e20,
            -65.0,
            -64.0 - COLLISION_EPSILON,
            -64.0,
            -64.0 + COLLISION_EPSILON,
            -63.0,
            -3.0,
            -1.0,
            -COLLISION_EPSILON,
            -0.0,
            0.0,
            COLLISION_EPSILON * 0.5,
            COLLISION_EPSILON,
            0.25,
            0.5 - COLLISION_EPSILON,
            0.5,
            0.5 + COLLISION_EPSILON,
            1.0 - COLLISION_EPSILON,
            1.0,
            1.0 + COLLISION_EPSILON,
            3.5,
            5.0,
            6.0,
            10.0,
            11.0,
            64.0,
            64.5,
            65.0,
            1.0e20,
            f64::INFINITY,
            f64::NAN,
        ];
        let amounts = [
            f64::NEG_INFINITY,
            -5.0,
            -1.0,
            -2.0 * COLLISION_EPSILON,
            -COLLISION_EPSILON,
            -0.5 * COLLISION_EPSILON,
            -0.0,
            0.0,
            0.5 * COLLISION_EPSILON,
            COLLISION_EPSILON,
            2.0 * COLLISION_EPSILON,
            1.0,
            5.0,
            f64::INFINITY,
            f64::NAN,
        ];

        let mut rng = 0x5eed_cafe_1234_5678u64;
        for (shape_idx, shape) in shapes.iter().enumerate() {
            for axis in Axis::ALL {
                for &amount in &amounts {
                    for _ in 0..40 {
                        let pick = |rng: &mut u64| {
                            probe_values[(next_rand(rng) as usize) % probe_values.len()]
                        };
                        let a = Vec3::new(pick(&mut rng), pick(&mut rng), pick(&mut rng));
                        let b = Vec3::new(pick(&mut rng), pick(&mut rng), pick(&mut rng));
                        let aabb = Aabb::new(a, b);
                        let actual = shape.collide(axis, aabb, amount);
                        let expected = collide_reference(shape, axis, aabb, amount);
                        assert_eq!(
                            canonical_bits(actual),
                            canonical_bits(expected),
                            "shape={shape_idx} axis={axis:?} amount={amount} aabb={aabb:?} actual={actual} expected={expected}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn inline_single_box_matches_complex_across_queries() {
        let aabb = Aabb::new(
            Vec3::new(3.25, -12.5, 7.125),
            Vec3::new(4.75, -10.25, 8.875),
        );
        let inline = VoxelShape::from_box(aabb).unwrap();
        assert!(matches!(inline.repr, ShapeRepr::SingleBox { .. }));
        // Build an equivalent Complex shape directly via build_complex to compare every method.
        let complex = VoxelShape {
            repr: ShapeRepr::Complex(Arc::new(
                VoxelShape::build_complex(
                    Axis::ALL.map(|axis| vec![aabb.min(axis), aabb.max(axis)]),
                    &[true],
                    false,
                    [false; 3],
                )
                .unwrap(),
            )),
        };

        assert_eq!(inline.is_empty(), complex.is_empty());
        assert_eq!(inline.bounds(), complex.bounds());
        assert_eq!(inline.boxes(), complex.boxes());
        assert_eq!(inline.sizes(), complex.sizes());
        assert_eq!(inline.is_cube(), complex.is_cube());
        assert_eq!(
            inline.cube_coordinate_axes(),
            complex.cube_coordinate_axes()
        );
        assert_eq!(inline.discrete_bounds(), complex.discrete_bounds());
        assert_eq!(inline.edges(), complex.edges());

        for axis in Axis::ALL {
            assert_eq!(inline.coordinates(axis), complex.coordinates(axis));
            assert_eq!(inline.min(axis).to_bits(), complex.min(axis).to_bits());
            assert_eq!(inline.max(axis).to_bits(), complex.max(axis).to_bits());
            for v in [-100.0, -12.5, 3.25, 4.0, 4.75, 8.875, 100.0, f64::NAN] {
                assert_eq!(inline.find_index(axis, v), complex.find_index(axis, v));
            }
            for (u, v) in [
                (-12.5, 7.125),
                (-11.0, 8.0),
                (4.0, -11.0),
                (8.0, 4.0),
                (100.0, 100.0),
                (f64::NAN, 8.0),
            ] {
                assert_eq!(
                    inline.min_at(axis, u, v).to_bits(),
                    complex.min_at(axis, u, v).to_bits()
                );
                assert_eq!(
                    inline.max_at(axis, u, v).to_bits(),
                    complex.max_at(axis, u, v).to_bits()
                );
            }
        }
        for x in -1..=2 {
            for y in -1..=2 {
                for z in -1..=2 {
                    assert_eq!(inline.is_full(x, y, z), complex.is_full(x, y, z));
                }
            }
        }
        for point in [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(4.0, -11.0, 8.0),
            Vec3::new(10.0, -5.0, 20.0),
        ] {
            assert_eq!(inline.closest_point(point), complex.closest_point(point));
        }
        let block = BlockPos::default();
        for (start, end) in [
            (Vec3::new(0.0, -11.0, 8.0), Vec3::new(6.0, -11.0, 8.0)),
            (Vec3::new(4.0, -11.0, 8.0), Vec3::new(6.0, -11.0, 8.0)),
            (Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0)),
        ] {
            assert_eq!(
                inline.clip(start, end, block),
                complex.clip(start, end, block)
            );
        }

        // with_discrete_bounds accepts exact bounds (preserving identity) and rejects mismatches.
        let same = inline.clone().with_discrete_bounds([0; 3], [1; 3]).unwrap();
        assert!(same.same_identity(&inline));
        assert_eq!(
            inline
                .clone()
                .with_discrete_bounds([0; 3], [0; 3])
                .unwrap_err(),
            ShapeError::InvalidBounds
        );
        assert!(!inline.same_identity(&inline.fresh_identity()));
        assert!(!inline.same_identity(&complex));

        let moved_inline = inline.moved(Vec3::new(-1.5, 2.5, 0.25));
        let moved_complex = complex.moved(Vec3::new(-1.5, 2.5, 0.25));
        assert_eq!(moved_inline.boxes(), moved_complex.boxes());
        assert_eq!(moved_inline.bounds(), moved_complex.bounds());
        for axis in Axis::ALL {
            assert_eq!(
                moved_inline.coordinates(axis),
                moved_complex.coordinates(axis)
            );
        }
        assert_eq!(
            inline.optimized().unwrap().boxes(),
            complex.optimized().unwrap().boxes()
        );
    }

    #[test]
    fn single_box_identities_are_unique_across_threads() {
        let base = Aabb::new(Vec3::new(10.0, 20.0, 30.0), Vec3::new(11.0, 21.0, 31.0));
        let per_thread = 256usize;
        let shapes: Vec<VoxelShape> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        (0..per_thread)
                            .map(|i| {
                                if i.is_multiple_of(2) {
                                    VoxelShape::from_box(base).unwrap()
                                } else {
                                    VoxelShape::block().moved(Vec3::new(10.0, 20.0, 30.0))
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect()
        });
        let ids: std::collections::HashSet<u64> = shapes
            .iter()
            .map(|shape| match shape.repr {
                ShapeRepr::SingleBox { identity, .. } => identity,
                ShapeRepr::Complex(_) => panic!("expected inline SingleBox"),
            })
            .collect();
        assert_eq!(ids.len(), shapes.len());
    }

    #[test]
    fn fresh_identity_shares_immutable_storage_with_distinct_identity_and_face_cache() {
        let block = VoxelShape::block();
        let block_fresh = block.fresh_identity();
        assert!(!block.same_identity(&block_fresh));
        assert!(!block_fresh.is_block());
        assert!(block.clone().same_identity(&block));
        assert_eq!(block.boxes(), block_fresh.boxes());
        assert!(std::ptr::eq(
            block.boxes().as_ptr(),
            block_fresh.boxes().as_ptr()
        ));
        for axis in Axis::ALL {
            assert!(std::ptr::eq(
                block.coordinates(axis).as_ptr(),
                block_fresh.coordinates(axis).as_ptr()
            ));
        }

        // Single-cell non-cube grids (e.g. carpets, chests) keep shared storage across
        // fresh_identity() while still moving inline into SingleBox without allocation.
        let single_grid = VoxelShape::from_mixed_grid(
            [vec![0.0, 1.0], vec![0.0, 0.0625], vec![0.0, 1.0]],
            &[true],
            [true, false, true],
        )
        .unwrap();
        let single_fresh = single_grid.fresh_identity();
        assert!(!single_grid.same_identity(&single_fresh));
        assert!(std::ptr::eq(
            single_grid.boxes().as_ptr(),
            single_fresh.boxes().as_ptr()
        ));
        for axis in Axis::ALL {
            assert!(std::ptr::eq(
                single_grid.coordinates(axis).as_ptr(),
                single_fresh.coordinates(axis).as_ptr()
            ));
        }
        assert!(matches!(
            single_grid.moved(Vec3::new(1.0, 2.0, 3.0)).repr,
            ShapeRepr::SingleBox { .. }
        ));

        // Complex multi-cell grids share coords, boxes, and occupancy while isolating
        // their lazy face caches; ordinary clones continue sharing the whole identity.
        let grid = VoxelShape::from_cube(
            [2, 2, 2],
            &[true, false, false, false, false, false, false, false],
        )
        .unwrap();
        let grid_fresh = grid.fresh_identity();
        assert!(!grid.same_identity(&grid_fresh));
        assert!(std::ptr::eq(
            grid.boxes().as_ptr(),
            grid_fresh.boxes().as_ptr()
        ));
        for axis in Axis::ALL {
            assert!(std::ptr::eq(
                grid.coordinates(axis).as_ptr(),
                grid_fresh.coordinates(axis).as_ptr()
            ));
        }
        let (ShapeRepr::Complex(a), ShapeRepr::Complex(b)) = (&grid.repr, &grid_fresh.repr) else {
            panic!("expected Complex grid representation");
        };
        assert!(Arc::ptr_eq(&a.occupancy, &b.occupancy));
        assert!(Arc::ptr_eq(&a.boxes, &b.boxes));
        assert!(Arc::ptr_eq(&a.coords.values, &b.coords.values));
        let face_a = grid.face_shape(Direction::Down).unwrap();
        assert!(b.faces.get().is_none());
        let face_b = grid_fresh.face_shape(Direction::Down).unwrap();
        assert!(!face_a.same_identity(&face_b));
        assert!(face_a.same_identity(&grid.clone().face_shape(Direction::Down).unwrap()));

        // Large grids share coordinate and box slices without copying.
        let mut occupied = vec![false; 65_536];
        occupied[0] = true;
        occupied[65_535] = true;
        let large = VoxelShape::from_grid(
            [
                (0..=65_536u32).map(f64::from).collect(),
                vec![0.0, 1.0],
                vec![0.0, 1.0],
            ],
            &occupied,
        )
        .unwrap();
        let large_fresh = large.fresh_identity();
        assert!(!large.same_identity(&large_fresh));
        assert_eq!(large.boxes().len(), 2);
        assert!(std::ptr::eq(
            large.boxes().as_ptr(),
            large_fresh.boxes().as_ptr()
        ));
        for axis in Axis::ALL {
            assert!(std::ptr::eq(
                large.coordinates(axis).as_ptr(),
                large_fresh.coordinates(axis).as_ptr()
            ));
        }

        // Empty-grid bound overrides also share immutable coordinate storage.
        let empty_grid = VoxelShape::from_grid(
            [
                (0..=1024u32).map(f64::from).collect(),
                vec![0.0, 1.0],
                vec![0.0, 1.0],
            ],
            &[false; 1024],
        )
        .unwrap();
        let bounded = empty_grid
            .clone()
            .with_discrete_bounds([i32::MAX; 3], [i32::MIN + 1; 3])
            .unwrap();
        assert!(!empty_grid.same_identity(&bounded));
        assert!(std::ptr::eq(
            empty_grid.coordinates(Axis::X).as_ptr(),
            bounded.coordinates(Axis::X).as_ptr()
        ));
    }
}
