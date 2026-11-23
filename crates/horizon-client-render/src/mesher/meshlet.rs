//! Sub-section meshlet partitioning (#17243 step 1): groups quads within each section into
//! clusters of up to 64 quads (256 vertices, 128 triangles), computing a compact 16-byte
//! descriptor (`MeshletDesc`) per meshlet.
//!
//! Preserves `mesher/sides.rs` bucket ordering and the 24-byte compact layout (`mesher/packed.rs`).
//! Each descriptor packs:
//! - `quad_offset`: u32 (starting quad index in section / arena page)
//! - `quad_count`: u8 (1..=64)
//! - `bucket`: u8 (0..7, corresponding to `Sides::BUCKETS`)
//! - `flags`: u8 (bit 0: cutout quad, bit 1: full-quad vertex fallback, bit 2: unbounded vertices)
//! - `plane_quant`: u8 (quantized plane threshold for directional culling)
//! - `sub_aabb`: [u8; 6] (quantized 4-bit min/max X, Y, Z coordinates [0..16] within the 16x16x16 section)
//! - `padding`: [u8; 2]

use super::Sides;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether background meshing workers should pre-build [`MeshletDesc`] descriptors alongside
/// `SectionMesh::sort_solid` (`HORIZON_MESH_SHADER=1` on a mesh-shader-capable device).
static PACK_MESHLETS: AtomicBool = AtomicBool::new(false);

pub fn pack_meshlets(on: bool) {
    PACK_MESHLETS.store(on, Ordering::Relaxed);
}

pub fn packing_meshlets() -> bool {
    PACK_MESHLETS.load(Ordering::Relaxed)
}

/// Maximum number of quads grouped into a single meshlet.
/// 64 quads = 256 vertices and 128 triangles, optimal for modern hardware mesh workgroups.
pub const MAX_MESHLET_QUADS: usize = 64;

/// A compact 16-byte descriptor of a sub-section meshlet.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshletDesc {
    /// Starting quad offset in the section / arena page.
    pub quad_offset: u32,
    /// Number of quads in this meshlet (1..=64).
    pub quad_count: u8,
    /// The directional bucket (0..7, see `Sides::BUCKETS`).
    pub bucket: u8,
    /// Flags: bit 0 = is_cutout, bit 1 = contains_full_vertices, bit 2 = unbounded_vertices.
    pub flags: u8,
    /// Quantized plane threshold across the bucket's axis.
    pub plane_quant: u8,
    /// Sub-section bounding box within 16x16x16 (min_x, min_y, min_z, max_x, max_y, max_z, each 0..16).
    pub sub_aabb: [u8; 6],
    /// Reserved alignment padding to 16 bytes.
    pub padding: [u8; 2],
}

impl MeshletDesc {
    pub const FLAG_CUTOUT: u8 = 1 << 0;
    pub const FLAG_FULL_VERTICES: u8 = 1 << 1;
    pub const FLAG_UNBOUNDED: u8 = 1 << 2;

    /// Creates a new meshlet descriptor.
    pub fn new(
        quad_offset: u32,
        quad_count: u8,
        bucket: u8,
        flags: u8,
        plane_quant: u8,
        sub_aabb: [u8; 6],
    ) -> Self {
        Self {
            quad_offset,
            quad_count,
            bucket,
            flags,
            plane_quant,
            sub_aabb,
            padding: [0, 0],
        }
    }

    /// Whether this meshlet contains cutout quads.
    pub fn is_cutout(&self) -> bool {
        self.flags & Self::FLAG_CUTOUT != 0
    }

    /// Whether any vertex in this meshlet extends beyond the representable `0..=255` bounds.
    pub fn is_unbounded(&self) -> bool {
        self.flags & Self::FLAG_UNBOUNDED != 0
    }

    /// Quantized min corner in section coordinates [0..16].
    pub fn min_corner(&self) -> [u8; 3] {
        [self.sub_aabb[0], self.sub_aabb[1], self.sub_aabb[2]]
    }

    /// Quantized max corner in section coordinates [0..16].
    pub fn max_corner(&self) -> [u8; 3] {
        [self.sub_aabb[3], self.sub_aabb[4], self.sub_aabb[5]]
    }

    /// Converts this section-relative meshlet descriptor into a GPU-ready descriptor (`GpuMeshlet`)
    /// by rebasing its `quad_offset` with the section's solid vertex base (`solid_base`).
    ///
    /// - For compact terrain quads in the arena page (`mesher/packed.rs`), `solid_base` is
    ///   `PACKED_BASE + 4 * slot_base`, yielding `vertex_base = PACKED_BASE + 4 * (slot_base + quad_offset)`.
    /// - For uncompressed full-vertex quads, `solid_base` is `4 * quad_base`, yielding
    ///   `vertex_base = 4 * (quad_base + quad_offset)`.
    #[inline]
    pub fn to_gpu(&self, solid_base: u32) -> GpuMeshlet {
        GpuMeshlet {
            vertex_base: solid_base.wrapping_add(self.quad_offset.wrapping_mul(4)),
            quad_count: self.quad_count,
            bucket: self.bucket,
            flags: self.flags,
            plane_quant: self.plane_quant,
            sub_aabb: self.sub_aabb,
            padding: [0, 0],
        }
    }
}

/// GPU representation of a meshlet descriptor as bound to `@group(2) @binding(0)` in `terrain_mesh.wgsl`.
///
/// Has the exact same 16-byte size and field alignment as `MeshletDesc`, but stores an absolute encoded
/// base vertex index (`vertex_base`) instead of a section-relative quad offset.
///
/// Binary layout (16 bytes, `[u32; 4]` / `vec4<u32>` in WGSL):
/// - `desc.x`: `vertex_base: u32` (`PACKED_BASE + 4 * slot` or uncompressed vertex index).
/// - `desc.y`: `quad_count: u8` (bits 0..7) | `bucket: u8` (bits 8..15) | `flags: u8` (bits 16..23) | `plane_quant: u8` (bits 24..31).
/// - `desc.z`: `min_x: u8` (0..7) | `min_y: u8` (8..15) | `min_z: u8` (16..23) | `max_x: u8` (24..31).
/// - `desc.w`: `max_y: u8` (0..7) | `max_z: u8` (8..15) | `padding: [u8; 2]` (16..31).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuMeshlet {
    /// Starting encoded vertex base index for the first quad in this meshlet.
    pub vertex_base: u32,
    /// Number of quads in this meshlet (1..=64).
    pub quad_count: u8,
    /// The directional bucket (0..7, see `Sides::BUCKETS`).
    pub bucket: u8,
    /// Flags: bit 0 = is_cutout, bit 1 = contains_full_vertices, bit 2 = unbounded_vertices.
    pub flags: u8,
    /// Quantized plane threshold across the bucket's axis.
    pub plane_quant: u8,
    /// Sub-section bounding box within 16x16x16 (min_x, min_y, min_z, max_x, max_y, max_z, each 0..16).
    pub sub_aabb: [u8; 6],
    /// Reserved alignment padding to 16 bytes.
    pub padding: [u8; 2],
}

impl GpuMeshlet {
    /// Creates a new GPU meshlet descriptor.
    pub fn new(
        vertex_base: u32,
        quad_count: u8,
        bucket: u8,
        flags: u8,
        plane_quant: u8,
        sub_aabb: [u8; 6],
    ) -> Self {
        Self {
            vertex_base,
            quad_count,
            bucket,
            flags,
            plane_quant,
            sub_aabb,
            padding: [0, 0],
        }
    }

    /// Whether this meshlet contains cutout quads.
    pub fn is_cutout(&self) -> bool {
        self.flags & MeshletDesc::FLAG_CUTOUT != 0
    }

