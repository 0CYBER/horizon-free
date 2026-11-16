//! Item declarations use the same per-mod ownership boundary as blocks.
#[test]
fn item_batches_are_separate_and_keep_each_boundary_answer() {
    struct Declares;
    #[mod_api::__subscriptions]
    impl Hooks for Declares {
        fn items(&self, _: &RegistrationContext, event: &mut ItemsEvent<'_>) {
            assert!(event.items.is_empty());
            event
                .items
                .push(mod_api::mod_api::Item { id: "spoon".into() });
        }
    }
    install(
        vec![
            ModEntry::new("first", Box::new(Declares)),
            ModEntry::new("second", Box::new(Declares)),
        ]
        .into(),
    );
    DECLARATION_OWNERS.with_borrow_mut(Vec::clear);
    let mut items = Vec::new();
    dispatch_items(&RegistrationContext("bootstrap"), &mut items);
    assert_eq!(
        items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["first/checked/spoon", "second/checked/spoon"]
    );
    DECLARATION_OWNERS.with_borrow(|calls| {
        assert_eq!(
            calls,
            &[
                ("first".into(), "spoon".into()),
                ("second".into(), "spoon".into())
            ]
        )
    });
}
