use super::*;

#[test]
#[ignore = "daily"]
fn targeted_item_fallthrough_keeps_hand_and_fail_vetoes_item_action() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "fallthrough_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        for client in [false, true] {
            with_interaction_game(client, |game| {
                let side = if client {
                    LogicalSide::Client
                } else {
                    LogicalSide::Server
                };
                game.forced_hit = Some(hit());
                for hand in [Hand::Main, Hand::Off] {
                    *observer.0.lock().unwrap() = Controls {
                        air_answer: Some(Answer::Fail),
                        ..Controls::default()
                    };
                    game.inventory[game.selected] = Some(ItemStack::new("minecraft:apple", 1));
                    game.offhand = Some(ItemStack::new("minecraft:apple", 1));
                    if client {
                        assert!(!game.remote_use_item(hand));
                    } else {
                        assert!(!game.with_hand(hand, |g| g.use_hand(None)).unwrap());
                    }
                    assert_eq!(observer.0.lock().unwrap().steps, [Step::Air(side, hand)]);
                    assert!(game.using.is_none());
                    assert!(!observer.0.lock().unwrap().steps.contains(&Step::ItemPart));
                }
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn block_pass_falls_through_but_fail_and_success_stop_the_apple() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "block_fallthrough",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.forced_hit = Some(hit());
            for (answer, expected) in [(Answer::Pass, 1), (Answer::Fail, 0), (Answer::Success, 0)] {
                game.inventory[game.selected] = Some(ItemStack::new("minecraft:apple", 1));
                *observer.0.lock().unwrap() = Controls {
                    hands: Some(Vec::new()),
                    block_answer: Some(Answer::Pass),
                    item_on_answer: Some(answer),
                    air_answer: Some(Answer::Fail),
                    ..Controls::default()
                };
                game.remote_use(mod_api::InteractionAttempt::Press);
                let state = observer.0.lock().unwrap();
                assert_eq!(
                    state
                        .steps
                        .iter()
                        .filter(|step| matches!(step, Step::Air(LogicalSide::Client, Hand::Main)))
                        .count(),
                    expected
                );
                assert!(!state.steps.contains(&Step::ItemPart));
                assert!(game.using.is_none());
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn entity_pass_falls_through_but_success_stops_the_apple() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "entity_fallthrough",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        for (entity, expected) in [("minecraft:zombie", 1), ("minecraft:villager", 0)] {
            with_interaction_game(true, |game| {
                game.inventory[game.selected] = Some(ItemStack::new("minecraft:apple", 1));
                game.offhand = None;
                let kind = game.res.mob_types().get(entity).unwrap().clone();
                game.mobs.push(horizon_client::mobs::Mob::new(
                    42,
                    kind,
                    Vec3::new(0.5, 64.0, 5.5),
                    0.0,
                ));
                assert!(game.remote_use_targets_entity());
                // Zombie interaction passes; the awake adult villager succeeds.
                // Entity CONSUME prediction belongs to arismoko/horizon#2931 and is deliberately excluded.
                *observer.0.lock().unwrap() = Controls {
                    air_answer: Some(Answer::Fail),
                    ..Controls::default()
                };
                game.remote_use(mod_api::InteractionAttempt::Press);
                let state = observer.0.lock().unwrap();
                let hands: Vec<_> = state
                    .steps
                    .iter()
                    .filter_map(|s| match s {
                        Step::Air(LogicalSide::Client, hand) => Some(*hand),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    hands,
                    if expected == 1 {
                        vec![Hand::Main]
                    } else {
                        vec![]
                    },
                    "{entity}"
                );
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn air_use_fires_once_and_passing_hands_fire_in_main_off_order() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "air_fallthrough",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:apple", 1));
            assert!(!game.remote_use_targets_entity());
            assert!(game.pick().is_none());
            for (offhand, expected) in [
                (false, vec![Hand::Main]),
                (true, vec![Hand::Main, Hand::Off]),
            ] {
                game.offhand = offhand.then(|| ItemStack::new("minecraft:apple", 1));
                *observer.0.lock().unwrap() = Controls {
                    air_answer: Some(Answer::Pass),
                    ..Controls::default()
                };
                game.remote_use(mod_api::InteractionAttempt::Press);
                let state = observer.0.lock().unwrap();
                let hands: Vec<_> = state
                    .steps
                    .iter()
                    .filter_map(|s| match s {
                        Step::Air(LogicalSide::Client, hand) => Some(*hand),
                        _ => None,
                    })
                    .collect();
                assert_eq!(hands, expected);
            }
        });
    });
}

fn with_packet_game(test: impl FnOnce(&mut Game<'_>)) {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let _receiver = setup_interaction_guest(game);
        test(game);
    });
}

#[test]
#[ignore = "daily"]
fn server_use_item_packet_fires_once_after_use_item_on_with_block_crosshair() {
    use horizon_protocol::packets::play::{self, Serverbound as S};
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "packet_fallthrough",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_packet_game(|game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.forced_hit = Some(hit());
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[g.selected] = Some(ItemStack::new("minecraft:apple", 1));
            });
            *observer.0.lock().unwrap() = Controls {
                item_on_answer: Some(Answer::Pass),
                air_answer: Some(Answer::Fail),
                ..Controls::default()
            };
            game.lan_guest_packet(
                12,
                S::UseItemOn(play::UseItemOn {
                    hand: 0,
                    pos: [0, 64, 0],
                    face: 1,
                    cursor: [0.5, 1.0, 0.5],
                    inside: false,
                    world_border: false,
                    sequence: 1,
                }),
            );
            assert!(
                observer
                    .0
                    .lock()
                    .unwrap()
                    .steps
                    .iter()
                    .any(|s| matches!(s, Step::Phase(_, LogicalSide::Server)))
            );
            assert!(
                !observer
                    .0
                    .lock()
                    .unwrap()
                    .steps
                    .iter()
                    .any(|s| matches!(s, Step::Air(_, _)))
            );
            game.as_guest(12, |g| assert!(g.pick().is_some())).unwrap();
            game.lan_guest_packet(
                12,
                S::UseItem(play::UseItem {
                    hand: 0,
                    sequence: 2,
                    yaw: 0.0,
                    pitch: 0.0,
                }),
            );
            let state = observer.0.lock().unwrap();
            let hands: Vec<_> = state
                .steps
                .iter()
                .filter_map(|s| match s {
                    Step::Air(LogicalSide::Server, hand) => Some(*hand),
                    _ => None,
                })
                .collect();
            assert_eq!(hands, [Hand::Main]);
        });
    });
}

