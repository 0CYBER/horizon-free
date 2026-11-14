pub(super) struct LootFunctionHooks;
impl LootFunctionHooks {
    pub(super) fn loot_function(&self, event: &mut mod_api::LootFunctionEvent<'_>) {
        let tag = event
            .tags
            .key(&mod_api::Identifier::parse("test:ingredients").unwrap())
            .unwrap();
        let members = event.tags.items(tag).unwrap();
        assert_eq!(members, vec![event.stack.item]);
        assert_eq!(event.tags.is_in_tag(event.stack.item, tag), Some(true));
        event.stack.count = 29;
    }
}
