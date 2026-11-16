//! Cleanup of constructor state that never joined a resident owner.
use super::*;
impl Game<'_> {
    pub(super) fn discard_unjoined_mob<T>(&mut self, id: u32) -> Option<T> {
        self.forget_entity(id);
        None
    }
}

impl Game<'_> {
    pub(in crate::game) fn clear_pending_mob_actions(&mut self) {
        self.creatures.pending.clear();
    }
}
