//! Real-game regressions for native handles used during a guest's turn.

use super::lan::LanSettings;
use horizon_client::{
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    mobs::Mob,
};
use horizon_core::geometry::Vec3;
use horizon_protocol::packets::configuration::ClientInformation;
use mod_api::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicI32, AtomicUsize},
};

pub(in crate::game) fn with_guests(test: impl FnOnce(&mut super::Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    with_guests_at(root, test);
}

pub(in crate::game) fn with_guests_at(
    root: std::path::PathBuf,
    test: impl FnOnce(&mut super::Game<'_>),
) {
    super::native_entity_refs::tests::with_game_at(root, |game| {
        let settings = LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
        let net = NetConfig {
            listen: false,
            announce: false,
            lod_distance: 0,
            ..NetConfig::lan(0, String::new(), 2)
        };
        let config = HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: game.dim.id().to_owned(),
            allow_flight: true,
            seed: 0,
            max_players: 2,
            integrated: true,
            levels: game.level_shapes,
        };
        game.lan_host(settings, net, config).unwrap();
        for id in [12, 13] {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: id,
                    uuid: u128::from(id),
                    name: format!("guest{id}"),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                id as i32,
                [3.0, 64.0, 5.0],
                10,
            );
            guest.placed = true;
            game.lan.as_mut().unwrap().hub.guests.push(guest);
        }
        test(game);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn player_dimensions_validate_identity_on_both_server_roles() {
    use super::execution_role::ExecutionRole;
    let previous_extra = super::EXTRA_DIMS.read().unwrap().clone();
    with_guests(|game| {
        super::Dim::add_extra("test:pocket");
        for role in [
            ExecutionRole::DedicatedServer,
            ExecutionRole::IntegratedServer,
        ] {
            game.options.execution_role = role;
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.dimension = "minecraft:overworld".to_owned();
                })
                .unwrap();
            let player = game.player(12).unwrap();
            let other = game.player(13).unwrap();
            let epoch = player.entity().epoch();
            let unknown = mod_api::__private::game::player_ref(player.entity(), 999);
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    mod_api::WorldEpoch::new(epoch.get() + 1),
                    player.entity().id(),
                ),
                player.uuid(),
            );
            let replaced = mod_api::__private::game::player_ref(other.entity(), player.uuid());
            for invalid in [unknown, stale, replaced] {
                assert_eq!(game.player_level(invalid), None, "{role:?}");
                assert_eq!(game.player_dimension(invalid), None, "{role:?}");
            }
            assert_eq!(
                game.player_level(player).unwrap().dimension_id(),
                "minecraft:overworld"
            );
            game.as_guest(12, |game| {
                assert_eq!(
                    game.player_dimension(player).unwrap().to_string(),
                    "minecraft:overworld"
                );
                game.as_guest(13, |game| {
                    assert_eq!(
                        game.player_level(player).unwrap().dimension_id(),
                        "minecraft:overworld"
                    );
                })
                .unwrap();
            })
            .unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.dimension = "minecraft:the_nether".to_owned();
                })
                .unwrap();
            assert_eq!(
                game.player_dimension(player).unwrap().to_string(),
                "minecraft:the_nether"
            );
            assert_eq!(
                game.player_level(player).unwrap().dimension_id(),
                "minecraft:the_nether"
            );
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.dimension = "test:pocket".to_owned();
                })
                .unwrap();
            assert_eq!(
                game.player_dimension(player).unwrap().to_string(),
                "test:pocket"
            );
            assert_eq!(
                game.player_level(player).unwrap().dimension_id(),
                "test:pocket"
            );
        }
    });
    *super::EXTRA_DIMS.write().unwrap() = previous_extra;
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn refused_wire_reload_preserves_hub_and_engine_generation() {
    with_guests(|game| {
        let before = game.lan.as_ref().unwrap().hub.lan.shared.sync();
        let old_tables = game.world_tables.as_ref().map(|tables| tables as *const _);
        let old_context = game.loop_context.clone();
        let root = std::env::temp_dir().join(format!("reload-wire-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let stack = super::functions::pack_stack(
            &game.world_root,
            &root,
            &game.pack.data_source,
            &game.pack.data_changes,
            false,
            &[],
            &[],
        );
        let error = game
            .prepare_world_reload(&stack, &root)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("wire reload") && error.contains("registries.json"),
            "{error}"
        );
        assert!(Arc::ptr_eq(
            &before,
            &game.lan.as_ref().unwrap().hub.lan.shared.sync()
        ));
        assert!(Arc::ptr_eq(&old_context, &game.loop_context));
        assert_eq!(
            old_tables,
            game.world_tables.as_ref().map(|tables| tables as *const _)
        );
        let good = game.options.resources.clone();
        let stack = super::functions::pack_stack(
            &game.world_root,
            &good,
            &game.pack.data_source,
            &game.pack.data_changes,
            false,
            &[],
            &[],
        );
        let prepared = game.prepare_world_reload(&stack, &good).unwrap();
        game.install_world_tables(prepared);
        game.run_native_advancement_grants();
        assert!(!Arc::ptr_eq(
            &before,
            &game.lan.as_ref().unwrap().hub.lan.shared.sync()
        ));
        std::fs::remove_dir_all(root).unwrap();
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn level_event_excludes_its_source_even_during_another_guests_turn() {
    use horizon_client::lan::PlayerSender;
    use horizon_world::level::{LevelEvent, LevelEventPlayer};

    with_guests(|game| {
        let mut receivers = Vec::new();
        let host = game.lan.as_mut().unwrap();
        host.owner = Some(super::lan::Owner {
            uuid: 12,
            saved: None,
        });
        host.hub.guests.update_all(|guest| {
            let (tx, rx) = std::sync::mpsc::channel();
            guest.sender = PlayerSender::new(tx, Arc::default());
            receivers.push(rx);
        });
        game.audio.capture = Some(Vec::new());
        game.as_guest(13, |game| {
            game.lan_record_events(&[LevelEvent::LevelEvent {
                id: 2001,
                pos: horizon_core::position::BlockPos::new(3, 64, 5),
                data: 1,
                source: Some(LevelEventPlayer(12)),
            }]);
        })
        .unwrap();
        game.lan_flush_effects();
        assert!(
            receivers[0].try_recv().is_err(),
            "integrated host is the source"
        );
        assert!(
            receivers[1].try_recv().is_ok(),
            "the acting guest receives another player's event"
        );

        game.lan_record_events(&[LevelEvent::LevelEvent {
            id: 1500,
            pos: horizon_core::position::BlockPos::new(3, 64, 5),
            data: 1,
            source: None,
        }]);
        game.lan_flush_effects();
        assert!(
            receivers[0].try_recv().is_ok(),
            "host receives null-source event"
        );
        assert!(
            receivers[1].try_recv().is_ok(),
            "guest receives null-source event"
        );
    });
}

/// Before #228, the host was absent during a guest turn and B and the
/// online list were invisible to native handles.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn another_guest_and_online_players_are_visible_during_a_turn() {
    with_guests(|game| {
        let rider = game.entity(13).unwrap();
        game.as_guest(12, |game| {
            assert_eq!(rider.position(game), Some([3.0, 64.0, 5.0]));
            let online = game.native_server_players();
            assert_eq!(
                online.iter().map(|p| p.uuid()).collect::<Vec<_>>(),
                [12, 13]
            );
            assert_eq!(online[1].entity().position(game).unwrap(), [3.0, 64.0, 5.0]);
        })
        .unwrap();
    });
}

/// B's live menu and motion stay available while A acts, but displaced
/// menus and an outer turn's motion must not use the swapped-out records.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn another_guests_menu_and_push_work_during_a_turn() {
    use horizon_client::items::ItemStack;

    with_guests(|game| {
        for conn in [12, 13] {
            game.as_guest(conn, |game| {
                game.player.as_mut().unwrap().velocity = Vec3::ZERO;
            })
            .unwrap();
        }
        let a = game.player(12).unwrap();
        let b = game.player(13).unwrap();
        let a_menu = game.player_container_menu(a).unwrap();
        let b_menu = game.player_container_menu(b).unwrap();
        let stone = ItemStack::new("minecraft:stone", 4);
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |guest| guest.inventory[0] = Some(stone.clone()))
            .unwrap();
        game.as_guest(12, |game| {
            assert!(game.player_container_menu(a).is_none());
            assert_eq!(a_menu.slot_count(game), None);
            assert_eq!(game.player_container_menu(b), Some(b_menu));
            assert_eq!(b_menu.slot_count(game), Some(46));
            assert_eq!(
                b_menu.slot(game, 36),
                Some(Some(crate::game::api_values::test_snapshot(&stone)))
            );
            assert!(game.entity_set_rotation(b.entity(), 90.0, 10.0));
            assert!(game.entity_set_head_yaw(b.entity(), 45.0));
            assert_eq!(game.entity_head_yaw(b.entity()), Some(45.0));
            assert!(game.entity_push(b.entity(), [0.25, 0.5, -0.25]));
            assert!(game.entity_push(a.entity(), [0.5, 0.0, 0.0]));
            game.as_guest(13, |game| {
                assert_eq!(
                    game.player.as_ref().unwrap().velocity,
                    Vec3::new(0.25, 0.5, -0.25)
                );
                assert!(game.player_container_menu(a).is_none());
                assert!(game.player_container_menu(b).is_none());
                assert_eq!(b_menu.slot_count(game), None);
                assert!(!game.entity_push(a.entity(), [1.0, 0.0, 0.0]));
            })
            .unwrap();
            assert_eq!(
                game.player.as_ref().unwrap().velocity,
                Vec3::new(0.5, 0.0, 0.0)
            );
        })
        .unwrap();
        assert_eq!(
            b_menu.slot(game, 36),
            Some(Some(crate::game::api_values::test_snapshot(&stone)))
        );
        assert_eq!(
            game.lan
                .as_ref()
                .unwrap()
                .hub
                .guest(13)
                .unwrap()
                .motion
                .as_ref()
                .unwrap()
                .velocity,
            Vec3::new(0.25, 0.5, -0.25)
        );
    });
}

