//! Horizontal generation write-zone predicate from the pinned chunk step.
use horizon_core::position::{BlockPos, ChunkPos};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationWriteZone {
    center: ChunkPos,
    radius: i32,
}
impl GenerationWriteZone {
    /// The caller supplies the center chunk and the generation step's write
    /// radius. Negative radii and wrapped coordinates retain Java semantics.
    pub fn new(center: ChunkPos, radius: i32) -> Self {
        Self { center, radius }
    }
    pub fn contains_chunk(self, chunk: ChunkPos) -> bool {
        self.center.x.wrapping_sub(chunk.x).wrapping_abs() <= self.radius
            && self.center.z.wrapping_sub(chunk.z).wrapping_abs() <= self.radius
    }
    pub fn contains_block(self, position: BlockPos) -> bool {
        self.contains_chunk(position.chunk())
    }
}

/// The center chunk's generation height while an upgrade is active. Normal
/// generation has no vertical write-policy gate (storage bounds are separate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpgradeWriteHeight {
    min_y: i32,
    max_y: i32,
}
impl UpgradeWriteHeight {
    pub const BELOW_ZERO: Self = Self {
        min_y: -64,
        max_y: -1,
    };
    pub fn new(min_y: i32, height: i32) -> Self {
        Self {
            min_y,
            max_y: min_y.wrapping_add(height).wrapping_sub(1),
        }
    }
    pub fn contains(self, y: i32) -> bool {
        y >= self.min_y && y <= self.max_y
    }
}
