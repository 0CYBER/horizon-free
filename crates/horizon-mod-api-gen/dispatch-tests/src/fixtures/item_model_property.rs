#[test]
fn item_model_property_dispatches_owned_stack_and_preserves_unhandled_answers() {
    struct Property;
    #[mod_api::__subscriptions]
    impl Hooks for Property {
        fn item_model_property(&self, _: &dyn ClientHost, event: &mut ItemModelPropertyEvent<'_>) {
            if event.property == "example:charged" {
                event.value = event.stack.count() > 1 && event.bundle_selected == 3;
            }
        }
    }
    let mods: Arc<[ModEntry]> = vec![ModEntry::new("property-8", Box::new(Property))].into();
    install(mods.clone());
    let host = TestHost;
    let mut value = false;
    dispatch_item_model_property(
        &host,
        "example:charged",
        &ItemStack {
            count: 2,
            ..Default::default()
        },
        3,
        &mut value,
    );
    assert!(value);
    dispatch_item_model_property(
        &host,
        "other:property",
        &ItemStack {
            count: 1,
            ..Default::default()
        },
        -1,
        &mut value,
    );
    assert!(value);
    dispatch_item_model_property(
        &host,
        "example:charged",
        &ItemStack {
            count: 1,
            ..Default::default()
        },
        -1,
        &mut value,
    );
    assert!(!value);
    install(Arc::from([]));
    dispatch_item_model_property(
        &host,
        "example:charged",
        &ItemStack {
            count: 2,
            ..Default::default()
        },
        3,
        &mut value,
    );
    assert!(!value);
}