/// Native services can reach B during A's turn without exposing displaced
/// inventories or changing ordinary command resolution during nested turns.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn another_guests_services_work_during_a_turn() {
    use super::{
        commands::Target,
        native_inventories::NativeInventoryKind::{Ender, Player},
    };
    use horizon_client::{advancements as adv, command::args::Selector, items::ItemStack};
    use mod_api::{CriterionContext, CriterionTest, CriterionTestError, TriggerCriterionError};

    struct Accept;
    impl CriterionTest for Accept {
        fn test(&self, criterion: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
            assert_eq!(criterion.conditions_json(), "null");
            Ok(true)
        }
    }
    let trigger = mod_api::Identifier::parse("turns:probe").unwrap();
    let vanilla_trigger = mod_api::Identifier::parse("minecraft:tick").unwrap();

    with_guests(|game| {
        game.build_registries();
        game.level.player_uuid = Some([0, 0, 0, 12]);
        let mut tree = adv::Tree::default();
        tree.list.push(adv::Advancement {
            id: "turns:probe".into(),
            parent: None,
            display: None,
            criteria: vec![adv::Criterion {
                name: "seen".into(),
                trigger: "turns:probe".into(),
                conditions: serde_json::Value::Null.into(),
            }],
            requirements: vec![vec!["seen".into()]],
            rewards: Default::default(),
        });
        tree.by_id.insert("turns:probe".into(), 0);
        tree.by_trigger.insert("turns:probe".into(), vec![(0, 0)]);
        game.adv.tree = Arc::new(tree);
        game.lan.as_mut().unwrap().hub.guests.update_all(|guest| {
            guest.advancements_loaded = true;
        });
        let stone = ItemStack::new("minecraft:stone", 4);
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |guest| guest.inventory[0] = Some(stone.clone()))
            .unwrap();
        let a = game.player(12).unwrap();
        let b = game.player(13).unwrap();
        game.as_guest(12, |game| {
            assert_eq!(game.native_entity_target(a.entity()), Some(Target::Player));
            assert_eq!(
                game.select(&Selector {
                    player_name: Some("guest12".into()),
                    ..Default::default()
                }),
                [Target::Player]
            );
            assert!(game.native_inventory(12, Player).is_err());
            assert_eq!(
                game.native_inventory(13, Player).unwrap().slots[0],
                Some(stone.clone())
            );
            game.native_move_stack((13, Player, 0), &stone, (13, Ender))
                .unwrap();
            assert_eq!(game.native_inventory(13, Player).unwrap().slots[0], None);
            assert_eq!(
                game.native_inventory(13, Ender).unwrap().slots[0],
                Some(stone.clone())
            );
            assert_eq!(
                a.trigger_criterion(game, &vanilla_trigger, Arc::new(Accept)),
                Err(TriggerCriterionError::InvalidTrigger)
            );
            assert_eq!(
                b.trigger_criterion(game, &trigger, Arc::new(Accept)),
                Ok(())
            );
            assert_eq!(
                a.trigger_criterion(game, &trigger, Arc::new(Accept)),
                Ok(())
            );
            assert!(game.native_run_command("give guest13 minecraft:diamond 2"));
            assert!(game.native_run_command("give @a[name=guest13] minecraft:diamond 1"));
            assert!(game.native_run_command(
                "execute as 00000000-0000-0000-0000-00000000000d run give @s minecraft:diamond 1"
            ));
            assert!(game.native_run_command("give guest12 minecraft:apple 1"));
            assert_eq!(
                game.inventory[0],
                Some(ItemStack::new("minecraft:apple", 1))
            );
            game.as_guest(13, |game| {
                assert!(game.native_inventory(12, Player).is_err());
                assert!(game.native_inventory(13, Player).is_err());
                assert_eq!(
                    a.trigger_criterion(game, &trigger, Arc::new(Accept)),
                    Err(TriggerCriterionError::PlayerUnavailable)
                );
                assert!(!game.native_run_command("give guest12 minecraft:dirt 1"));
                assert!(!game.native_run_command("give @a[name=guest12] minecraft:dirt 1"));
                let inventory = game.inventory.clone();
                // An empty execute fork succeeds, but must not touch acting B.
                assert!(game.native_run_command(
                    "execute as 00000000-0000-0000-0000-00000000000c run give @s minecraft:dirt 1"
                ));
                assert_eq!(game.inventory, inventory);
                // The native-only filter is restored, including after failure:
                // ordinary names, UUIDs and selectors still see the outer guest.
                for selector in [
                    Selector {
                        player_name: Some("guest12".into()),
                        ..Default::default()
                    },
                    Selector {
                        uuid: Some(12),
                        ..Default::default()
                    },
                    Selector {
                        kind: 'a',
                        names: vec![("guest12".into(), false)],
                        ..Default::default()
                    },
                ] {
                    assert_eq!(game.select(&selector), [Target::Guest(12)]);
                }
                assert_eq!(
                    game.select(&Selector {
                        player_name: Some("guest13".into()),
                        ..Default::default()
                    }),
                    [Target::Player]
                );
            })
            .unwrap();
        })
        .unwrap();
        let host = game.lan.as_ref().unwrap();
        assert!(
            host.hub
                .guest(12)
                .unwrap()
                .advancements
                .has("turns:probe", "seen")
        );
        let b = host.hub.guest(13).unwrap();
        assert!(b.advancements.has("turns:probe", "seen"));
        assert_eq!(b.ender_items[0], Some(stone));
        assert_eq!(b.inventory[0], Some(ItemStack::new("minecraft:diamond", 4)));
    });
}

