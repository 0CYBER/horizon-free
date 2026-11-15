#[test]
fn recipe_sources_append_in_mod_order_only_for_subscribers() {
    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {}
    struct Lender([i32; 3]);
    #[mod_api::__subscriptions]
    impl Hooks for Lender {
        fn recipe_sources(&self, _: &mut dyn ServerHost, event: &mut RecipeSourcesEvent<'_>) {
            assert_eq!(event.player, 0x1234);
            assert_eq!(event.menu, "fixture:machine");
            assert_eq!(event.pos, [10, 64, -10]);
            event.sources.push(self.0);
        }
    }
    install(
        vec![
            ModEntry::new("first", Box::new(Lender([11, 64, -10]))),
            ModEntry::new("quiet", Box::new(Quiet)),
            ModEntry::new("second", Box::new(Lender([9, 64, -10]))),
        ]
        .into(),
    );
    ENTERED.with(|entered| entered.borrow_mut().clear());
    let mut sources = vec![[10, 63, -10]];
    dispatch_recipe_sources(
        &mut TestHost,
        0x1234,
        "fixture:machine",
        [10, 64, -10],
        &mut sources,
    );
    assert_eq!(sources, [[10, 63, -10], [11, 64, -10], [9, 64, -10]]);
    ENTERED.with(|entered| {
        assert_eq!(
            entered.borrow().as_slice(),
            [(0, "recipe_sources"), (2, "recipe_sources")]
        )
    });
}
