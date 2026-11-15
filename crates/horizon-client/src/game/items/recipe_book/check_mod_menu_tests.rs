use super::*;

#[test]
#[ignore = "daily"]
fn a_remote_non_grid_menu_can_request_a_recipe_check_only_in_an_eligible_screen() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let name = "check_fixture:non_grid";
        assert!(horizon_client::containers::register_mod_menu_types([(
            name.into(),
            9,
            0,
            Vec::new(),
            None,
            None,
        )]));
        let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "RecipeCheck").unwrap();
        game.remote
            .as_mut()
            .unwrap()
            .menu
            .set(Some((7, name.into())));
        game.remote.as_mut().unwrap().recipe_viewer.check = true;
        game.ensure_book();
        let display = game.recipe_ui.book.as_ref().unwrap().entries[0].display_id;
        game.screen = Screen::Playing;
        assert!(game.transfer_recipe_check(display).is_none());
        assert!(game.recipe_ui.checks.asked.is_none());
        game.screen = Screen::Inventory;
        game.mode = horizon_client::command::args::GameMode::Spectator;
        assert!(game.transfer_recipe_check(display).is_none());
        assert!(game.recipe_ui.checks.asked.is_none());
        game.mode = horizon_client::command::args::GameMode::Survival;
        assert!(
            !game.mod_recipe_screen(),
            "the fixture has no crafting grid"
        );
        game.remote.as_mut().unwrap().recipe_viewer.check = false;
        assert!(game.transfer_recipe_check(display).is_none());
        assert!(game.recipe_ui.checks.asked.is_none());
        game.remote.as_mut().unwrap().recipe_viewer.check = true;
        assert!(
            game.transfer_recipe_check(display).is_none(),
            "reply is asynchronous"
        );
        assert_eq!(
            game.recipe_ui.checks.asked.map(|(id, _, _)| id),
            Some(display)
        );
    });
}