/// The body/saddle store and existing armor/offhand move with nested turns.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn nested_guest_turns_restore_each_equipment_store() {
    use horizon_client::items::ItemStack;
    use horizon_items::equipment::{BODY, SADDLE};

    with_guests(|game| {
        let stack = |id| Some(ItemStack::new(id, 1));
        game.armor[0] = stack("minecraft:iron_helmet");
        game.offhand = stack("minecraft:torch");
        game.equipment[BODY] = stack("minecraft:iron_horse_armor");
        for (conn, body, saddle) in [
            (12, "minecraft:golden_horse_armor", "minecraft:saddle"),
            (
                13,
                "minecraft:diamond_horse_armor",
                "minecraft:leather_horse_armor",
            ),
        ] {
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |guest| {
                    guest.equipment[BODY] = stack(body);
                    guest.equipment[SADDLE] = stack(saddle);
                })
                .unwrap();
        }
        game.as_guest(12, |game| {
            assert_eq!(game.equipment[BODY], stack("minecraft:golden_horse_armor"));
            game.as_guest(13, |game| {
                assert_eq!(game.equipment[BODY], stack("minecraft:diamond_horse_armor"));
                game.equipment[SADDLE] = stack("minecraft:saddle");
            })
            .unwrap();
            assert_eq!(game.equipment[BODY], stack("minecraft:golden_horse_armor"));
            assert_eq!(game.equipment[SADDLE], stack("minecraft:saddle"));
        })
        .unwrap();
        assert_eq!(game.armor[0], stack("minecraft:iron_helmet"));
        assert_eq!(game.offhand, stack("minecraft:torch"));
        assert_eq!(game.equipment[BODY], stack("minecraft:iron_horse_armor"));
        assert_eq!(
            game.lan.as_ref().unwrap().hub.guest(13).unwrap().equipment[SADDLE],
            stack("minecraft:saddle")
        );
    });
}

/// A merchant's live session belongs to B even while A is acting.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn another_guests_live_merchant_session_is_visible_during_a_turn() {
    use super::{
        native_merchant_refs::tests::{offer, spawn},
        trading::MerchantState,
    };
    use horizon_client::merchant::{Session, Trader};

    with_guests(|game| {
        let villager = spawn(game, 40, "minecraft:villager");
        let old = offer("minecraft:wheat", "minecraft:emerald");
        let mut live = offer("minecraft:coal", "minecraft:diamond");
        // Opening the screen recomputes the special price for this guest.
        live.special_price_diff = 0;
        game.mobs[0].extra.trade = Some(Box::new(Trader {
            offers: vec![old],
            ..Trader::default()
        }));
        game.mobs[0].extra.flag2 = true;
        let mut session = Session::new(vec![live.clone()], true);
        session.xp = 17;
        session.level = 2;
        game.lan_open_merchant(
            13,
            MerchantState {
                session: Box::new(session),
                scroll: 0,
                dragging: false,
                focused: None,
                mob: Some(40),
                name: None,
            },
        );
        game.as_guest(12, |game| {
            assert_eq!(
                villager.offers(game),
                Some(vec![crate::game::api_values::test_snapshot(&live)])
            );
            assert_eq!(villager.xp(game), Some(17));
            assert_eq!(villager.level(game), Some(2));
            assert_eq!(villager.trading_player(game).map(|p| p.uuid()), Some(13));
        })
        .unwrap();
    });
}

