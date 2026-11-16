#[test]
fn menu_transfer_asks_the_menus_part_and_none_leaves_the_grid_transfer() {
    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {}
    struct Machine;
    impl MenuPart for Machine {
        fn menu_subscribed(&self) -> &'static [Option<HookId>] {
            &[]
        }
        fn menu_transfer(
            &self,
            _: &mut dyn ServerHost,
            event: &mut MenuTransferEvent<'_>,
        ) -> Option<RecipeTransferResult> {
            assert_eq!((event.player, event.pos), (42, Some([0, 64, 0])));
            assert_eq!(event.recipe, "fixture:press");
            // A dry run says what it would do; a transfer moves (here: claims it moved).
            Some(if event.dry_run {
                RecipeTransferResult::Missing
            } else if event.max {
                RecipeTransferResult::NoRoom
            } else {
                RecipeTransferResult::Moves
            })
        }
    }
    install(vec![ModEntry::new("machine", Box::new(Quiet))].into());
    install_parts(Parts {
        menus: vec![leak(Machine)],
        ..Parts::default()
    });
    let transfer = |menu: &str, max: bool, dry_run: bool| {
        let mut result = None;
        dispatch_menu_transfer(
            &mut TestHost,
            calls::MenuTransferCall {
                player: 42,
                menu,
                pos: Some([0, 64, 0]),
                recipe: "fixture:press",
                max,
                dry_run,
                result: &mut result,
            },
        );
        result
    };
    assert_eq!(
        transfer("test:menu", false, false),
        Some(RecipeTransferResult::Moves)
    );
    assert_eq!(
        transfer("test:menu", true, false),
        Some(RecipeTransferResult::NoRoom)
    );
    assert_eq!(
        transfer("test:menu", false, true),
        Some(RecipeTransferResult::Missing)
    );
}
