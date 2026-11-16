use super::*;

pub(super) struct ItemPickedUpHooks;
impl ItemPickedUpHooks {
    pub(super) fn item_picked_up_post(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ItemPickedUpPostEvent<'_>,
    ) {
        PICKED_UP_POST.set(Some((
            event.player,
            event.item.entity().id(),
            event.original.count(),
            event.remaining.as_ref().map(|rest| rest.count()),
        )));
        let mut rest = event.original.clone();
        rest.set_count(3).unwrap();
        *event.remaining = Some(rest);
    }
    pub(super) fn item_picked_up_partial(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ItemPickedUpPartialEvent<'_>,
    ) {
        PICKED_UP_PARTIAL.set(Some((
            event.player,
            event.original.count(),
            event.remaining.as_ref().map(|rest| rest.count()),
        )));
        *event.remaining = None;
    }
}
