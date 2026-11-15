use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `PlayerInteractEvent.RightClickItem`, immediately before `Item.use`.
    /// Cancel skips its Part and vanilla behaviour; its result defaults to
    /// `Pass`. On the client `Success`, `SuccessServer` and `Consume` end the
    /// hand loop; only `Success` predicts a swing.
    pub(in crate::game) fn native_right_click_item(
        &mut self,
        click: &ItemClick<'_>,
        cancellation_result: &mut mod_api::InteractionResult,
        canceled: &mut bool,
    ) {
        self.fire_right_click_item(click, cancellation_result, canceled);
    }
}
