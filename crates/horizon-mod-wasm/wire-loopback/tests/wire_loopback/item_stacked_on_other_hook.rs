use super::*;

pub(super) struct ItemStackedOnOtherHooks;
impl ItemStackedOnOtherHooks {
    pub(super) fn item_stacked_on_other(
        &self,
        _: &dyn LoopContext,
        event: &mut mod_api::ItemStackedOnOtherEvent<'_, '_>,
    ) -> mod_api::Flow {
        *event.event.carried.0 = None;
        event.event.cancellation_result = true;
        mod_api::Flow::Continue
    }
}
