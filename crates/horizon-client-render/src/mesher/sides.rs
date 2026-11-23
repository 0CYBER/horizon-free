//! A mesh's solid quads sorted by the side they face, so that the renderer
//! can leave out the quads turned away from the eye before the GPU shades
//! their vertices.
//!
//! A quad in the plane `x = p` whose corners go counter-clockwise seen from
//! +X faces +X. From an eye at `x < p` it is seen from behind, and at
//! `x = p` edge on; the solid pipeline culls back faces, so it draws nothing
//! from there. A bucket of such quads draws nothing at all while the eye is
//! at or below the lowest of their planes, and likewise for the other five
//! axis directions. The seventh bucket holds every other quad (sloped,
//! rotated, degenerate, or whose two triangles wind opposite ways): it is
//! drawn from everywhere.
//!
//! The buckets lie one after another in the mesh, in an order chosen so that
//! the buckets an eye can see are usually one or two runs of quads: one draw
//! each. Axis-facing buckets (both opaque and cutout) are ordered from the
//! outside toward the section interior by canonical plane, then by 2D Morton
//! block-cell order within each plane so 64-quad meshlets form compact spatial
//! tiles while coplanar overlapping quads (including those crossing cell
//! boundaries or using signed zero planes) keep their model order so overlays
//! still draw over their base. The other bucket is ordered by 3D Morton
//! block-cell order with the same overlap preservation.
//!
//! The opaque quads (`OPAQUE_LAYER`) and the cutout ones of each bucket are
//! apart, so that each of Metal's two solid passes draws only its own quads
//! (`Part`). Where the renderer draws them in two passes (`group_parts`), all
//! the opaque buckets come first, then all the cutout ones, so each part is
//! one or two runs; elsewhere each bucket holds its opaque quads, then its
//! cutout ones, so one pass over all of them draws the runs it drew before.
use std::sync::atomic::{AtomicBool, Ordering};

use super::{OPAQUE_LAYER, QuadLook, TerrainSource, TerrainVertex};

/// The buckets in mesh order: +X, +Z, other, +Y, -Z, -X, -Y. Seen from
/// above (+Y, the commonest case), each pair of horizontal sides with a
/// common corner is then one run, or two.
const BUCKETS: [Bucket; 7] = [
    Bucket::Side(0, true),
    Bucket::Side(2, true),
    Bucket::Other,
    Bucket::Side(1, true),
    Bucket::Side(2, false),
    Bucket::Side(0, false),
    Bucket::Side(1, false),
];
const OTHER: usize = 2;
/// How far, in blocks, the eye must be behind a bucket's planes before the
/// bucket is left out. The renderer computes both the eye and the section
/// origins in `f32`, with errors far below this.
const MARGIN: f32 = 1.0 / 16.0;

#[derive(Clone, Copy)]
enum Bucket {
    /// Quads in a plane across `axis` facing its positive (`true`) or
    /// negative direction.
    Side(usize, bool),
    Other,
}

/// Whether meshes sorted from now on put every bucket's opaque quads before
/// any cutout ones: set by the renderer when it draws them in two passes.
static GROUPED: AtomicBool = AtomicBool::new(false);

/// Sorts later meshes for two solid passes (`true`) or one.
pub fn group_parts(on: bool) {
    GROUPED.store(on, Ordering::Relaxed);
}

/// The bucket and whether it holds cutout quads of the `n`th part of a
/// mesh, in mesh order.
fn slot(grouped: bool, n: usize) -> (usize, bool) {
    if grouped {
        (n % 7, n >= 7)
    } else {
        (n / 2, n % 2 == 1)
    }
}

/// Where each part of each bucket of a mesh's solid quads ends, and the
/// planes that tell whether a bucket can face the eye.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sides {
    /// The solid quads up to the end of each part of each bucket, in mesh
    /// order (`slot`).
    ends: [u32; 14],
    /// Whether all the opaque parts come first (`group_parts`).
    grouped: bool,
    /// Whether opaque and cutout quads are apart; else either part is every
    /// quad.
    split: bool,
    /// Per bucket facing a positive direction, its lowest plane; facing a
    /// negative one, its highest (unused for the other bucket).
    planes: [f32; 7],
}

impl Sides {
    /// `quads` solid quads in one bucket drawn from everywhere: for meshes
    /// that were not sorted.
    pub fn unsorted(quads: u32) -> Self {
        let mut ends = [0; 14];
        ends[OTHER * 2..].fill(quads);
        Self {
            ends,
            grouped: false,
            split: false,
            planes: [0.0; 7],
        }
    }

    /// A sort with these parts' `ends` and `planes`, for tests.
    #[cfg(test)]
    pub(crate) fn from_parts(
        ends: [u32; 14],
        grouped: bool,
        split: bool,
        planes: [f32; 7],
    ) -> Self {
        Self {
            ends,
            grouped,
            split,
            planes,
        }
    }

    /// The number of solid quads the buckets hold.
    pub fn quads(&self) -> u32 {
        self.ends[13]
    }

    /// Whether this sort grouped all opaque quads before cutout quads.
    pub fn is_grouped(&self) -> bool {
        self.grouped
    }

    /// Whether opaque and cutout quads were placed in separate parts.
    pub fn is_split(&self) -> bool {
        self.split
    }

    /// Where the `n`th part ends (0..14).
    pub fn part_end(&self, n: usize) -> u32 {
        self.ends[n]
    }

    /// The plane threshold for bucket `b` (0..7).
    pub fn bucket_plane(&self, b: usize) -> f32 {
        self.planes[b]
    }

    /// Calls `draw(first, count)` for each run of quads an eye at `eye`
    /// (relative to the mesh's origin) can see the front of, nearest the
    /// start first. Empty buckets do not break a run.
    pub fn visible(&self, eye: [f32; 3], draw: impl FnMut(u32, u32)) {
        self.visible_from(eye, eye[1], Part::All, draw);
    }

    /// `visible` for every eye from `eye` straight down to the height `low`: the camera and
    /// its water mirror's eye below the plane, which differ only in y. Only the quads of
    /// `part`, but every quad of a mesh whose parts are not split.
    pub fn visible_from(&self, eye: [f32; 3], low: f32, part: Part, draw: impl FnMut(u32, u32)) {
        let seen = |i: usize| match BUCKETS[i] {
            Bucket::Side(axis, true) => eye[axis] > self.planes[i] - MARGIN,
            Bucket::Side(1, false) => low < self.planes[i] + MARGIN,
            Bucket::Side(axis, false) => eye[axis] < self.planes[i] + MARGIN,
            Bucket::Other => true,
        };
        self.runs(seen, part, draw);
    }

    /// For GPU culling's copy of `visible_from` with every part (`gfx::gpu_culling`),
    /// which then only compares: where each bucket of a mesh sorted for one pass ends,
    /// in `BUCKETS` order, and what `visible_from` compares the eye with for each. That
    /// is the lowest plane less `MARGIN` for a bucket facing a positive direction, the
    /// highest plus `MARGIN` for one facing a negative direction, and 0 for the other
    /// bucket, which it never compares. `None` for a mesh sorted for two passes
    /// (`group_parts`), whose buckets lie in two stretches each.
    pub fn thresholds(&self) -> Option<([u32; 7], [f32; 7])> {
        if self.grouped {
            return None;
        }
        let t: [f32; 7] = std::array::from_fn(|i| match BUCKETS[i] {
            Bucket::Side(_, true) => self.planes[i] - MARGIN,
            Bucket::Side(_, false) => self.planes[i] + MARGIN,
            Bucket::Other => 0.0,
        });
        // A bucket's cutout quads end it (`slot`).
        Some((std::array::from_fn(|b| self.ends[b * 2 + 1]), t))
    }

    /// The runs of `part`'s quads facing every way.
    pub fn every(&self, part: Part, draw: impl FnMut(u32, u32)) {
        self.runs(|_| true, part, draw);
    }

    /// Calls `draw` for each run of `part`'s quads in the buckets `seen` keeps.
    fn runs(&self, seen: impl Fn(usize) -> bool, part: Part, mut draw: impl FnMut(u32, u32)) {
        let mut start = 0;
        let mut run: Option<u32> = None;
        for (n, &end) in self.ends.iter().enumerate() {
            if end == start {
                continue;
            }
            let (bucket, cutout) = slot(self.grouped, n);
            let wanted = match part {
                Part::All => true,
                _ if !self.split => true,
                Part::Opaque => !cutout,
                Part::Cutout => cutout,
            };
            match (wanted && seen(bucket), run) {
                (true, None) => run = Some(start),
                (false, Some(first)) => {
                    draw(first, start - first);
                    run = None;
                }
                _ => {}
            }
            start = end;
        }
        if let Some(first) = run {
            draw(first, start - first);
        }
    }
}

/// The quads `Sides::visible_from` draws: all of them, or those of one of
/// Metal's two solid passes (terrain.wgsl's `opaque_quad`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    All,
    Opaque,
    Cutout,
}

/// The bucket of a quad: the axis plane it lies in and the side its
/// triangles (corners 0, 1, 2 and 0, 2, 3, as the renderer draws them) face.
#[inline]
fn bucket(quad: &[TerrainVertex; 4]) -> usize {
    const SIDE_BUCKET: [[usize; 2]; 3] = [[5, 0], [6, 3], [4, 1]];
    let p0 = quad[0].pos;
    let p1 = quad[1].pos;
    let p2 = quad[2].pos;
    let p3 = quad[3].pos;
    for axis in 0..3 {
        let c = p0[axis];
        if p1[axis] != c || p2[axis] != c || p3[axis] != c {
            continue;
        }
        // The normal's component along `axis`, twice each triangle's area.
        let (u, w) = match axis {
            0 => (1, 2),
            1 => (2, 0),
            _ => (0, 1),
        };
        let du1 = p1[u] - p0[u];
        let dw1 = p1[w] - p0[w];
        let du2 = p2[u] - p0[u];
        let dw2 = p2[w] - p0[w];
        let du3 = p3[u] - p0[u];
        let dw3 = p3[w] - p0[w];
        let first = du1 * dw2 - dw1 * du2;
        let second = du2 * dw3 - dw2 * du3;
        let positive = first >= 0.0 && second >= 0.0;
        let negative = first <= 0.0 && second <= 0.0;
        if positive == negative {
            // Opposite windings, or no area at all.
            return OTHER;
        }
        return SIDE_BUCKET[axis][usize::from(positive)];
    }
    OTHER
}

/// Reorders `solid`'s quads by bucket, opaque ones first in each. Axis-facing
/// buckets are ordered outside-in by canonical plane and 2D Morton block-cell
/// order, and the `OTHER` bucket by 3D Morton block-cell order; coplanar quads
/// whose footprints can overlap (within one cell, across cell boundaries, or
/// across signed-zero planes) share a spatial key so their original index
/// preserves model draw order.
#[cfg(test)]
fn sort(solid: &mut Vec<TerrainVertex>) -> Sides {
    sort_tinted(solid, &mut Vec::new())
}

/// `sort`, moving each quad's tint in `tints` (one per quad, or none) with it.
#[cfg(test)]
pub fn sort_tinted(solid: &mut Vec<TerrainVertex>, tints: &mut Vec<u32>) -> Sides {
    sort_sourced(solid, tints, &mut Vec::new(), &mut Vec::new())
}

/// `sort`, moving each quad's tint, source and look with it.
pub(super) fn sort_sourced(
    solid: &mut Vec<TerrainVertex>,
    tints: &mut Vec<u32>,
    sources: &mut Vec<TerrainSource>,
    looks: &mut Vec<QuadLook>,
) -> Sides {
    if !sources.is_empty() {
        sources.resize(solid.len() / 4, TerrainSource::default());
    }
    if !looks.is_empty() {
        looks.resize(solid.len() / 4, QuadLook::default());
    }
    let grouped = GROUPED.load(Ordering::Relaxed);
    SORT_SCRATCH
        .with_borrow_mut(|scratch| sort_with(solid, tints, sources, looks, grouped, scratch))
}

