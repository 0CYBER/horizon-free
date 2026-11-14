use super::*;

pub(super) struct ItemFishedHooks;
impl ItemFishedHooks {
    pub(super) fn item_fished(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ItemFishedEvent<'_>,
    ) -> mod_api::Flow {
        assert_eq!(event.fished.hook, 19);
        assert_eq!(event.drops.len(), 1);
        event.drops.clear();
        event
            .drops
            .push(mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 2).unwrap());
        event.rod_damage = 5;
        mod_api::Flow::Continue
    }
}