fn assert_block_item_before_fallthrough(
    item: &str,
    block: &str,
    expected: &str,
    expected_events: usize,
) {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "use_on_order",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(false, |game| {
            game.set_mode_id(0);
            let state = game.pack.assets.registry.default_state(block).unwrap();
            let dirt = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dirt")
                .unwrap();
            game.world.set(0, 63, 0, dirt).unwrap();
            game.world.set(0, 64, 0, state).unwrap();
            game.forced_hit = Some(hit());
            game.inventory[game.selected] = Some(ItemStack::new(item, 2));
            *observer.0.lock().unwrap() = Controls {
                air_answer: Some(Answer::Fail),
                ..Controls::default()
            };
            let used = game.use_hand(Some(&hit())).unwrap();
            assert_eq!(used, expected_events == 0);
            let y = if item == "minecraft:stone" { 65 } else { 64 };
            assert_eq!(&*game.block_name([0, y, 0]), expected);
            if item == "minecraft:bone_meal" || item == "minecraft:stone" {
                assert_eq!(game.held().unwrap().count, 1);
            }
            let state = observer.0.lock().unwrap();
            assert_eq!(
                state
                    .steps
                    .iter()
                    .filter(|s| matches!(s, Step::Air(_, _)))
                    .count(),
                expected_events
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn bone_meal_grows_grass_before_item_fallthrough() {
    assert_block_item_before_fallthrough(
        "minecraft:bone_meal",
        "minecraft:short_grass",
        "minecraft:tall_grass",
        0,
    );
}

#[test]
#[ignore = "daily"]
fn axe_strips_oak_log_before_item_fallthrough() {
    assert_block_item_before_fallthrough(
        "minecraft:iron_axe",
        "minecraft:oak_log",
        "minecraft:stripped_oak_log",
        0,
    );
}

#[test]
#[ignore = "daily"]
fn stone_places_before_item_fallthrough() {
    assert_block_item_before_fallthrough(
        "minecraft:stone",
        "minecraft:stone",
        "minecraft:stone",
        0,
    );
}

#[test]
#[ignore = "daily"]
fn apple_on_stone_passes_to_item_fallthrough_once() {
    assert_block_item_before_fallthrough(
        "minecraft:apple",
        "minecraft:stone",
        "minecraft:stone",
        1,
    );
}
