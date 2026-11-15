use crate::game::commands::Target;
use crate::game::{entity_residency::SlotAccess, native_guest_turn_tests::with_guests};
use horizon_client::join_seq::residency::table::EntityLookup;
use horizon_client::join_seq::residency::{Kind, Source};
use horizon_core::geometry::Vec3;
use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
};

#[test]
#[ignore = "daily"]
fn actual_guest_membership_and_lifetime_survive_nested_body_loans_and_reset() {
    with_guests(|game| {
        use horizon_mod_runtime::attachments::{AttachmentType, Holder, index::AttachmentIndex};
        const VALUE: AttachmentType<i32> = AttachmentType {
            namespace: "source_test",
            name: "value",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let dimension = game.dim.key();
        let id = NonZeroU32::new(12).unwrap();
        let host = game.lan.as_mut().unwrap();
        host.hub.guests.update(0, |guest| guest.placed = false);
        let attachments = host.hub.guests.attachments_mut(12).unwrap();
        VALUE.set(attachments, Holder::Level, 7).unwrap();
        let registration = attachments.bind_index(&index, 12u32);
        assert!(matches!(
            game.server_resident_entity(12),
            EntityLookup::Found {
                value: Target::Guest(12),
                ..
            }
        ));
        assert_eq!(game.uuid_for(12), [0, 0, 0, 12]);
        assert_eq!(
            game.with_resident_slot(&dimension, id, |slot| slot
                .registration()
                .unwrap()
                .same_registration(&registration)),
            SlotAccess::Done(true)
        );
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| guest.placed = true);
        game.as_guest(12, |outer| {
            assert!(matches!(
                outer.server_resident_entity(12),
                EntityLookup::Found {
                    value: Target::Player,
                    ..
                }
            ));
            assert_eq!(
                outer.with_resident_slot(&dimension, id, |slot| slot
                    .registration()
                    .unwrap()
                    .same_registration(&registration)),
                SlotAccess::Done(true)
            );
            outer
                .as_guest(13, |inner| {
                    assert_eq!(inner.server_resident_entity(12), EntityLookup::Taken);
                    assert_eq!(
                        inner.with_resident_slot::<()>(&dimension, id, |_| panic!(
                            "displaced body lent a slot"
                        )),
                        SlotAccess::Taken
                    );
                    inner.reset_entity_residency();
                    assert_eq!(inner.server_resident_entity(12), EntityLookup::Taken);
                    assert_eq!(inner.uuid_for(12), [0, 0, 0, 12]);
                    assert!(matches!(
                        inner.server_resident_entity(13),
                        EntityLookup::Found {
                            value: Target::Player,
                            ..
                        }
                    ));
                })
                .unwrap();
            assert_eq!(
                outer.with_resident_slot(&dimension, id, |slot| slot
                    .registration()
                    .unwrap()
                    .same_registration(&registration)),
                SlotAccess::Done(true)
            );
        })
        .unwrap();
        assert_eq!(
            game.with_resident_slot(&dimension, id, |slot| slot
                .registration()
                .unwrap()
                .same_registration(&registration)),
            SlotAccess::Done(true)
        );
        let removed = game.lan.as_mut().unwrap().hub.guests.remove(0);
        assert_eq!(game.server_resident_entity(12), EntityLookup::Missing);
        assert_eq!(
            index
                .lock()
                .unwrap()
                .snapshot(&("source_test".into(), "value".into()))
                .len(),
            0
        );
        drop(removed);
    });
}