    /// Whether any vertex in this meshlet extends beyond the representable `0..=255` bounds.
    pub fn is_unbounded(&self) -> bool {
        self.flags & MeshletDesc::FLAG_UNBOUNDED != 0
    }

    /// Quantized min corner in section coordinates [0..16].
    pub fn min_corner(&self) -> [u8; 3] {
        [self.sub_aabb[0], self.sub_aabb[1], self.sub_aabb[2]]
    }

    /// Quantized max corner in section coordinates [0..16].
    pub fn max_corner(&self) -> [u8; 3] {
        [self.sub_aabb[3], self.sub_aabb[4], self.sub_aabb[5]]
    }
}

/// Quantizes a lower-bound coordinate in `[0.0, 255.0]` to `u8`, matching piecewise
/// `floor().clamp(0, 15)` on `0.0..=16.0` and `floor().clamp(16, 254)` on `16.0..=255.0`
/// without calling libm `floorf`.
#[inline]
fn quantize_lo(v: f32) -> u8 {
    (v as u8).min(254) - u8::from(v == 16.0)
}

/// Quantizes an upper-bound coordinate in `[0.0, 255.0]` to `u8`, matching piecewise
/// `ceil().clamp(1, 16)` on `0.0..=16.0` and `ceil().clamp(17, 255)` on `16.0..=255.0`
/// without calling libm `ceilf`.
#[inline]
fn quantize_hi(v: f32) -> u8 {
    let i = v as u8;
    (i + u8::from(f32::from(i) < v)).max(1)
}

/// Bounding box helper for building quantized sub-AABBs [0..16] (or conservative bounds up to 255).
/// Accumulates branchless float extrema across vertices and quantizes once at [`Self::build`].
#[derive(Clone, Copy, Debug)]
pub struct SubAabbBuilder {
    min_f: [f32; 3],
    max_f: [f32; 3],
    pub unbounded: bool,
}

impl Default for SubAabbBuilder {
    fn default() -> Self {
        Self {
            min_f: [f32::INFINITY; 3],
            max_f: [f32::NEG_INFINITY; 3],
            unbounded: false,
        }
    }
}

impl SubAabbBuilder {
    /// Expands the bounding box with a block coordinate inside a 16x16x16 section.
    #[inline]
    pub fn include_block(&mut self, x: u8, y: u8, z: u8) {
        let coords = [x.min(15), y.min(15), z.min(15)];
        for axis in 0..3 {
            let lo = f32::from(coords[axis]);
            let hi = f32::from(coords[axis] + 1);
            if lo < self.min_f[axis] {
                self.min_f[axis] = lo;
            }
            if hi > self.max_f[axis] {
                self.max_f[axis] = hi;
            }
        }
    }

    /// Expands the bounding box using a quad's four section-local vertices.
    #[inline]
    pub fn include_vertices(&mut self, quad: &[super::TerrainVertex; 4]) {
        let p0 = quad[0].pos;
        let p1 = quad[1].pos;
        let p2 = quad[2].pos;
        let p3 = quad[3].pos;
        let mut qmin = p0;
        let mut qmax = p0;
        for p in [p1, p2, p3] {
            for axis in 0..3 {
                let c = p[axis];
                if c < qmin[axis] {
                    qmin[axis] = c;
                }
                if c > qmax[axis] {
                    qmax[axis] = c;
                }
            }
        }
        // Any NaN coordinate makes `sum` NaN so `sum >= 0.0` is false; without NaN,
        // `qmin`/`qmax` hold the exact extrema across all four vertices.
        let sum = (p0[0] + p0[1] + p0[2])
            + (p1[0] + p1[1] + p1[2])
            + (p2[0] + p2[1] + p2[2])
            + (p3[0] + p3[1] + p3[2]);
        if sum >= 0.0
            && qmin[0] >= 0.0
            && qmin[1] >= 0.0
            && qmin[2] >= 0.0
            && qmax[0] <= 255.0
            && qmax[1] <= 255.0
            && qmax[2] <= 255.0
        {
            for axis in 0..3 {
                if qmin[axis] < self.min_f[axis] {
                    self.min_f[axis] = qmin[axis];
                }
                if qmax[axis] > self.max_f[axis] {
                    self.max_f[axis] = qmax[axis];
                }
            }
        } else {
            self.include_vertices_slow(quad);
        }
    }

    #[cold]
    fn include_vertices_slow(&mut self, quad: &[super::TerrainVertex; 4]) {
        for v in quad {
            for (axis, &coord) in v.pos.iter().enumerate() {
                if (0.0..=255.0).contains(&coord) {
                    if coord < self.min_f[axis] {
                        self.min_f[axis] = coord;
                    }
                    if coord > self.max_f[axis] {
                        self.max_f[axis] = coord;
                    }
                } else {
                    self.unbounded = true;
                    if 0.0 < self.min_f[axis] {
                        self.min_f[axis] = 0.0;
                    }
                    if 16.0 > self.max_f[axis] {
                        self.max_f[axis] = 16.0;
                    }
                }
            }
        }
    }

    /// Expands the bounding box from a compact 24-byte `packed` record if not spilled (`FULL`).
    pub fn include_packed_record(&mut self, words: &[u32; 6]) -> bool {
        if words[5] & super::packed::FULL != 0 {
            return false;
        }
        let orient = (words[5] >> 12) & 0x3_ffff;
        self.include_block(
            (orient & 15) as u8,
            ((orient >> 4) & 15) as u8,
            ((orient >> 8) & 15) as u8,
        );
        true
    }

    /// Converts into a [u8; 6] array (min_xyz, max_xyz).
    #[inline]
    pub fn build(self) -> [u8; 6] {
        if self.min_f[0] > self.max_f[0] {
            [0, 0, 0, 16, 16, 16]
        } else {
            let mut out = [0u8; 6];
            for axis in 0..3 {
                let lo = quantize_lo(self.min_f[axis]);
                let hi = quantize_hi(self.max_f[axis]).max(lo + 1);
                out[axis] = lo;
                out[axis + 3] = hi;
            }
            out
        }
    }
}

/// Shared partitioning loop preserving `Sides` bucket boundaries and optional per-quad
/// cutout splitting (`quad_cutout`), chunking homogeneous runs into <= `MAX_MESHLET_QUADS`.
fn partition_meshlets_into(
    sides: &Sides,
    quad_cutout: impl Fn(u32, bool) -> bool,
    mut build_range: impl FnMut(u32, u8) -> (u8, [u8; 6]),
    meshlets: &mut Vec<MeshletDesc>,
) {
    meshlets.clear();
    let total_quads = sides.quads();
    if total_quads == 0 {
        return;
    }

    let is_split = sides.is_split();
    let mut prev_end = 0u32;
    for part_idx in 0..14 {
        let (bucket, is_cutout) = sides_slot(sides.is_grouped(), part_idx);
        let end = sides.part_end(part_idx);
        if end <= prev_end {
            continue;
        }

        let plane_quant = quantize_plane(sides.bucket_plane(bucket));
        let mut run_start = prev_end;
        while run_start < end {
            let (run_cutout, run_end) = if is_split {
                (is_cutout, end)
            } else {
                let run_cutout = quad_cutout(run_start, is_cutout);
                let mut run_end = run_start + 1;
                while run_end < end && quad_cutout(run_end, is_cutout) == run_cutout {
                    run_end += 1;
                }
                (run_cutout, run_end)
            };

            let base_flags = if run_cutout {
                MeshletDesc::FLAG_CUTOUT
            } else {
                0
            };
            let mut offset = run_start;
            while offset < run_end {
                let count = (run_end - offset).min(MAX_MESHLET_QUADS as u32) as u8;
                let (extra_flags, sub_aabb) = build_range(offset, count);

                meshlets.push(MeshletDesc::new(
                    offset,
                    count,
                    bucket as u8,
                    base_flags | extra_flags,
                    plane_quant,
                    sub_aabb,
                ));
                offset += u32::from(count);
            }

            run_start = run_end;
        }

        prev_end = end;
    }
}

