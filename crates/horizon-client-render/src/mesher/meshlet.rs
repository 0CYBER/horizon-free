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

/// Bounding box helper for building quantized sub-AABBs [0..16] (or conservative bounds up to 255).
#[derive(Clone, Copy, Debug)]
pub struct SubAabbBuilder {
    pub min: [u8; 3],
    pub max: [u8; 3],
    pub unbounded: bool,
}

impl Default for SubAabbBuilder {
    fn default() -> Self {
        Self {
            min: [u8::MAX; 3],
            max: [0, 0, 0],
            unbounded: false,
        }
    }
}

impl SubAabbBuilder {
    /// Expands the bounding box with a block coordinate inside a 16x16x16 section.
    pub fn include_block(&mut self, x: u8, y: u8, z: u8) {
        let x = x.min(15);
        let y = y.min(15);
        let z = z.min(15);
        self.min[0] = self.min[0].min(x);
        self.min[1] = self.min[1].min(y);
        self.min[2] = self.min[2].min(z);
        self.max[0] = self.max[0].max(x + 1);
        self.max[1] = self.max[1].max(y + 1);
        self.max[2] = self.max[2].max(z + 1);
    }

    /// Expands the bounding box using a quad's four section-local vertices.
    pub fn include_vertices(&mut self, quad: &[super::TerrainVertex; 4]) {
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
                    self.unbounded = true;
                    (0, 16)
                };
                self.min[axis] = self.min[axis].min(lo);
                self.max[axis] = self.max[axis].max(hi);
            }
        }
        for axis in 0..3 {
            if self.min[axis] <= 254 {
                self.max[axis] = self.max[axis].max(self.min[axis] + 1);
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
    pub fn build(self) -> [u8; 6] {
        if self.min[0] > self.max[0] {
            [0, 0, 0, 16, 16, 16]
        } else {
            [
                self.min[0],
                self.min[1],
                self.min[2],
                self.max[0],
                self.max[1],
                self.max[2],
            ]
        }
    }
}

/// Shared partitioning loop preserving `Sides` bucket boundaries and optional per-quad
/// cutout splitting (`quad_cutout`), chunking homogeneous runs into <= `MAX_MESHLET_QUADS`.
fn partition_meshlets(
    sides: &Sides,
    quad_cutout: impl Fn(u32, bool) -> bool,
    mut build_range: impl FnMut(u32, u8) -> (u8, [u8; 6]),
) -> Vec<MeshletDesc> {
    let mut meshlets = Vec::new();
    let total_quads = sides.quads();
    if total_quads == 0 {
        return meshlets;
    }

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
            let run_cutout = quad_cutout(run_start, is_cutout);
            let mut run_end = run_start + 1;
            while run_end < end && quad_cutout(run_end, is_cutout) == run_cutout {
                run_end += 1;
            }

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

    meshlets
}

/// Partitions a section's quads into meshlets conforming to `Sides` bucket and part boundaries.
pub fn build_meshlets(
    sides: &Sides,
    quad_bounds: impl Fn(u32) -> (u8, u8, u8),
) -> Vec<MeshletDesc> {
    partition_meshlets(
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
    )
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
/// and optional compact 24-byte `packed` records (`mesher/packed.rs`).
///
/// Guarantees every emitted meshlet descriptor is homogeneous in `FLAG_CUTOUT`: even when given an
/// unsorted mesh (`Sides::unsorted`), contiguous opaque and cutout runs are split into separate
/// descriptors so `ts_terrain` pass filtering matches per-quad `opaque_quad`.
pub fn build_section_meshlets(
    sides: &Sides,
    solid: &[super::TerrainVertex],
    packed: &[[u32; 6]],
) -> Vec<MeshletDesc> {
    partition_meshlets(
        sides,
        |q, default_cutout| quad_is_cutout(q, default_cutout, solid, packed),
        |offset, count| {
            let mut aabb = SubAabbBuilder::default();
            let mut extra_flags = 0u8;
            for q in offset..(offset + u32::from(count)) {
                extra_flags |= include_section_quad(q, solid, packed, &mut aabb);
            }
            if aabb.unbounded {
                extra_flags |= MeshletDesc::FLAG_UNBOUNDED;
            }
            (extra_flags, aabb.build())
        },
    )
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
}