/// `sort`, with all the opaque parts first when `grouped`.
#[cfg(test)]
pub(super) fn sort_parts(solid: &mut Vec<TerrainVertex>, grouped: bool) -> Sides {
    SORT_SCRATCH.with_borrow_mut(|scratch| {
        sort_with(
            solid,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            grouped,
            scratch,
        )
    })
}

/// `sort`'s buffers, kept by each meshing thread so sorting a mesh allocates
/// nothing: each quad's part, its sort key, and the sorted quads, which are
/// copied back into the mesh's own (recycled) buffer.
#[derive(Default)]
struct SortScratch {
    slots: Vec<u8>,
    order: Vec<u32>,
    keys: Vec<u64>,
    crosser_keys: Vec<u64>,
    uf3d: Vec<u16>,
    sorted: Vec<[TerrainVertex; 4]>,
    tints: Vec<u32>,
    sources: Vec<TerrainSource>,
    looks: Vec<QuadLook>,
}
thread_local! {
    static SORT_SCRATCH: std::cell::RefCell<SortScratch> = Default::default();
}

/// `f32::total_cmp`'s order as an unsigned integer's.
fn total_order_key(v: f32) -> u32 {
    let bits = v.to_bits() as i32;
    (bits ^ (((bits >> 31) as u32) >> 1) as i32) as u32 ^ 0x8000_0000
}

/// Canonicalizes `-0.0` to `+0.0` so coplanar signed-zero quads share one geometric plane.
#[inline]
fn canonical_plane(p: f32) -> f32 {
    if p == 0.0 { 0.0 } else { p }
}

/// Exact integer plane in `0..=16`, after canonicalizing `-0.0` to `+0.0`.
#[inline]
fn integer_plane(p: f32) -> Option<usize> {
    let p = canonical_plane(p);
    if (0.0..=16.0).contains(&p) {
        let i = p as u8;
        if f32::from(i) == p {
            return Some(usize::from(i));
        }
    }
    None
}

#[inline]
fn spread_4_to_8(x: u8) -> u8 {
    let mut x = x & 0x0F;
    x = (x | (x << 2)) & 0x33;
    (x | (x << 1)) & 0x55
}

/// Interleaves the low 4 bits of `u` and `v` into an 8-bit 2D Morton code.
#[inline]
fn morton2d(u: u8, v: u8) -> u8 {
    spread_4_to_8(u) | (spread_4_to_8(v) << 1)
}

#[inline]
fn spread_4_to_12(x: u8) -> u16 {
    let mut x = u16::from(x & 0x0F);
    x = (x | (x << 4)) & 0x00C3;
    (x | (x << 2)) & 0x0249
}

/// Interleaves the low 4 bits of `x`, `y`, and `z` into a 12-bit 3D Morton code.
#[inline]
fn morton3d(x: u8, y: u8, z: u8) -> u16 {
    spread_4_to_12(x) | (spread_4_to_12(y) << 1) | (spread_4_to_12(z) << 2)
}

#[inline]
fn plane_axes(axis: usize) -> (usize, usize) {
    match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

/// Finite `[min, max]` coordinate span of `quad` along `axis`, or `None` if any vertex
/// coordinate is non-finite (or their sum overflows).
#[inline]
fn axis_bounds(quad: &[TerrainVertex; 4], axis: usize) -> Option<(f32, f32)> {
    let a = quad[0].pos[axis];
    let b = quad[1].pos[axis];
    let c = quad[2].pos[axis];
    let d = quad[3].pos[axis];
    if !((a + b) + (c + d)).is_finite() {
        return None;
    }
    let lo = a.min(b).min(c.min(d));
    let hi = a.max(b).max(c.max(d));
    Some((lo, hi))
}

/// Section-local block cell `0..=15` along `axis` when `quad` fits completely inside a
/// single unit cell `[cell, cell + 1]` within `[0, 16]`. Returns `None` if `quad` crosses
/// an integer cell boundary, extends outside `[0, 16]`, or has non-finite coordinates.
#[inline]
fn single_cell_coord(quad: &[TerrainVertex; 4], axis: usize) -> Option<u8> {
    let (lo, hi) = axis_bounds(quad, axis)?;
    if lo >= 0.0 && hi <= 16.0 {
        let cell = (lo as u8).min(15);
        if hi <= f32::from(cell + 1) {
            return Some(cell);
        }
    }
    None
}

/// 2D Morton key when `quad` lies within a single `[0, 16)` block cell on `axis`'s plane.
#[inline]
fn single_cell_morton2d(quad: &[TerrainVertex; 4], axis: usize) -> Option<u8> {
    let (u, v) = plane_axes(axis);
    Some(morton2d(
        single_cell_coord(quad, u)?,
        single_cell_coord(quad, v)?,
    ))
}

/// 3D Morton key when `quad` lies within a single `[0, 16)` block cell in all three axes.
#[inline]
fn single_cell_morton3d(quad: &[TerrainVertex; 4]) -> Option<u16> {
    Some(morton3d(
        single_cell_coord(quad, 0)?,
        single_cell_coord(quad, 1)?,
        single_cell_coord(quad, 2)?,
    ))
}

/// Integer cell coverage of a quad along one axis relative to `[0, 16]`.
#[derive(Clone, Copy)]
enum AxisSpan {
    /// Non-empty integer cell range `lo..=hi` inside `0..=15`, plus whether the quad
    /// also extends outside `[0.0, 16.0]`.
    Cells {
        lo: u8,
        hi: u8,
        touches_outside: bool,
    },
    /// Completely outside `[0.0, 16.0]` (no interior overlap with `[0, 16]`).
    OutsideOnly,
    /// Non-finite coordinates: conservatively overlaps everything.
    Unbounded,
}

#[inline]
fn axis_span(quad: &[TerrainVertex; 4], axis: usize) -> AxisSpan {
    let Some((lo, hi)) = axis_bounds(quad, axis) else {
        return AxisSpan::Unbounded;
    };
    if (lo < 0.0 && hi <= 0.0) || (lo >= 16.0 && hi > 16.0) {
        return AxisSpan::OutsideOnly;
    }
    let touches_outside = lo < 0.0 || hi > 16.0;
    let clamped_lo = lo.max(0.0);
    let clamped_hi = hi.min(16.0);
    let cell_lo = (clamped_lo as u8).min(15);
    let hi_int = clamped_hi as u8;
    let ceil_hi = hi_int + u8::from(f32::from(hi_int) < clamped_hi);
    let cell_hi = ceil_hi.saturating_sub(1).clamp(cell_lo, 15);
    AxisSpan::Cells {
        lo: cell_lo,
        hi: cell_hi,
        touches_outside,
    }
}

const OUTSIDE_2D: u16 = 256;
const OUTSIDE_3D: u16 = 4096;
/// Maximum quad count whose zero-based indices (`0..=(1 << 24) - 1`) fit in the
/// 24-bit index field of the compact packed-key fast path.
const MAX_COMPACT_QUADS: usize = 1 << 24;
const COMPACT_IDX_MASK: u32 = 0x00FF_FFFF;
const COMPACT_IDX_MASK_U64: u64 = 0x00FF_FFFF;
const COMPACT_PLANE_AND_IDX_MASK: u64 = 0xFFFF_FFFF_00FF_FFFF;

#[inline]
fn uf_find(parent: &mut [u16], mut x: u16) -> u16 {
    let mut root = x;
    while parent[usize::from(root)] != root {
        root = parent[usize::from(root)];
    }
    while parent[usize::from(x)] != root {
        let next = parent[usize::from(x)];
        parent[usize::from(x)] = root;
        x = next;
    }
    root
}

#[inline]
fn uf_union(parent: &mut [u16], a: u16, b: u16) {
    let ra = uf_find(parent, a);
    let rb = uf_find(parent, b);
    match ra.cmp(&rb) {
        std::cmp::Ordering::Less => parent[usize::from(rb)] = ra,
        std::cmp::Ordering::Greater => parent[usize::from(ra)] = rb,
        std::cmp::Ordering::Equal => {}
    }
}

/// Unions every cell touched by `quad` along `(u_axis, v_axis)` into `uf`, returning `false` if `quad`
/// has non-finite coordinates along either plane axis (conservatively overlapping everything).
#[inline]
fn union_plane_quad(
    uf: &mut [u16; 257],
    quad: &[TerrainVertex; 4],
    u_axis: usize,
    v_axis: usize,
) -> bool {
    match (axis_span(quad, u_axis), axis_span(quad, v_axis)) {
        (AxisSpan::Unbounded, _) | (_, AxisSpan::Unbounded) => return false,
        (AxisSpan::OutsideOnly, _) | (_, AxisSpan::OutsideOnly) => {}
        (
            AxisSpan::Cells {
                lo: u0,
                hi: u1,
                touches_outside: u_out,
            },
            AxisSpan::Cells {
                lo: v0,
                hi: v1,
                touches_outside: v_out,
            },
        ) => {
            let touches_outside = u_out || v_out;
            if u0 != u1 || v0 != v1 || touches_outside {
                let rep = if touches_outside {
                    OUTSIDE_2D
                } else {
                    u16::from(morton2d(u0, v0))
                };
                for v in v0..=v1 {
                    for u in u0..=u1 {
                        uf_union(uf, rep, u16::from(morton2d(u, v)));
                    }
                }
            }
        }
    }
    true
}

#[inline]
fn plane_quad_node(quad: &[TerrainVertex; 4], u_axis: usize, v_axis: usize) -> u16 {
    match (axis_span(quad, u_axis), axis_span(quad, v_axis)) {
        (AxisSpan::OutsideOnly, _) | (_, AxisSpan::OutsideOnly) => OUTSIDE_2D,
        (AxisSpan::Cells { lo: u0, .. }, AxisSpan::Cells { lo: v0, .. }) => {
            u16::from(morton2d(u0, v0))
        }
        (AxisSpan::Unbounded, _) | (_, AxisSpan::Unbounded) => unreachable!(),
    }
}

/// Unions every 3D cell touched by `quad` into `uf`, returning `false` if `quad` has non-finite
/// coordinates along any axis (conservatively overlapping everything).
#[inline]
fn union_other_quad(uf: &mut [u16], quad: &[TerrainVertex; 4]) -> bool {
    match (axis_span(quad, 0), axis_span(quad, 1), axis_span(quad, 2)) {
        (AxisSpan::Unbounded, _, _) | (_, AxisSpan::Unbounded, _) | (_, _, AxisSpan::Unbounded) => {
            return false;
        }
        (AxisSpan::OutsideOnly, _, _)
        | (_, AxisSpan::OutsideOnly, _)
        | (_, _, AxisSpan::OutsideOnly) => {}
        (
            AxisSpan::Cells {
                lo: x0,
                hi: x1,
                touches_outside: x_out,
            },
            AxisSpan::Cells {
                lo: y0,
                hi: y1,
                touches_outside: y_out,
            },
            AxisSpan::Cells {
                lo: z0,
                hi: z1,
                touches_outside: z_out,
            },
        ) => {
            let touches_outside = x_out || y_out || z_out;
            if x0 != x1 || y0 != y1 || z0 != z1 || touches_outside {
                let rep = if touches_outside {
                    OUTSIDE_3D
                } else {
                    morton3d(x0, y0, z0)
                };
                for z in z0..=z1 {
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            uf_union(uf, rep, morton3d(x, y, z));
                        }
                    }
                }
            }
        }
    }
    true
}

