//! Sub-section meshlet partitioning (#17243 step 1): groups quads within each section into
//! clusters of up to 64 quads (256 vertices, 128 triangles), computing a compact 16-byte
//! descriptor (`MeshletDesc`) per meshlet.
//!
//! Preserves `mesher/sides.rs` bucket ordering and the 24-byte compact layout (`mesher/packed.rs`).
//! Each descriptor packs:
//! - `quad_offset`: u32 (starting quad index in section / arena page)
//! - `quad_count`: u8 (1..=64)
//! - `bucket`: u8 (0..7, corresponding to `Sides::BUCKETS`)
//! - `flags`: u8 (bit 0: cutout quad, bit 1: full-quad vertex fallback)
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
    /// Flags: bit 0 = is_cutout, bit 1 = contains_full_vertices.
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

    /// Quantized min corner in section coordinates [0..16].
    pub fn min_corner(&self) -> [u8; 3] {
        [self.sub_aabb[0], self.sub_aabb[1], self.sub_aabb[2]]
    }

    /// Quantized max corner in section coordinates [0..16].
    pub fn max_corner(&self) -> [u8; 3] {
        [self.sub_aabb[3], self.sub_aabb[4], self.sub_aabb[5]]
    }
}

/// Bounding box helper for building quantized sub-AABBs [0..16].
#[derive(Clone, Copy, Debug)]
pub struct SubAabbBuilder {
    pub min: [u8; 3],
    pub max: [u8; 3],
}

impl Default for SubAabbBuilder {
    fn default() -> Self {
        Self {
            min: [16, 16, 16],
            max: [0, 0, 0],
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

/// Partitions a section's quads into meshlets conforming to `Sides` bucket and part boundaries.
pub fn build_meshlets(
    sides: &Sides,
    quad_bounds: impl Fn(u32) -> (u8, u8, u8),
) -> Vec<MeshletDesc> {
    let mut meshlets = Vec::new();
    let total_quads = sides.quads();
    if total_quads == 0 {
        return meshlets;
    }

    // Partition by bucket runs as defined by Sides
    let mut prev_end = 0u32;
    for part_idx in 0..14 {
        let (bucket, is_cutout) = sides_slot(sides.is_grouped(), part_idx);
        let end = sides.part_end(part_idx);
        if end <= prev_end {
            continue;
        }

        let flags = if is_cutout {
            MeshletDesc::FLAG_CUTOUT
        } else {
            0
        };
        let plane_quant = quantize_plane(sides.bucket_plane(bucket));

        let mut offset = prev_end;
        while offset < end {
            let count = (end - offset).min(MAX_MESHLET_QUADS as u32) as u8;
            let mut aabb = SubAabbBuilder::default();
            for q in offset..(offset + count as u32) {
                let (bx, by, bz) = quad_bounds(q);
                aabb.include_block(bx, by, bz);
            }

            meshlets.push(MeshletDesc::new(
                offset,
                count,
                bucket as u8,
                flags,
                plane_quant,
                aabb.build(),
            ));
            offset += count as u32;
        }

        prev_end = end;
    }

    meshlets
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
}
