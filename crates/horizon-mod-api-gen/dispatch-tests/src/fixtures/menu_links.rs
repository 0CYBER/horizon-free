#[test]
fn menu_links_append_only_subscribed_mods_in_order_and_quiet_dispatch_does_no_work() {
    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {}
    struct Linker([i32; 3]);
    #[mod_api::__subscriptions]
    impl Hooks for Linker {
        fn menu_links(&self, _: &mut dyn ServerHost, event: &mut MenuLinksEvent<'_>) {
            assert_eq!(event.player, 42);
            assert_eq!(event.menu, "fixture:machine");
            assert_eq!(event.pos, [0, 64, 0]);
            event.links.push(self.0);
        }
    }
    install(vec![ModEntry::new("quiet", Box::new(Quiet))].into());
    ENTERED.with(|entered| entered.borrow_mut().clear());
    let mut links = Vec::new();
    dispatch_menu_links(&mut TestHost, 42, "fixture:machine", [0, 64, 0], &mut links);
    assert_eq!(links.capacity(), 0);
    ENTERED.with(|entered| assert!(entered.borrow().is_empty()));
    install(
        vec![
            ModEntry::new("first", Box::new(Linker([1, 64, 0]))),
            ModEntry::new("quiet", Box::new(Quiet)),
            ModEntry::new("second", Box::new(Linker([-1, 64, 0]))),
        ]
        .into(),
    );
    dispatch_menu_links(&mut TestHost, 42, "fixture:machine", [0, 64, 0], &mut links);
    assert_eq!(links, [[1, 64, 0], [-1, 64, 0]]);
    ENTERED.with(|entered| {
        assert_eq!(
            entered.borrow().as_slice(),
            [(0, "menu_links"), (2, "menu_links")]
        )
    });
}
