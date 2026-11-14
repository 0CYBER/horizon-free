use super::*;

impl GameLevel<'_> {
    /// `IBlockExtension.onBlockStateChange`: a write through `Level.setBlock` held and the
    /// block at `pos` went from `old` to `new`; the parts of `state`'s block hear it, at the
    /// end of the write. Both sides; the level is read-only.
    #[inline(always)]
    pub(in crate::game) fn native_on_block_state_change(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        old: BlockStateId,
        new: BlockStateId,
        predicted: bool,
    ) {
        self.fire_on_block_state_change(dimension, pos, state, old, new, predicted);
    }

    /// The states whose parts hear a change from `old` to `new`: `old`'s, then `new`'s when
    /// the block changed.
    fn state_change_listeners(
        &self,
        old: BlockStateId,
        new: BlockStateId,
    ) -> impl Iterator<Item = BlockStateId> {
        let changed = self.behaviours.block(old) != self.behaviours.block(new);
        [Some(old), changed.then_some(new)]
            .into_iter()
            .flatten()
            .filter(|state| {
                crate::game::api_values::block_state::block_listens(
                    mod_api::HookId::OnBlockStateChange,
                    *state,
                )
            })
    }

    /// Whether any part hears a change from `old` to `new`.
    pub(in crate::game) fn block_state_change_heard(
        &self,
        old: BlockStateId,
        new: BlockStateId,
    ) -> bool {
        mod_api::__private::has_scoped_hook(mod_api::HookId::OnBlockStateChange)
            && self.state_change_listeners(old, new).next().is_some()
    }

    /// `LevelAccess::observe_block_state_change`, and a client's own block writes: the parts
    /// of `old`'s block hear the change, then the parts of `new`'s block when the block
    /// changed. A panicking part is contained with a warning.
    pub(in crate::game) fn block_state_change(
        &self,
        pos: BlockPos,
        old: BlockStateId,
        new: BlockStateId,
        predicted: bool,
    ) {
        if old == new || !mod_api::__private::has_scoped_hook(mod_api::HookId::OnBlockStateChange) {
            return;
        }
        for state in self.state_change_listeners(old, new) {
            if let Err(error) = horizon_client::crash::contain(|| {
                self.native_on_block_state_change(self.dimension, pos, state, old, new, predicted);
            }) {
                log::warn!("on_block_state_change callback failed: {error}");
            }
        }
    }
}
