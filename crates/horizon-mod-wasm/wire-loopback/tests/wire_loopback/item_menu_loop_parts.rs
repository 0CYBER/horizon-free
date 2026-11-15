use super::*;
thread_local! {
    static SHARD: std::cell::Cell<Option<mod_api::ItemKey>> = const { std::cell::Cell::new(None) };
    static HEARD: std::cell::RefCell<Vec<(&'static str, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// A part on the amethyst shard: the grindstone repairs it, the crafting grid doesn't combine
/// it, and it fits inside container items only on the client.
struct Part;
impl mod_api::ItemPart for Part {
    fn item_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[
            Some(mod_api::HookId::CanGrindstoneRepair),
            Some(mod_api::HookId::IsCombineRepairable),
            Some(mod_api::HookId::CanFitInsideContainerItems),
        ]
    }
    fn can_grindstone_repair(
        &self,
        context: &dyn LoopContext,
        event: &mut mod_api::CanGrindstoneRepairEvent<'_>,
    ) {
        assert_eq!(context.side(), mod_api::LoopSide::Server);
        HEARD.with_borrow_mut(|heard| heard.push(("grindstone", event.can_repair)));
        event.can_repair = true;
    }
    fn is_combine_repairable(
        &self,
        _: &dyn LoopContext,
        event: &mut mod_api::IsCombineRepairableEvent<'_>,
    ) {
        HEARD.with_borrow_mut(|heard| heard.push(("combine", event.repairable)));
        event.repairable = false;
    }
    fn can_fit_inside_container_items(
        &self,
        context: &dyn LoopContext,
        event: &mut mod_api::CanFitInsideContainerItemsEvent<'_>,
    ) {
        assert_eq!(event.stack.count(), 3);
        HEARD.with_borrow_mut(|heard| heard.push(("fits", event.fits)));
        event.fits = context.side() == mod_api::LoopSide::Client;
    }
}
pub(super) fn register(registrar: &mut dyn mod_api::Registrar) {
    let item = registrar.item_key("minecraft:amethyst_shard");
    SHARD.set(Some(item));
    registrar.item_part(item, Part);
}
#[test]
fn generated_item_loop_dispatch_starts_from_vanilla_and_writes_back_the_answers() {
    install_counter();
    let shard = mod_api::ItemStack::new(SHARD.get().unwrap(), 3).unwrap();
    for vanilla in [false, true] {
        let mut can_repair = vanilla;
        mod_api::dispatch_can_grindstone_repair(&Context, &shard, &mut can_repair);
        assert!(can_repair);
        let mut repairable = vanilla;
        mod_api::dispatch_is_combine_repairable(&Context, &shard, &mut repairable);
        assert!(!repairable);
        let mut fits = vanilla;
        mod_api::dispatch_can_fit_inside_container_items(&Context, &shard, &mut fits);
        // `Context` is the server's loop.
        assert!(!fits);
        assert_eq!(
            HEARD.with_borrow_mut(std::mem::take),
            [
                ("grindstone", vanilla),
                ("combine", vanilla),
                ("fits", vanilla)
            ]
        );
    }
}

pub(super) struct ItemMenuLoopPartsHooks;
impl ItemMenuLoopPartsHooks {
    pub(super) fn register(
        &self,
        registrar: &mut dyn mod_api::Registrar,
        cx: &mod_api::RegistrationContext,
    ) {
        self::register(registrar);
    }
}
