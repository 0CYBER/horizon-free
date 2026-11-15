//! `StonecutterScreen.containerChanged` (`StonecutterScreen.java:205-209`): the
//! recipe list goes back to the top whenever the menu's input container
//! changes, and only then.
use super::*;
use horizon_client::menu::Input;

fn deepslate(count: u32) -> ItemStack {
    ItemStack::new("minecraft:deepslate", count)
}

/// The scroller at the end of its track: the last of the 17 deepslate
/// recipes' rows (`startIndex` 8).
fn scroll_to_bottom(game: &mut Game<'_>) {
    game.station_scroll(-2.0);
    let s = game.station.as_ref().unwrap();
    assert_eq!(
        (s.start, s.scroll),
        (8, 1.0),
        "the list is at its bottom row"
    );
}

fn shown(game: &Game<'_>) -> (usize, f32) {
    let s = game.station.as_ref().unwrap();
    (s.start, s.scroll)
}

fn pick_recipe(game: &mut Game<'_>, index: i32) -> bool {
    let mut taken = false;
    game.menu_session(|menu, env, _| {
        taken = menu.button(env, index);
        Some(true)
    });
    taken
}

#[test]
#[ignore = "daily"]
fn the_stonecutter_list_returns_to_the_top_when_its_input_changes() {
    crate::game::native_entity_refs::tests::with_game(|g| {
        g.options.dedicated = Some(Vec::new());
        g.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        assert!(g.open_station([0, 64, 0], "minecraft:stonecutter"));
        // Two deepslate in the input: 17 recipes, so the list scrolls.
        *g.slot_mut(9) = Some(deepslate(2));
        assert!(g.click_open_inventory(Some(9), 0, Input::Pickup));
        assert!(g.click_open_inventory(Some(INPUT), 0, Input::Pickup));
        let s = g.station.as_ref().unwrap();
        assert_eq!(s.inputs[0].as_ref().map(|i| i.count), Some(2));
        assert_eq!(
            s.stone.recipes.len(),
            17,
            "deepslate's stonecutting recipes"
        );

        // Taking a result uses up one deepslate, so the input changes: the list is
        // back at the top.
        assert!(pick_recipe(g, 8));
        scroll_to_bottom(g);
        assert!(g.click_open_inventory(Some(RESULT), 0, Input::Pickup));
        let s = g.station.as_ref().unwrap();
        assert_eq!(s.inputs[0].as_ref().map(|i| i.count), Some(1));
        assert_eq!(shown(g), (0, 0.0), "taking a result left the list scrolled");

        // Adding the rest of the stack to the input: the same.
        *g.slot_mut(10) = Some(deepslate(1));
        scroll_to_bottom(g);
        // A click in the player's inventory swaps the carried result for the
        // deepslate and does not touch the input: the list stays.
        assert!(g.click_open_inventory(Some(10), 0, Input::Pickup));
        assert_eq!(g.carried, Some(deepslate(1)), "the swap took the deepslate");
        assert_eq!(shown(g), (8, 1.0), "an inventory click scrolled the list");
        assert!(g.click_open_inventory(Some(INPUT), 0, Input::Pickup));
        assert_eq!(g.carried, None, "the deepslate went into the input");
        let s = g.station.as_ref().unwrap();
        assert_eq!(s.inputs[0].as_ref().map(|i| i.count), Some(2));
        assert_eq!(shown(g), (0, 0.0), "adding to the input left it scrolled");

        // Picking a recipe does not touch the input either.
        scroll_to_bottom(g);
        assert!(pick_recipe(g, 0));
        assert_eq!(shown(g), (8, 1.0), "picking a recipe scrolled the list");
    });
}
