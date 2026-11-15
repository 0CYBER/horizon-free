use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The block's storage calls `setChanged` (`Container.setChanged` and
    /// `BlockEntity.setChanged`), after its contents were written and before
    /// dirty/comparator updates. A native transaction tells it once per backing
    /// block at the root commit, never for an abort or a simulation. Explicit
    /// `set_changed` also tells it, even if the contents compare equal. Writes
    /// inside this notice still apply, but do not re-enter it for the same block;
    /// a write to another backing block has its own notice.
    ///
    /// A server-only, non-cancellable BlockPart notice, not a global bus event.
    /// `level` and `pos` name the backing block, not an automation entity or its
    /// menu's placeholder coordinates. A double container tells each component;
    /// the comparator refresh of its other half is not another notice. Replaces
    /// Create's positional `contents_changed` injection for depots, packages and
    /// schematic inventories; NeoForge has no corresponding bus event.
    pub(in crate::game) fn native_contents_changed(
        &mut self,
        level: LevelRef,
        pos: BlockPos,
        state: BlockStateId,
    ) {
        if self.contents_changing.contains(&(level, pos)) {
            return;
        }
        self.contents_changing.push((level, pos));
        let notice = ContentsChangedGuard { game: self };
        notice.game.fire_contents_changed(level, pos, state);
    }
}