/// The passenger walk (horizon-rust/horizon#8383) keeps #228's host visibility.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn another_guest_follows_a_vehicle_teleported_during_a_turn() {
    with_guests(|game| {
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |guest| guest.vehicle = Some(42))
            .unwrap();
        let horse = game.res.mob_types().get("minecraft:horse").unwrap().clone();
        game.mobs
            .push(Mob::new(42, horse, Vec3::new(3.0, 64.0, 5.0), 0.0));
        // Boarding publishes the seat back to the hub after positioning the body.
        game.guest_boarded(13, None);
        let vehicle = game.entity(42).unwrap();
        let rider = game.entity(13).unwrap();
        let before = [3.0, 64.0 + (1.44375 - 0.6), 5.0];
        let after = [10.0, 70.0 + (1.44375 - 0.6), -4.0];
        game.as_guest(12, |game| {
            assert_eq!(rider.position(game), Some(before));
            assert!(vehicle.teleport_to(game, [10.0, 70.0, -4.0], None));
            assert_eq!(vehicle.position(game), Some([10.0, 70.0, -4.0]));
            assert_eq!(rider.position(game), Some(after));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guest(13).unwrap().vehicle,
                Some(42)
            );
        })
        .unwrap();
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_mounted_guests_turn_publishes_its_teleport_without_dismounting() {
    use super::native_sound_refs::tests::packets;
    use horizon_protocol::packets::play::{Clientbound as C, relative};

    with_guests(|game| {
        let horse = game.res.mob_types().get("minecraft:horse").unwrap().clone();
        game.mobs
            .push(Mob::new(42, horse, Vec3::new(3.0, 64.0, 5.0), 0.0));
        // Ordinary seating sends no teleport. A passenger transition does,
        // even to the same position; an unmounted command keeps its relative axes.
        for (vehicle, teleported, destination) in [
            (Some(42), false, [10.0, 70.0, -4.0]),
            (Some(42), true, [10.0, 70.0, -4.0]),
            (Some(42), true, [3.0, 64.84375, 5.0]),
            (None, true, [10.0, 70.0, -4.0]),
        ] {
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| guest.vehicle = vehicle)
                .unwrap();
            if vehicle.is_some() {
                game.guest_boarded(13, None);
            }
            let (tx, rx) = std::sync::mpsc::channel();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.sender = PlayerSender::new(tx, Arc::default())
                })
                .unwrap();
            game.as_guest(13, |game| {
                // The body left by a passenger transition, before the guest
                // turn publishes it back to the hub and the connection.
                let body = game.player.as_mut().unwrap();
                body.position = Vec3::new(destination[0], destination[1], destination[2]);
                body.teleported = teleported;
                body.teleported_delta = [true, false, true];
                game.yaw = 120.0;
                game.pitch = -15.0;
            })
            .unwrap();
            let guest = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
            assert_eq!(guest.vehicle, vehicle);
            let mut positions = Vec::new();
            for packet in packets(&rx) {
                match packet {
                    C::PlayerPosition(packet) => positions.push(packet),
                    C::SetPassengers(_) => panic!("a passenger transition must not dismount"),
                    _ => {}
                }
            }
            assert_eq!(positions.len(), usize::from(teleported), "{vehicle:?}");
            if teleported {
                let packet = &positions[0];
                assert_eq!(packet.position, destination);
                assert_eq!(packet.delta, [0.0; 3]);
                assert_eq!([packet.yaw, packet.pitch], [120.0, -15.0]);
                let expected = if vehicle.is_some() {
                    relative::DELTA_X | relative::DELTA_Y | relative::DELTA_Z
                } else {
                    relative::DELTA_X | relative::DELTA_Z
                };
                assert_eq!(packet.relatives, expected);
                assert_eq!(guest.position, destination);
                assert_eq!(
                    guest.awaiting_teleport,
                    Some((packet.teleport, destination))
                );
            }
        }
    });
}

/// The split services retain connected identities even while another player's
/// body is swapped in, and still queue acting-player messages after the hook.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn server_services_preserve_identity_and_message_delivery() {
    use horizon_protocol::packets::play::Clientbound as C;
    use mod_api::{ServerHost, Text};
    with_guests(|game| {
        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| {
            g.permission = 3;
            g.game_mode = 1;
            g.address = "192.0.2.12:25565".parse().unwrap();
        });
        guests.update(1, |g| {
            g.permission = 1;
            g.address = "[2001:db8::13]:25566".parse().unwrap();
        });
        let players = game.players().server_players();
        assert_eq!(
            players.iter().map(|p| p.uuid()).collect::<Vec<_>>(),
            [12, 13]
        );
        let a = players[0];
        let b = players[1];
        let host: &dyn ServerHost = game;
        assert_eq!(host.players().player_by_name("GUEST12"), Some(a));
        assert_eq!(host.players().player_by_name("missing"), None);
        assert!(game.players().acting_player().is_none());
        assert_eq!(a.name(game).as_deref(), Some("guest12"));
        assert_eq!(a.permission_level(game), Some(3));
        assert_eq!(b.permission_level(game), Some(1));
        assert_eq!(a.can_use_game_master_blocks(game), Some(true));
        assert_eq!(a.address(game).as_deref(), Some("192.0.2.12"));
        assert_eq!(b.address(game).as_deref(), Some("2001:db8::13"));
        let stale = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(a.entity().epoch().get() + 1),
                a.entity().id(),
            ),
            a.uuid(),
        );
        let wrong_entity = mod_api::__private::game::player_ref(b.entity(), a.uuid());
        let wrong_uuid = mod_api::__private::game::player_ref(a.entity(), 99);
        for invalid in [stale, wrong_entity, wrong_uuid] {
            assert_eq!(invalid.address(game), None);
            assert_eq!(invalid.permission_level(game), None);
        }
        assert!(game.commands().run_command("list"));
        game.chat_capture = Some(Vec::new());
        game.commands()
            .feedback(Text::literal("command answer"))
            .unwrap();
        assert_eq!(
            game.chat_capture.take().unwrap()[0].plain(),
            "command answer"
        );
        assert_eq!(game.messages().text_allowed("message"), Some(true));
        game.as_guest(12, |game| {
            assert_eq!(game.players().acting_player(), Some(a));
            assert_eq!(game.players().server_players(), players);
            assert_eq!(a.address(game).as_deref(), Some("192.0.2.12"));
            assert_eq!(b.address(game).as_deref(), Some("2001:db8::13"));
            assert_eq!(a.permission_level(game), Some(3));
            assert_eq!(b.permission_level(game), Some(1));
            assert_eq!(a.can_use_game_master_blocks(game), Some(true));
            // Acting permissions come from the swapped state, not a parked record.
            game.ui.registries.permission = 1;
            assert_eq!(a.permission_level(game), Some(1));
            assert_eq!(a.can_use_game_master_blocks(game), Some(false));
            game.ui.registries.permission = 3;
            assert!(a.tell(game, Text::literal("chat")).unwrap());
            assert!(a.action_bar(game, Text::literal("overlay")).unwrap());
            assert!(
                a.tab_list(game, &Text::literal("header"), &Text::literal("footer"))
                    .unwrap()
            );
            let packets = &game.guest_packets;
            assert!(matches!(&packets[packets.len() - 3], C::SystemChat(p) if !p.overlay));
            assert!(matches!(&packets[packets.len() - 2], C::SystemChat(p) if p.overlay));
            assert!(matches!(&packets[packets.len() - 1], C::TabList(_)));
            assert!(b.tell(game, Text::literal("other player")).unwrap());
            assert!(b.set_nickname(game, Some("B")));
            assert!(b.set_sleep_ignored(game, true));
            game.as_guest(13, |game| {
                assert_eq!(game.players().acting_player(), Some(b));
                assert_eq!(game.players().server_players(), players);
                assert_eq!(a.name(game).as_deref(), Some("guest12"));
                assert_eq!(a.permission_level(game), Some(3));
                assert_eq!(b.permission_level(game), Some(1));
                assert_eq!(a.address(game).as_deref(), Some("192.0.2.12"));
                assert_eq!(b.address(game).as_deref(), Some("2001:db8::13"));
                assert_eq!(a.game_mode(game), None, "body remains displaced");
                assert_eq!(a.can_use_game_master_blocks(game), None);
                for invalid in [stale, wrong_entity, wrong_uuid] {
                    assert_eq!(invalid.address(game), None);
                    assert_eq!(invalid.permission_level(game), None);
                }
                assert!(
                    a.tell(game, Text::literal("displaced but connected"))
                        .unwrap()
                );
            })
            .unwrap();
        })
        .unwrap();
        // Vanish is UUID-persistent, not a live-body operation.
        game.lan.as_mut().unwrap().hub.guests.clear();
        assert!(a.hide(game, Some(2)));
        assert_eq!(game.lan.as_ref().unwrap().hub.hidden.get(&12), Some(&2));
        assert!(!a.tell(game, Text::literal("offline")).unwrap());
        assert!(!a.set_nickname(game, Some("offline")));
        assert!(!a.set_sleep_ignored(game, true));
        assert_eq!(a.address(game), None);
        assert_eq!(a.permission_level(game), None);
        assert!(!stale.hide(game, None));
        assert_eq!(stale.name(game), None);
    });
}