#[inline]
fn other_quad_node(quad: &[TerrainVertex; 4]) -> u16 {
    match (axis_span(quad, 0), axis_span(quad, 1), axis_span(quad, 2)) {
        (AxisSpan::OutsideOnly, _, _)
        | (_, AxisSpan::OutsideOnly, _)
        | (_, _, AxisSpan::OutsideOnly) => OUTSIDE_3D,
        (
            AxisSpan::Cells { lo: x0, .. },
            AxisSpan::Cells { lo: y0, .. },
            AxisSpan::Cells { lo: z0, .. },
        ) => morton3d(x0, y0, z0),
        _ => unreachable!(),
    }
}

/// Sorts a single axis-aligned plane slice (currently in emission order with quad indices in the
/// low 24 bits) by connected-component 2D Morton key, preserving exact emission order among any
/// coplanar quads whose 2D footprints can overlap across cell boundaries while keeping 2D Morton
/// clustering for independent single-cell quads on the same plane.
fn sort_plane_with_crossers(
    quads: &[[TerrainVertex; 4]],
    axis: usize,
    slice: &mut [u32],
    crosser_keys: &mut Vec<u64>,
) {
    let (u_axis, v_axis) = plane_axes(axis);
    let mut uf: [u16; 257] = std::array::from_fn(|k| k as u16);
    let mut unbounded = false;

    for &entry in slice.iter() {
        let quad = &quads[(entry & COMPACT_IDX_MASK) as usize];
        if !union_plane_quad(&mut uf, quad, u_axis, v_axis) {
            unbounded = true;
            break;
        }
    }
    if unbounded {
        for entry in slice.iter_mut() {
            *entry &= COMPACT_IDX_MASK;
        }
        return;
    }

    crosser_keys.clear();
    crosser_keys.reserve(slice.len());
    for &entry in slice.iter() {
        let idx = entry & COMPACT_IDX_MASK;
        let node = plane_quad_node(&quads[idx as usize], u_axis, v_axis);
        let key = uf_find(&mut uf, node);
        crosser_keys.push((u64::from(key) << 24) | u64::from(idx));
    }
    crosser_keys.sort_unstable();
    for (dst, &k) in slice.iter_mut().zip(crosser_keys.iter()) {
        *dst = (k & COMPACT_IDX_MASK_U64) as u32;
    }
}

/// Sorts an `OTHER` part (whose `part_keys` are currently in emission order with quad indices in
/// the low 24 bits) by connected-component 3D Morton key, preserving exact emission order among
/// quads whose 3D footprints can overlap across cell boundaries while keeping 3D Morton clustering
/// for independent single-cell quads.
fn sort_other_with_crossers(
    quads: &[[TerrainVertex; 4]],
    part_keys: &mut [u64],
    part_order: &mut [u32],
    uf: &mut Vec<u16>,
) {
    uf.clear();
    uf.extend(0..=OUTSIDE_3D);

    for &k in part_keys.iter() {
        let quad = &quads[(k & COMPACT_IDX_MASK_U64) as usize];
        if !union_other_quad(uf, quad) {
            for (dst, &entry) in part_order.iter_mut().zip(part_keys.iter()) {
                *dst = (entry & COMPACT_IDX_MASK_U64) as u32;
            }
            return;
        }
    }

    for k in part_keys.iter_mut() {
        let idx = *k & COMPACT_IDX_MASK_U64;
        let node = other_quad_node(&quads[idx as usize]);
        let key = uf_find(uf, node);
        *k = (u64::from(key) << 24) | idx;
    }
    part_keys.sort_unstable();
    for (dst, &k) in part_order.iter_mut().zip(part_keys.iter()) {
        *dst = (k & COMPACT_IDX_MASK_U64) as u32;
    }
}

/// Sorts one coplanar run of full-width `u32` quad indices (currently in emission order) by
/// connected-component 2D Morton key with original index tie-breaking.
fn sort_plane_full_width(
    quads: &[[TerrainVertex; 4]],
    axis: usize,
    run: &mut [u32],
    pairs: &mut Vec<(u16, u32)>,
) {
    let (u_axis, v_axis) = plane_axes(axis);
    let mut uf: [u16; 257] = std::array::from_fn(|k| k as u16);
    for &idx in run.iter() {
        if !union_plane_quad(&mut uf, &quads[idx as usize], u_axis, v_axis) {
            return;
        }
    }
    pairs.clear();
    pairs.reserve(run.len());
    for &idx in run.iter() {
        let node = plane_quad_node(&quads[idx as usize], u_axis, v_axis);
        pairs.push((uf_find(&mut uf, node), idx));
    }
    pairs.sort_unstable();
    for (dst, &(_, idx)) in run.iter_mut().zip(pairs.iter()) {
        *dst = idx;
    }
}

/// Sorts an `OTHER` part of full-width `u32` quad indices (currently in emission order) by
/// connected-component 3D Morton key with original index tie-breaking.
fn sort_other_full_width(
    quads: &[[TerrainVertex; 4]],
    part: &mut [u32],
    uf: &mut Vec<u16>,
    pairs: &mut Vec<(u16, u32)>,
) {
    uf.clear();
    uf.extend(0..=OUTSIDE_3D);
    for &idx in part.iter() {
        if !union_other_quad(uf, &quads[idx as usize]) {
            return;
        }
    }
    pairs.clear();
    pairs.reserve(part.len());
    for &idx in part.iter() {
        let node = other_quad_node(&quads[idx as usize]);
        pairs.push((uf_find(uf, node), idx));
    }
    pairs.sort_unstable();
    for (dst, &(_, idx)) in part.iter_mut().zip(pairs.iter()) {
        *dst = idx;
    }
}

/// Full-width (`u32`, matching `Sides::ends`) fallback used when `quads.len() > compact_limit`.
/// Produces the exact same outside-in plane + connected-component Morton + emission-order
/// permutation as the 24-bit compact fast path without packing or masking quad indices.
#[cold]
fn sort_order_full_width(
    quads: &[[TerrainVertex; 4]],
    slots: &[u8],
    slot_ends: &[u32; 14],
    grouped: bool,
    order: &mut [u32],
    uf3d: &mut Vec<u16>,
) {
    let mut slot_starts = [0usize; 14];
    for (start, &prev_end) in slot_starts[1..].iter_mut().zip(&slot_ends[..13]) {
        *start = prev_end as usize;
    }
    let mut write = slot_starts;
    for (i, &part) in slots.iter().enumerate() {
        let p = usize::from(part);
        order[write[p]] = i as u32;
        write[p] += 1;
    }

    let mut pairs = Vec::new();
    for (p, (&start, &end_u32)) in slot_starts.iter().zip(slot_ends.iter()).enumerate() {
        let end = end_u32 as usize;
        if end - start <= 1 {
            continue;
        }
        let part = &mut order[start..end];
        match BUCKETS[slot(grouped, p).0] {
            Bucket::Side(axis, positive) => {
                part.sort_unstable_by_key(|&idx| {
                    let plane = total_order_key(canonical_plane(quads[idx as usize][0].pos[axis]));
                    (if positive { !plane } else { plane }, idx)
                });
                let mut run_start = 0;
                while run_start < part.len() {
                    let plane_bits =
                        canonical_plane(quads[part[run_start] as usize][0].pos[axis]).to_bits();
                    let mut run_end = run_start + 1;
                    while run_end < part.len()
                        && canonical_plane(quads[part[run_end] as usize][0].pos[axis]).to_bits()
                            == plane_bits
                    {
                        run_end += 1;
                    }
                    if run_end - run_start > 1 {
                        sort_plane_full_width(
                            quads,
                            axis,
                            &mut part[run_start..run_end],
                            &mut pairs,
                        );
                    }
                    run_start = run_end;
                }
            }
            Bucket::Other => {
                sort_other_full_width(quads, part, uf3d, &mut pairs);
            }
        }
    }
}

fn sort_with(
    solid: &mut Vec<TerrainVertex>,
    tints: &mut Vec<u32>,
    sources: &mut Vec<TerrainSource>,
    looks: &mut Vec<QuadLook>,
    grouped: bool,
    scratch: &mut SortScratch,
) -> Sides {
    sort_with_compact_limit(
        solid,
        tints,
        sources,
        looks,
        grouped,
        scratch,
        MAX_COMPACT_QUADS,
    )
}

