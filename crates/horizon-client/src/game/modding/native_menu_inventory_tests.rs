use super::api_values::TryIntoApi;
use super::native_guest_turn_tests::with_guests;
use horizon_client::{items::ItemStack, lan::playerdata};
use horizon_core::nbt::Tag;
use mod_api::__private::game::Fields;
use mod_api::{HostTransaction, ItemPlace, ServerHost};

struct Listener;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Listener {
    fn menu_clicked(&self, host: &mut dyn ServerHost, event: &mut mod_api::MenuClickedEvent) {
        assert_eq!(event.uuid, 12);
        assert_eq!(
            ItemPlace::PlayerSlot {
                player: event.uuid,
                slot: 9
            }
            .stack(host)
            .unwrap()
            .count(),
            10
        );
        assert_eq!(
            ItemPlace::PlayerHand {
                player: event.uuid,
                hand: mod_api::Hand::Main
            }
            .stack(host)
            .unwrap()
            .count(),
            3
        );
        let menu = host
            .player(event.uuid)
            .unwrap()
            .container_menu(host)
            .unwrap();
        let stone = mod_api::ItemResource::of(
            &ItemStack::new("minecraft:stone", 1).try_into_api().unwrap(),
        );
        let dirt =
            mod_api::ItemResource::of(&ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap());
        let pearl = mod_api::ItemResource::of(
            &ItemStack::new("minecraft:ender_pearl", 1)
                .try_into_api()
                .unwrap(),
        );
        let take = ItemPlace::MenuSlot { menu, slot: 9 };
        let give = ItemPlace::MenuSlot { menu, slot: 10 };
        let partial = ItemPlace::MenuSlot { menu, slot: 11 };
        host.transaction(|host, tx| {
            assert_eq!(take.extract(host, tx, &stone, 4), 4);
            assert_eq!(give.insert(host, tx, &dirt, 5), 5);
            assert_eq!(partial.insert(host, tx, &pearl, 10), 4);
            Ok::<_, ()>(())
        })
        .unwrap();
    }
}

fn exercise(reopen: bool) {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("inventory", Box::new(Listener))].into(),
        || {
            with_guests(|game| {
                let root = tempfile::tempdir().unwrap();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |g| {
                        g.inventory[0] = Some(ItemStack::new("minecraft:stone", 3));
                        g.inventory[9] = Some(ItemStack::new("minecraft:stone", 10));
                        g.inventory[11] = Some(ItemStack::new("minecraft:ender_pearl", 12));
                        if reopen {
                            let mut level = horizon_client::save::Level {
                                player_uuid: Some(playerdata::uuid_ints(g.uuid)),
                                ..Default::default()
                            };
                            playerdata::store(g, &mut level, None);
                            level.write(root.path()).unwrap();
                            playerdata::save_now(g, root.path()).unwrap();
                            g.inventory.fill(None);
                            let reopened = horizon_client::save::Level::read(root.path()).unwrap();
                            assert!(playerdata::load_owner(g, root.path(), Some(&reopened)));
                            assert_eq!(g.uuid, 12);
                        }
                    })
                    .unwrap();
                let window = game
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .open_native_menu(12, 1, Tag::String("inventory".into()), vec![None; 9])
                    .unwrap();
                let player = game.player(12).unwrap();
                let menu = player.container_menu(game).unwrap();
                let stone = mod_api::ItemResource::of(
                    &ItemStack::new("minecraft:stone", 1).try_into_api().unwrap(),
                );
                let dirt = mod_api::ItemResource::of(
                    &ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap(),
                );
                let take = ItemPlace::MenuSlot { menu, slot: 9 };
                let give = ItemPlace::MenuSlot { menu, slot: 10 };
                let partial = ItemPlace::MenuSlot { menu, slot: 11 };
                // The hook runs without a guest swapped in; use that same Host state.
                assert!(game.lan_acting.is_none());
                let hand = ItemPlace::PlayerHand {
                    player: 12,
                    hand: mod_api::Hand::Main,
                };
                assert_eq!(hand.stack(game).unwrap().count(), 3);
                let direct = ItemPlace::PlayerSlot {
                    player: 12,
                    slot: 9,
                };
                assert_eq!(direct.stack(game).unwrap().count(), 10);
                let foreign_player = mod_api::__private::game::player_ref(player.entity(), 13);
                let foreign = mod_api::__private::game::menu_ref(
                    foreign_player,
                    window,
                    menu.fields().generation,
                );
                let foreign = ItemPlace::MenuSlot {
                    menu: foreign,
                    slot: 9,
                };
                (game as &mut dyn ServerHost)
                    .transaction(|host, tx| {
                        assert_eq!(foreign.extract(host, tx, &stone, 4), 0);
                        assert_eq!(foreign.insert(host, tx, &dirt, 5), 0);
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                game.native_menu_clicked(12, window, 9, 0, 0);
                assert_eq!(take.stack(game).unwrap().count(), 6);
                assert_eq!(give.stack(game).unwrap().count(), 5);
                assert_eq!(partial.stack(game).unwrap().count(), 16);
                let aborted: Result<(), ()> =
                    (game as &mut dyn ServerHost).transaction(|host, tx| {
                        assert_eq!(give.insert(host, tx, &dirt, 3), 3);
                        Err(())
                    });
                assert!(aborted.is_err());
                assert_eq!(give.stack(game).unwrap().count(), 5);
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .open_native_menu(12, 1, Tag::String("replacement".into()), vec![None; 9])
                    .unwrap();
                (game as &mut dyn ServerHost)
                    .transaction(|host, tx| {
                        assert_eq!(take.extract(host, tx, &stone, 4), 0);
                        assert_eq!(give.insert(host, tx, &dirt, 5), 0);
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                assert_ne!(
                    player.container_menu(game).unwrap().container_id(game),
                    Some(window)
                );
                assert_eq!(direct.stack(game).unwrap().count(), 6);
            });
        },
    );
}

#[test]
#[ignore = "requires prepared resources"]
fn menu_player_inventory_transactions() {
    exercise(false);
}

#[test]
#[ignore = "requires prepared resources"]
fn menu_player_inventory_after_world_save_reload() {
    exercise(true);
}
