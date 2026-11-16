//! Save visibility while authoritative rows are lent out.
use super::*;

impl Game<'_> {
    /// Keep a held-out entity inaccessible until its copy has been stored back.
    pub(in crate::game) fn with_entity_held_out<R>(
        &mut self,
        id: u32,
        action: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let _save_lease = horizon_client::join_seq::visibility::PassLease::enter();
        if mod_api::__private::mods().is_empty() {
            return action(self);
        }
        self.held_out.push(id);
        let result = action(self);
        self.held_out.pop();
        result
    }
}
