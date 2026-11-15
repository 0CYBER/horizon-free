use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `IDispensibleContainerItemExtension.emptyContents`: `stack` is about
    /// to be emptied at `pos` by `player`, who clicked `hit`, or by a
    /// dispenser (both `None`). `None` keeps vanilla's emptying; `Some(true)`
    /// the part placed the contents, `Some(false)` it failed.
    pub(in crate::game) fn native_empty_contents(
        &mut self,
        player: Option<PlayerRef>,
        pos: BlockPos,
        hit: Option<(BlockPos, Direction)>,
        stack: &ItemStack,
        answer: &mut Option<bool>,
    ) {
        self.fire_empty_contents(player, pos, hit, stack, answer);
    }
}

impl Game<'_> {
    /// Whether `item` has an `empty_contents` Part.
    pub(in crate::game) fn empty_contents_listens(&self, item: &str) -> bool {
        mod_api::__private::has_part_hook(mod_api::HookId::EmptyContents)
            && mod_api::__private::item_listens(mod_api::HookId::EmptyContents, item)
    }
    /// `emptyContents` for `stack` emptied at `pos`, by the acting player when
    /// `hit` names what they clicked, else by a dispenser: the Part's answer,
    /// `None` for vanilla's (and for an item without the Part).
    pub(in crate::game) fn native_empty_contents_event(
        &mut self,
        pos: BlockPos,
        hit: Option<(BlockPos, Direction)>,
        stack: &ItemStack,
    ) -> Option<bool> {
        if !self.empty_contents_listens(&stack.item) {
            return None;
        }
        let player = hit.and_then(|_| self.native_acting_player());
        let mut answer = None;
        self.native_empty_contents(player, pos, hit, stack, &mut answer);
        answer
    }
}