/// Permission reads keep the existing owner and fake-player contracts, not
/// just the connected-player list that the services enumerate.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn canonical_permissions_preserve_owner_and_fake_player_reads() {
    use horizon_client::motion::PlayerMotion;
    use mod_api::GameMode;

    with_guests(|game| {
        let _serving = super::integrated::tests::install_serving(game, false);
        game.level.player_uuid = Some([0, 0, 0, 99]);
        let mut body = PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap();
        body.entity_id = 99;
        game.player = Some(body);
        game.set_mode(horizon_client::command::args::GameMode::Creative);
        let owner = game.player(99).unwrap();
        for (allow, permission) in [(false, 0), (true, 4)] {
            game.set_allow_commands(allow);
            assert_eq!(owner.permission_level(game), Some(permission));
            assert_eq!(owner.can_use_game_master_blocks(game), Some(allow));
        }
        game.set_mode(horizon_client::command::args::GameMode::Survival);
        assert_eq!(owner.permission_level(game), Some(4));
        assert_eq!(owner.can_use_game_master_blocks(game), Some(false));

        game.next_entity = 100;
        let level = game.level(game.dim.id()).unwrap();
        let fake = game.fake_player(level, 1234, "machine").unwrap();
        assert!(!game.players().server_players().contains(&fake));
        assert_eq!(game.players().player_by_name("machine"), None);
        assert_eq!(fake.permission_level(game), Some(0));
        assert_eq!(
            fake.address(game),
            None,
            "no connection outside an acting turn"
        );
        assert_eq!(fake.can_use_game_master_blocks(game), Some(false));
        assert!(fake.set_game_mode(game, GameMode::Creative));
        assert_eq!(fake.permission_level(game), Some(0));
        assert_eq!(fake.can_use_game_master_blocks(game), Some(false));
        let conn = game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == fake.uuid())
            .unwrap()
            .conn;
        game.as_guest(conn, |game| {
            assert_eq!(fake.permission_level(game), Some(0));
            assert_eq!(
                fake.address(game).as_deref(),
                Some(""),
                "unknown acting address"
            );
            assert_eq!(fake.can_use_game_master_blocks(game), Some(false));
        })
        .unwrap();
        assert!(game.player_remove_fake(fake));
        assert_eq!(fake.permission_level(game), None);
        assert_eq!(fake.address(game), None);
    });
}

/// The UUIDs of the players in the level, checked against the full reader.
fn level_uuids(game: &super::Game<'_>) -> Vec<u128> {
    let uuids: Vec<u128> = game.level_player_uuids(game.dim.id()).collect();
    let players: Vec<u128> = game.level_players().iter().map(|p| p.uuid).collect();
    assert_eq!(uuids, players, "both enumerations select the same players");
    uuids
}

/// The host in place, with a body and UUID 99.
pub(in crate::game) fn place_host(game: &mut super::Game<'_>) {
    let mut body =
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap();
    body.entity_id = 99;
    game.player = Some(body);
    game.level.player_uuid = Some([0, 0, 0, 99]);
}

