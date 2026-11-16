struct TestTags;
impl engine::LootTags for TestTags {
    fn members(&self, _: &str, _: &str) -> Vec<String> {
        Vec::new()
    }
}

struct TagReader;
#[mod_api::__subscriptions]
impl Hooks for TagReader {
    fn loot_function(&self, event: &mut LootFunctionEvent<'_>) {
        let tag = event
            .tags
            .key(&mod_api::Identifier::parse("test:ingredients").unwrap())
            .unwrap();
        let members = event.tags.items(tag).unwrap();
        event.stack.count = if event.tags.is_in_tag(event.stack.item, tag) == Some(true) {
            assert_eq!(
                members
                    .iter()
                    .map(|key| mod_api::__private::item_key_id(*key).unwrap())
                    .collect::<Vec<_>>(),
                ["minecraft:potion", "minecraft:stick"]
            );
            2
        } else {
            assert!(members.is_empty());
            0
        };
    }
    fn loot_condition(&self, event: &mut mod_api::LootConditionEvent<'_>) {
        let tag = event
            .tags
            .key(&mod_api::Identifier::parse("test:ingredients").unwrap())
            .unwrap();
        let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
        event.holds = event.tags.is_in_tag(stick, tag).unwrap();
        let missing = event
            .tags
            .key(&mod_api::Identifier::parse("test:missing").unwrap())
            .unwrap();
        assert_eq!(event.tags.is_in_tag(stick, missing), Some(false));
        assert!(event.tags.items(missing).unwrap().is_empty());
    }
}

#[test]
fn loot_callbacks_read_current_membership_and_order_after_tag_reload() {
    use horizon_items::items::{Items, TagRegistry};
    use std::collections::HashMap;
    super::super::prepare_stack_boundary_tests();
    let mut items = Items::for_tests(HashMap::new());
    with_mods(
        vec![ModEntry::new("tag_reader", Box::new(TagReader))].into(),
        || {
            for (members, expected_count, expected_holds) in [
                (
                    vec!["minecraft:potion".to_owned(), "minecraft:stick".to_owned()],
                    2,
                    true,
                ),
                (Vec::new(), 0, false),
            ] {
                items.set_tags(
                    TagRegistry::Item,
                    HashMap::from([("test:ingredients".into(), members)]),
                );
                let mut value = stack();
                let mut holds = false;
                dispatch_loot_function(
                    "test:count",
                    &serde_json::Value::Null,
                    &context(),
                    &mut value,
                    &items,
                );
                dispatch_loot_condition(
                    "test:member",
                    &serde_json::Value::Null,
                    &context(),
                    &mut holds,
                    &items,
                );
                assert_eq!(value.count, expected_count);
                assert_eq!(holds, expected_holds);
            }
        },
    );
}