#[test]
#[ignore = "daily"]
fn refused_body_construction_does_not_begin_guest_or_hook_loans() {
    with_guests(|game| {
        game.as_guest(12, |g| {
            g.install_fishing_hook(crate::game::fishing::Hook::remote(
                900,
                Vec3::ZERO,
                [1, 2, 3, 4],
            ))
        })
        .unwrap();
        assert!(matches!(
            game.server_resident_entity(12),
            EntityLookup::Found {
                value: Target::Guest(12),
                ..
            }
        ));
        assert!(matches!(
            game.server_resident_entity(900),
            EntityLookup::Found {
                value: Target::Hook(900),
                ..
            }
        ));
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let mut bad =
            horizon_client::motion::MotionResources::load(&root, &game.pack.assets).unwrap();
        bad.attributes = std::sync::Arc::new(horizon_world::attributes::entity::Catalog::from_json(&serde_json::json!({
            "data_version": horizon_core::version::DATA_VERSION, "attributes": [], "entities": {}
        })).unwrap());
        assert!(horizon_client::motion::PlayerMotion::new(&bad, Vec3::ZERO).is_err());
        let original = game.motion;
        game.motion = Box::leak(Box::new(bad));
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(12, |g| g.motion = None);
        let permission = game.ui.registries.permission;
        assert!(game.as_guest(12, |_| panic!("refused turn ran")).is_none());
        game.motion = original;
        assert_eq!(game.ui.registries.permission, permission);
        assert!(game.lan_guest_swaps.is_empty());
        assert!(matches!(
            game.server_resident_entity(12),
            EntityLookup::Found {
                value: Target::Guest(12),
                ..
            }
        ));
        assert!(matches!(
            game.server_resident_entity(900),
            EntityLookup::Found {
                value: Target::Hook(900),
                ..
            }
        ));
        game.as_guest(12, |outer| {
            outer
                .as_guest(13, |inner| assert!(inner.as_guest(12, |_| ()).is_none()))
                .unwrap();
        })
        .expect("same connection can enter after refusal");
        assert!(game.lan_guest_swaps.is_empty());
        assert!(matches!(
            game.server_resident_entity(900),
            EntityLookup::Found { .. }
        ));
    });
}

#[test]
#[ignore = "daily"]
fn owned_hooks_keep_their_own_uuid_and_source_across_nested_actor_loans() {
    with_guests(|game| {
        let first = [101, 102, 103, 104];
        let second = [201, 202, 203, 204];
        game.as_guest(12, |outer| {
            outer.install_fishing_hook(crate::game::fishing::Hook::remote(900, Vec3::ZERO, first));
            assert!(matches!(
                outer.server_resident_entity(900),
                EntityLookup::Found {
                    value: Target::Hook(900),
                    ..
                }
            ));
            assert_eq!(outer.uuid_for(900), first);
            outer
                .as_guest(13, |inner| {
                    assert_eq!(inner.server_resident_entity(900), EntityLookup::Taken);
                    assert_eq!(inner.uuid_for(900), first);
                    inner.install_fishing_hook(crate::game::fishing::Hook::remote(
                        901,
                        Vec3::ZERO,
                        second,
                    ));
                    assert_eq!(inner.uuid_for(901), second);
                    assert!(inner.fishing_hook_by_id(900).is_none());
                })
                .unwrap();
            assert_eq!(outer.uuid_for(900), first);
            assert_eq!(outer.fishing_hook_by_id(900).unwrap().1, Target::Guest(12));
            assert_eq!(outer.fishing_hook_by_id(901).unwrap().1, Target::Guest(13));
            outer.reset_entity_residency();
            assert_eq!(outer.uuid_for(900), first);
            assert_eq!(outer.uuid_for(901), second);
            outer.remove_fishing_hook(901);
            assert_eq!(outer.server_resident_entity(901), EntityLookup::Missing);
            assert!(outer.fishing_hook_by_id(900).is_some());
        })
        .unwrap();
        assert_eq!(game.fishing_hook_by_id(900).unwrap().1, Target::Guest(12));
        game.remove_fishing_hook(900);
        assert_eq!(game.server_resident_entity(900), EntityLookup::Missing);
    });
}

#[test]
#[ignore = "daily"]
fn production_login_and_local_body_constructor_admit_profile_and_real_loan() {
    use horizon_client::lan::{NetConfig, guest::Guest, hub::HubConfig};
    use horizon_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C},
    };
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let (pipe, _server) = Pipe::pair();
        game.connect_local(pipe, "SourceProfile").unwrap();
        let profile = game.remote.as_ref().unwrap().profile();
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 2,
            simulation_distance: 2,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            online_mode: false,
            enforces_secure_chat: false,
            spawn: play::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 1,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
        }))
        .unwrap();
        assert_eq!(game.server_resident_entity(7), EntityLookup::Missing);
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [0.5, 65.0, 0.5],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives: 0,
        }))
        .unwrap();
        let EntityLookup::Found {
            location,
            value: Target::Player,
        } = game.server_resident_entity(7)
        else {
            panic!("actual local constructor was not admitted");
        };
        assert_eq!(location.source, Source::LocalPlayer);
        let expected = horizon_client::lan::playerdata::uuid_ints(profile);
        assert_eq!(game.uuid_for(7), expected);
        let settings = crate::game::lan::LanSettings {
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
            dimension: game.dim.id().into(),
            allow_flight: true,
            seed: 0,
            max_players: 2,
            integrated: true,
            levels: game.level_shapes,
            level_seeds: game.levels.runtime.seeds(),
        };
        game.lan_host(settings, net, config).unwrap();
        let motion = horizon_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
        let mut guest = Guest::connectionless(12, 12, "SourceGuest".into(), 12, [0.0; 3], motion);
        guest.placed = true;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        game.as_guest(12, |acting| {
            assert_eq!(acting.server_resident_entity(7), EntityLookup::Taken);
            assert_eq!(acting.uuid_for(7), expected);
            assert!(matches!(
                acting.server_resident_entity(12),
                EntityLookup::Found {
                    value: Target::Player,
                    ..
                }
            ));
            acting.reset_entity_residency();
            assert_eq!(acting.server_resident_entity(7), EntityLookup::Taken);
        })
        .unwrap();
        assert!(matches!(
            game.server_resident_entity(7),
            EntityLookup::Found {
                value: Target::Player,
                ..
            }
        ));
        assert_eq!(game.uuid_for(7), expected);
        game.remove_local_player_body();
        assert_eq!(game.server_resident_entity(7), EntityLookup::Missing);
    });
}

