use super::*;

impl Game<'_> {
    /// `IBlockExtension.canEntityDestroy`: the parts of `state` at `pos` may answer whether
    /// `entity` may destroy it. The host is read-only.
    #[inline(always)]
    pub(in crate::game) fn native_can_entity_destroy(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        entity: EntityRef,
        may_destroy: &mut Option<bool>,
    ) {
        self.fire_can_entity_destroy(pos, state, entity, may_destroy);
    }
}
