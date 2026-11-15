#![cfg(feature = "testing")]
use mod_api::prelude::ItemsHost;
use mod_api::*;
use std::sync::Arc;

#[test]
fn tooltip_appender_restores_owner_even_on_panic() {
    mod_api::__private::initialize().unwrap();
    struct Check(bool);
    impl TooltipAppender for Check {
        fn append(
            &self,
            _: &dyn ClientHost,
            _: &ItemStack,
            _: &TooltipAppendContext,
            _: &mut Vec<(Text, u32)>,
        ) {
            assert_eq!(mod_api::content::crash::current_native_mod(), Some("inner"));
            assert!(!self.0, "test unwind");
        }
    }
    struct Inner;
    #[mod_api::__subscriptions]
    impl Hooks for Inner {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            for panic in [false, true] {
                registrar.register_tooltip_appender(
                    "test:item",
                    TooltipPlacement::AfterAll,
                    Arc::new(Check(panic)),
                );
            }
        }
    }
    let mods = [ModEntry::new("inner", Box::new(Inner))];
    mod_api::__private::part_tests::install_parts_fixture(&mods).unwrap();
    let context = TooltipAppendContext {
        player: 0,
        player_entity_id: 0,
        advanced: false,
        creative: false,
        nested: false,
    };
    let mut host = mod_api::testing::FakeHost::new();
    let id = Identifier::parse("minecraft:stick").unwrap();
    host.registries.resolve_items([id.clone()]).unwrap();
    let stack = ItemStack::of(host.item_key(&id).unwrap());
    let outer = mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: "outer",
        hook: "outer",
        part: None,
    });
    let appenders = mod_api::__private::tooltip_appenders();
    assert_eq!(appenders.len(), 2);
    for (part, panic) in appenders.iter().zip([false, true]) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            part.appender()
                .append(&host, &stack, &context, &mut Vec::new())
        }));
        assert_eq!(result.is_err(), panic);
        assert_eq!(mod_api::content::crash::current_native_mod(), Some("outer"));
    }
    drop(outer);
    assert_eq!(mod_api::content::crash::current_native_mod(), None);
}
