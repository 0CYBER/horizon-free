use super::*;

pub(super) struct ItemTossHooks;
impl ItemTossHooks {
    pub(super) fn item_toss(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ItemTossEvent,
    ) -> mod_api::Flow {
        TOSSED.set(Some((event.player, event.item.entity().id())));
        mod_api::Flow::Cancel
    }
}
