use crate::game::native_entity_refs::tests::with_game;
use crate::game::native_item_places::tests::{guest, open_to_lan};
use horizon_client::items::ItemStack;

#[test]
#[ignore = "daily"]
fn bundle_handler_weight_front_order_and_refusal() {
    with_game(|game| {
        let empty = ItemStack::new("minecraft:bundle", 1);
        let pearls = ItemStack::new("minecraft:ender_pearl", 16);
        let (moved, full) = game.item_bundle_insert_slot(&empty, 0, &pearls).unwrap();
        assert_eq!(moved, 16);
        assert_eq!(
            game.item_bundle_insert_slot(&full, 0, &ItemStack::new("minecraft:ender_pearl", 1))
                .unwrap(),
            (0, full.clone())
        );
        assert_eq!(game.item_bundle_capacity(&full, 0, &pearls), 0);
        assert_eq!(
            game.item_bundle_insert_slot(&empty, 0, &ItemStack::new("minecraft:shulker_box", 1))
                .unwrap()
                .0,
            0
        );
        let (_, arrows) = game
            .item_bundle_insert_slot(&empty, 0, &ItemStack::new("minecraft:arrow", 2))
            .unwrap();
        let (_, mixed) = game
            .item_bundle_insert_slot(&arrows, 0, &ItemStack::new("minecraft:ender_pearl", 3))
            .unwrap();
        let slots = game.item_bundle_slots(&mixed).unwrap();
        assert_eq!(
            slots,
            vec![
                Some(ItemStack::new("minecraft:ender_pearl", 3)),
                Some(ItemStack::new("minecraft:arrow", 2))
            ]
        );
        let (out, rest) = game.item_bundle_extract(&mixed, 0, 64).unwrap();
        assert_eq!(out, Some(ItemStack::new("minecraft:ender_pearl", 3)));
        assert_eq!(
            game.item_bundle_slots(&rest).unwrap(),
            vec![Some(ItemStack::new("minecraft:arrow", 2))]
        );
        // A UI selection must not redirect automation away from the front.
        let mut selected = mixed.clone();
        selected.data_mut().bundle_selected = horizon_items::item_data::BundleSelected(1);
        assert_eq!(
            game.item_bundle_extract(&selected, 0, 1).unwrap().0,
            Some(ItemStack::new("minecraft:ender_pearl", 1))
        );
        let mut overfull = empty.clone();
        overfull.data_mut().bundle_contents = vec![
            ItemStack::new("minecraft:ender_pearl", 20),
            ItemStack::new("minecraft:arrow", 64),
        ];
        let (out, rest) = game.item_bundle_extract(&overfull, 0, 1).unwrap();
        assert_eq!(out, Some(ItemStack::new("minecraft:ender_pearl", 1)));
        assert_eq!(
            game.item_bundle_slots(&rest).unwrap(),
            vec![
                Some(ItemStack::new("minecraft:ender_pearl", 19)),
                Some(ItemStack::new("minecraft:arrow", 64))
            ]
        );
        assert!(
            game.item_bundle_slots(&ItemStack::new("minecraft:stone", 1))
                .is_none()
        );
    });
}

#[test]
#[ignore = "daily"]
fn bundle_items_place_handler_reads_and_rolls_back() {
    use mod_api::{HostTransaction, ITEM_ITEMS, ItemPlace};
    with_game(|game| {
        game.creative = false;
        let mut bundle = ItemStack::new("minecraft:bundle", 1);
        bundle.data_mut().bundle_contents = vec![
            ItemStack::new("minecraft:ender_pearl", 3),
            ItemStack::new("minecraft:arrow", 2),
        ];
        open_to_lan(game);
        let mut player = guest(1);
        player.inventory[0] = Some(bundle.clone());
        game.lan.as_mut().unwrap().hub.guests.push(player);
        let place = ItemPlace::PlayerHand {
            player: 123,
            hand: mod_api::Hand::Main,
        };
        let handler = place.capability(game, &ITEM_ITEMS).unwrap();
        assert_eq!(handler.size(game), 2);
        assert_eq!(handler.amount(game, 0), 3);
        assert_eq!(handler.amount(game, 1), 2);
        let pearl = handler.resource(game, 0).unwrap();
        assert_eq!(
            crate::game::api_values::stack_value(pearl.as_stack())
                .unwrap()
                .item
                .as_ref(),
            "minecraft:ender_pearl"
        );
        assert_eq!(
            crate::game::api_values::stack_value(handler.resource(game, 1).unwrap().as_stack())
                .unwrap()
                .item
                .as_ref(),
            "minecraft:arrow"
        );
        let result = game.transaction(|host, tx| {
            assert_eq!(handler.extract(host, tx, 0, &pearl, 2), 2);
            Err::<(), _>(())
        });
        assert_eq!(result, Err(()));
        assert_eq!(
            game.lan.as_ref().unwrap().hub.guests[0].inventory[0],
            Some(bundle)
        );
    });
}