/// A turn lists the acting player once, under their own UUID and with their
/// live body. The host's body is displaced for the turn, so it is not listed.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn level_players_name_each_body_once_during_turns() {
    with_guests(|game| {
        place_host(game);
        assert_eq!(level_uuids(game), [99, 12, 13]);
        game.as_guest(12, |game| {
            assert_eq!(level_uuids(game), [12, 13]);
            assert_eq!(game.player_view().unwrap().uuid, [0, 0, 0, 12]);
            game.player.as_mut().unwrap().position.x = 40.0;
            assert_eq!(game.level_players()[0].position.x, 40.0);
            game.as_guest(13, |game| {
                assert_eq!(level_uuids(game), [13, 12]);
                assert!(game.as_guest(12, |_| ()).is_none());
            })
            .unwrap();
            assert_eq!(level_uuids(game), [12, 13]);
        })
        .unwrap();
        assert_eq!(level_uuids(game), [99, 12, 13]);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn level_players_exclude_guests_in_other_levels() {
    with_guests(|game| {
        place_host(game);
        let host = game.lan.as_mut().unwrap();
        let elsewhere = host.hub.guests.connection_index(13).unwrap();
        host.hub.guests.update(elsewhere, |guest| {
            guest.dimension = "minecraft:the_nether".to_owned();
        });
        assert_eq!(level_uuids(game), [99, 12]);
        game.as_guest(12, |game| assert_eq!(level_uuids(game), [12]))
            .unwrap();
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn level_players_exclude_fake_players() {
    with_guests(|game| {
        let _serving = super::integrated::tests::install_serving(game, false);
        place_host(game);
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
            .find(|g| g.uuid == fake.uuid())
            .unwrap()
            .conn;
        assert_eq!(level_uuids(game), [99, 12, 13]);
        game.as_guest(conn, |game| {
            assert_eq!(level_uuids(game), [12, 13]);
            game.as_guest(12, |game| assert_eq!(level_uuids(game), [12, 13]))
                .unwrap();
        })
        .unwrap();
        assert_eq!(level_uuids(game), [99, 12, 13]);
    });
}

/// A listener sees the newly granted criterion before its recipe reward on
/// both a player's first tick and a resource reload.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn tick_advancement_notice_precedes_recipe_reward() {
    use crate::game::native_entity_refs::tests::run_fresh_cases;
    use mod_api::{Hooks, ModEntry, Mods, PlayerAdvancementProgressEvent, ServerHost};
    use std::sync::Mutex;

    if run_fresh_cases(
        "game::native_guest_turn_tests::tick_advancement_notice_precedes_recipe_reward",
        "HORIZON_ADVANCEMENT_ORDER_CHILD",
        &["first", "reload"],
    ) {
        return;
    }
    #[derive(Clone)]
    struct Observer(Arc<Mutex<Vec<Option<bool>>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn player_advancement_progress(
            &self,
            host: &mut dyn ServerHost,
            event: &mut PlayerAdvancementProgressEvent<'_>,
        ) {
            if event.advancement == "testmod:tick" {
                self.0.lock().unwrap().push(
                    host.player(event.player)
                        .and_then(|p| p.has_discovered_recipe(host, "testmod:reward")),
                );
            }
        }
    }
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "observer",
        Box::new(Observer(observed.clone())),
    )]
    .into();
    mod_platform::__private::install(mods);
    let scratch =
        std::env::temp_dir().join(format!("horizon-tick-advancement-{}", std::process::id()));
    let dir = scratch.join("data/testmod/advancement");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tick.json"), r#"{"criteria":{"seen":{"trigger":"minecraft:tick"}},"rewards":{"recipes":["testmod:reward"]}}"#).unwrap();
    with_guests(|game| {
        let mut sync = horizon_client::lan::data::SyncData::empty();
        sync.advancements = horizon_client::lan::ServerAdvancements::load(
            &scratch,
            &horizon_world::loot::PackOverlay::NONE,
            // Rewards must name a recipe registered in the server's index.
            std::collections::BTreeMap::from([(
                "testmod:reward".into(),
                r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:diamond","count":1}}"#.into(),
            )]),
            Default::default(),
        );
        let reload = std::env::var("HORIZON_ADVANCEMENT_ORDER_CHILD").as_deref() == Ok("reload");
        let host = game.lan.as_mut().unwrap();
        host.hub.guests.update_all(|guest| {
            guest.advancements_loaded = reload;
        });
        host.hub.mods_loaded = reload;
        game.with_advancement_hub(|hub, owners| hub.reload_resources(Arc::new(sync), owners));
        if reload {
            game.run_native_advancement_grants();
        } else {
            game.tick_lan(false);
        }
        let host = game.lan.as_ref().unwrap();
        assert!(
            host.hub
                .guest(12)
                .unwrap()
                .recipe_book
                .knows("testmod:reward")
        );
        assert_eq!(*observed.lock().unwrap(), vec![Some(false), Some(false)]);
    });
    std::fs::remove_dir_all(scratch).unwrap();
}

/// `PlayerRef::set_selected_slot` from inside the guest's own turn: their state is in place, so
/// the slot is written there, and their client is still told (`leave_guest` sends nothing).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn set_selected_slot_tells_an_acting_guests_client() {
    with_guests(|game| {
        let (tx, rx) = std::sync::mpsc::channel();
        let host = game.lan.as_mut().unwrap();
        let index = host.hub.guests.iter().position(|g| g.conn == 13).unwrap();
        host.hub.guests.update(index, |guest| {
            guest.sender = PlayerSender::new(tx, Arc::default())
        });
        game.as_guest(13, |game| {
            let player = game.player(13).unwrap();
            while rx.try_recv().is_ok() {}
            assert!(player.set_selected_slot(game, 8));
            assert!(
                rx.try_recv().is_ok(),
                "SetHeldSlot reaches the acting guest's client"
            );
            assert!(!player.set_selected_slot(game, 9));
        })
        .unwrap();
        assert_eq!(game.lan.as_ref().unwrap().hub.guests[index].selected, 8);
        // Parked again: the write goes to the guest record and is sent the same way.
        let player = game.player(13).unwrap();
        while rx.try_recv().is_ok() {}
        assert!(player.set_selected_slot(game, 0));
        assert!(
            rx.try_recv().is_ok(),
            "SetHeldSlot reaches a parked guest's client"
        );
        assert_eq!(game.lan.as_ref().unwrap().hub.guests[index].selected, 0);
    });
}

/// `PlayerRef::drop_stack` hands a refused stack back: an empty one, and one a mod's `item_toss`
/// vetoes, which the game itself would discard.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_vetoed_drop_gives_the_stack_back() {
    use super::api_values::TryIntoApi;
    use horizon_client::items::ItemStack;
    use mod_api::{Flow, Hooks, ItemTossEvent, ModEntry, ServerHost};
    use mod_platform::testing::with_mods;

    struct Veto;
    #[mod_api::__subscriptions]
    impl Hooks for Veto {
        fn item_toss(&self, _: &mut dyn ServerHost, _: &mut ItemTossEvent) -> Flow {
            Flow::Cancel
        }
    }
    let mods = vec![ModEntry::new("veto", Box::new(Veto))];
    with_mods(mods.into(), || {
        with_guests(|game| {
            let player = game.player(13).unwrap();
            let stone = ItemStack::new("minecraft:stone", 3);
            let before = game.dropped.len();
            assert_eq!(
                player.drop_stack(game, stone.try_into_api().unwrap(), true),
                Ok(Some(stone.try_into_api().unwrap())),
                "a vetoed toss comes back whole"
            );
            assert_eq!(game.dropped.len(), before, "and makes no item entity");
            let empty = ItemStack::new("minecraft:stone", 0);
            assert_eq!(
                game.player_drop_stack(player, empty.clone(), false),
                Some(empty)
            );
        });
    });
}

