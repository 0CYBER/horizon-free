//! A station take's `Stats.ITEM_CRAFTED` count on the server path: a
//! single-player window's clicks are a guest's, and the guest's statistics are
//! the server's (`ServerPlayer.awardStat`).

use super::*;

/// `StonecutterMenu.onTake` counts the taken result on the server's copy of
/// the player too, not only in the menu the click ran in.
#[test]
#[ignore = "daily"]
fn a_guest_stonecutter_take_counts_the_item_crafted() {
    let mut resources = None;
    let mut game = crate::game::game_tests::tests::server_game(&mut resources);
    let _rx = crate::game::lan::world_spawn_sync_tests::host_for(&mut game);
    let conn = game.lan.as_ref().expect("a LAN host").hub.guests[0].conn;
    let mut menu = Menu::new(Kind::Stonecutter);
    let took = game.guest_menu_op(0, &mut menu, |menu, env| {
        menu.set_slot(env, 0, Some(ItemStack::new("minecraft:stone", 4)));
        let bricks = menu
            .stone
            .recipes
            .iter()
            .position(|&r| {
                &*env.station_recipes.stonecutting[r].result.item == "minecraft:stone_bricks"
            })
            .expect("stone cuts into stone bricks");
        assert!(menu.button(env, bricks as i32));
        menu.click(env, 1, 0, Input::Pickup).unwrap();
        menu.slot(0).map(|s| s.count)
    });
    let stats = &game
        .lan
        .as_ref()
        .expect("the host back")
        .play
        .get(&conn)
        .expect("the guest")
        .stats;
    assert_eq!(took, Some(Some(3)), "the take used one stone");
    assert_eq!(
        stats.get("crafted", "minecraft:stone_bricks"),
        1,
        "the guest's statistics count the crafted item"
    );
}