#[test]
#[ignore = "daily"]
fn actual_dragon_admits_exactly_eight_checked_parts_with_independent_uuids() {
    const VALUE: mod_api::AttachmentType<i32> = mod_api::AttachmentType {
        namespace: "source_test",
        name: "part_lifetime",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(game.dim);
        let parent = game
            .spawn_dragon_spawned([0.0; 3], crate::game::creatures::SpawnSite::Plain)
            .unwrap();
        let uuids = std::array::from_fn::<_, 8, _>(|i| game.dragon.as_ref().unwrap().parts[i].3);
        let parts: Vec<_> = game
            .resident_entities
            .table
            .borrow_mut()
            .admitted_metadata()
            .filter(|(_, at, _)| at.owner.kind == Kind::DragonPart)
            .collect();
        assert_eq!(parts.len(), 8);
        for offset in 0..8u8 {
            let id = parent + 1 + u32::from(offset);
            let EntityLookup::Found {
                location,
                value: Target::DragonPart(found),
            } = game.server_resident_entity(id)
            else {
                panic!("real semantic part not admitted");
            };
            assert_eq!(found, id);
            assert_eq!(location.source, Source::DragonPart(offset));
            assert_eq!(game.uuid_for(id), uuids[usize::from(offset)]);
            assert!(game.level_entity(id as i32).is_none());
            assert_eq!(game.level_dragon_part(id), Some(Target::DragonPart(id)));
            assert_eq!(
                game.with_resident_slot(&game.dim.key(), NonZeroU32::new(id).unwrap(), |slot| slot
                    .is_bound()),
                SlotAccess::Done(false)
            );
            assert_eq!(
                VALUE.set(game, mod_api::Holder::Entity(id), i32::from(offset) + 7),
                Ok(())
            );
            assert_eq!(
                VALUE.get(game, mod_api::Holder::Entity(id)),
                Ok(i32::from(offset) + 7)
            );
        }
        assert_eq!(VALUE.set(game, mod_api::Holder::Entity(parent), 99), Ok(()));
        for offset in 0..8u8 {
            assert_eq!(
                VALUE.get(
                    game,
                    mod_api::Holder::Entity(parent + 1 + u32::from(offset))
                ),
                Ok(i32::from(offset) + 7)
            );
            use horizon_mod_runtime::attachments::HasAttachmentLifetime;
            assert!(
                game.dragon.as_mut().unwrap().parts[usize::from(offset)]
                    .attachment_lifetime()
                    .is_bound()
            );
        }
        assert_eq!(VALUE.get(game, mod_api::Holder::Entity(parent)), Ok(99));
        let index = game.attachment_indexes[&game.dim].clone();
        let key = ("source_test".into(), "part_lifetime".into());
        let retained = index.lock().unwrap().snapshot(&key);
        assert_eq!(retained.len(), 8);
        assert_eq!(
            VALUE.remove(game, mod_api::Holder::Entity(parent + 1)),
            Ok(Some(7))
        );
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 7);
        assert_eq!(VALUE.get(game, mod_api::Holder::Entity(parent + 2)), Ok(8));
        assert_eq!(VALUE.get(game, mod_api::Holder::Entity(parent)), Ok(99));
        VALUE
            .set(game, mod_api::Holder::Entity(parent + 1), 7)
            .unwrap();
        let home = game.dim;
        game.enter_level(crate::game::Dim::Nether).unwrap();
        game.prune_entity_attachments();
        game.prune_entity_memory();
        for id in parent + 1..=parent + 8 {
            assert_eq!(
                game.with_resident_slot(&home.key(), NonZeroU32::new(id).unwrap(), |slot| slot
                    .is_bound()),
                SlotAccess::Done(true)
            );
        }
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 8);
        game.enter_level(home).unwrap();
        for offset in 0..8u8 {
            assert_eq!(
                VALUE.get_existing(
                    game,
                    mod_api::Holder::Entity(parent + 1 + u32::from(offset))
                ),
                Ok(Some(i32::from(offset) + 7))
            );
        }
        assert_eq!(
            game.server_resident_entity(parent + 9),
            EntityLookup::Missing
        );
        game.dragon.discard();
        for id in parent..=parent + 8 {
            assert_eq!(game.server_resident_entity(id), EntityLookup::Missing);
        }
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(retained.len(), 8);
        assert_eq!(
            VALUE.set(game, mod_api::Holder::Entity(parent + 1), 42),
            Err(mod_api::AttachmentError::MissingHolder)
        );
    });
}

