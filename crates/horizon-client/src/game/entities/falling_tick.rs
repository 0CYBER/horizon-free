//! `falling_tick`: a falling block's parts hear each tick it ends still in the air, once the
//! ordered pass that ticked it is over (the falling block is lent out during its own tick).
use super::Game;
use horizon_client::join_seq::JoinSeq;
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    /// Record that the falling block `id` (`join`) of `state` ended its tick still falling,
    /// when a part listens to its state.
    pub(super) fn note_falling_tick(&mut self, id: u32, join: JoinSeq, state: BlockStateId) {
        if super::api_values::block_state::block_listens(mod_api::HookId::FallingTick, state) {
            self.ordered.falling_ticked.push((id, join));
        }
    }

    /// Deliver the recorded falling ticks in order, each to the parts of the falling block's
    /// state as it is now; one gone since is skipped.
    pub(super) fn deliver_falling_ticks(&mut self) {
        if self.ordered.falling_ticked.is_empty() {
            return;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            self.ordered.falling_ticked.clear();
            return;
        };
        let mut ticked = std::mem::take(&mut self.ordered.falling_ticked);
        for (id, join) in ticked.drain(..) {
            let Some(i) = self
                .falling
                .locate(join)
                .filter(|&i| self.falling[i].id == id)
            else {
                continue;
            };
            let (state, p) = (self.falling[i].state, self.falling[i].position);
            let pos = BlockPos::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
            self.native_falling_tick(state, pos, mod_api::EntityRef::new(epoch, id));
        }
        self.ordered.falling_ticked = ticked;
    }
}
