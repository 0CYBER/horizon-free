use super::*;

impl GameLevel<'_> {
    /// `Block.affectNeighborsAfterRemoval` for a mod's block: an authoritative server removed the
    /// block `state` from `pos` (`moved`: a piston pushed it) and has run its template class's own
    /// updates. Push onto `updates` the neighbour updates it wants, in order: `OutputSignal`
    /// (`updateNeighbourForOutputSignal`), `Neighbours` (`updateNeighborsAt`) and
    /// `NeighboursExcept(direction)` (`updateNeighborsAtExceptFromFacing`); the level runs each
    /// at once, as the block's own override would. Answered from inside the level's write: it
    /// reads, and cannot write blocks or spawn entities.
    #[inline(always)]
    pub(in crate::game) fn native_affect_neighbors_after_removal(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        moved: bool,
        updates: &mut Vec<NeighbourUpdate>,
    ) {
        self.fire_affect_neighbors_after_removal(dimension, pos, state, moved, updates);
    }
}
