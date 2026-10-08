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

impl Behaviours {
    /// The update half of `Level.setBlock` for a state the caller already
    /// wrote to storage (entity landing rules write blocks directly):
    /// neighbour updates, then neighbour shape updates, per `flags`.
    pub fn notify_written<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        flags: i32,
    ) {
        use super::flags as f;
        let state = level.block_state(pos);
        let mut sim = self.sim(level, ticks);
        if flags & f::UPDATE_NEIGHBORS != 0 {
            sim.update_neighbors_at(pos, self.block(state));
        }
        if flags & f::UPDATE_KNOWN_SHAPE == 0 {
            let neighbour_flags = flags & !(f::UPDATE_NEIGHBORS | f::UPDATE_SUPPRESS_DROPS);
            sim.update_neighbour_shapes(state, pos, neighbour_flags, f::UPDATE_LIMIT - 1);
        }
    }
}
