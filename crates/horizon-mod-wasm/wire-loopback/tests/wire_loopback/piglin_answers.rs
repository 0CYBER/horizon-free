use super::*;
use horizon_core::nbt::{Compound, JavaString, Tag};
thread_local! {
    static COIN: std::cell::Cell<Option<mod_api::ItemKey>> = const { std::cell::Cell::new(None) };
    static WEARER: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}
/// Whether `stack` carries `minecraft:custom_data`: the guest reads the components the wire
/// carried across.
fn coin(stack: &mod_api::ItemStack) -> bool {
    stack.components().is_some_and(|c| {
        c.as_nbt()
            .contains_key(&JavaString::from("minecraft:custom_data"))
    })
}
struct Part;
impl mod_api::ItemPart for Part {
    fn item_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[
            Some(mod_api::HookId::IsPiglinCurrency),
            Some(mod_api::HookId::MakesPiglinsNeutral),
        ]
    }
    fn is_piglin_currency(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::IsPiglinCurrencyEvent<'_>,
    ) {
        event.currency = coin(event.stack);
    }
    fn makes_piglins_neutral(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::MakesPiglinsNeutralEvent<'_>,
    ) {
        WEARER.set(Some(event.wearer.id()));
        event.neutral = coin(event.stack);
    }
}
pub(super) fn register(registrar: &mut dyn mod_api::Registrar) {
    let item = registrar.item_key("minecraft:emerald");
    COIN.set(Some(item));
    registrar.item_part(item, Part);
}
#[test]
fn generated_guest_item_dispatch_writes_back_both_piglin_answers() {
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    install_counter();
    let plain = mod_api::ItemStack::new(COIN.get().unwrap(), 1).unwrap();
    let mut coined = plain.clone();
    coined
        .set_components(Compound::from([(
            JavaString::from("minecraft:custom_data"),
            Tag::Compound(Compound::from([(JavaString::from("coin"), Tag::Byte(1))])),
        )]))
        .unwrap();
    let wearer = mod_api::LivingRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(3), 91));
    for (stack, answer) in [(&coined, true), (&plain, false)] {
        for vanilla in [false, true] {
            let mut currency = vanilla;
            mod_api::dispatch_is_piglin_currency(&World, stack, &mut currency);
            assert_eq!(currency, answer);
            WEARER.set(None);
            let mut neutral = vanilla;
            mod_api::dispatch_makes_piglins_neutral(&World, stack, wearer, &mut neutral);
            assert_eq!(neutral, answer);
            assert_eq!(WEARER.get(), Some(91));
        }
    }
}

pub(super) struct PiglinAnswersHooks;
impl PiglinAnswersHooks {
    pub(super) fn register(
        &self,
        registrar: &mut dyn mod_api::Registrar,
        cx: &mod_api::RegistrationContext,
    ) {
        self::register(registrar);
    }
}