fn sort_with_compact_limit(
    solid: &mut Vec<TerrainVertex>,
    tints: &mut Vec<u32>,
    sources: &mut Vec<TerrainSource>,
    looks: &mut Vec<QuadLook>,
    grouped: bool,
    scratch: &mut SortScratch,
    compact_limit: usize,
) -> Sides {
    let SortScratch {
        slots,
        order,
        keys,
        crosser_keys,
        uf3d,
        sorted,
        tints: sorted_tints,
        sources: sorted_sources,
        looks: sorted_looks,
    } = scratch;
    let (quads, rest) = solid.as_chunks::<4>();
    debug_assert!(rest.is_empty(), "solid vertices come in quads");
    let quads_len = quads.len();
    let use_compact = quads_len <= compact_limit;
    let cutout = |q: &[TerrainVertex; 4]| q[0].layer & OPAQUE_LAYER == 0;
    // Each quad's part, by its place in mesh order (`slot`).
    let position = |b: usize, cutout: bool| match grouped {
        true => b + 7 * usize::from(cutout),
        false => b * 2 + usize::from(cutout),
    };
    slots.clear();
    slots.reserve(quads_len);
    let mut counts = [0u32; 7];
    let mut slot_counts = [0u32; 14];
    let mut planes = [0f32; 7];
    let mut int_planes = [use_compact; 14];
    let mut bin_counts = [[0u32; 17]; 14];
    for quad in quads {
        let b = bucket(quad);
        let is_cutout = cutout(quad);
        let part = position(b, is_cutout);
        slots.push(part as u8);
        slot_counts[part] += 1;
        if let Bucket::Side(axis, positive) = BUCKETS[b] {
            let p = canonical_plane(quad[0].pos[axis]);
            planes[b] = match (counts[b], positive) {
                (0, _) => p,
                (_, true) => planes[b].min(p),
                (_, false) => planes[b].max(p),
            };
            if int_planes[part] {
                match integer_plane(p) {
                    Some(plane_idx) => {
                        let bin = if positive { 16 - plane_idx } else { plane_idx };
                        bin_counts[part][bin] += 1;
                    }
                    None => int_planes[part] = false,
                }
            }
        }
        counts[b] += 1;
    }
    let mut slot_starts = [0u32; 14];
    let mut slot_ends = [0u32; 14];
    let mut total = 0u32;
    for ((start, end), count) in slot_starts.iter_mut().zip(&mut slot_ends).zip(slot_counts) {
        *start = total;
        total += count;
        *end = total;
    }
    if quads_len <= 1 {
        return Sides {
            ends: slot_ends,
            grouped,
            split: true,
            planes,
        };
    }

    // For meshes within the 24-bit index limit (`quads_len <= MAX_COMPACT_QUADS`), axis-facing
    // parts with exact integer planes `0..=16` scatter directly into 17 outside-in bins in O(N)
    // and sort only tiny per-plane slices by `(morton2d, original_index)`, while fractional/
    // out-of-range axis parts and multi-quad `OTHER` parts sort via compact `u64` keys. Whenever
    // any quad on a plane (or in `OTHER`) crosses a cell boundary or extends outside `[0, 16]`,
    // that plane/part falls back to connected-component Morton keys so overlapping coplanar quads
    // keep exact emission order while independent cells stay Morton-clustered. Larger meshes bypass
    // 24-bit packing and run the full-width fallback with the exact same ordering.
    order.resize(quads_len, 0);
    if use_compact {
        debug_assert!(compact_limit <= MAX_COMPACT_QUADS);
        let mut use_int_bins = [false; 14];
        let mut bin_starts = [[0u32; 17]; 14];
        let mut key_starts = [0u32; 14];
        let mut key_ends = [0u32; 14];
        let mut key_total = 0u32;
        for p in 0..14 {
            let is_int_axis = slot(grouped, p).0 != OTHER && int_planes[p];
            use_int_bins[p] = is_int_axis;
            if is_int_axis {
                let mut off = slot_starts[p];
                for bin in 0..17 {
                    bin_starts[p][bin] = off;
                    off += bin_counts[p][bin];
                }
            } else if slot_counts[p] > 1 {
                key_starts[p] = key_total;
                key_total += slot_counts[p];
                key_ends[p] = key_total;
            }
        }

        if key_total > 0 {
            keys.resize(key_total as usize, 0);
        }
        let mut bin_write = bin_starts;
        let mut bin_has_crosser = [0u32; 14];
        let mut part_has_crosser = [false; 14];
        let mut key_write = key_starts;
        let mut direct_write = slot_starts;
        for (i, (quad, &part)) in quads.iter().zip(slots.iter()).enumerate() {
            let p = usize::from(part);
            let b = slot(grouped, p).0;
            if use_int_bins[p] {
                let Bucket::Side(axis, positive) = BUCKETS[b] else {
                    unreachable!();
                };
                let plane_idx = canonical_plane(quad[0].pos[axis]) as usize;
                let bin = if positive { 16 - plane_idx } else { plane_idx };
                let m2 = match single_cell_morton2d(quad, axis) {
                    Some(m) => m,
                    None => {
                        bin_has_crosser[p] |= 1u32 << bin;
                        0
                    }
                };
                let dst = bin_write[p][bin] as usize;
                order[dst] = (u32::from(m2) << 24) | (i as u32);
                bin_write[p][bin] += 1;
            } else if slot_counts[p] <= 1 {
                order[direct_write[p] as usize] = i as u32;
                direct_write[p] += 1;
            } else {
                let key = match BUCKETS[b] {
                    Bucket::Side(axis, positive) => {
                        let plane = total_order_key(canonical_plane(quad[0].pos[axis]));
                        let plane = if positive { !plane } else { plane };
                        let m2 = match single_cell_morton2d(quad, axis) {
                            Some(m) => m,
                            None => {
                                part_has_crosser[p] = true;
                                0
                            }
                        };
                        (u64::from(plane) << 32) | (u64::from(m2) << 24) | (i as u64)
                    }
                    Bucket::Other => {
                        let m3 = match single_cell_morton3d(quad) {
                            Some(m) => m,
                            None => {
                                part_has_crosser[p] = true;
                                0
                            }
                        };
                        (u64::from(m3) << 24) | (i as u64)
                    }
                };
                let dst = key_write[p] as usize;
                keys[dst] = key;
                key_write[p] += 1;
            }
        }

        for p in 0..14 {
            if use_int_bins[p] {
                let Bucket::Side(axis, _) = BUCKETS[slot(grouped, p).0] else {
                    unreachable!();
                };
                for bin in 0..17 {
                    let start = bin_starts[p][bin] as usize;
                    let end = bin_write[p][bin] as usize;
                    if end - start > 1 {
                        if bin_has_crosser[p] & (1u32 << bin) != 0 {
                            sort_plane_with_crossers(
                                quads,
                                axis,
                                &mut order[start..end],
                                crosser_keys,
                            );
                        } else {
                            order[start..end].sort_unstable();
                            for entry in &mut order[start..end] {
                                *entry &= COMPACT_IDX_MASK;
                            }
                        }
                    } else if end > start {
                        order[start] &= COMPACT_IDX_MASK;
                    }
                }
            } else if slot_counts[p] > 1 {
                let k_start = key_starts[p] as usize;
                let k_end = key_ends[p] as usize;
                let o_start = slot_starts[p] as usize;
                let o_end = slot_ends[p] as usize;
                if !part_has_crosser[p] {
                    keys[k_start..k_end].sort_unstable();
                    for (dst, &k) in order[o_start..o_end].iter_mut().zip(&keys[k_start..k_end]) {
                        *dst = (k & COMPACT_IDX_MASK_U64) as u32;
                    }
                } else {
                    match BUCKETS[slot(grouped, p).0] {
                        Bucket::Side(axis, _) => {
                            for k in &mut keys[k_start..k_end] {
                                *k &= COMPACT_PLANE_AND_IDX_MASK;
                            }
                            keys[k_start..k_end].sort_unstable();
                            let part_keys = &keys[k_start..k_end];
                            let part_order = &mut order[o_start..o_end];
                            for (dst, &k) in part_order.iter_mut().zip(part_keys.iter()) {
                                *dst = (k & COMPACT_IDX_MASK_U64) as u32;
                            }
                            let mut run_start = 0;
                            while run_start < part_keys.len() {
                                let plane = part_keys[run_start] >> 32;
                                let mut run_end = run_start + 1;
                                while run_end < part_keys.len()
                                    && (part_keys[run_end] >> 32) == plane
                                {
                                    run_end += 1;
                                }
                                if run_end - run_start > 1 {
                                    let run = &mut part_order[run_start..run_end];
                                    let mut has_crosser = false;
                                    for entry in run.iter_mut() {
                                        let idx = *entry;
                                        if let Some(m2) =
                                            single_cell_morton2d(&quads[idx as usize], axis)
                                        {
                                            *entry = (u32::from(m2) << 24) | idx;
                                        } else {
                                            has_crosser = true;
                                        }
                                    }
                                    if has_crosser {
                                        sort_plane_with_crossers(quads, axis, run, crosser_keys);
                                    } else {
                                        run.sort_unstable();
                                        for entry in run.iter_mut() {
                                            *entry &= COMPACT_IDX_MASK;
                                        }
                                    }
                                }
                                run_start = run_end;
                            }
                        }
                        Bucket::Other => {
                            sort_other_with_crossers(
                                quads,
                                &mut keys[k_start..k_end],
                                &mut order[o_start..o_end],
                                uf3d,
                            );
                        }
                    }
                }
            }
        }
    } else {
        sort_order_full_width(quads, slots, &slot_ends, grouped, order, uf3d);
    }

    let already_sorted = order
        .iter()
        .enumerate()
        .all(|(pos, &idx)| pos == idx as usize);
    if !already_sorted {
        sorted.clear();
        sorted.extend(order.iter().map(|&i| quads[i as usize]));
        solid.clear();
        solid.extend_from_slice(sorted.as_flattened());
        if tints.len() == quads_len {
            sorted_tints.clear();
            sorted_tints.extend(order.iter().map(|&i| tints[i as usize]));
            tints.clear();
            tints.extend_from_slice(sorted_tints);
        }
        if sources.len() == quads_len {
            sorted_sources.clear();
            sorted_sources.extend(order.iter().map(|&i| sources[i as usize]));
            sources.clear();
            sources.extend_from_slice(sorted_sources);
        }
        if looks.len() == quads_len {
            sorted_looks.clear();
            sorted_looks.extend(order.iter().map(|&i| looks[i as usize]));
            looks.clear();
            looks.extend_from_slice(sorted_looks);
        }
    }
    Sides {
        ends: slot_ends,
        grouped,
        split: true,
        planes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference 2D connected-component Morton keys on one axis-aligned plane run.
    fn reference_plane_keys(run: &[[TerrainVertex; 4]], axis: usize) -> Vec<u16> {
        let (u_axis, v_axis) = plane_axes(axis);
        let mut uf: [u16; 257] = std::array::from_fn(|k| k as u16);
        for q in run {
            match (axis_span(q, u_axis), axis_span(q, v_axis)) {
                (AxisSpan::Unbounded, _) | (_, AxisSpan::Unbounded) => {
                    return vec![0; run.len()];
                }
                (AxisSpan::OutsideOnly, _) | (_, AxisSpan::OutsideOnly) => {}
                (
                    AxisSpan::Cells {
                        lo: u0,
                        hi: u1,
                        touches_outside: u_out,
                    },
                    AxisSpan::Cells {
                        lo: v0,
                        hi: v1,
                        touches_outside: v_out,
                    },
                ) => {
                    let rep = if u_out || v_out {
                        OUTSIDE_2D
                    } else {
                        u16::from(morton2d(u0, v0))
                    };
                    for v in v0..=v1 {
                        for u in u0..=u1 {
                            uf_union(&mut uf, rep, u16::from(morton2d(u, v)));
                        }
                    }
                }
            }
        }
        run.iter()
            .map(|q| {
                let node = match (axis_span(q, u_axis), axis_span(q, v_axis)) {
                    (AxisSpan::OutsideOnly, _) | (_, AxisSpan::OutsideOnly) => OUTSIDE_2D,
                    (AxisSpan::Cells { lo: u0, .. }, AxisSpan::Cells { lo: v0, .. }) => {
                        u16::from(morton2d(u0, v0))
                    }
                    _ => unreachable!(),
                };
                uf_find(&mut uf, node)
            })
            .collect()
    }

    /// Reference 3D connected-component Morton keys on an `OTHER` part.
    fn reference_other_keys(part: &[[TerrainVertex; 4]]) -> Vec<u16> {
        let mut uf: Vec<u16> = (0..=OUTSIDE_3D).collect();
        for q in part {
            match (axis_span(q, 0), axis_span(q, 1), axis_span(q, 2)) {
                (AxisSpan::Unbounded, _, _)
                | (_, AxisSpan::Unbounded, _)
                | (_, _, AxisSpan::Unbounded) => {
                    return vec![0; part.len()];
                }
                (AxisSpan::OutsideOnly, _, _)
                | (_, AxisSpan::OutsideOnly, _)
                | (_, _, AxisSpan::OutsideOnly) => {}
                (
                    AxisSpan::Cells {
                        lo: x0,
                        hi: x1,
                        touches_outside: x_out,
                    },
                    AxisSpan::Cells {
                        lo: y0,
                        hi: y1,
                        touches_outside: y_out,
                    },
                    AxisSpan::Cells {
                        lo: z0,
                        hi: z1,
                        touches_outside: z_out,
                    },
                ) => {
                    let rep = if x_out || y_out || z_out {
                        OUTSIDE_3D
                    } else {
                        morton3d(x0, y0, z0)
                    };
                    for z in z0..=z1 {
                        for y in y0..=y1 {
                            for x in x0..=x1 {
                                uf_union(&mut uf, rep, morton3d(x, y, z));
                            }
                        }
                    }
                }
            }
        }
        part.iter()
            .map(|q| {
                let node = match (axis_span(q, 0), axis_span(q, 1), axis_span(q, 2)) {
                    (AxisSpan::OutsideOnly, _, _)
                    | (_, AxisSpan::OutsideOnly, _)
                    | (_, _, AxisSpan::OutsideOnly) => OUTSIDE_3D,
                    (
                        AxisSpan::Cells { lo: x0, .. },
                        AxisSpan::Cells { lo: y0, .. },
                        AxisSpan::Cells { lo: z0, .. },
                    ) => morton3d(x0, y0, z0),
                    _ => unreachable!(),
                };
                uf_find(&mut uf, node)
            })
            .collect()
    }

    /// Simple stable reference for the spatial front-to-back sort: quads partitioned by
    /// part, axis-facing buckets sorted outside-in by canonical plane and connected-component
    /// 2D Morton key, and `OTHER` sorted by connected-component 3D Morton key.
    fn reference(solid: &[TerrainVertex], grouped: bool) -> Vec<TerrainVertex> {
        let quads = solid.as_chunks::<4>().0;
        let mut sorted = Vec::new();
        for n in 0..14 {
            let (b, cutout) = slot(grouped, n);
            let mut part: Vec<[TerrainVertex; 4]> = quads
                .iter()
                .filter(|q| bucket(q) == b && (q[0].layer & OPAQUE_LAYER == 0) == cutout)
                .copied()
                .collect();
            match BUCKETS[b] {
                Bucket::Side(axis, positive) => {
                    part.sort_by(|a, b| {
                        let pa = canonical_plane(a[0].pos[axis]);
                        let pb = canonical_plane(b[0].pos[axis]);
                        if positive {
                            pb.total_cmp(&pa)
                        } else {
                            pa.total_cmp(&pb)
                        }
                    });
                    let mut start = 0;
                    while start < part.len() {
                        let plane_bits = canonical_plane(part[start][0].pos[axis]).to_bits();
                        let mut end = start + 1;
                        while end < part.len()
                            && canonical_plane(part[end][0].pos[axis]).to_bits() == plane_bits
                        {
                            end += 1;
                        }
                        let run = &mut part[start..end];
                        if run.len() > 1 {
                            let keys = reference_plane_keys(run, axis);
                            let mut indices: Vec<usize> = (0..run.len()).collect();
                            indices.sort_by_key(|&idx| keys[idx]);
                            let reordered: Vec<_> =
                                indices.into_iter().map(|idx| run[idx]).collect();
                            run.copy_from_slice(&reordered);
                        }
                        start = end;
                    }
                }
                Bucket::Other => {
                    if part.len() > 1 {
                        let keys = reference_other_keys(&part);
                        let mut indices: Vec<usize> = (0..part.len()).collect();
                        indices.sort_by_key(|&idx| keys[idx]);
                        part = indices.into_iter().map(|idx| part[idx]).collect();
                    }
                }
            }
            sorted.extend(part);
        }
        sorted.into_flattened()
    }

    #[test]
    fn integer_keys_sort_as_the_stable_sort_did() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..120u32 {
            let grouped = !round.is_multiple_of(2);
            let int_only = round.is_multiple_of(4);
            let mut solid = Vec::new();
            for q_idx in 0..(next() % 300) {
                let origin = if int_only {
                    [0, 1, 2].map(|_| (next() % 16) as f32)
                } else {
                    let o = [0, 1, 2].map(|_| (next() % 34) as f32 / 2.0 - 0.5);
                    // Some negative and signed-zero planes too.
                    if next().is_multiple_of(9) {
                        o.map(|v| -v)
                    } else {
                        o
                    }
                };
                let opaque = if round.is_multiple_of(3) {
                    OPAQUE_LAYER
                } else {
                    (next() % 2) as u16 * OPAQUE_LAYER
                };
                // Tag each quad's low layer bits so tie preservation is verified byte-for-byte.
                let layer = opaque | ((q_idx as u16) & 0x7FFF);
                if next().is_multiple_of(10) {
                    let [x, y, z] = origin;
                    let slope = [
                        [x, y, z],
                        [x + 1.0, y, z],
                        [x + 1.0, y + 1.0, z + 1.0],
                        [x, y + 1.0, z + 1.0],
                    ];
                    solid.extend(quad(slope, layer));
                } else {
                    let faces = cube(origin, layer);
                    let face = (next() % 6) as usize;
                    solid.extend_from_slice(&faces[face * 4..face * 4 + 4]);
                }
            }
            let expected = reference(&solid, grouped);
            let sides = sort_parts(&mut solid, grouped);
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&solid),
                bytemuck::cast_slice::<_, u8>(&expected),
                "round {round}"
            );
            assert!(sides.is_split());
            assert_eq!(sides.quads() as usize * 4, solid.len());

            // Geometric invariant: within every axis-facing part, no two coplanar quads whose
            // 2D open footprints overlap are ever inverted relative to emission order.
            let sorted_quads = solid.as_chunks::<4>().0;
            let mut prev_end = 0usize;
            for n in 0..14 {
                let end = sides.part_end(n) as usize;
                let (b, _) = slot(grouped, n);
                if let Bucket::Side(axis, _) = BUCKETS[b] {
                    let (u_axis, v_axis) = plane_axes(axis);
                    let part = &sorted_quads[prev_end..end];
                    let mut run_start = 0;
                    while run_start < part.len() {
                        let plane_bits = canonical_plane(part[run_start][0].pos[axis]).to_bits();
                        let mut run_end = run_start + 1;
                        while run_end < part.len()
                            && canonical_plane(part[run_end][0].pos[axis]).to_bits() == plane_bits
                        {
                            run_end += 1;
                        }
                        let run = &part[run_start..run_end];
                        for (i, q_a) in run.iter().enumerate() {
                            let (u0a, u1a) = axis_bounds(q_a, u_axis).unwrap();
                            let (v0a, v1a) = axis_bounds(q_a, v_axis).unwrap();
                            let id_a = q_a[0].layer & 0x7FFF;
                            for q_b in &run[(i + 1)..] {
                                let (u0b, u1b) = axis_bounds(q_b, u_axis).unwrap();
                                let (v0b, v1b) = axis_bounds(q_b, v_axis).unwrap();
                                if u0a < u1b && u0b < u1a && v0a < v1b && v0b < v1a {
                                    let id_b = q_b[0].layer & 0x7FFF;
                                    assert!(
                                        id_a < id_b,
                                        "round {round}: overlapping coplanar quads inverted ({id_a} before {id_b})"
                                    );
                                }
                            }
                        }
                        run_start = run_end;
                    }
                }
                prev_end = end;
            }
        }
    }

    fn quad(corners: [[f32; 3]; 4], layer: u16) -> [TerrainVertex; 4] {
        corners.map(|p| TerrainVertex::new(p, [0.0; 2], layer, 0, 0))
    }

    /// A unit block's six faces at `origin`, wound counter-clockwise seen
    /// from outside, as block models are.
    fn cube(origin: [f32; 3], layer: u16) -> Vec<TerrainVertex> {
        let [x, y, z] = origin;
        let (a, b, c) = (x + 1.0, y + 1.0, z + 1.0);
        [
            [[a, y, z], [a, b, z], [a, b, c], [a, y, c]], // +X
            [[x, y, z], [x, y, c], [x, b, c], [x, b, z]], // -X
            [[x, b, z], [x, b, c], [a, b, c], [a, b, z]], // +Y
            [[x, y, z], [a, y, z], [a, y, c], [x, y, c]], // -Y
            [[x, y, c], [a, y, c], [a, b, c], [x, b, c]], // +Z
            [[x, y, z], [x, b, z], [a, b, z], [a, y, z]], // -Z
        ]
        .into_iter()
        .flat_map(|q| quad(q, layer))
        .collect()
    }

    /// Where each bucket ends, in a mesh sorted for one pass.
    fn bucket_ends(sides: &Sides) -> [u32; 7] {
        assert!(!sides.grouped);
        std::array::from_fn(|b| sides.ends[b * 2 + 1])
    }

    fn runs(sides: &Sides, eye: [f32; 3]) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        sides.visible(eye, |first, count| out.push((first, count)));
        out
    }

    /// The quads of `solid` whose triangles face `eye`, by their layer.
    fn front_facing(solid: &[TerrainVertex], eye: [f32; 3]) -> Vec<u16> {
        let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        solid
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|q| {
                [[0, 1, 2], [0, 2, 3]].iter().any(|t| {
                    let (e1, e2) = (sub(q[t[1]].pos, q[t[0]].pos), sub(q[t[2]].pos, q[t[0]].pos));
                    let n = [
                        e1[1] * e2[2] - e1[2] * e2[1],
                        e1[2] * e2[0] - e1[0] * e2[2],
                        e1[0] * e2[1] - e1[1] * e2[0],
                    ];
                    let to_eye = sub(eye, q[t[0]].pos);
                    n[0] * to_eye[0] + n[1] * to_eye[1] + n[2] * to_eye[2] > 0.0
                })
            })
            .map(|q| q[0].layer)
            .collect()
    }

    #[test]
    fn cube_faces_go_to_their_buckets_in_order() {
        let mut solid = cube([3.0, 4.0, 5.0], 0);
        for (i, v) in solid.iter_mut().enumerate() {
            v.layer = (i / 4) as u16;
        }
        let sides = sort(&mut solid);
        assert_eq!(bucket_ends(&sides), [1, 2, 2, 3, 4, 5, 6]);
        let layers: Vec<u16> = solid.iter().step_by(4).map(|v| v.layer).collect();
        // +X, +Z, +Y, -Z, -X, -Y.
        assert_eq!(layers, [0, 4, 2, 5, 1, 3]);
        assert_eq!(sides.planes[0], 4.0); // +X at x = 4
        assert_eq!(sides.planes[5], 3.0); // -X at x = 3
        assert_eq!(sides.planes[3], 5.0); // +Y at y = 5
    }

    #[test]
    fn the_drawn_runs_hold_every_quad_that_faces_the_eye() {
        // Blocks across a section, with a sloped quad and a bowtie.
        let mut solid = Vec::new();
        let mut layer = 0;
        for origin in [
            [0.0, 0.0, 0.0],
            [7.0, 3.0, 12.0],
            [15.0, 15.0, 15.0],
            [2.0, 9.0, 4.0],
        ] {
            for q in cube(origin, 0).chunks(4) {
                solid.extend(q.iter().map(|v| TerrainVertex { layer, ..*v }));
                layer += 1;
            }
        }
        let slope = [
            [1.0, 1.0, 1.0],
            [2.0, 1.0, 1.0],
            [2.0, 2.0, 2.0],
            [1.0, 2.0, 2.0],
        ];
        let bowtie = [
            [5.0, 1.0, 1.0],
            [5.0, 2.0, 2.0],
            [5.0, 1.0, 2.0],
            [5.0, 2.0, 1.0],
        ];
        for corners in [slope, bowtie] {
            solid.extend(quad(corners, layer));
            layer += 1;
        }
        let sides = sort(&mut solid);
        assert_eq!(sides.quads(), u32::from(layer));
        let eyes = [-20.0f32, -0.5, 0.0, 3.5, 8.0, 15.99, 16.0, 30.0];
        for &x in &eyes {
            for &y in &eyes {
                for &z in &eyes {
                    let eye = [x, y, z];
                    let mut drawn = Vec::new();
                    for (first, count) in runs(&sides, eye) {
                        drawn.extend(
                            solid[first as usize * 4..(first + count) as usize * 4]
                                .iter()
                                .step_by(4)
                                .map(|v| v.layer),
                        );
                    }
                    for layer in front_facing(&solid, eye) {
                        assert!(drawn.contains(&layer), "{eye:?} misses quad {layer}");
                    }
                }
            }
        }
    }

    #[test]
    fn runs_merge_across_hidden_and_empty_buckets() {
        let mut solid = cube([0.0; 3], 0);
        let sides = sort(&mut solid);
        // From above, +X and +Z: +X, +Z, (no other), +Y is one run.
        assert_eq!(runs(&sides, [8.0, 8.0, 8.0]), [(0, 3)]);
        // From below, -X and -Z: -Z, -X, -Y.
        assert_eq!(runs(&sides, [-8.0, -8.0, -8.0]), [(3, 3)]);
        // From above, -X and +Z: +Z, +Y, then -X.
        assert_eq!(runs(&sides, [-8.0, 8.0, 8.0]), [(1, 2), (4, 1)]);
        // Inside the block every face is seen from behind.
        assert!(runs(&sides, [0.5; 3]).is_empty());
        // Level with its top, the eye may see it: the margin keeps it.
        assert_eq!(runs(&sides, [0.5, 1.0, 0.5]), [(2, 1)]);
    }

    #[test]
    fn an_eye_and_its_mirror_below_see_the_faces_of_both() {
        let mut solid = cube([0.0; 3], 0);
        let sides = sort(&mut solid);
        let both = |eye: [f32; 3], low: f32| {
            let mut out = Vec::new();
            sides.visible_from(eye, low, Part::All, |first, count| out.push((first, count)));
            out
        };
        // Above the block: +X, +Z and +Y; with the mirror's eye below it, -Y too.
        assert_eq!(runs(&sides, [8.0, 8.0, 8.0]), [(0, 3)]);
        assert_eq!(both([8.0, 8.0, 8.0], -8.0), [(0, 3), (5, 1)]);
        // A mirror above the block's bottom adds nothing.
        assert_eq!(both([8.0, 8.0, 8.0], 0.5), [(0, 3)]);
        // The sides depend on x and z alone.
        assert_eq!(both([-8.0, 8.0, -8.0], -8.0), [(2, 4)]);
    }

    /// Opaque and cutout quads of each bucket split: each pass's runs hold
    /// exactly its own quads facing the eye, and all of them together the
    /// same runs as one pass.
    #[test]
    fn each_part_draws_its_own_quads_facing_the_eye() {
        // Two blocks: one opaque, one cutout, then an opaque one again, so
        // every bucket holds both kinds out of order.
        let mut solid = Vec::new();
        let mut id = 0u16;
        for (origin, opaque) in [
            ([0.0, 0.0, 0.0], true),
            ([4.0, 2.0, 6.0], false),
            ([9.0, 5.0, 3.0], true),
        ] {
            for q in cube(origin, 0).chunks(4) {
                let layer = id | if opaque { OPAQUE_LAYER } else { 0 };
                solid.extend(q.iter().map(|v| TerrainVertex { layer, ..*v }));
                id += 1;
            }
        }
        let unsorted = solid;
        for grouped in [false, true] {
            let mut solid = unsorted.clone();
            let sides = sort_parts(&mut solid, grouped);
            let layers: Vec<u16> = solid
                .as_chunks::<4>()
                .0
                .iter()
                .map(|q| q[0].layer)
                .collect();
            for eye in [[20.0, 30.0, -10.0], [-5.0, 1.0, 4.0], [5.0, 3.0, 7.0]] {
                let quads = |part| {
                    let mut out = Vec::new();
                    sides.visible_from(eye, eye[1], part, |first, count| {
                        out.extend(
                            layers[first as usize..(first + count) as usize]
                                .iter()
                                .copied(),
                        )
                    });
                    out.sort_unstable();
                    out
                };
                let mut facing = front_facing(&solid, eye);
                facing.sort_unstable();
                let opaque = quads(Part::Opaque);
                let cutout = quads(Part::Cutout);
                assert!(opaque.iter().all(|l| l & OPAQUE_LAYER != 0), "{eye:?}");
                assert!(cutout.iter().all(|l| l & OPAQUE_LAYER == 0), "{eye:?}");
                for l in &facing {
                    let mine = if l & OPAQUE_LAYER != 0 {
                        &opaque
                    } else {
                        &cutout
                    };
                    assert!(
                        mine.contains(l),
                        "{eye:?}: quad {l} facing the eye left out"
                    );
                }
                let mut both = [opaque, cutout].concat();
                both.sort_unstable();
                assert_eq!(both, quads(Part::All), "{eye:?}");
            }
            // Every way: each part once, whole.
            let mut every = Vec::new();
            sides.every(Part::Opaque, |first, count| every.push((first, count)));
            sides.every(Part::Cutout, |first, count| every.push((first, count)));
            let total: u32 = every.iter().map(|r| r.1).sum();
            assert_eq!(total, sides.quads());

            // Two passes: from above a corner, each part is one or two runs.
            if grouped {
                let mut count = 0;
                sides.visible_from([20.0, 30.0, -10.0], 30.0, Part::Opaque, |_, _| count += 1);
                assert_eq!(count, 2);
            }
        }
    }

    /// A mesh that was not sorted has no parts: each pass draws all of it
    /// (the shader leaves out the other pass's quads).
    #[test]
    fn unsorted_meshes_draw_everything_in_each_part() {
        let sides = Sides::unsorted(9);
        for part in [Part::Opaque, Part::Cutout] {
            let mut out = Vec::new();
            sides.every(part, |first, count| out.push((first, count)));
            assert_eq!(out, [(0, 9)]);
        }
    }

    #[test]
    fn unsorted_meshes_draw_everything() {
        let sides = Sides::unsorted(9);
        assert_eq!(sides.quads(), 9);
        assert_eq!(runs(&sides, [-100.0; 3]), [(0, 9)]);
        assert!(runs(&Sides::unsorted(0), [0.0; 3]).is_empty());
    }

    #[test]
    fn one_bucket_keeps_the_mesh_as_it_is() {
        let mut solid: Vec<_> = (0..3)
            .flat_map(|i| {
                quad(
                    [
                        [1.0, 0.0, 0.0],
                        [1.0, 1.0, 0.0],
                        [1.0, 1.0, 1.0],
                        [1.0, 0.0, 1.0],
                    ],
                    i,
                )
            })
            .collect();
        let before = solid.clone();
        let sides = sort(&mut solid);
        assert_eq!(bucket_ends(&sides), [3; 7]);
        assert_eq!(
            solid.iter().map(|v| v.layer).collect::<Vec<_>>(),
            before.iter().map(|v| v.layer).collect::<Vec<_>>()
        );
    }

    #[test]
    fn front_to_back_sort_orders_positive_faces_and_keeps_ties_stable() {
        let mut solid: Vec<_> = [(1.0, 10), (3.0, 30), (3.0, 31)]
            .into_iter()
            .flat_map(|(x, layer)| {
                quad(
                    [[x, 0.0, 0.0], [x, 1.0, 0.0], [x, 1.0, 1.0], [x, 0.0, 1.0]],
                    layer,
                )
            })
            .collect();
        sort(&mut solid);
        let layers: Vec<u16> = solid
            .as_chunks::<4>()
            .0
            .iter()
            .map(|q| q[0].layer)
            .collect();
        assert_eq!(layers, [30, 31, 10]);
    }

    #[test]
    fn front_to_back_sort_orders_negative_faces_toward_the_eye() {
        let mut solid: Vec<_> = [(3.0, 30), (1.0, 10), (2.0, 20)]
            .into_iter()
            .flat_map(|(x, layer)| {
                quad(
                    [[x, 0.0, 0.0], [x, 0.0, 1.0], [x, 1.0, 1.0], [x, 1.0, 0.0]],
                    layer,
                )
            })
            .collect();
        sort(&mut solid);
        let layers: Vec<u16> = solid
            .as_chunks::<4>()
            .0
            .iter()
            .map(|q| q[0].layer)
            .collect();
        assert_eq!(layers, [10, 20, 30]);
    }

    #[test]
    fn pure_opaque_buckets_sort_outside_in_and_cluster_by_morton() {
        // Pure-opaque +X faces across multiple planes and cells: must sort outside-in by x
        // and by 2D Morton on (y, z) within each plane.
        let mut solid = Vec::new();
        let specs: &[(f32, f32, f32, u16)] = &[
            (2.0, 0.0, 0.0, 1),
            (10.0, 2.0, 0.0, 2),
            (10.0, 0.0, 1.0, 3),
            (10.0, 1.0, 0.0, 4),
            (10.0, 0.0, 0.0, 5),
            (10.0, 1.0, 1.0, 6),
            (15.0, 7.0, 7.0, 7),
        ];
        for &(x, y, z, id) in specs {
            solid.extend(quad(
                [
                    [x, y, z],
                    [x, y + 1.0, z],
                    [x, y + 1.0, z + 1.0],
                    [x, y, z + 1.0],
                ],
                OPAQUE_LAYER | id,
            ));
        }
        let sides = sort(&mut solid);
        assert!(sides.is_split());
        let ids: Vec<u16> = solid
            .as_chunks::<4>()
            .0
            .iter()
            .map(|q| q[0].layer & !OPAQUE_LAYER)
            .collect();
        // Plane 15 first, then plane 10 in 2D Morton order: (0,0)->0, (1,0)->1, (0,1)->2, (1,1)->3, (2,0)->4, then plane 2.
        assert_eq!(ids, [7, 5, 4, 3, 6, 2, 1]);

        // Pure-opaque `OTHER` quads sort by 3D Morton cell order.
        let mut other = Vec::new();
        for (x, y, z, id) in [
            (9.0, 9.0, 9.0, 10u16),
            (0.0, 0.0, 1.0, 11),
            (0.0, 1.0, 0.0, 12),
            (1.0, 0.0, 0.0, 13),
            (0.0, 0.0, 0.0, 14),
        ] {
            other.extend(quad(
                [
                    [x, y, z],
                    [x + 1.0, y, z],
                    [x + 1.0, y + 1.0, z + 1.0],
                    [x, y + 1.0, z + 1.0],
                ],
                OPAQUE_LAYER | id,
            ));
        }
        sort(&mut other);
        let other_ids: Vec<u16> = other
            .as_chunks::<4>()
            .0
            .iter()
            .map(|q| q[0].layer & !OPAQUE_LAYER)
            .collect();
        // 3D Morton: (0,0,0)->0, (1,0,0)->1, (0,1,0)->2, (0,0,1)->4, (9,9,9)->high.
        assert_eq!(other_ids, [14, 13, 12, 11, 10]);
    }

    #[test]
    fn coplanar_overlay_draw_order_is_strictly_preserved() {
        // Test coplanar same-block base + overlay quads (including rotated vertex corners and
        // partial sub-block overlays) across opaque and cutout parts and both integer and
        // fractional plane paths.
        for (plane_x, grouped) in [(5.0f32, false), (5.0, true), (5.5, false), (5.5, true)] {
            for flag in [OPAQUE_LAYER, 0u16] {
                let mut solid = Vec::new();
                let mut tints = Vec::new();
                // Distractor block at a different cell on the same plane.
                solid.extend(quad(
                    [
                        [plane_x, 8.0, 8.0],
                        [plane_x, 9.0, 8.0],
                        [plane_x, 9.0, 9.0],
                        [plane_x, 8.0, 9.0],
                    ],
                    flag | 99,
                ));
                tints.push(99);
                // Block (y=3, z=4): base quad (id 1), rotated-corner overlay (id 2), partial quarter overlay (id 3).
                solid.extend(quad(
                    [
                        [plane_x, 3.0, 4.0],
                        [plane_x, 4.0, 4.0],
                        [plane_x, 4.0, 5.0],
                        [plane_x, 3.0, 5.0],
                    ],
                    flag | 1,
                ));
                tints.push(1);
                solid.extend(quad(
                    [
                        [plane_x, 4.0, 5.0],
                        [plane_x, 3.0, 5.0],
                        [plane_x, 3.0, 4.0],
                        [plane_x, 4.0, 4.0],
                    ],
                    flag | 2,
                ));
                tints.push(2);
                solid.extend(quad(
                    [
                        [plane_x, 3.5, 4.5],
                        [plane_x, 4.0, 4.5],
                        [plane_x, 4.0, 5.0],
                        [plane_x, 3.5, 5.0],
                    ],
                    flag | 3,
                ));
                tints.push(3);
                // Distractor on an outer plane.
                solid.extend(quad(
                    [
                        [plane_x + 2.0, 0.0, 0.0],
                        [plane_x + 2.0, 1.0, 0.0],
                        [plane_x + 2.0, 1.0, 1.0],
                        [plane_x + 2.0, 0.0, 1.0],
                    ],
                    flag | 50,
                ));
                tints.push(50);

                let sides = SORT_SCRATCH.with_borrow_mut(|scratch| {
                    sort_with(
                        &mut solid,
                        &mut tints,
                        &mut Vec::new(),
                        &mut Vec::new(),
                        grouped,
                        scratch,
                    )
                });
                assert!(sides.is_split());
                let ids: Vec<u16> = solid
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|q| q[0].layer & !OPAQUE_LAYER)
                    .collect();
                assert_eq!(
                    ids,
                    [50, 1, 2, 3, 99],
                    "plane_x={plane_x}, grouped={grouped}"
                );
                assert_eq!(tints, [50, 1, 2, 3, 99]);
            }
        }
    }

    #[test]
    fn coplanar_overlay_order_preserved_across_cell_boundaries() {
        let plus_x_quad = |x: f32, y0: f32, y1: f32, z0: f32, z1: f32, layer: u16| {
            quad([[x, y0, z0], [x, y1, z0], [x, y1, z1], [x, y0, z1]], layer)
        };

        for (plane_x, grouped) in [(5.0f32, false), (5.0, true), (5.5, false), (5.5, true)] {
            for flag in [OPAQUE_LAYER, 0u16] {
                // 1. Base spans y=2.5..4.0 (centroid cell 3) with both a lower overlay y=2.5..3.0
                // (centroid cell 2) and an upper overlay y=3.5..4.0 (centroid cell 3), plus
                // independent quads on the same plane and on an outer plane.
                let mut solid = Vec::new();
                let mut tints = Vec::new();
                // Independent high-Morton quad on same plane emitted first.
                solid.extend(plus_x_quad(plane_x, 8.0, 9.0, 8.0, 9.0, flag | 90));
                tints.push(90);
                // Cell-crossing base (id 1), lower-cell overlay (id 2), upper-cell overlay (id 3).
                solid.extend(plus_x_quad(plane_x, 2.5, 4.0, 4.0, 5.0, flag | 1));
                tints.push(1);
                solid.extend(plus_x_quad(plane_x, 2.5, 3.0, 4.0, 5.0, flag | 2));
                tints.push(2);
                solid.extend(plus_x_quad(plane_x, 3.5, 4.0, 4.0, 5.0, flag | 3));
                tints.push(3);
                // Independent low-Morton quad on same plane emitted after (8,8).
                solid.extend(plus_x_quad(plane_x, 0.0, 1.0, 0.0, 1.0, flag | 10));
                tints.push(10);
                // Independent quads on an outer plane must still get full 2D Morton order.
                solid.extend(plus_x_quad(plane_x + 2.0, 2.0, 3.0, 0.0, 1.0, flag | 62));
                tints.push(62);
                solid.extend(plus_x_quad(plane_x + 2.0, 0.0, 1.0, 0.0, 1.0, flag | 60));
                tints.push(60);

                let sides = SORT_SCRATCH.with_borrow_mut(|scratch| {
                    sort_with(
                        &mut solid,
                        &mut tints,
                        &mut Vec::new(),
                        &mut Vec::new(),
                        grouped,
                        scratch,
                    )
                });
                assert!(sides.is_split());
                let ids: Vec<u16> = solid
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|q| q[0].layer & !OPAQUE_LAYER)
                    .collect();
                // Outer plane sorted by Morton ([60, 62]); inner plane keeps independent quads
                // Morton-ordered (10 < {1,2,3} < 90) while base 1 stays before both overlays 2, 3.
                assert_eq!(
                    ids,
                    [60, 62, 10, 1, 2, 3, 90],
                    "plane_x={plane_x}, grouped={grouped}, flag={flag}"
                );
                assert_eq!(tints, [60, 62, 10, 1, 2, 3, 90]);

                // 2. Single-cell base in upper cell (y=3.5..4.0) followed by cell-crossing
                // overlay (y=2.5..4.0) must also keep [base, overlay] order.
                let mut rev = Vec::new();
                rev.extend(plus_x_quad(plane_x, 3.5, 4.0, 4.0, 5.0, flag | 11));
                rev.extend(plus_x_quad(plane_x, 2.5, 4.0, 4.0, 5.0, flag | 12));
                sort_parts(&mut rev, grouped);
                let rev_ids: Vec<u16> = rev
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|q| q[0].layer & !OPAQUE_LAYER)
                    .collect();
                assert_eq!(rev_ids, [11, 12]);
            }
        }

        // 3. `OTHER` bucket: sloped coplanar base spanning x=2.5..4.0 followed by partial
        // overlays in x=2.5..3.0 and x=3.5..4.0 stay in emission order while independent sloped
        // quads keep 3D Morton clustering.
        let mut other = Vec::new();
        other.extend(quad(
            [
                [9.0, 9.0, 9.0],
                [10.0, 9.0, 9.0],
                [10.0, 10.0, 10.0],
                [9.0, 10.0, 10.0],
            ],
            OPAQUE_LAYER | 90,
        ));
        other.extend(quad(
            [
                [2.5, 4.0, 4.0],
                [4.0, 4.0, 4.0],
                [4.0, 5.0, 5.0],
                [2.5, 5.0, 5.0],
            ],
            OPAQUE_LAYER | 1,
        ));
        other.extend(quad(
            [
                [2.5, 4.0, 4.0],
                [3.0, 4.0, 4.0],
                [3.0, 5.0, 5.0],
                [2.5, 5.0, 5.0],
            ],
            OPAQUE_LAYER | 2,
        ));
        other.extend(quad(
            [
                [3.5, 4.0, 4.0],
                [4.0, 4.0, 4.0],
                [4.0, 5.0, 5.0],
                [3.5, 5.0, 5.0],
            ],
            OPAQUE_LAYER | 3,
        ));
        other.extend(quad(
            [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ],
            OPAQUE_LAYER | 10,
        ));
        sort(&mut other);
        let other_ids: Vec<u16> = other
            .as_chunks::<4>()
            .0
            .iter()
            .map(|q| q[0].layer & !OPAQUE_LAYER)
            .collect();
        assert_eq!(other_ids, [10, 1, 2, 3, 90]);
    }

    #[test]
    fn signed_zero_planes_share_one_geometric_plane_and_keep_emission_order() {
        let plus_x = |x: f32, layer: u16| {
            quad(
                [[x, 3.0, 4.0], [x, 4.0, 4.0], [x, 4.0, 5.0], [x, 3.0, 5.0]],
                layer,
            )
        };
        let minus_x = |x: f32, layer: u16| {
            quad(
                [[x, 3.0, 4.0], [x, 3.0, 5.0], [x, 4.0, 5.0], [x, 4.0, 4.0]],
                layer,
            )
        };

        for grouped in [false, true] {
            for flag in [OPAQUE_LAYER, 0u16] {
                // 1. Integer-plane fast path (+X): -0.0 base followed by +0.0 overlay stays
                // [base, overlay] on plane 0, ordered after positive planes.
                let mut int_pos = Vec::new();
                int_pos.extend(plus_x(-0.0, flag | 1));
                int_pos.extend(plus_x(0.0, flag | 2));
                int_pos.extend(plus_x(2.0, flag | 20));
                int_pos.extend(plus_x(1.0, flag | 10));
                sort_parts(&mut int_pos, grouped);
                let ids: Vec<u16> = int_pos
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|q| q[0].layer & !OPAQUE_LAYER)
                    .collect();
                assert_eq!(ids, [20, 10, 1, 2], "int +X grouped={grouped} flag={flag}");

                // 2. Integer-plane fast path (-X): +0.0 base followed by -0.0 overlay stays
                // [base, overlay] on plane 0, ordered before positive planes.
                let mut int_neg = Vec::new();
                int_neg.extend(minus_x(1.0, flag | 10));
                int_neg.extend(minus_x(0.0, flag | 1));
                int_neg.extend(minus_x(-0.0, flag | 2));
                sort_parts(&mut int_neg, grouped);
                let ids: Vec<u16> = int_neg
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|q| q[0].layer & !OPAQUE_LAYER)
                    .collect();
                assert_eq!(ids, [1, 2, 10], "int -X grouped={grouped} flag={flag}");

                // 3. Compact-key fallback path (+X): fractional & negative planes force `u64`
                // keys; nonzero planes keep descending order while -0.0 base precedes +0.0 overlay.
                let mut frac_pos = Vec::new();
                frac_pos.extend(plus_x(-0.5, flag | 10));
                frac_pos.extend(plus_x(-0.0, flag | 1));
                frac_pos.extend(plus_x(0.0, flag | 2));
                frac_pos.extend(plus_x(0.5, flag | 20));
                sort_parts(&mut frac_pos, grouped);
                let ids: Vec<u16> = frac_pos
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|q| q[0].layer & !OPAQUE_LAYER)
                    .collect();
                assert_eq!(ids, [20, 1, 2, 10], "frac +X grouped={grouped} flag={flag}");

                // 4. Compact-key fallback path (-X): nonzero planes keep ascending order while
                // +0.0 base precedes -0.0 overlay.
                let mut frac_neg = Vec::new();
                frac_neg.extend(minus_x(0.5, flag | 20));
                frac_neg.extend(minus_x(0.0, flag | 1));
                frac_neg.extend(minus_x(-0.0, flag | 2));
                frac_neg.extend(minus_x(-0.5, flag | 10));
                sort_parts(&mut frac_neg, grouped);
                let ids: Vec<u16> = frac_neg
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|q| q[0].layer & !OPAQUE_LAYER)
                    .collect();
                assert_eq!(ids, [10, 1, 2, 20], "frac -X grouped={grouped} flag={flag}");

                // 5. Combined signed-zero plane + cell-crossing overlay on both paths:
                // -0.0 base spanning y=2.5..4.0 followed by +0.0 partial overlay y=2.5..3.0.
                for extra_plane in [None, Some(0.5f32)] {
                    let mut combined = Vec::new();
                    if let Some(px) = extra_plane {
                        combined.extend(plus_x(px, flag | 99));
                    }
                    combined.extend(quad(
                        [
                            [-0.0, 2.5, 4.0],
                            [-0.0, 4.0, 4.0],
                            [-0.0, 4.0, 5.0],
                            [-0.0, 2.5, 5.0],
                        ],
                        flag | 1,
                    ));
                    combined.extend(quad(
                        [
                            [0.0, 2.5, 4.0],
                            [0.0, 3.0, 4.0],
                            [0.0, 3.0, 5.0],
                            [0.0, 2.5, 5.0],
                        ],
                        flag | 2,
                    ));
                    sort_parts(&mut combined, grouped);
                    let ids: Vec<u16> = combined
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|q| q[0].layer & !OPAQUE_LAYER)
                        .collect();
                    let expected: &[u16] = if extra_plane.is_some() {
                        &[99, 1, 2]
                    } else {
                        &[1, 2]
                    };
                    assert_eq!(
                        ids, expected,
                        "combined signed-zero+crosser extra={extra_plane:?} grouped={grouped} flag={flag}"
                    );
                }
            }
        }
    }

    #[test]
    fn compact_path_boundary_and_full_width_fallback_match_bit_for_bit() {
        // 1. Verify exact 24-bit boundary constants: `MAX_COMPACT_QUADS == 1 << 24` holds indices
        // `0..=0x00FF_FFFF` without overflowing into the Morton or plane bitfields.
        assert_eq!(MAX_COMPACT_QUADS, 1 << 24);
        assert_eq!(MAX_COMPACT_QUADS - 1, COMPACT_IDX_MASK as usize);
        assert_eq!(u64::from(COMPACT_IDX_MASK), COMPACT_IDX_MASK_U64);
        let max_idx = (MAX_COMPACT_QUADS - 1) as u32;
        let max_idx_u64 = u64::from(max_idx);

        let packed_int = (u32::from(u8::MAX) << 24) | max_idx;
        assert_eq!(packed_int & COMPACT_IDX_MASK, max_idx);
        assert_eq!(packed_int >> 24, u32::from(u8::MAX));

        let packed_axis = (u64::from(u32::MAX) << 32) | (u64::from(u8::MAX) << 24) | max_idx_u64;
        assert_eq!((packed_axis & COMPACT_IDX_MASK_U64) as u32, max_idx);
        assert_eq!((packed_axis >> 24) & 0xFF, u64::from(u8::MAX));
        assert_eq!(packed_axis >> 32, u64::from(u32::MAX));
        let cleared_m2 = packed_axis & COMPACT_PLANE_AND_IDX_MASK;
        assert_eq!((cleared_m2 & COMPACT_IDX_MASK_U64) as u32, max_idx);
        assert_eq!((cleared_m2 >> 24) & 0xFF, 0);
        assert_eq!(cleared_m2 >> 32, u64::from(u32::MAX));

        let packed_2d_crosser = (u64::from(OUTSIDE_2D) << 24) | max_idx_u64;
        assert_eq!((packed_2d_crosser & COMPACT_IDX_MASK_U64) as u32, max_idx);
        assert_eq!(packed_2d_crosser >> 24, u64::from(OUTSIDE_2D));

        let packed_3d = (u64::from(OUTSIDE_3D) << 24) | max_idx_u64;
        assert_eq!((packed_3d & COMPACT_IDX_MASK_U64) as u32, max_idx);
        assert_eq!(packed_3d >> 24, u64::from(OUTSIDE_3D));

        let assert_boundary_and_fallback = |base: &[TerrainVertex], grouped: bool, label: &str| {
            let quads_len = base.len() / 4;
            assert!(quads_len >= 2);
            let make_aux = || {
                let tints: Vec<u32> = (0..quads_len as u32).map(|i| i + 100).collect();
                let sources: Vec<TerrainSource> = (0..quads_len as u32)
                    .map(|i| {
                        TerrainSource::new(
                            i + 1,
                            [(i % 16) as i32, ((i / 16) % 16) as i32, 0],
                            (i % 16) as u8,
                            i.is_multiple_of(2),
                        )
                    })
                    .collect();
                let looks: Vec<QuadLook> = (0..quads_len as u32)
                    .map(|i| QuadLook([i, i.wrapping_mul(7)]))
                    .collect();
                (tints, sources, looks)
            };

            let run_with_limit = |limit: usize| {
                let mut solid = base.to_vec();
                let (mut tints, mut sources, mut looks) = make_aux();
                let sides = SORT_SCRATCH.with_borrow_mut(|scratch| {
                    sort_with_compact_limit(
                        &mut solid,
                        &mut tints,
                        &mut sources,
                        &mut looks,
                        grouped,
                        scratch,
                        limit,
                    )
                });
                (solid, tints, sources, looks, sides)
            };

            let expected = reference(base, grouped);
            let prod = run_with_limit(MAX_COMPACT_QUADS);
            // Exact boundary: `quads_len == compact_limit` uses the compact 24-bit path.
            let boundary = run_with_limit(quads_len);
            // One past boundary: `quads_len == compact_limit + 1` uses the full-width fallback.
            let fallback = run_with_limit(quads_len - 1);
            // Zero limit also routes straight to the full-width fallback.
            let zero_limit = run_with_limit(0);

            for (name, res) in [
                ("prod", &prod),
                ("boundary", &boundary),
                ("fallback", &fallback),
                ("zero_limit", &zero_limit),
            ] {
                assert_eq!(
                    bytemuck::cast_slice::<_, u8>(&res.0),
                    bytemuck::cast_slice::<_, u8>(&expected),
                    "{label} ({name}, grouped={grouped}): vertex bytes diverged from reference"
                );
                assert_eq!(res.1, prod.1, "{label} ({name}): tints mismatch");
                assert_eq!(res.2, prod.2, "{label} ({name}): sources mismatch");
                assert_eq!(res.3, prod.3, "{label} ({name}): looks mismatch");
                assert_eq!(res.4, prod.4, "{label} ({name}): Sides mismatch");
                // Every original quad must appear exactly once (no duplicates, no drops).
                let mut seen = res.1.clone();
                seen.sort_unstable();
                let expected_tints: Vec<u32> = (0..quads_len as u32).map(|i| i + 100).collect();
                assert_eq!(seen, expected_tints, "{label} ({name}): permutation broken");
            }

            // Already-sorted input must remain untouched on the full-width fallback path too.
            let mut already = prod.0.clone();
            let (mut already_tints, mut already_sources, mut already_looks) =
                (prod.1.clone(), prod.2.clone(), prod.3.clone());
            let already_sides = SORT_SCRATCH.with_borrow_mut(|scratch| {
                sort_with_compact_limit(
                    &mut already,
                    &mut already_tints,
                    &mut already_sources,
                    &mut already_looks,
                    grouped,
                    scratch,
                    quads_len - 1,
                )
            });
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&already),
                bytemuck::cast_slice::<_, u8>(&prod.0),
                "{label}: already-sorted fallback changed vertices"
            );
            assert_eq!(already_tints, prod.1);
            assert_eq!(already_sources, prod.2);
            assert_eq!(already_looks, prod.3);
            assert_eq!(already_sides, prod.4);
        };

        // 2. Targeted pure-opaque and mixed meshes exercising integer planes, fractional/negative
        // planes, signed-zero planes, cell-boundary crossers, outside-[0, 16] quads, single-quad
        // parts, `OTHER` crossers, and unbounded (`2e38` tangent sum / `NaN`) quads.
        for opaque_only in [true, false] {
            for grouped in [false, true] {
                let mut solid = Vec::new();
                let mut id = 1u16;
                let mut push_quad = |corners: [[f32; 3]; 4], cutout: bool| {
                    let flag = if opaque_only || !cutout {
                        OPAQUE_LAYER
                    } else {
                        0
                    };
                    solid.extend(quad(corners, flag | id));
                    id += 1;
                };
                let plus_x = |x: f32, y0: f32, y1: f32, z0: f32, z1: f32| {
                    [[x, y0, z0], [x, y1, z0], [x, y1, z1], [x, y0, z1]]
                };
                // Integer planes with signed-zero + cell-crossing overlays + outside-only quad.
                push_quad(plus_x(-0.0, 2.5, 4.0, 4.0, 5.0), false);
                push_quad(plus_x(0.0, 2.5, 3.0, 4.0, 5.0), false);
                push_quad(plus_x(0.0, -2.0, -1.0, 0.0, 1.0), false);
                push_quad(plus_x(5.0, 8.0, 9.0, 8.0, 9.0), false);
                push_quad(plus_x(5.0, 1.0, 2.0, 1.0, 2.0), false);
                push_quad(plus_x(12.0, 0.0, 1.0, 0.0, 1.0), true);
                // Fractional & negative planes with crossers on a cutout part.
                push_quad(plus_x(-0.5, 2.5, 4.0, 4.0, 5.0), true);
                push_quad(plus_x(-0.5, 2.5, 3.0, 4.0, 5.0), true);
                push_quad(plus_x(5.5, 0.0, 1.0, 0.0, 1.0), true);
                // Pure integer-plane `+Y` part with a cell-boundary crosser so the integer-bin
                // fast path is exercised alongside the fractional `u64` key path even on pure-opaque
                // meshes.
                let plus_y = |y: f32, x0: f32, x1: f32, z0: f32, z1: f32| {
                    [[x0, y, z0], [x0, y, z1], [x1, y, z1], [x1, y, z0]]
                };
                push_quad(plus_y(5.0, 2.5, 4.0, 4.0, 5.0), false);
                push_quad(plus_y(5.0, 2.5, 3.0, 4.0, 5.0), false);
                push_quad(plus_y(5.0, 0.0, 1.0, 0.0, 1.0), false);
                push_quad(plus_y(12.0, 8.0, 9.0, 8.0, 9.0), false);
                // Single-quad -X face (exercises single-quad part path).
                push_quad(
                    [
                        [2.0, 0.0, 0.0],
                        [2.0, 0.0, 1.0],
                        [2.0, 1.0, 1.0],
                        [2.0, 1.0, 0.0],
                    ],
                    false,
                );
                // `OTHER` sloped quads with cell-boundary crosser and independent cells.
                push_quad(
                    [
                        [9.0, 9.0, 9.0],
                        [10.0, 9.0, 9.0],
                        [10.0, 10.0, 10.0],
                        [9.0, 10.0, 10.0],
                    ],
                    false,
                );
                push_quad(
                    [
                        [2.5, 4.0, 4.0],
                        [4.0, 4.0, 4.0],
                        [4.0, 5.0, 5.0],
                        [2.5, 5.0, 5.0],
                    ],
                    false,
                );
                push_quad(
                    [
                        [2.5, 4.0, 4.0],
                        [3.0, 4.0, 4.0],
                        [3.0, 5.0, 5.0],
                        [2.5, 5.0, 5.0],
                    ],
                    false,
                );
                push_quad(
                    [
                        [0.0, 0.0, 0.0],
                        [1.0, 0.0, 0.0],
                        [1.0, 1.0, 1.0],
                        [0.0, 1.0, 1.0],
                    ],
                    true,
                );
                assert_boundary_and_fallback(
                    &solid,
                    grouped,
                    if opaque_only {
                        "targeted opaque"
                    } else {
                        "targeted mixed"
                    },
                );

                // Unbounded plane (overflowing tangent sum) and unbounded `OTHER` (`NaN` vertex).
                let mut unbounded = solid;
                unbounded.extend(quad(
                    [
                        [5.0, 0.0, 0.0],
                        [5.0, 2.0e38, 0.0],
                        [5.0, 2.0e38, 1.0],
                        [5.0, 0.0, 1.0],
                    ],
                    OPAQUE_LAYER | id,
                ));
                id += 1;
                unbounded.extend(quad(
                    [
                        [f32::NAN, 1.0, 1.0],
                        [2.0, 1.0, 1.0],
                        [2.0, 2.0, 2.0],
                        [1.0, 2.0, 2.0],
                    ],
                    OPAQUE_LAYER | id,
                ));
                assert_boundary_and_fallback(&unbounded, grouped, "targeted unbounded");
            }
        }

        // 3. Randomized fuzz comparison across pure-opaque, mixed, integer-only, and fractional
        // meshes checking exact compact boundary vs full-width fallback.
        let mut state = 0xD1B5_4A32_D192_ED03u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..40u32 {
            let grouped = !round.is_multiple_of(2);
            let int_only = round.is_multiple_of(4);
            let pure_opaque = round.is_multiple_of(3);
            let count = 2 + (next() % 180);
            let mut solid = Vec::with_capacity(count as usize * 4);
            for q_idx in 0..count {
                let origin = if int_only {
                    [0, 1, 2].map(|_| (next() % 16) as f32)
                } else {
                    let o = [0, 1, 2].map(|_| (next() % 34) as f32 / 2.0 - 0.5);
                    if next().is_multiple_of(9) {
                        o.map(|v| -v)
                    } else {
                        o
                    }
                };
                let opaque = if pure_opaque {
                    OPAQUE_LAYER
                } else {
                    (next() % 2) as u16 * OPAQUE_LAYER
                };
                let layer = opaque | ((q_idx as u16) & 0x7FFF);
                if next().is_multiple_of(8) {
                    let [x, y, z] = origin;
                    let slope = [
                        [x, y, z],
                        [x + 1.0, y, z],
                        [x + 1.0, y + 1.0, z + 1.0],
                        [x, y + 1.0, z + 1.0],
                    ];
                    solid.extend(quad(slope, layer));
                } else {
                    let faces = cube(origin, layer);
                    let face = (next() % 6) as usize;
                    solid.extend_from_slice(&faces[face * 4..face * 4 + 4]);
                }
            }
            assert_boundary_and_fallback(&solid, grouped, "fuzz");
        }
    }
}
