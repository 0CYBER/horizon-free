//! Output-signal notifications for block entities the caller owns.
use super::{Behaviours, LevelAccess, Ticks};
use horizon_core::position::BlockPos;

impl Behaviours {
    /// `BlockEntity.setChanged`: a container at `pos` changed its contents, so
    /// comparators reading it (`Level.updateNeighbourForOutputSignal`) update.
    pub fn container_changed<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        let state = level.block_state(pos);
        if !self.is_air(state) {
            let block = self.block(state);
            self.sim(level, ticks)
                .update_neighbour_for_output_signal(pos, block);
        }
    }
}
