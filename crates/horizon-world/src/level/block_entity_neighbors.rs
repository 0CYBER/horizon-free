use super::{BlockPos, BlockStateId};

/// Notifications from the block entity comparator-invalidation walk.
/// Worlds without subscribers retain vanilla behaviour through these defaults.
pub trait BlockEntityNeighbors {
    /// A block entity changed beside this cell, after vanilla comparator processing.
    fn observe_block_entity_neighbor(
        &self,
        _pos: BlockPos,
        _state: BlockStateId,
        _neighbor: BlockPos,
    ) {
    }
    /// Whether this cell receives changes through an intervening conductor.
    fn receives_weak_changes(&self, _pos: BlockPos, _state: BlockStateId) -> bool {
        false
    }
}