#[test]
#[ignore = "daily"]
fn dragon_part_registrations_retire_on_replacement_command_and_chunk_unload() {
    const VALUE: mod_api::AttachmentType<i32> = mod_api::AttachmentType {
        namespace: "source_test",
        name: "part_retirement",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    for operation in 0..3 {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(game.dim);
            game.spawn_remote_dragon(420, [1.0, 80.0, 1.0], 0.0);
            for id in 421..=428 {
                VALUE
                    .set(game, mod_api::Holder::Entity(id), id as i32)
                    .unwrap();
            }
            let index = game.attachment_indexes[&game.dim].clone();
            let key = ("source_test".into(), "part_retirement".into());
            let retained = index.lock().unwrap().snapshot(&key);
            assert_eq!(retained.len(), 8);
            // Keeping the maps and snapshot alive must not keep registrations alive.
            let maps = game.entity_attachments.clone();
            match operation {
                0 => game.spawn_remote_dragon(840, [1.0, 80.0, 1.0], 0.0),
                1 => {
                    game.kill_target(Target::Dragon(420));
                }
                _ => {
                    game.entity_chunks(&[(0, 0)].into_iter().collect(), true);
                }
            }
            assert!(index.lock().unwrap().snapshot(&key).is_empty());
            assert_eq!(retained.len(), 8);
            assert!(maps.contains_key(&421));
            game.prune_entity_attachments();
            game.prune_entity_memory();
            for id in 421..=428 {
                assert!(!game.entity_attachments.contains_key(&id));
            }
            for id in 421..=428 {
                assert_eq!(game.server_resident_entity(id), EntityLookup::Missing);
                assert_eq!(
                    game.with_resident_slot(&game.dim.key(), NonZeroU32::new(id).unwrap(), |_| ()),
                    SlotAccess::Missing
                );
            }
            if operation == 0 {
                assert_eq!(VALUE.get(game, mod_api::Holder::Entity(841)), Ok(0));
                VALUE.set(game, mod_api::Holder::Entity(841), 17).unwrap();
                assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
            } else {
                game.spawn_remote_dragon(420, [1.0, 80.0, 1.0], 0.0);
                assert_eq!(
                    VALUE.get_existing(game, mod_api::Holder::Entity(421)),
                    Ok(None)
                );
                VALUE.set(game, mod_api::Holder::Entity(421), 17).unwrap();
                assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
            }
        });
    }
}

#[test]
#[ignore = "daily"]
fn parked_mob_attachment_values_survive_other_level_cleanup() {
    const VALUE: mod_api::AttachmentType<i32> = mod_api::AttachmentType {
        namespace: "source_test",
        name: "parked_cleanup",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(game.dim);
        let home = game.dim;
        let id = game
            .spawn_mob_spawned(
                "cow",
                [1.0, 70.0, 3.0],
                crate::game::creatures::SpawnSite::Plain,
            )
            .unwrap();
        VALUE.set(game, mod_api::Holder::Entity(id), 73).unwrap();
        game.enter_level(crate::game::Dim::Nether).unwrap();
        game.prune_entity_attachments();
        game.prune_entity_memory();
        assert!(game.entity_attachments.contains_key(&id));
        game.enter_level(home).unwrap();
        assert_eq!(
            VALUE.get_existing(game, mod_api::Holder::Entity(id)),
            Ok(Some(73))
        );
    });
}
