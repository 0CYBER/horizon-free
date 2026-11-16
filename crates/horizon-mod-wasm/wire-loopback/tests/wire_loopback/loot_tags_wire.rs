use super::*;

struct TagView;
impl mod_api::Tags for TagView {
    fn key(&self, id: &mod_api::Identifier) -> Option<mod_api::TagKey> {
        mod_api::__private::tag_key(id)
    }
    fn is_in_tag(&self, item: mod_api::ItemKey, tag: mod_api::TagKey) -> Option<bool> {
        assert_eq!(item, REGISTERED_ITEM.get().unwrap());
        mod_api::__private::with_tag_name(tag, |name| Some(name? == "test:ingredients"))
    }
    fn items(&self, tag: mod_api::TagKey) -> Option<Vec<mod_api::ItemKey>> {
        mod_api::__private::with_tag_name(tag, |name| {
            Some(if name? == "test:ingredients" {
                vec![REGISTERED_ITEM.get().unwrap()]
            } else {
                vec![]
            })
        })
    }
}

#[test]
fn loot_events_lend_tags_over_wire_without_a_host() {
    let mods = [mod_api::ModEntry::new(
        "wire_test",
        Box::new(WasmMod {
            group: 0,
            subscribed: &[
                Some(mod_api::HookId::LootFunction),
                Some(mod_api::HookId::LootCondition),
            ],
            listeners: &[],
            corrupt: false,
        }),
    )];
    mod_api::__private::install_parts(
        &mods,
        &mod_api::__private::part_tests::rule_schema(),
        |_| false,
    )
    .unwrap();
    mod_api::testing::with_mods(Arc::from(mods), || {
        let ctx = mod_api::LootContext::new(7);
        let mut stack = mod_api::LootStack::new(REGISTERED_ITEM.get().unwrap(), 1);
        mod_api::loot_callbacks::native_loot_function(
            "test:count",
            &serde_json::Value::Null,
            &ctx,
            &mut stack,
            &TagView,
        );
        assert_eq!(stack.count, 29);
        let mut holds = false;
        mod_api::loot_callbacks::native_loot_condition(
            "test:member",
            &serde_json::Value::Null,
            &ctx,
            &mut holds,
            &TagView,
        );
        assert!(holds);
    });
}
