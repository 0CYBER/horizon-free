use super::*;
#[test]
#[ignore = "daily"]
fn actual_game_stage_refuses_nested_frame_and_mutated_actor() {
    use horizon_client::{
        containers::{BlockEntity, ModBlockEntity, ModMenuType},
        crafting::{Ingredient, Placement},
        lan::menu::{Menu as Open, Slots},
        recipe_book::{
            settings_connection::Connection, settings_sync::Authority,
            slot_layout::CheckedSlotLayout,
        },
    };
    super::super::native_guest_turn_tests::with_guests(|game| {
        let pos = (3, 64, 5);
        let kind = Box::leak(Box::new(ModMenuType {
            id: "review:menu",
            slots: 3,
            data_slots: 0,
            results: vec![2],
            crafting: None,
            links: None,
        }));
        let mut block = ModBlockEntity::new("review:block".into(), Default::default());
        block.items = vec![
            Some(horizon_client::items::ItemStack::new("minecraft:dirt", 1)),
            None,
            None,
        ];
        game.world
            .block_entities
            .insert(pos, BlockEntity::Mod(Box::new(block)));
        game.world.block_entities.set_mod_containers(
            [(
                game.world.block(pos.0, pos.1, pos.2).0,
                std::sync::Arc::new(horizon_client::containers::ModContainer {
                    slots: 3,
                    ..Default::default()
                }),
            )]
            .into_iter()
            .collect(),
        );
        game.world.modified.clear();
        game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.menu_generation = 1;
            g.menu = Some(Open {
                window: 1,
                kind: 0,
                registry_id: "review:menu".into(),
                pos: Some(pos),
                slots: Slots::Block(pos),
                size: 3,
                state: 0,
                sent: vec![],
                drag: None,
                sent_data: vec![],
                hidden: vec![],
                mod_type: Some(kind),
                craft: None,
                links: vec![],
                asks_still_valid: false,
            });
        });
        let before = game.mod_book_menu_snapshot(12, 1, "review:menu").unwrap();
        let catalog = sync::CatalogIdentity {
            epoch: 1,
            number: 1,
            window: 1,
            nonce: 1,
            instance: 1,
            generation: 1,
            menu: "review:menu".to_owned().try_into().unwrap(),
            book: "review:book".to_owned().try_into().unwrap(),
        };
        let selection = sync::Selection {
            catalog: catalog.clone(),
            number: 2,
            display: sync::DisplayId {
                recipe: "review:recipe".to_owned().try_into().unwrap(),
                source: 0,
            },
        };
        let mut server = super::super::native_book_placement::Server::new(1);
        server.peer.hello(1, sync::schema()).unwrap();
        server.peer.publish(catalog).unwrap();
        game.native_net.placement.servers.insert(12, server);
        let mut connection = Connection::default();
        connection.authority = Some(Authority::negotiate(
            1,
            vec!["review:book".into()],
            &["review:book".into()].into_iter().collect(),
        ));
        game.native_net.book_connections.insert(12, connection);
        let layout = CheckedSlotLayout::new(
            mod_api::RecipeBookLayout::Furnace {
                input: 0,
                fuel: 1,
                result: 2,
            },
            3,
        )
        .unwrap();
        let plan = MenuPlacement::new(
            "review:menu".into(),
            Placement::of(
                vec![Some(Ingredient::of(vec!["minecraft:iron_ore".into()]))],
                None,
            ),
            &layout,
        )
        .unwrap();
        let root = game.transaction_open();
        assert!(game.mod_book_placement_context(&before, &selection));
        let child = game.transaction_open();
        assert_eq!(
            game.stage_mod_book_place(&before, &selection, &plan, false),
            Err(sync::Refusal::Changed)
        );
        game.transaction_close(&child, false);
        assert!(game.mod_book_placement_context(&before, &selection));
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.health = 1.0);
        assert_eq!(
            game.stage_mod_book_place(&before, &selection, &plan, false),
            Err(sync::Refusal::Changed)
        );
        assert_eq!(
            game.world.block_entities.get(&pos).unwrap().slots()[0]
                .as_ref()
                .unwrap()
                .count,
            1
        );
        assert!(game.world.modified.is_empty());
        game.transaction_close(&root, false);
    });
}