struct PanicOnce(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for PanicOnce {
    fn living_incoming_damage(
        &self,
        _: &mut dyn mod_api::LevelRandomHost,
        _: &mut mod_api::LivingIncomingDamageEvent<'_>,
    ) -> mod_api::Flow {
        if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            std::panic::panic_any(73129_u32);
        }
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn guest_damage_unwind_restores_and_next_same_connection_hit_lands() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "panic_once",
        Box::new(PanicOnce(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.loaded = true;
                    guest.load_timeout = 0;
                })
                .unwrap();
            game.as_guest(13, |g| {
                use horizon_client::command::args::GameMode;
                g.set_game_mode(GameMode::Creative);
                g.set_game_mode(GameMode::Survival);
            })
            .unwrap();
            let outer = game.lan_acting;
            let position = game.player.as_ref().map(|p| p.position);
            let health = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(13)
                .unwrap()
                .motion
                .as_ref()
                .unwrap()
                .health
                .health();
            let hit = |g: &mut super::Game<'_>| {
                g.pvp_land(
                    super::lan_pvp::Victim::Guest(13),
                    &super::lan_pvp::Strike::test_blow(12),
                )
            };
            let panic =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hit(game))).unwrap_err();
            assert_eq!(panic.downcast_ref::<u32>(), Some(&73129));
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert!(game.lan_guest_swaps.is_empty());
            assert_eq!(game.lan_acting, outer);
            assert_eq!(game.player.as_ref().map(|p| p.position), position);
            assert!(hit(game));
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
            assert!(
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(13)
                    .unwrap()
                    .motion
                    .as_ref()
                    .unwrap()
                    .health
                    .health()
                    < health
            );
            assert!(game.lan_guest_swaps.is_empty());
            assert_eq!(game.lan_acting, outer);
            assert_eq!(game.player.as_ref().map(|p| p.position), position);
            assert_eq!(
                game.as_guest(13, |g| {
                    assert!(g.as_guest(13, |_| ()).is_none());
                    42
                }),
                Some(42)
            );
            assert_eq!(game.as_guest(u32::MAX, |_| 0), None);
        })
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
/// A real native panic inside `as_guest_messaged` puts the outer message capture back, and the
/// same connection's next messaged turn runs and captures nothing outside it (#2244).
fn guest_message_capture_is_restored_after_a_real_native_panic() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "capture_repro",
        Box::new(PanicOnce(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            for conn in [12, 13] {
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |guest| {
                        guest.loaded = true;
                        guest.load_timeout = 0;
                    })
                    .unwrap();
                game.as_guest(conn, |g| {
                    use horizon_client::command::args::GameMode;
                    g.set_game_mode(GameMode::Creative);
                    g.set_game_mode(GameMode::Survival);
                })
                .unwrap();
            }
            assert!(game.chat_capture.is_none());
            let outer = game.lan_acting;
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                game.as_guest_messaged(13, |g| {
                    g.pvp_land(
                        super::lan_pvp::Victim::Guest(12),
                        &super::lan_pvp::Strike::test_blow(13),
                    )
                })
            }))
            .unwrap_err();
            assert_eq!(panic.downcast_ref::<u32>(), Some(&73129));
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert!(
                game.lan_guest_swaps.is_empty(),
                "guest swap fix must have run"
            );
            assert_eq!(game.lan_acting, outer);
            assert!(
                game.chat_capture.is_none(),
                "real native panic leaked the guest message capture"
            );
            assert!(
                game.as_guest_messaged(13, |g| g.pvp_land(
                    super::lan_pvp::Victim::Guest(12),
                    &super::lan_pvp::Strike::test_blow(13),
                ))
                .is_some(),
                "the same connection's next messaged turn runs"
            );
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
            assert!(game.chat_capture.is_none() && game.lan_guest_swaps.is_empty());
        });
    });
}

struct PanicOnFood(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for PanicOnFood {
    fn food_level_changed(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::FoodLevelChangedEvent<'_>,
    ) -> mod_api::Flow {
        if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            std::panic::panic_any(52417_u32);
        }
        mod_api::Flow::Continue
    }
}

/// A native `food_level_changed` listener that panics while `leave_guest` reports the turn's
/// food change still lets the guest swap be restored and the turn's progress be written back,
/// and the same connection's next turn runs and is heard (#2240).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_panic_in_a_turn_end_callback_still_restores_the_guest() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "turn_end_panic",
        Box::new(PanicOnFood(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.loaded = true;
                    guest.load_timeout = 0;
                })
                .unwrap();
            let food =
                |game: &super::Game<'_>| game.lan.as_ref().unwrap().hub.guest(13).unwrap().food;
            let before = food(game);
            let outer = game.lan_acting;
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                game.as_guest(13, |g| g.player.as_mut().unwrap().health.food.food -= 3)
            }))
            .unwrap_err();
            assert_eq!(panic.downcast_ref::<u32>(), Some(&52417));
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert!(
                game.lan_guest_swaps.is_empty(),
                "the guest swap was restored"
            );
            assert_eq!(game.lan_acting, outer);
            assert_eq!(
                food(game),
                before - 3,
                "the turn's food change is written back"
            );

            let next = game.as_guest(13, |g| {
                g.player.as_mut().unwrap().health.food.food -= 1;
                7
            });
            assert_eq!(next, Some(7), "the same connection's next turn runs");
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
            assert_eq!(food(game), before - 4);
            assert!(game.lan_guest_swaps.is_empty());
        });
    });
}

/// An armor stand summoned at (1.5, 70, 1.5) in the level in place, which `with_guests` puts
/// its two players beside, at (3, 64, 5).
fn summon_stand(game: &mut super::Game<'_>) -> mod_api::EntityRef {
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
    let (horizon_core::nbt::Tag::Compound(tag), _) =
        horizon_core::nbt::parse_snbt("{NoGravity:1b}").unwrap()
    else {
        unreachable!()
    };
    let id = game
        .summon(
            game.dim.id(),
            "minecraft:armor_stand",
            [1.5, 70.0, 1.5],
            tag,
        )
        .unwrap();
    game.entity(id).unwrap()
}

/// Whether anything the server sent the acting player since `before` is a chat line.
fn told_since(game: &super::Game<'_>, before: usize) -> bool {
    use horizon_protocol::packets::play::Clientbound as C;
    game.guest_packets[before..]
        .iter()
        .any(|p| matches!(p, C::SystemChat(_)))
}

/// `withPermission(level)`: the line is checked at the level the mod gives, whatever the
/// entity's own (a guest's is 0 here), with no ceiling. `TickCommand` needs admins (3), so game
/// masters (2) are refused it and admins run it; `GiveCommand` needs game masters.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn run_command_checks_the_line_at_the_level_given() {
    use mod_api::PermissionLevel::{Admins, All, Gamemasters, Owners};

    with_guests(|game| {
        game.build_registries();
        let a = game.player(12).unwrap().entity();
        assert!(!a.run_command(game, "give @s minecraft:diamond 1", All));
        assert!(a.run_command(game, "give @s minecraft:diamond 1", Gamemasters));
        assert!(!a.run_command(game, "tick query", Gamemasters));
        assert!(a.run_command(game, "tick query", Admins));
        assert!(a.run_command(game, "tick query", Owners));
        let stand = summon_stand(game);
        assert!(!stand.run_command(game, "tick query", Gamemasters));
        assert!(stand.run_command(game, "tick query", Admins));
    });
}

