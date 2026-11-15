use super::*;

#[test]
#[ignore = "daily"]
fn identical_instant_stacks_report_the_selected_interaction_hand() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "hand_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(false, |game| {
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
            game.offhand = Some(ItemStack::new("minecraft:stick", 1));
            for hand in [Hand::Main, Hand::Off] {
                *observer.0.lock().unwrap() = Controls {
                    expected_hand: Some(hand),
                    ..Controls::default()
                };
                game.swing = Default::default();
                assert!(game.using.is_none());
                game.guest_use = Some(super::super::super::GuestUse::Item(hand));
                assert_eq!(game.use_item_as_player().unwrap(), Some(hand));
                assert_eq!(game.swing.attack_hand(), hand);
                assert!(game.using.is_none());
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Air(LogicalSide::Server, hand), Step::ItemPart]
                );
            }
            // The integrated server's local action also enters the same scoped hand context.
            *observer.0.lock().unwrap() = Controls {
                expected_hand: Some(Hand::Main),
                ..Controls::default()
            };
            game.guest_use = None;
            assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Main));
        });
    });
}

/// One `use_item` event as `(side, hand, player, player_entity_id)`.
type ItemUser = (LogicalSide, Hand, u128, i32);

fn item_users(observer: &Observer) -> Vec<ItemUser> {
    std::mem::take(&mut observer.0.lock().unwrap().item_users)
}

fn item_observer() -> (Observer, mod_api::Mods) {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "item_users",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    (observer, mods)
}

#[test]
#[ignore = "daily"]
fn the_item_event_names_the_guest_who_clicked_not_the_host() {
    let (observer, mods) = item_observer();
    mod_platform::testing::with_mods(mods, || {
        // The fixture's guest has connection 12: UUID 12 and entity id 12.
        with_interaction_game(false, |game| {
            game.offhand = Some(ItemStack::new("minecraft:stick", 1));
            for hand in [Hand::Main, Hand::Off] {
                game.guest_use = Some(super::super::super::GuestUse::Item(hand));
                game.use_item_as_player().unwrap();
                assert_eq!(
                    item_users(&observer),
                    [(LogicalSide::Server, hand, 12, 12)],
                    "{hand:?}"
                );
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn the_item_event_names_the_host_player_on_its_own_click() {
    let (observer, mods) = item_observer();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.5)).unwrap();
            player.entity_id = 77;
            game.player = Some(player);
            game.level.player_uuid = Some([1, 2, 3, 4]);
            game.set_mode_id(1);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let _guest = setup_interaction_guest(game);
            observer.0.lock().unwrap().host_click = true;
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
            game.guest_use = Some(super::super::super::GuestUse::Item(Hand::Main));
            game.use_item_as_player().unwrap();
            assert_eq!(
                item_users(&observer),
                [(
                    LogicalSide::Server,
                    Hand::Main,
                    0x0000_0001_0000_0002_0000_0003_0000_0004,
                    77
                )]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn the_item_event_names_this_client_on_a_predicted_use() {
    let (observer, mods) = item_observer();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            let remote = game.remote.as_mut().unwrap();
            remote.profile = 0xC0FF_EE00_0000_0000_0000_0000_0000_002A;
            remote.entity_id = 42;
            game.offhand = Some(ItemStack::new("minecraft:stick", 1));
            for hand in [Hand::Main, Hand::Off] {
                game.remote_use_item(hand);
                assert_eq!(
                    item_users(&observer),
                    [(
                        LogicalSide::Client,
                        hand,
                        0xC0FF_EE00_0000_0000_0000_0000_0000_002A,
                        42
                    )],
                    "{hand:?}"
                );
            }
        });
    });
}
