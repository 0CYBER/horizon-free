use super::*;

impl Game<'_> {
    /// Declare that a registered block entity type uses actual-state creation.
    /// Asked once per block per level. Set true and never clear an earlier answer.
    pub(in crate::game) fn native_stateful_block_entity(&self, block: &str, stateful: &mut bool) {
        self.fire_stateful_block_entity(block, stateful);
    }
}