/// Partitions a section's quads into meshlets conforming to `Sides` bucket and part boundaries.
pub fn build_meshlets(
    sides: &Sides,
    quad_bounds: impl Fn(u32) -> (u8, u8, u8),
) -> Vec<MeshletDesc> {
    let mut meshlets = Vec::new();
    partition_meshlets_into(
        sides,
        |_q, default_cutout| default_cutout,
        |offset, count| {
            let mut aabb = SubAabbBuilder::default();
            for q in offset..(offset + u32::from(count)) {
                let (bx, by, bz) = quad_bounds(q);
                aabb.include_block(bx, by, bz);
            }
            (0, aabb.build())
        },
        &mut meshlets,
    );
    meshlets
}

/// Returns the 4 spilled `[u32; 6]` vertex words for quad `q` when `packed[q]` carries `FULL`.
fn spilled_quad_words(packed: &[[u32; 6]], q: u32) -> Option<&[[u32; 6]]> {
    let words = packed.get(q as usize)?;
    if words[5] & super::packed::FULL == 0 || words[0] == 0 {
        return None;
    }
    let spill = (q as usize).checked_add(words[0] as usize)?;
    let end = spill.checked_add(4)?;
    packed.get(spill..end)
}

/// Determines whether quad `q` requires cutout filtering (`PASS_CUTOUT`).
fn quad_is_cutout(
    q: u32,
    default_cutout: bool,
    solid: &[super::TerrainVertex],
    packed: &[[u32; 6]],
) -> bool {
    if let Some(v) = solid.get((q as usize).saturating_mul(4)) {
        return (v.layer & super::OPAQUE_LAYER) == 0;
    }
    if let Some(words) = packed.get(q as usize)
        && words[5] & super::packed::FULL == 0
    {
        return ((words[3] & 0xFFFF) as u16 & super::OPAQUE_LAYER) == 0;
    }
    if let Some(raw_quad) = spilled_quad_words(packed, q) {
        let v: &super::TerrainVertex = bytemuck::cast_ref(&raw_quad[0]);
        return (v.layer & super::OPAQUE_LAYER) == 0;
    }
    default_cutout
}

/// Includes quad `q`'s bounds from unpacked `solid` vertices first (staying agnostic to packed
/// sub-block encodings) and falls back to `packed` records or spilled `FULL` words when `solid` is empty.
fn include_section_quad(
    q: u32,
    solid: &[super::TerrainVertex],
    packed: &[[u32; 6]],
    aabb: &mut SubAabbBuilder,
) -> u8 {
    let packed_record = packed.get(q as usize);
    let extra_flags = if packed_record.is_none_or(|w| w[5] & super::packed::FULL != 0) {
        MeshletDesc::FLAG_FULL_VERTICES
    } else {
        0
    };
    if let Some(base) = (q as usize).checked_mul(4)
        && let Some(verts) = solid.get(base..(base + 4))
        && let Ok(quad) = <&[super::TerrainVertex; 4]>::try_from(verts)
    {
        aabb.include_vertices(quad);
        return extra_flags;
    }
    if let Some(words) = packed_record
        && aabb.include_packed_record(words)
    {
        return 0;
    }
    if let Some(raw_quad) = spilled_quad_words(packed, q) {
        let quad: [super::TerrainVertex; 4] = std::array::from_fn(|i| bytemuck::cast(raw_quad[i]));
        aabb.include_vertices(&quad);
    }
    MeshletDesc::FLAG_FULL_VERTICES
}

/// Extracts sub-section meshlets directly from a section's `Sides`, unpacked `solid` vertices,
/// and optional compact 24-byte `packed` records (`mesher/packed.rs`) into `out`.
///
/// Guarantees every emitted meshlet descriptor is homogeneous in `FLAG_CUTOUT`: even when given an
/// unsorted mesh (`Sides::unsorted`), contiguous opaque and cutout runs are split into separate
/// descriptors so `ts_terrain` pass filtering matches per-quad `opaque_quad`.
pub fn build_section_meshlets_into(
    sides: &Sides,
    solid: &[super::TerrainVertex],
    packed: &[[u32; 6]],
    out: &mut Vec<MeshletDesc>,
) {
    let _ = build_section_meshlets_with_bounds(sides, solid, packed, out);
}

/// Same as [`build_section_meshlets_into`], also returning the exact raw float `[min, max]`
/// extrema across `solid` when every quad in `sides` comes directly from `solid` and all
/// vertices lie within `[0.0, 255.0]`, letting `SectionMesh::sort_solid` reuse them instead
/// of re-scanning `solid` in `compute_bounds`.
pub(super) fn build_section_meshlets_with_bounds(
    sides: &Sides,
    solid: &[super::TerrainVertex],
    packed: &[[u32; 6]],
    out: &mut Vec<MeshletDesc>,
) -> Option<([f32; 3], [f32; 3])> {
    let total_quads = sides.quads() as usize;
    let (solid_quads, solid_rest) = solid.as_chunks::<4>();
    let has_all_solid =
        total_quads > 0 && solid_rest.is_empty() && solid_quads.len() == total_quads;
    let has_all_packed = packed.len() >= total_quads;
    let mut solid_min = [f32::INFINITY; 3];
    let mut solid_max = [f32::NEG_INFINITY; 3];
    let mut bounds_valid = has_all_solid;

    partition_meshlets_into(
        sides,
        |q, default_cutout| quad_is_cutout(q, default_cutout, solid, packed),
        |offset, count| {
            let mut aabb = SubAabbBuilder::default();
            let start = offset as usize;
            let end = start + usize::from(count);
            let mut extra_flags = if end <= solid_quads.len() {
                for quad in &solid_quads[start..end] {
                    aabb.include_vertices(quad);
                }
                if has_all_packed {
                    if packed[start..end]
                        .iter()
                        .any(|w| w[5] & super::packed::FULL != 0)
                    {
                        MeshletDesc::FLAG_FULL_VERTICES
                    } else {
                        0
                    }
                } else if packed.is_empty()
                    || (start..end).any(|q| {
                        packed
                            .get(q)
                            .is_none_or(|w| w[5] & super::packed::FULL != 0)
                    })
                {
                    MeshletDesc::FLAG_FULL_VERTICES
                } else {
                    0
                }
            } else {
                bounds_valid = false;
                let mut flags = 0u8;
                for q in offset..(offset + u32::from(count)) {
                    flags |= include_section_quad(q, solid, packed, &mut aabb);
                }
                flags
            };
            if aabb.unbounded {
                extra_flags |= MeshletDesc::FLAG_UNBOUNDED;
                bounds_valid = false;
            } else if bounds_valid {
                for axis in 0..3 {
                    if aabb.min_f[axis] < solid_min[axis] {
                        solid_min[axis] = aabb.min_f[axis];
                    }
                    if aabb.max_f[axis] > solid_max[axis] {
                        solid_max[axis] = aabb.max_f[axis];
                    }
                }
            }
            (extra_flags, aabb.build())
        },
        out,
    );

    if bounds_valid && solid_min[0] <= solid_max[0] {
        Some((solid_min, solid_max))
    } else {
        None
    }
}

/// Extracts sub-section meshlets directly from a section's `Sides`, unpacked `solid` vertices,
/// and optional compact 24-byte `packed` records (`mesher/packed.rs`).
pub fn build_section_meshlets(
    sides: &Sides,
    solid: &[super::TerrainVertex],
    packed: &[[u32; 6]],
) -> Vec<MeshletDesc> {
    let mut out = Vec::new();
    build_section_meshlets_into(sides, solid, packed, &mut out);
    out
}

