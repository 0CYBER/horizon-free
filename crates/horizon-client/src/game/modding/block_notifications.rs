//! A client block notification is not a save mutation.
use super::Game;

impl Game<'_> {
    pub(super) fn notify_block_clients(&mut self, pos: (i32, i32, i32)) {
        self.world
            .path_type_block_updated(horizon_core::position::BlockPos::new(pos.0, pos.1, pos.2));
        if let Some(log) = self.world.block_log.as_mut()
            && !log.contains(&pos)
        {
            log.push(pos);
        }
    }
}
