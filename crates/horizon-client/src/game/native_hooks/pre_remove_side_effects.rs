use super::*;
use mod_api::ModBlockEntity;

impl GameLevel<'_> {
    /// `BlockEntity.preRemoveSideEffects` of a mod's block entity: an
    /// authoritative server is removing `entity` from `pos`, where the block
    /// `state` stood, and its updates may have side effects (not under flag
    /// 256, `UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS`). Put in `drops` the stacks
    /// it held that the game doesn't: they scatter as `Containers.dropItemStack`
    /// scatters them, after the block entity's own container. The block entity
    /// is still in place, so its attachments read as they were. Answered from
    /// inside the level's write: don't change the world here.
    #[inline(always)]
    pub(in crate::game) fn native_pre_remove_side_effects(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        entity: &ModBlockEntity,
        drops: &mut Vec<ItemStack>,
    ) {
        self.fire_pre_remove_side_effects(pos, state, entity, drops);
    }
}
