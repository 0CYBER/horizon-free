use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `IItemExtension.onItemUseFirst`: the held item's first say on a click
    /// on a block, after `use_item_on_block` (`ItemBeforeBlock`) passed and
    /// before the block's `useItemOn`. An answer ends the click; `Pass` goes
    /// on to the block.
    pub(in crate::game) fn native_on_item_use_first(
        &mut self,
        stack: &mod_api::ItemStack,
        click: &BlockClick<'_>,
        answer: &mut Option<mod_api::InteractionResult>,
    ) {
        self.fire_on_item_use_first(stack, click, answer);
    }
}