/// Quantizes a float plane threshold [0.0..16.0] to a u8 in [0..255].
fn quantize_plane(plane: f32) -> u8 {
    (plane.clamp(0.0, 16.0) * (255.0 / 16.0)).round() as u8
}

/// Slot mapping identical to mesher/sides.rs.
fn sides_slot(grouped: bool, n: usize) -> (usize, bool) {
    if grouped {
        (n % 7, n >= 7)
    } else {
        (n / 2, n % 2 == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_quad(x: f32, y: f32, z: f32, layer: u16) -> [super::super::TerrainVertex; 4] {
        let v = |dx, dy, dz, u, v_coord| super::super::TerrainVertex {
            pos: [x + dx, y + dy, z + dz],
            uv: [u, v_coord],
            layer,
            light: 0xF0F0,
            color: 0xFFFF_FFFF,
        };
        [
            v(0.0, 1.0, 0.0, 0, 0),
            v(0.0, 1.0, 1.0, 0, 32 << 10),
            v(1.0, 1.0, 1.0, 32 << 10, 32 << 10),
            v(1.0, 1.0, 0.0, 32 << 10, 0),
        ]
    }

    #[test]
    fn meshlet_desc_size_and_alignment() {
        assert_eq!(std::mem::size_of::<MeshletDesc>(), 16);
        assert_eq!(std::mem::align_of::<MeshletDesc>(), 4);
    }

    #[test]
    fn sub_aabb_builder_expands_correctly() {
        let mut b = SubAabbBuilder::default();
        b.include_block(2, 3, 4);
        b.include_block(10, 12, 14);
        assert_eq!(b.build(), [2, 3, 4, 11, 13, 15]);
    }

    #[test]
    fn meshlet_partition_preserves_quad_count() {
        let sides = Sides::unsorted(150);
        let meshlets = build_meshlets(&sides, |q| ((q % 16) as u8, 0, 0));
        assert_eq!(meshlets.len(), 3); // 64 + 64 + 22 = 150
        assert_eq!(meshlets[0].quad_count, 64);
        assert_eq!(meshlets[1].quad_count, 64);
        assert_eq!(meshlets[2].quad_count, 22);

        let sum: u32 = meshlets.iter().map(|m| m.quad_count as u32).sum();
        assert_eq!(sum, 150);
    }

    #[test]
    fn build_section_meshlets_from_packed_and_unpacked_quads() {
        let q0 = test_quad(2.0, 3.0, 4.0, super::super::OPAQUE_LAYER);
        let q1 = test_quad(5.0, 3.0, 6.0, super::super::OPAQUE_LAYER);
        let solid: Vec<super::super::TerrainVertex> = q0.into_iter().chain(q1).collect();
        let packed_words = [
            super::super::packed::pack_quad(&q0, super::super::packed::WHITE).expect("packs"),
            [0, 0, 0, 0, 0, super::super::packed::FULL],
        ];
        let sides = Sides::unsorted(2);
        let meshlets = build_section_meshlets(&sides, &solid, &packed_words);
        assert_eq!(meshlets.len(), 1);
        assert_eq!(meshlets[0].quad_count, 2);
        assert_ne!(meshlets[0].flags & MeshletDesc::FLAG_FULL_VERTICES, 0);
        assert!(!meshlets[0].is_cutout());
        assert_eq!(meshlets[0].min_corner(), [2, 4, 4]);
        assert_eq!(meshlets[0].max_corner(), [6, 5, 7]);

        // Rebase via canonical Step 2 `to_gpu(solid_base)` conversion.
        let gpu = meshlets[0].to_gpu(super::super::packed::PACKED_BASE + 128);
        assert_eq!(gpu.vertex_base, super::super::packed::PACKED_BASE + 128);

        // Verify pure packed layout including spilled FULL records read directly from `packed` when `solid` is empty.
        let mut q_non_grid = q1;
        q_non_grid[0].pos[0] += 0.25; // forces spill to FULL in `packed::layout`
        let all_quads = [q0, q0, q0, q0, q_non_grid];
        let spill_solid: Vec<_> = all_quads.into_iter().flatten().collect();
        let spill_tints = [super::super::packed::WHITE; 5];
        let layout = super::super::packed::layout(&spill_solid, &spill_tints);
        assert!(!layout.is_empty());
        let spill_meshlets = build_section_meshlets(&Sides::unsorted(5), &[], &layout);
        assert_eq!(spill_meshlets.len(), 1);
        assert_ne!(spill_meshlets[0].flags & MeshletDesc::FLAG_FULL_VERTICES, 0);
        assert_eq!(spill_meshlets[0].min_corner(), [2, 4, 4]);
        assert_eq!(spill_meshlets[0].max_corner(), [6, 5, 7]);

        // Pure compact quads without any FULL records must leave FLAG_FULL_VERTICES unset.
        let all_packed = vec![
            super::super::packed::pack_quad(&q0, super::super::packed::WHITE).unwrap(),
            super::super::packed::pack_quad(&q1, super::super::packed::WHITE).unwrap(),
        ];
        let packed_only = build_section_meshlets(&sides, &solid, &all_packed);
        assert_eq!(packed_only[0].flags & MeshletDesc::FLAG_FULL_VERTICES, 0);
        assert_eq!(packed_only[0].min_corner(), [2, 4, 4]);
        assert_eq!(packed_only[0].max_corner(), [6, 5, 7]);
    }

    #[test]
    fn build_section_meshlets_splits_mixed_unsorted_cutout_runs() {
        let q0 = test_quad(1.0, 2.0, 3.0, super::super::OPAQUE_LAYER);
        let q1 = test_quad(2.0, 2.0, 3.0, 0); // cutout
        let q2 = test_quad(3.0, 2.0, 3.0, 0); // cutout
        let q3 = test_quad(4.0, 2.0, 3.0, super::super::OPAQUE_LAYER);
        let quads = [q0, q1, q2, q3];
        let solid: Vec<_> = quads.into_iter().flatten().collect();
        let sides = Sides::unsorted(4);

        let meshlets = build_section_meshlets(&sides, &solid, &[]);
        assert_eq!(meshlets.len(), 3);
        assert_eq!(
            (
                meshlets[0].quad_offset,
                meshlets[0].quad_count,
                meshlets[0].is_cutout()
            ),
            (0, 1, false)
        );
        assert_eq!(
            (
                meshlets[1].quad_offset,
                meshlets[1].quad_count,
                meshlets[1].is_cutout()
            ),
            (1, 2, true)
        );
        assert_eq!(
            (
                meshlets[2].quad_offset,
                meshlets[2].quad_count,
                meshlets[2].is_cutout()
            ),
            (3, 1, false)
        );

        let covered: u32 = meshlets.iter().map(|m| u32::from(m.quad_count)).sum();
        assert_eq!(covered, 4);
    }

    #[test]
    fn build_section_meshlets_preserves_sorted_buckets_and_planes() {
        let quad_top_opaque = test_quad(2.0, 3.0, 4.0, super::super::OPAQUE_LAYER);
        // Bottom face (-Y) cutout quad at y = 1.0
        let v = |x, y, z, layer| super::super::TerrainVertex {
            pos: [x, y, z],
            uv: [0, 0],
            layer,
            light: 0xF0F0,
            color: 0xFFFF_FFFF,
        };
        let quad_bottom_cutout = [
            v(6.0, 1.0, 6.0, 0),
            v(7.0, 1.0, 6.0, 0),
            v(7.0, 1.0, 7.0, 0),
            v(6.0, 1.0, 7.0, 0),
        ];
        let mut solid: Vec<_> = quad_top_opaque
            .into_iter()
            .chain(quad_bottom_cutout)
            .collect();
        let mut tints = vec![super::super::packed::WHITE; 2];
        let sides = super::super::sort_tinted(&mut solid, &mut tints);
        let packed = super::super::packed::layout(&solid, &tints);

        let meshlets = build_section_meshlets(&sides, &solid, &packed);
        assert_eq!(meshlets.len(), 2);
        let total_quads: u32 = meshlets.iter().map(|m| u32::from(m.quad_count)).sum();
        assert_eq!(total_quads, 2);
        assert!(meshlets.iter().any(|m| m.bucket == 3 && !m.is_cutout()));
        assert!(meshlets.iter().any(|m| m.bucket == 6 && m.is_cutout()));
        for m in &meshlets {
            let start = m.quad_offset as usize;
            for q in start..(start + m.quad_count as usize) {
                let q_cutout = (solid[q * 4].layer & super::super::OPAQUE_LAYER) == 0;
                assert_eq!(m.is_cutout(), q_cutout);
            }
        }
    }

    fn desc_in_frustum(desc: &MeshletDesc, planes: &[[f32; 4]; 6]) -> bool {
        if desc.is_unbounded() {
            return true;
        }
        let bmin = desc.min_corner().map(f32::from);
        let bmax = desc.max_corner().map(f32::from);
        planes.iter().all(|&[nx, ny, nz, w]| {
            let px = if nx >= 0.0 { bmax[0] } else { bmin[0] };
            let py = if ny >= 0.0 { bmax[1] } else { bmin[1] };
            let pz = if nz >= 0.0 { bmax[2] } else { bmin[2] };
            nx * px + ny * py + nz * pz + w >= 0.0
        })
    }

    #[test]
    fn build_section_meshlets_keeps_conservative_bounds_across_section_edges() {
        let mut q_ext_east = test_quad(15.0, 2.0, 2.0, super::super::OPAQUE_LAYER);
        q_ext_east[2].pos[0] = 16.25;
        q_ext_east[3].pos[0] = 16.25;

        let mut q_ext_west = test_quad(0.0, 2.0, 2.0, super::super::OPAQUE_LAYER);
        q_ext_west[0].pos[0] = -0.25;
        q_ext_west[1].pos[0] = -0.25;

        let q_in = test_quad(4.0, 2.0, 2.0, super::super::OPAQUE_LAYER);

        // Frustum plane between section edge x=16.0 and extending vertex x=16.25 (x >= 16.125).
        let east_planes = [
            [1.0, 0.0, 0.0, -16.125],
            [-1.0, 0.0, 0.0, 32.0],
            [0.0, 1.0, 0.0, 16.0],
            [0.0, -1.0, 0.0, 16.0],
            [0.0, 0.0, 1.0, 16.0],
            [0.0, 0.0, -1.0, 16.0],
        ];
        // Frustum plane between section edge x=0.0 and extending vertex x=-0.25 (x <= -0.125).
        let west_planes = [
            [1.0, 0.0, 0.0, 16.0],
            [-1.0, 0.0, 0.0, -0.125],
            [0.0, 1.0, 0.0, 16.0],
            [0.0, -1.0, 0.0, 16.0],
            [0.0, 0.0, 1.0, 16.0],
            [0.0, 0.0, -1.0, 16.0],
        ];

        // In-range geometry retains compact [0..16] bounds and is correctly culled beyond x=16.125.
        let in_solid: Vec<_> = q_in.into_iter().collect();
        let in_meshlets = build_section_meshlets(&Sides::unsorted(1), &in_solid, &[]);
        assert!(!in_meshlets[0].is_unbounded());
        assert_eq!(in_meshlets[0].max_corner(), [5, 4, 3]);
        assert!(!desc_in_frustum(&in_meshlets[0], &east_planes));

        // 1. Raw solid quads extending past east (+X = 16.25) and west (-X = -0.25) section edges.
        let east_raw: Vec<_> = q_ext_east.into_iter().collect();
        let east_meshlets = build_section_meshlets(&Sides::unsorted(1), &east_raw, &[]);
        assert!(!east_meshlets[0].is_unbounded());
        assert_eq!(east_meshlets[0].max_corner()[0], 17);
        assert!(desc_in_frustum(&east_meshlets[0], &east_planes));

        let west_raw: Vec<_> = q_ext_west.into_iter().collect();
        let west_meshlets = build_section_meshlets(&Sides::unsorted(1), &west_raw, &[]);
        assert!(west_meshlets[0].is_unbounded());
        assert!(desc_in_frustum(&west_meshlets[0], &west_planes));

        // 2. Spilled FULL packed quads preserve the exact same conservative/unbounded behavior.
        let spill_tints = [super::super::packed::WHITE; 5];
        let east_spill_solid: Vec<_> = [q_in, q_in, q_in, q_in, q_ext_east]
            .into_iter()
            .flatten()
            .collect();
        let east_packed = super::super::packed::layout(&east_spill_solid, &spill_tints);
        assert!(!east_packed.is_empty());
        let east_spill_meshlets = build_section_meshlets(&Sides::unsorted(5), &[], &east_packed);
        assert!(!east_spill_meshlets[0].is_unbounded());
        assert_eq!(east_spill_meshlets[0].max_corner()[0], 17);
        assert!(desc_in_frustum(&east_spill_meshlets[0], &east_planes));

        let west_spill_solid: Vec<_> = [q_in, q_in, q_in, q_in, q_ext_west]
            .into_iter()
            .flatten()
            .collect();
        let west_packed = super::super::packed::layout(&west_spill_solid, &spill_tints);
        assert!(!west_packed.is_empty());
        let west_spill_meshlets = build_section_meshlets(&Sides::unsorted(5), &[], &west_packed);
        assert!(west_spill_meshlets[0].is_unbounded());
        assert!(desc_in_frustum(&west_spill_meshlets[0], &west_planes));
    }

    #[test]
    fn sort_solid_populates_meshlets_only_when_pack_meshlets_is_enabled() {
        let prev = packing_meshlets();
        let make_mesh = || {
            let v = super::super::TerrainVertex {
                pos: [1.0, 2.0, 3.0],
                uv: [0, 0],
                layer: super::super::OPAQUE_LAYER,
                light: 0xF0F0,
                color: 0xFFFF_FFFF,
            };
            super::super::SectionMesh {
                solid: vec![v; 4],
                ..Default::default()
            }
        };

        pack_meshlets(false);
        let mut off_mesh = make_mesh();
        off_mesh.sort_solid();
        assert!(off_mesh.meshlets.is_empty());
        assert_eq!(off_mesh.meshlets.capacity(), 0);
        assert_eq!(off_mesh.bounds, off_mesh.compute_bounds());

        pack_meshlets(true);
        let mut on_mesh = make_mesh();
        on_mesh.sort_solid();
        assert_eq!(on_mesh.meshlets.len(), 1);
        assert_eq!(on_mesh.meshlets[0].quad_count, 1);
        assert_eq!(on_mesh.bounds, on_mesh.compute_bounds());

        pack_meshlets(prev);
    }

    /// Reference per-quad `floor`/`ceil` quantization matching the original `SubAabbBuilder`,
    /// which quantized and enforced non-degeneracy (`max >= min + 1`) after each quad.
    fn reference_sub_aabb_multi(quads: &[[super::super::TerrainVertex; 4]]) -> ([u8; 6], bool) {
        let mut min = [u8::MAX; 3];
        let mut max = [0u8; 3];
        let mut unbounded = false;
        for quad in quads {
            for v in quad {
                for (axis, &coord) in v.pos.iter().enumerate() {
                    let (lo, hi) = if (0.0..=16.0).contains(&coord) {
                        (
                            coord.floor().clamp(0.0, 15.0) as u8,
                            coord.ceil().clamp(1.0, 16.0) as u8,
                        )
                    } else if (16.0..=255.0).contains(&coord) {
                        (
                            coord.floor().clamp(16.0, 254.0) as u8,
                            coord.ceil().clamp(17.0, 255.0) as u8,
                        )
                    } else {
                        unbounded = true;
                        (0, 16)
                    };
                    min[axis] = min[axis].min(lo);
                    max[axis] = max[axis].max(hi);
                }
            }
            for axis in 0..3 {
                if min[axis] <= 254 {
                    max[axis] = max[axis].max(min[axis] + 1);
                }
            }
        }
        if min[0] > max[0] {
            ([0, 0, 0, 16, 16, 16], unbounded)
        } else {
            ([min[0], min[1], min[2], max[0], max[1], max[2]], unbounded)
        }
    }

    /// Reference single-quad `floor`/`ceil` quantization matching the original `SubAabbBuilder`.
    fn reference_sub_aabb(quad: &[super::super::TerrainVertex; 4]) -> ([u8; 6], bool) {
        reference_sub_aabb_multi(std::slice::from_ref(quad))
    }

    #[test]
    fn sub_aabb_builder_matches_reference_bit_for_bit() {
        // Empty builder returns conservative full-section bounds.
        assert_eq!(SubAabbBuilder::default().build(), [0, 0, 0, 16, 16, 16]);

        let coords: &[f32] = &[
            -100.0,
            -0.25,
            -0.0001,
            -0.0,
            0.0,
            f32::from_bits(1),
            1e-6,
            0.25,
            0.5,
            0.9999,
            1.0,
            1.0001,
            7.5,
            15.0,
            15.9999,
            16.0,
            16.0001,
            16.25,
            17.0,
            17.5,
            254.0,
            254.9999,
            255.0,
            255.0001,
            300.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];
        let mk = |pos: [f32; 3]| super::super::TerrainVertex {
            pos,
            uv: [0, 0],
            layer: super::super::OPAQUE_LAYER,
            light: 0,
            color: 0,
        };

        // Every single boundary coordinate on all 4 corners.
        for &c in coords {
            let quad = [mk([c, c, c]); 4];
            let (exp_aabb, exp_unb) = reference_sub_aabb(&quad);
            let mut b = SubAabbBuilder::default();
            b.include_vertices(&quad);
            assert_eq!(b.unbounded, exp_unb, "unbounded mismatch for {c:?}");
            assert_eq!(b.build(), exp_aabb, "sub_aabb mismatch for {c:?}");
        }

        // Pseudo-random mixed integer, fractional, negative, >16, >255, inf, and NaN quads.
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for case in 0..2000 {
            let quad: [super::super::TerrainVertex; 4] = std::array::from_fn(|_| {
                let p = [0, 1, 2].map(|_| coords[(next() as usize) % coords.len()]);
                mk(p)
            });
            let (exp_aabb, exp_unb) = reference_sub_aabb(&quad);
            let mut b = SubAabbBuilder::default();
            b.include_vertices(&quad);
            assert_eq!(b.unbounded, exp_unb, "case {case}");
            assert_eq!(b.build(), exp_aabb, "case {case}");
        }
    }

    #[test]
    fn sub_aabb_builder_multi_quad_documents_tighter_conservative_bounds() {
        let mk_vert = |pos: [f32; 3]| super::super::TerrainVertex {
            pos,
            uv: [0, 0],
            layer: super::super::OPAQUE_LAYER,
            light: 0xF0F0,
            color: 0xFFFF_FFFF,
        };
        let mk_quad = |corners: [[f32; 3]; 4]| corners.map(mk_vert);
        let build_new = |quads: &[[super::super::TerrainVertex; 4]]| {
            let mut b = SubAabbBuilder::default();
            for q in quads {
                b.include_vertices(q);
            }
            (b.build(), b.unbounded)
        };
        let assert_encloses_in_range =
            |quads: &[[super::super::TerrainVertex; 4]], aabb: [u8; 6], label: &str| {
                for axis in 0..3 {
                    assert!(
                        aabb[axis + 3] > aabb[axis],
                        "{label}: axis {axis} degenerate ({aabb:?})"
                    );
                }
                for q in quads {
                    for v in q {
                        for (axis, &coord) in v.pos.iter().enumerate() {
                            if (0.0..=255.0).contains(&coord) {
                                assert!(
                                    f32::from(aabb[axis]) <= coord
                                        && coord <= f32::from(aabb[axis + 3]),
                                    "{label}: axis {axis} coord {coord} outside {aabb:?}"
                                );
                            }
                        }
                    }
                }
            };

        // 1. Multi-plane front-to-back `+X` quads (`x = 10.0` then `x = 5.0`, with fractional Y/Z).
        // The old builder applied `max = max.max(min + 1)` after EACH quad, so the flat integer
        // quad at `x = 10.0` immediately inflated `max_x` to `11`, leaving a stale `[5, 11]` span
        // (and making the bound order-dependent: `[5, 10]` in reverse order). Deferring quantization
        // and non-degeneracy to `build()` yields the tighter, order-independent `[5, 10]` span
        // while conservatively enclosing every vertex in `[5.0, 10.0]`.
        let q_high = mk_quad([
            [10.0, 2.25, 4.5],
            [10.0, 3.75, 4.5],
            [10.0, 3.75, 5.25],
            [10.0, 2.25, 5.25],
        ]);
        let q_low = mk_quad([
            [5.0, 2.0, 4.0],
            [5.0, 3.0, 4.0],
            [5.0, 3.0, 5.0],
            [5.0, 2.0, 5.0],
        ]);
        assert_eq!(
            reference_sub_aabb_multi(&[q_high, q_low]),
            ([5, 2, 4, 11, 4, 6], false)
        );
        assert_eq!(
            reference_sub_aabb_multi(&[q_low, q_high]),
            ([5, 2, 4, 10, 4, 6], false)
        );
        let (new_fwd, unb_fwd) = build_new(&[q_high, q_low]);
        let (new_rev, unb_rev) = build_new(&[q_low, q_high]);
        assert_eq!((new_fwd, unb_fwd), ([5, 2, 4, 10, 4, 6], false));
        assert_eq!((new_rev, unb_rev), (new_fwd, unb_fwd));
        assert_encloses_in_range(&[q_high, q_low], new_fwd, "front-to-back +X");

        // End-to-end meshlet build after front-to-back sort produces the same tight bound.
        let mut sorted_solid: Vec<_> = [q_low, q_high].into_iter().flatten().collect();
        let sides = super::super::sides::sort_parts(&mut sorted_solid, false);
        assert_eq!(sorted_solid[0].pos[0], 10.0);
        assert_eq!(sorted_solid[4].pos[0], 5.0);
        let meshlets = build_section_meshlets(&sides, &sorted_solid, &[]);
        assert_eq!(meshlets.len(), 1);
        assert_eq!(meshlets[0].quad_count, 2);
        assert_eq!(meshlets[0].sub_aabb, [5, 2, 4, 10, 4, 6]);

        // 2. Coplanar multi-quad meshlet (all vertices on `x = 10.0`): deferred non-degeneracy
        // in `build()` still expands `[10, 10]` to `[10, 11]`, matching the old bound.
        let q_coplanar_a = mk_quad([
            [10.0, 2.0, 1.0],
            [10.0, 3.0, 1.0],
            [10.0, 3.0, 2.0],
            [10.0, 2.0, 2.0],
        ]);
        let q_coplanar_b = mk_quad([
            [10.0, 6.25, 4.0],
            [10.0, 7.5, 4.0],
            [10.0, 7.5, 5.0],
            [10.0, 6.25, 5.0],
        ]);
        assert_eq!(
            reference_sub_aabb_multi(&[q_coplanar_a, q_coplanar_b]),
            ([10, 2, 1, 11, 8, 5], false)
        );
        assert_eq!(
            build_new(&[q_coplanar_a, q_coplanar_b]),
            ([10, 2, 1, 11, 8, 5], false)
        );

        // 3. Out-of-section (`> 16.0`), fractional, section-edge (`16.0`), and negative/unbounded
        // multi-quad combinations.
        let q_17 = mk_quad([
            [17.0, 1.0, 1.0],
            [17.0, 2.0, 1.0],
            [17.0, 2.0, 2.0],
            [17.0, 1.0, 2.0],
        ]);
        let q_frac = mk_quad([
            [15.25, 1.25, 1.25],
            [15.75, 1.25, 1.25],
            [15.75, 1.75, 1.75],
            [15.25, 1.75, 1.75],
        ]);
        assert_eq!(
            reference_sub_aabb_multi(&[q_17, q_frac]),
            ([15, 1, 1, 18, 2, 2], false)
        );
        let (aabb_17_frac, unb_17_frac) = build_new(&[q_17, q_frac]);
        assert_eq!((aabb_17_frac, unb_17_frac), ([15, 1, 1, 17, 2, 2], false));
        assert_encloses_in_range(&[q_17, q_frac], aabb_17_frac, ">16 + fractional");

        let q_17_b = mk_quad([
            [17.0, 2.0, 2.0],
            [17.0, 3.0, 2.0],
            [17.0, 3.0, 3.0],
            [17.0, 2.0, 3.0],
        ]);
        let q_neg = mk_quad([
            [-0.25, 2.0, 2.0],
            [0.5, 2.0, 2.0],
            [0.5, 3.0, 3.0],
            [-0.25, 3.0, 3.0],
        ]);
        assert_eq!(
            reference_sub_aabb_multi(&[q_17_b, q_neg]),
            ([0, 2, 2, 18, 3, 3], true)
        );
        let (aabb_17_neg, unb_17_neg) = build_new(&[q_17_b, q_neg]);
        assert_eq!((aabb_17_neg, unb_17_neg), ([0, 2, 2, 17, 3, 3], true));
        assert_encloses_in_range(&[q_17_b, q_neg], aabb_17_neg, ">16 + negative");

        // Section-edge `16.0` maps its lower bound to `15`, so `[16.0, 15.0]` matches the old
        // bound `[15, 16]` whereas `[17.0, 16.0]` tightens from `[15, 18]` to `[15, 17]`.
        let q_16 = mk_quad([
            [16.0, 2.0, 2.0],
            [16.0, 3.0, 2.0],
            [16.0, 3.0, 3.0],
            [16.0, 2.0, 3.0],
        ]);
        let q_15 = mk_quad([
            [15.0, 2.0, 2.0],
            [15.0, 3.0, 2.0],
            [15.0, 3.0, 3.0],
            [15.0, 2.0, 3.0],
        ]);
        assert_eq!(
            reference_sub_aabb_multi(&[q_16, q_15]),
            ([15, 2, 2, 16, 3, 3], false)
        );
        assert_eq!(build_new(&[q_16, q_15]), ([15, 2, 2, 16, 3, 3], false));
        assert_eq!(
            reference_sub_aabb_multi(&[q_17_b, q_16]),
            ([15, 2, 2, 18, 3, 3], false)
        );
        assert_eq!(build_new(&[q_17_b, q_16]), ([15, 2, 2, 17, 3, 3], false));

        // 4. Randomized multi-quad invariant sweep: across 2..=6 quads (including flat integer
        // planes, fractional coords, negative/unbounded, >16, >255, inf, and NaN), the new
        // builder always matches `unbounded` and `min`, tightens `max` by at most 1 where per-quad
        // inflation fired, stays order-independent, and conservatively encloses every in-range
        // vertex.
        let coords: &[f32] = &[
            -0.5,
            -0.0,
            0.0,
            0.25,
            1.0,
            5.0,
            7.5,
            10.0,
            15.0,
            15.75,
            16.0,
            16.25,
            17.0,
            254.0,
            255.0,
            256.0,
            f32::INFINITY,
            f32::NAN,
        ];
        let mut state = 0xA076_1D64_78BD_642Fu64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut tighter_cases = 0usize;
        for case in 0..500 {
            let count = 2 + (next() as usize % 5);
            let mut quads = Vec::with_capacity(count);
            for _ in 0..count {
                let q = if next().is_multiple_of(2) {
                    // Flat integer plane on one axis to exercise per-quad inflation differences.
                    let flat_axis = (next() as usize) % 3;
                    let plane = coords[(next() as usize) % coords.len()];
                    std::array::from_fn(|_| {
                        let mut p = [0, 1, 2].map(|_| coords[(next() as usize) % coords.len()]);
                        p[flat_axis] = plane;
                        mk_vert(p)
                    })
                } else {
                    std::array::from_fn(|_| {
                        mk_vert([0, 1, 2].map(|_| coords[(next() as usize) % coords.len()]))
                    })
                };
                quads.push(q);
            }
            let (old_aabb, old_unb) = reference_sub_aabb_multi(&quads);
            let (new_aabb, new_unb) = build_new(&quads);
            assert_eq!(new_unb, old_unb, "case {case}: unbounded mismatch");
            for axis in 0..3 {
                assert_eq!(
                    new_aabb[axis], old_aabb[axis],
                    "case {case}: axis {axis} min mismatch (new={new_aabb:?}, old={old_aabb:?})"
                );
                assert!(
                    new_aabb[axis + 3] <= old_aabb[axis + 3]
                        && old_aabb[axis + 3] - new_aabb[axis + 3] <= 1,
                    "case {case}: axis {axis} max out of expected tightening range (new={new_aabb:?}, old={old_aabb:?})"
                );
            }
            if new_aabb != old_aabb {
                tighter_cases += 1;
            }
            assert_encloses_in_range(&quads, new_aabb, "random multi-quad");

            let mut reversed = quads;
            reversed.reverse();
            assert_eq!(
                build_new(&reversed),
                (new_aabb, new_unb),
                "case {case}: SubAabbBuilder must be order-independent"
            );
        }
        assert!(
            tighter_cases > 0,
            "randomized multi-quad sweep should exercise tighter bounds"
        );
    }

    #[test]
    fn spatial_clustering_shrinks_sub_aabb_volume_on_terrain_patterns() {
        let make_face = |corners: [[f32; 3]; 4], layer: u16| {
            corners.map(|pos| super::super::TerrainVertex {
                pos,
                uv: [0, 0],
                layer,
                light: 0xF0F0,
                color: 0xFFFF_FFFF,
            })
        };
        let cube_faces = |x: f32, y: f32, z: f32| {
            let (a, b, c) = (x + 1.0, y + 1.0, z + 1.0);
            [
                [[a, y, z], [a, b, z], [a, b, c], [a, y, c]], // +X
                [[x, y, z], [x, y, c], [x, b, c], [x, b, z]], // -X
                [[x, b, z], [x, b, c], [a, b, c], [a, b, z]], // +Y
                [[x, y, z], [a, y, z], [a, y, c], [x, y, c]], // -Y
                [[x, y, c], [a, y, c], [a, b, c], [x, b, c]], // +Z
                [[x, y, z], [x, b, z], [a, b, z], [a, y, z]], // -Z
            ]
        };

        // 1. Pure-opaque multi-wall cliff section (4 vertical +X walls at x=1, 5, 10, 15,
        // emitted in y -> z -> x scanline order as the mesher walks a section).
        let mut wall_scanline = Vec::new();
        for y in 0..16 {
            for z in 0..16 {
                for x in [0, 4, 9, 14] {
                    let faces = cube_faces(x as f32, y as f32, z as f32);
                    wall_scanline.extend(make_face(faces[0], super::super::OPAQUE_LAYER));
                }
            }
        }
        let avg_volume = |meshlets: &[MeshletDesc]| -> f64 {
            let sum: u64 = meshlets
                .iter()
                .map(|m| {
                    let mn = m.min_corner();
                    let mx = m.max_corner();
                    u64::from(mx[0] - mn[0]) * u64::from(mx[1] - mn[1]) * u64::from(mx[2] - mn[2])
                })
                .sum();
            sum as f64 / meshlets.len() as f64
        };
        // Baseline: pure bucket-only scanline order (no plane sort, no Morton).
        let raw_meshlets = build_section_meshlets(
            &Sides::unsorted((wall_scanline.len() / 4) as u32),
            &wall_scanline,
            &[],
        );
        let mut wall_sorted = wall_scanline;
        let wall_sides = super::super::sides::sort_parts(&mut wall_sorted, false);
        let sorted_meshlets = build_section_meshlets(&wall_sides, &wall_sorted, &[]);
        let raw_vol = avg_volume(&raw_meshlets);
        let sorted_vol = avg_volume(&sorted_meshlets);
        // Scanline mixes all 4 walls per Y-layer (x in 1..15 -> 14x1x16 = 224 vol) into every
        // meshlet; plane + 2D Morton isolates each plane into exact 1x8x8 = 64 vol quadrants
        // (3.5x smaller volume and 14x tighter X-axis span for GPU directional culling).
        assert!(
            sorted_vol * 3.0 < raw_vol,
            "expected >3x volume shrinkage on multi-wall pure-opaque terrain: raw={raw_vol}, sorted={sorted_vol}"
        );
        for m in &raw_meshlets {
            assert_eq!(m.max_corner()[0] - m.min_corner()[0], 14);
        }
        for m in &sorted_meshlets {
            let mn = m.min_corner();
            let mx = m.max_corner();
            assert_eq!(mx[0] - mn[0], 1, "each +X meshlet must lie on one plane");
            assert_eq!(mx[1] - mn[1], 8, "2D Morton must form 8x8 tiles");
            assert_eq!(mx[2] - mn[2], 8, "2D Morton must form 8x8 tiles");
        }

        // 2. Scattered `OTHER` cross-plants in two opposite octants across all Y layers:
        // scanline interleaves both octants, whereas 3D Morton isolates each octant cleanly.
        let mut other_scanline = Vec::new();
        for y in 0..16 {
            for (bx, bz) in [(1, 1), (2, 1), (13, 13), (14, 13)] {
                let (x, y, z) = (bx as f32, y as f32, bz as f32);
                let cross = [
                    [x + 0.15, y, z + 0.15],
                    [x + 0.85, y, z + 0.85],
                    [x + 0.85, y + 0.9, z + 0.85],
                    [x + 0.15, y + 0.9, z + 0.15],
                ];
                other_scanline.extend(make_face(cross, 0));
            }
        }
        // 64 quads -> emit 2 quads per block (128 total -> 2 meshlets of 64).
        let other_double: Vec<_> = other_scanline
            .iter()
            .copied()
            .chain(other_scanline.iter().copied())
            .collect();
        let raw_other = build_section_meshlets(
            &Sides::unsorted((other_double.len() / 4) as u32),
            &other_double,
            &[],
        );
        let mut sorted_other_verts = other_double;
        let other_sides = super::super::sides::sort_parts(&mut sorted_other_verts, false);
        let sorted_other = build_section_meshlets(&other_sides, &sorted_other_verts, &[]);
        assert!(
            avg_volume(&sorted_other) * 4.0 < avg_volume(&raw_other),
            "expected >4x volume shrinkage on OTHER cross-plants: raw={}, sorted={}",
            avg_volume(&raw_other),
            avg_volume(&sorted_other)
        );
    }

    #[test]
    fn split_sides_fast_path_matches_unsplit_cutout_meshlet_counts() {
        let mut solid = Vec::new();
        // Mix >64 opaque and >64 cutout quads across multiple axis buckets and `OTHER`
        // so both single-meshlet and multi-meshlet parts are exercised.
        for i in 0..90u32 {
            let x = (i % 16) as f32;
            let z = ((i / 16) % 16) as f32;
            let y = (i % 8) as f32;
            solid.extend(test_quad(x, y, z, super::super::OPAQUE_LAYER));
            solid.extend(test_quad(x, y + 1.0, z, 0));
            // Sloped `OTHER` quad (alternating opaque/cutout).
            let layer = if i.is_multiple_of(2) {
                super::super::OPAQUE_LAYER
            } else {
                0
            };
            let v = |px, py, pz| super::super::TerrainVertex {
                pos: [px, py, pz],
                uv: [0, 0],
                layer,
                light: 0xF0F0,
                color: 0xFFFF_FFFF,
            };
            solid.extend([
                v(x, y, z),
                v(x + 1.0, y, z),
                v(x + 1.0, y + 1.0, z + 1.0),
                v(x, y + 1.0, z + 1.0),
            ]);
        }

        for grouped in [false, true] {
            let mut sorted = solid.clone();
            let sides = super::super::sides::sort_parts(&mut sorted, grouped);
            assert!(sides.is_split());

            let ends = std::array::from_fn(|p| sides.part_end(p));
            let planes = std::array::from_fn(|b| sides.bucket_plane(b));
            let unsplit = Sides::from_parts(ends, sides.is_grouped(), false, planes);
            assert!(!unsplit.is_split());

            let fast = build_section_meshlets(&sides, &sorted, &[]);
            let scanned = build_section_meshlets(&unsplit, &sorted, &[]);
            assert_eq!(fast, scanned, "grouped={grouped}");

            let fast_opaque = fast.iter().filter(|m| !m.is_cutout()).count();
            let fast_cutout = fast.iter().filter(|m| m.is_cutout()).count();
            assert!(fast_opaque > 0 && fast_cutout > 0);
            assert_eq!(
                fast_opaque,
                scanned.iter().filter(|m| !m.is_cutout()).count()
            );
            assert_eq!(
                fast_cutout,
                scanned.iter().filter(|m| m.is_cutout()).count()
            );
        }
    }

    #[test]
    fn build_section_meshlets_with_bounds_matches_compute_bounds() {
        // 1. In-bounds solid + translucent vertices.
        let mut mesh = super::super::SectionMesh::default();
        mesh.solid
            .extend(test_quad(2.0, 3.0, 4.0, super::super::OPAQUE_LAYER));
        mesh.solid.extend(test_quad(15.0, 12.0, 14.0, 0));
        mesh.translucent.extend(test_quad(0.5, 1.0, 1.5, 0));
        let sides = super::super::sides::sort_parts(&mut mesh.solid, false);
        let mut out = Vec::new();
        let raw = build_section_meshlets_with_bounds(&sides, &mesh.solid, &[], &mut out)
            .expect("in-bounds solid mesh must yield fused bounds");
        assert_eq!(
            Some(mesh.finish_bounds(raw.0, raw.1)),
            mesh.compute_bounds()
        );

        // 2. Vertex extending below 0.0 sets `unbounded` and yields `None` so caller falls back.
        let mut neg_quad = test_quad(0.0, 2.0, 2.0, super::super::OPAQUE_LAYER);
        neg_quad[0].pos[0] = -0.25;
        let neg_solid: Vec<_> = neg_quad.into_iter().collect();
        let neg_sides = Sides::unsorted(1);
        let raw_neg = build_section_meshlets_with_bounds(&neg_sides, &neg_solid, &[], &mut out);
        assert!(raw_neg.is_none());
        assert!(out[0].is_unbounded());
    }
}
