use super::*;

impl GameLevel<'_> {
    /// The block at `pos` (`state`) is about to tell its neighbours it
    /// changed (NeoForge's `BlockEvent.NeighborNotifyEvent`): at
    /// `ServerLevel.updateNeighborsAt`, which notifies all six sides
    /// (`skipped_side` is `None`), and `updateNeighborsAtExceptFromFacing`,
    /// which notifies all but `skipped_side`. Set `cancel` and no neighbour
    /// hears, on both paths. Asked for every such update, redstone
    /// included: answer fast. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_neighbor_notify(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        skipped_side: Option<Direction>,
        cancel: &mut bool,
    ) {
        self.fire_neighbor_notify(dimension, pos, state, skipped_side, cancel);
    }
}
