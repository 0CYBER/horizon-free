use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `UseItemOnBlockEvent`: after a phase was chosen, before its Part or
    /// vanilla behaviour. Cancel skips that behaviour and returns
    /// `cancellation_result` (initially `TryWithEmptyHand`). In `ItemBeforeBlock`,
    /// `Pass` continues to the block, skipping the item's `on_item_use_first`
    /// Part; only `Fail` or a consuming result ends the block interaction.
    /// A block-phase `TryWithEmptyHand` can still try the empty-hand behaviour.
    pub(in crate::game) fn native_use_item_on_block(
        &mut self,
        click: &BlockClick<'_>,
        phase: UsePhase,
        cancellation_result: &mut mod_api::InteractionResult,
        canceled: &mut bool,
    ) {
        self.fire_use_item_on_block(click, phase, cancellation_result, canceled);
    }
}