#[test]
#[ignore = "daily"]
fn nested_bundle_weight_limits_arrow_capacity_and_insertion() {
    with_game(|game| {
        let empty = ItemStack::new("minecraft:bundle", 1);
        let arrows = ItemStack::new("minecraft:arrow", 64);
        for (contents, room) in [
            (vec![], 60),
            (vec![ItemStack::new("minecraft:ender_pearl", 1)], 56),
        ] {
            let mut nested = empty.clone();
            nested.data_mut().bundle_contents = contents;
            let mut outer = empty.clone();
            outer.data_mut().bundle_contents = vec![nested.clone()];
            assert_eq!(game.item_bundle_capacity(&outer, 0, &arrows), room);
            let (moved, filled) = game.item_bundle_insert_slot(&outer, 0, &arrows).unwrap();
            assert_eq!(moved, room);
            assert_eq!(
                game.item_bundle_slots(&filled).unwrap(),
                vec![Some(ItemStack::new("minecraft:arrow", room)), Some(nested)]
            );
            assert_eq!(game.item_bundle_capacity(&filled, 0, &arrows), 0);
        }
    });
}

#[test]
#[ignore = "daily"]
fn stacked_bundle_insertion_splits_one_and_restores_inventory_on_abort() {
    use crate::game::api_values::TryIntoApi;
    use mod_api::{HostTransaction, ITEM_ITEMS, ItemPlace};
    with_game(|game| {
        game.creative = false;
        open_to_lan(game);
        let mut player = guest(1);
        player.inventory[0] = Some(ItemStack::new("minecraft:bundle", 2));
        let original = player.inventory.clone();
        game.lan.as_mut().unwrap().hub.guests.push(player);
        let place = ItemPlace::PlayerHand {
            player: 123,
            hand: mod_api::Hand::Main,
        };
        let handler = place.capability(game, &ITEM_ITEMS).unwrap();
        let arrow = mod_api::ItemResource::of(
            &ItemStack::new("minecraft:arrow", 1).try_into_api().unwrap(),
        );
        assert_eq!(
            game.transaction(|host, tx| {
                assert_eq!(handler.insert(host, tx, 0, &arrow, 3), 3);
                Err::<(), _>(())
            }),
            Err(())
        );
        assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].inventory, original);
        assert_eq!(
            game.transaction(|host, tx| Ok::<_, ()>(handler.insert(host, tx, 0, &arrow, 3))),
            Ok(3)
        );
        let mut changed = ItemStack::new("minecraft:bundle", 1);
        changed.data_mut().bundle_contents = vec![ItemStack::new("minecraft:arrow", 3)];
        let inventory = &game.lan.as_ref().unwrap().hub.guests[0].inventory;
        let stacks: Vec<_> = inventory.iter().flatten().cloned().collect();
        assert_eq!(stacks.len(), 2);
        assert!(stacks.contains(&ItemStack::new("minecraft:bundle", 1)));
        assert!(stacks.contains(&changed));
    });
}

struct BundleMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for BundleMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.item_capability(
            &mod_api::ITEM_ITEMS,
            "minecraft:paper",
            |_: &mut dyn mod_api::ServerHost, place: &mod_api::ItemPlace| {
                Some(Box::new(mod_api::BundleItems::new(place.clone()))
                    as Box<dyn mod_api::ResourceHandler<mod_api::ItemResource>>)
            },
        );
    }
}

#[test]
#[ignore = "daily"]
fn mod_registered_bundle_items_uses_weight_and_front_extraction() {
    use mod_api::{HostTransaction, ITEM_ITEMS, ItemPlace};
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("bundle_test", Box::new(BundleMod))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.creative = false;
            let mut bundle = ItemStack::new("minecraft:paper", 1);
            bundle.data_mut().bundle_contents = vec![ItemStack::new("minecraft:ender_pearl", 16)];
            open_to_lan(game);
            let mut player = guest(1);
            player.inventory[0] = Some(bundle.clone());
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let place = ItemPlace::PlayerHand {
                player: 123,
                hand: mod_api::Hand::Main,
            };
            let handler = place.capability(game, &ITEM_ITEMS).unwrap();
            let pearl = handler.resource(game, 0).unwrap();
            assert_eq!(
                game.transaction(|host, tx| Ok::<_, ()>(handler.insert(host, tx, 0, &pearl, 1))),
                Ok(0)
            );
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].inventory[0],
                Some(bundle)
            );
            assert_eq!(
                game.transaction(|host, tx| Ok::<_, ()>(handler.extract(host, tx, 0, &pearl, 3))),
                Ok(3)
            );
            assert_eq!(handler.amount(game, 0), 13);
            use crate::game::api_values::TryIntoApi;
            let arrow = mod_api::ItemResource::of(
                &ItemStack::new("minecraft:arrow", 1).try_into_api().unwrap(),
            );
            assert_eq!(
                game.transaction(|host, tx| Ok::<_, ()>(handler.insert(host, tx, 0, &arrow, 2))),
                Ok(2)
            );
            assert_eq!(
                game.transaction(|host, tx| Ok::<_, ()>(handler.insert(host, tx, 0, &pearl, 1))),
                Ok(1)
            );
            assert_eq!(handler.resource(game, 0), Some(pearl.clone()));
            assert_eq!(handler.amount(game, 0), 14);
            assert_eq!(
                game.transaction(|host, tx| Ok::<_, ()>(handler.extract(host, tx, 0, &pearl, 64))),
                Ok(14)
            );
            assert_eq!(handler.resource(game, 0), Some(arrow));
            assert_eq!(handler.amount(game, 0), 2);
        })
    });
}
