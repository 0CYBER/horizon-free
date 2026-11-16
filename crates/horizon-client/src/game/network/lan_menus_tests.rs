//! A guest's menu owners live in `LanHost::menus`, outside the `GuestPlay` that a guest's turn
//! lifts, and the hub's click runs with the host lifted out of the game. The fixtures use the
//! LAN host with two guests (`with_guests`), which needs prepared resources.
use super::transactions::decision::{
    Admission,
    engine::{Owner, Target},
};
use crate::game::{Game, native_guest_turn_tests::with_guests};
use horizon_client::menu::{Kind, Menu};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Give guest `conn` an inventory menu carrying `count` sticks on its cursor, and some play
/// state, as a guest who has clicked in their inventory has.
fn open_inventory(game: &mut Game<'_>, conn: u32, count: u32) {
    let mut menu = Box::new(Menu::new(Kind::Inventory));
    menu.carried = Some(horizon_client::items::ItemStack::new(
        "minecraft:stick",
        count,
    ));
    let host = game.lan.as_mut().unwrap();
    *host.menus.entry(conn).or_default().crafting.inventory = Some(menu);
    host.play.entry(conn).or_default().chat_spam = 5;
}

fn carried(game: &Game<'_>, conn: u32) -> Option<u32> {
    let menu = game
        .lan
        .as_ref()?
        .menus
        .get(&conn)?
        .crafting
        .inventory
        .as_ref()?;
    menu.carried.as_ref().map(|stack| stack.count)
}

#[test]
#[ignore = "daily"]
fn menu_owners_stay_reachable_through_nested_guest_turns() {
    with_guests(|game| {
        open_inventory(game, 12, 3);
        open_inventory(game, 13, 7);
        let outside = Target::admit(game, 12, Owner::Inventory).unwrap();
        assert!(game.lan.as_ref().unwrap().play.contains_key(&12));

        let inside = game
            .as_guest(12, |game| {
                // The turn lifted the guest's play state, as before...
                assert!(!game.lan.as_ref().unwrap().play.contains_key(&12));
                // ...but not the menu: the owner is found, with the same identity, and its
                // content is the guest's own.
                let found = Target::admit(game, 12, Owner::Inventory).unwrap();
                assert_eq!(carried(game, 12), Some(3));
                // A nested turn of another guest sees both guests' owners.
                let nested = game.as_guest(13, |game| {
                    assert!(!game.lan.as_ref().unwrap().play.contains_key(&13));
                    assert!(Target::admit(game, 13, Owner::Inventory).is_ok());
                    assert!(Target::admit(game, 12, Owner::Inventory).is_ok());
                    (carried(game, 12), carried(game, 13))
                });
                // The same guest cannot nest in their own turn.
                assert!(game.as_guest(12, |_| ()).is_none());
                (found, nested)
            })
            .unwrap();
        assert_eq!(inside.1, Some((Some(3), Some(7))));
        // Same owner before, during and after: nothing was replaced by a placeholder.
        assert_eq!(inside.0.owner_id(), outside.owner_id());
        let after = Target::admit(game, 12, Owner::Inventory).unwrap();
        assert_eq!(after.owner_id(), outside.owner_id());
        // The play state came back as it was.
        assert_eq!(game.lan.as_ref().unwrap().play[&12].chat_spam, 5);
        assert_eq!(game.lan.as_ref().unwrap().play[&13].chat_spam, 5);
    });
}

#[test]
#[ignore = "daily"]
fn a_panicking_turn_leaves_play_state_and_menus_restored() {
    with_guests(|game| {
        open_inventory(game, 12, 3);
        let before = Target::admit(game, 12, Owner::Inventory).unwrap();
        let panicked = catch_unwind(AssertUnwindSafe(|| {
            game.as_guest(12, |game| {
                let host = game.lan.as_mut().unwrap();
                host.menus
                    .get_mut(&12)
                    .unwrap()
                    .crafting
                    .inventory
                    .as_mut()
                    .unwrap()
                    .carried = None;
                panic!("a listener panicked in the guest's turn");
            })
        }));
        assert!(panicked.is_err());
        let host = game.lan.as_ref().unwrap();
        // The swap is undone: the guest's play state is back, and the menu is as the turn left it.
        assert_eq!(host.play[&12].chat_spam, 5);
        assert!(game.lan_guest_swaps.is_empty());
        assert_eq!(carried(game, 12), None);
        let after = Target::admit(game, 12, Owner::Inventory).unwrap();
        assert_eq!(after.owner_id(), before.owner_id());
        // And the next turn works.
        assert_eq!(game.as_guest(12, |_| 4), Some(4));
    });
}

#[test]
#[ignore = "daily"]
fn a_lifted_host_comes_back_after_a_panic_and_is_never_lifted_twice() {
    with_guests(|game| {
        open_inventory(game, 12, 3);
        let guests = game.lan.as_ref().unwrap().hub.guests.len();
        // The game and the host are separate while lifted, and a second lift finds none.
        let seen = game
            .with_lan_lifted(|game, host| {
                let nested = game.with_lan_lifted(|_, _| ());
                (game.lan.is_none(), nested.is_none(), host.hub.guests.len())
            })
            .unwrap();
        assert_eq!(seen, (true, true, guests));
        assert!(game.lan.is_some());

        let panicked = catch_unwind(AssertUnwindSafe(|| {
            game.with_lan_lifted(|_, host| {
                host.menus.remove(&12);
                panic!("a hook panicked in a lifted click");
            })
        }));
        assert!(panicked.is_err());
        let host = game.lan.as_ref().unwrap();
        assert_eq!(host.hub.guests.len(), guests);
        // What the lifted code did to the host stays done; nothing was swapped back for it.
        assert!(!host.menus.contains_key(&12));
        assert!(game.with_lan_lifted(|_, _| 1).is_some());
    });
}

#[test]
#[ignore = "daily"]
fn removing_a_fake_player_drops_its_menu_owners() {
    with_guests(|game| {
        let _serving = super::integrated::tests::install_serving(game, false);
        game.next_entity = 100;
        let level = game.level(game.dim.id()).unwrap();
        let fake = game.fake_player(level, 1234, "machine").unwrap();
        let conn = game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .iter()
            .find(|g| g.is_fake())
            .unwrap()
            .conn;
        open_inventory(game, conn, 2);
        assert_eq!(carried(game, conn), Some(2));
        // A guest of the same host, untouched by the removal.
        open_inventory(game, 12, 3);
        assert!(game.player_remove_fake(fake));
        let host = game.lan.as_ref().unwrap();
        assert!(!host.menus.contains_key(&conn));
        assert!(!host.play.contains_key(&conn));
        assert_eq!(carried(game, 12), Some(3));
        assert!(matches!(
            Target::admit(game, conn, Owner::Inventory),
            Err(Admission::MissingHolder)
        ));
    });
}