/// The entity is the executor, from its own position in its level: `@s` is it and `~` its
/// position, not the server spawn's (from there the stand would end at (2, 64, -1)). A player
/// out of a turn, the acting player and another player during that turn are each `@s`.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn run_command_runs_as_the_entity_from_where_it_is() {
    use super::native_inventories::NativeInventoryKind::Player;
    use horizon_client::items::ItemStack;
    use mod_api::PermissionLevel::Gamemasters;

    with_guests(|game| {
        game.build_registries();
        let a = game.player(12).unwrap().entity();
        let b = game.player(13).unwrap().entity();
        let holds = |game: &super::Game<'_>, uuid: u128, item: &str| {
            let stack = ItemStack::new(item, 1);
            let slots = game.native_inventory(uuid, Player).unwrap().slots;
            slots.iter().flatten().any(|s| *s == stack)
        };
        let stand = summon_stand(game);
        let guest = |game: &super::Game<'_>, conn: u32| {
            game.lan.as_ref().unwrap().hub.guest(conn).unwrap().position
        };

        assert!(stand.run_command(game, "/tp @s ~2 ~ ~-1", Gamemasters));
        assert_eq!(stand.position(game), Some([3.5, 70.0, 0.5]));
        assert_eq!(guest(game, 12), [3.0, 64.0, 5.0]);
        assert_eq!(guest(game, 13), [3.0, 64.0, 5.0]);

        assert!(a.run_command(game, "give @s minecraft:diamond 1", Gamemasters));
        game.as_guest(12, |game| {
            assert!(a.run_command(game, "give @s minecraft:apple 1", Gamemasters));
            assert!(b.run_command(game, "give @s minecraft:stick 1", Gamemasters));
        })
        .unwrap();
        assert!(holds(game, 12, "minecraft:diamond"));
        assert!(holds(game, 12, "minecraft:apple"));
        assert!(holds(game, 13, "minecraft:stick"));
        assert!(!holds(game, 13, "minecraft:diamond"));
        assert!(!holds(game, 12, "minecraft:stick"));
    });
}

/// A player's source answers them as their own command does, feedback and failure both
/// (`ServerPlayer.commandSource`); another entity's says nothing to anyone, not even the
/// player whose turn it runs in (`CommandSource.NULL`).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn run_command_answers_a_player_and_no_one_for_another_entity() {
    use mod_api::PermissionLevel::Admins;

    with_guests(|game| {
        game.build_registries();
        let a = game.player(12).unwrap().entity();
        let stand = summon_stand(game);
        game.as_guest(12, |game| {
            for (line, ran) in [("tick query", true), ("kill @e[type=minecraft:pig]", false)] {
                let before = game.guest_packets.len();
                assert_eq!(a.run_command(game, line, Admins), ran, "{line}");
                assert!(told_since(game, before), "the player is told: {line}");
                let before = game.guest_packets.len();
                assert_eq!(stand.run_command(game, line, Admins), ran, "{line}");
                assert!(!told_since(game, before), "no one is told: {line}");
            }
        })
        .unwrap();
    });
}

/// A line that doesn't parse, or parses and fails, answers false without a panic, and so does a
/// handle of an earlier open or of an entity that is gone.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn run_command_answers_false_for_a_bad_line_or_a_stale_handle() {
    use mod_api::PermissionLevel::Owners;

    with_guests(|game| {
        game.build_registries();
        let a = game.player(12).unwrap().entity();
        let stand = summon_stand(game);
        for entity in [a, stand] {
            for line in [
                "",
                "/",
                "no_such_command",
                "give @s minecraft:diamond many",
                "tp @s ~ ~ ~ ~ ~ ~ ~",
                "kill @e[type=minecraft:pig]",
            ] {
                assert!(!entity.run_command(game, line, Owners), "{line:?}");
            }
        }
        let stale = mod_api::EntityRef::new(mod_api::WorldEpoch::new(a.epoch().get() + 1), a.id());
        assert!(!stale.run_command(game, "tick query", Owners));
        assert!(stand.run_command(game, "kill @s", Owners));
        assert!(!stand.run_command(game, "tick query", Owners));
    });
}

/// A live player in another dimension is outside the level in place: that level is parked
/// while the hook runs, so the call answers false and nothing runs, neither the part about
/// the player (`give @s`) nor the part about the server (`gamerule`). Back in the level in
/// place, the same player and line run.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn run_command_refuses_a_player_whose_level_is_parked() {
    use super::native_inventories::NativeInventoryKind::Player;
    use horizon_client::{gamerules::vanilla::SEND_COMMAND_FEEDBACK, items::ItemStack};
    use mod_api::PermissionLevel::Owners;

    with_guests(|game| {
        game.build_registries();
        let b = game.player(13).unwrap().entity();
        let move_to = |game: &mut super::Game<'_>, dimension: &str| {
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| guest.dimension = dimension.to_owned())
                .unwrap();
        };
        let has_stick = |game: &super::Game<'_>| {
            let stick = ItemStack::new("minecraft:stick", 1);
            let slots = game.native_inventory(13, Player).unwrap().slots;
            slots.iter().flatten().any(|s| *s == stick)
        };
        assert!(game.ui.rules.bool_at(SEND_COMMAND_FEEDBACK));

        move_to(game, "minecraft:the_nether");
        assert!(!b.run_command(game, "give @s minecraft:stick 1", Owners));
        assert!(!b.run_command(game, "gamerule send_command_feedback false", Owners));
        assert!(game.ui.rules.bool_at(SEND_COMMAND_FEEDBACK));

        move_to(game, "minecraft:overworld");
        assert!(!has_stick(game));
        assert!(b.run_command(game, "give @s minecraft:stick 1", Owners));
        assert!(has_stick(game));
    });
}

/// `ServerPlayer.createCommandSourceStack`: a player's source is their own position and
/// rotation, also when the mod calls from inside an outer command run from elsewhere
/// (`execute positioned 50 80 50 rotated 90 30 run <a mod's command>`), for a player entered
/// for it and for the acting player in their own turn. From the outer source, `~1 ~ ~ ~10 ~`
/// would put them at (51, 80, 50) facing yaw 100, pitch 30.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn run_command_runs_from_the_players_own_place_inside_an_outer_command() {
    use super::commands::execute::Source;
    use mod_api::PermissionLevel::Gamemasters;

    with_guests(|game| {
        game.build_registries();
        let a = game.player(12).unwrap().entity();
        let b = game.player(13).unwrap().entity();
        let place = |game: &super::Game<'_>, conn: u32| {
            let g = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            (g.position, [g.yaw, g.pitch])
        };
        let (start, [yaw, pitch]) = place(game, 12);
        assert_eq!(place(game, 13), (start, [yaw, pitch]));
        let outer = Source {
            pos: [50.0, 80.0, 50.0],
            rot: [90.0, 30.0],
            ..game.server_source()
        };
        game.with_source(outer, |game| {
            assert!(b.run_command(game, "tp @s ~1 ~ ~ ~10 ~", Gamemasters));
            game.as_guest(12, |game| {
                assert!(a.run_command(game, "tp @s ~1 ~ ~ ~10 ~", Gamemasters));
            })
            .unwrap();
        });
        for conn in [12, 13] {
            assert_eq!(
                place(game, conn),
                ([start[0] + 1.0, start[1], start[2]], [yaw + 10.0, pitch]),
                "guest {conn}"
            );
        }
    });
}
