use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `BlockBehaviour.useWithoutItem`: the captured state's empty-hand method,
    /// after useItemOn asks for it, on the server and main hand only.
    pub(in crate::game) fn native_use_without_item(
        &mut self,
        state: BlockStateId,
        click: &BlockClick<'_>,
        answer: &mut Option<mod_api::InteractionResult>,
    ) {
        self.fire_use_without_item(state, click, answer);
    }
}
