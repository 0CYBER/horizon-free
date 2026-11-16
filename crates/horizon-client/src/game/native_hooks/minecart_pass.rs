use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A minecart passed over a rail with a part for it. The part gets `&mut dyn ServerHost`
    /// once the cart's batch has ticked, so its writes are not lost to the cart's own tick.
    pub(in crate::game) fn native_minecart_pass(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
        entity: EntityRef,
    ) {
        self.fire_minecart_pass(state, pos, entity);
    }
}
