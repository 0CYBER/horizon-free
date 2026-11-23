use super::super::api_values::TryIntoApi;

mod predicted_block_notice_tests {
    include!("predicted_block_notice_tests.rs");
}

#[test]
#[ignore = "daily"]
fn menu_title_snapshot_tracks_open_replacement_and_close() {
    super::super::native_entity_refs::tests::with_client_game(|game| {
        let (client, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(client, "menu-title-test").unwrap();
        let remote = game.remote.as_mut().unwrap();
        for value in [Some("first"), Some("replacement"), None] {
            remote.set_menu_title(value.map(horizon_client::text::Text::literal));
            assert_eq!(
                remote.menu_title_snapshot(),
                remote.menu_title.try_into_api().unwrap().as_ref()
            );
        }
    });
}
#[test]
#[ignore = "daily"]
fn menu_title_snapshot_follows_packet_and_close_paths() {
    use horizon_core::nbt::Tag;
    use horizon_protocol::packets::play::{self, Clientbound as C};
    use std::sync::Arc;

    super::super::native_entity_refs::tests::with_client_game(|game| {
        let (client, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(client, "menu-title-paths").unwrap();
        let menu_id = |game: &super::Game<'_>, name: &str| {
            game.remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:menu")
                .unwrap()
                .id(name)
                .unwrap()
        };
        let chest = menu_id(game, "minecraft:generic_9x3");
        let lectern = menu_id(game, "minecraft:lectern");
        let open = |game: &mut super::Game<'_>, menu, title: &str| {
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 7,
                menu,
                title: Tag::String(title.into()),
            }))
            .unwrap();
        };
        let snapshot = |game: &super::Game<'_>| {
            let shared = game
                .remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .unwrap();
            let observed = game.test_observed_title().unwrap();
            assert!(Arc::ptr_eq(&observed, &shared));
            shared
        };
        open(game, chest, "first");
        let first = snapshot(game);
        open(game, chest, "replacement");
        let replacement = snapshot(game);
        assert!(!Arc::ptr_eq(&first, &replacement));
        assert_ne!(first, replacement);
        game.handle_packet(C::ContainerClose(play::ContainerClose { container: 7 }))
            .unwrap();
        assert!(
            game.remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        assert!(game.test_observed_title().is_none());
        open(game, chest, "again");
        snapshot(game);
        game.remote_close_container();
        assert!(
            game.remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        open(game, chest, "before reject");
        game.remote_reject_container(7);
        assert!(
            game.remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        open(game, chest, "before dimension change");
        spawn_menu_test(game);
        assert!(
            game.remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        open(game, chest, "before lectern");
        snapshot(game);
        open(game, lectern, "lectern");
        assert!(
            game.remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        game.remote_lectern_page(1);
        assert!(
            game.remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        game.handle_packet(C::ContainerClose(play::ContainerClose { container: 7 }))
            .unwrap();
        assert!(
            game.remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        assert!(game.test_observed_title().is_none());
    });
}
#[test]
#[ignore = "daily"]
fn stopping_a_retained_client_dig_aborts_only_once() {
    use crate::game::{Hit, native_entity_refs::tests::with_client_game};
    use horizon_client::{chunk::Chunk, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;
    use horizon_protocol::connection::Pipe;
    use mod_api::{
        ClientHost, ClientMiningPredictEvent, ClientPacketSendEvent, Flow, Hooks, ModEntry,
    };
    use mod_platform::testing::with_mods;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct CountActions {
        packets: Arc<AtomicUsize>,
        vetoes: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for CountActions {
        fn client_mining_predict(
            &self,
            _: &dyn ClientHost,
            event: &mut ClientMiningPredictEvent,
        ) -> Flow {
            assert!(!event.prediction.starting);
            assert_eq!(event.prediction.pos, [0, 64, 0]);
            assert!(event.prediction.progress > 0.5);
            self.vetoes.fetch_add(1, Ordering::Relaxed);
            Flow::Cancel
        }

        fn client_packet_send(
            &self,
            _: &dyn ClientHost,
            event: &mut ClientPacketSendEvent<'_>,
        ) -> Flow {
            if event.packet.id.path() == "player_action" {
                self.packets.fetch_add(1, Ordering::Relaxed);
            }
            Flow::Continue
        }
    }
    let count = Arc::new(AtomicUsize::new(0));
    let vetoes = Arc::new(AtomicUsize::new(0));
    let mods = vec![ModEntry::new(
        "count_actions",
        Box::new(CountActions {
            packets: count.clone(),
            vetoes: vetoes.clone(),
        }),
    )]
    .into();
    with_mods(mods, || {
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "MiningTest").unwrap();
            let pos = [0, 64, 0];
            game.set_mode_id(0);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.5)).unwrap());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.tick_hit.set(Some(Some(Hit {
                pos,
                face: [0, 1, 0],
                point: [0.5, 65.0, 0.5],
                inside: false,
            })));
            let held = game.main_hand_key();
            let remote = game.remote.as_mut().unwrap();
            remote.destroying = Some((pos, 1, 0.5));
            remote.destroying_item = held;
            remote.is_destroying = true;
            game.remote_continue_attack(true);
            assert_eq!(vetoes.load(Ordering::Relaxed), 1);
            let remote = game.remote.as_ref().unwrap();
            assert!(remote.is_destroying);
            assert_eq!(remote.destroying, Some((pos, 1, 0.5)));
            assert_eq!(count.load(Ordering::Relaxed), 0);
            game.remote_continue_attack(false);
            game.remote_continue_attack(false);
            assert_eq!(
                game.remote.as_ref().unwrap().destroying,
                Some((pos, 1, 0.0))
            );
        })
    });
    assert_eq!(count.load(Ordering::Relaxed), 1);
}
/// The remote player animation uses the scoped item answer once per tick.
#[test]
#[ignore = "daily"]
fn remote_player_use_animation_dispatches_once() {
    use mod_api::{ClientHost, Hooks, ItemPart, ModEntry, Mods, Registrar, UseAnimationEvent};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[derive(Clone)]
    struct Part(Arc<AtomicUsize>);
    #[mod_api::horizon_part]
    impl ItemPart for Part {
        fn use_animation(&self, _: &dyn ClientHost, event: &mut UseAnimationEvent<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
            *event.animation = mod_api::UseAnimation::Drink;
        }
    }
    struct Mod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Mod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let item = r.item_key("minecraft:bow");
            r.item_part(item, Part(self.0.clone()));
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: Mods = vec![ModEntry::new(
        "remote_animation",
        Box::new(Mod(calls.clone())),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let (client, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(client, "animation-client").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.entity_id = 42;
            remote.level_players.insert(66, 789);
            let kind = remote.player_type.as_ref().unwrap().clone();
            let mut mob =
                horizon_client::mobs::Mob::new(66, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
            mob.extra.using_item = 1;
            mob.extra.set_slot_stack(
                horizon_items::equipment::MAINHAND,
                Some(horizon_client::items::ItemStack::new("minecraft:bow", 1)),
            );
            game.mobs.push(mob);
            game.tick_remote_entities(false);
            assert_eq!(
                game.mobs[0].extra.use_animation,
                mod_api::UseAnimation::Drink
            );
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        });
    });
}

/// After `ClientboundUpdateAttributes`, the local player's `LivingRef`
/// reads its attribute value, base, modifiers and attribute list from the
/// client's own copy of the synced instances. A mob the server tracks
/// keeps no attribute state of its own on the client, so its reads are
/// `None` there.
#[test]
#[ignore = "daily"]
fn update_attributes_answers_the_local_players_living_reads() {
    use super::super::native_entity_refs::tests::with_client_game;
    use horizon_client::{mobs::Mob, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;
    use horizon_protocol::{
        buf::Reader,
        connection::Pipe,
        packets::play::{self, Clientbound as C},
    };

    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "attributes-client").unwrap();
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.player.as_mut().unwrap().entity_id = 42;
        game.mobs.push(Mob::new(60, zombie, Vec3::ZERO, 0.0));
        let remote = game.remote.as_mut().unwrap();
        remote.entity_id = 42;
        remote.logged_in = true;
        remote.entities.insert(
            60,
            super::Tracked {
                kind: "minecraft:zombie".into(),
                position: [0.0; 3],
                codec_base: [0.0; 3],
                silent: false,
                vehicle: None,
            },
        );

        let health = "minecraft:max_health";
        let living = game.living(game.entity(42).unwrap()).unwrap();
        // Before the server says anything, the client's own map holds the
        // type's defaults, as a vanilla client's does before its sync.
        assert_eq!(living.attribute_value(game, health), Some(20.0));
        assert_eq!(living.attribute_modifiers(game, health), Some(Vec::new()));

        let registry_id = game.motion.attributes.id(health).unwrap().registry_id();
        let packet = C::UpdateAttributes(play::UpdateAttributes {
            id: 42,
            values: vec![play::AttributeSnapshot {
                attribute: registry_id,
                base: 30.0,
                modifiers: vec![play::AttributeModifier {
                    id: "test:bonus".into(),
                    amount: 5.0,
                    operation: 0,
                }],
            }],
        });
        let body = packet.encode();
        let mut reader = Reader::new(&body);
        let id = reader.varint().unwrap();
        game.handle_packet(C::decode(id, reader.rest()).unwrap())
            .unwrap();

        assert_eq!(living.attribute_value(game, health), Some(35.0));
        assert_eq!(living.attribute_base(game, health), Some(30.0));
        assert_eq!(
            living.attribute_modifiers(game, health),
            Some(vec![mod_api::AttributeModifier::new(
                mod_api::Identifier::parse("test:bonus").unwrap(),
                5.0,
                mod_api::AttributeOperation::AddValue,
            )])
        );
        let listed = living.attributes(game).unwrap();
        assert!(listed.contains(&mod_api::Identifier::parse(health).unwrap()));

        // The mob's attributes are the server's business: no client state,
        // no read.
        let mob = mod_api::LivingRef::new(game.entity(60).unwrap());
        assert_eq!(mob.attribute_value(game, health), None);
        assert_eq!(mob.attribute_base(game, health), None);
        assert_eq!(mob.attribute_modifiers(game, health), None);
        assert_eq!(mob.attributes(game), None);
    });
}

/// Sleeping metadata and the local tick use the loaded bed's shape, not
/// a red-bed constant. Missing client data must not replace a server pose
/// or decide to wake the player.
#[test]
#[ignore = "daily"]
fn client_sleep_metadata_and_tick_preserve_shape_and_authoritative_positions() {
    use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
    use horizon_client::{chunk::Chunk, mobs::Mob, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;
    use horizon_protocol::{
        buf::Reader,
        connection::Pipe,
        entity_data::{Value, serializer},
        packets::play::{self, Clientbound as C},
    };

    let deliver = |game: &mut super::super::Game<'_>, packet: C| {
        let body = packet.encode();
        let mut reader = Reader::new(&body);
        let id = reader.varint().unwrap();
        game.handle_packet(C::decode(id, reader.rest()).unwrap())
            .unwrap();
    };
    let metadata = |id, bed: Option<[i32; 3]>| {
        C::SetEntityData(play::SetEntityData {
            id,
            values: vec![
                (
                    6,
                    serializer::POSE,
                    Value::Id(if bed.is_some() { 2 } else { 0 }),
                ),
                (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(bed)),
            ],
        })
    };
    let teleport = |position: Vec3| {
        C::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [position.x, position.y, position.z],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 30.0,
            relatives: 0,
        })
    };

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        let (client, server) = Pipe::pair();
        game.connect_local(client, "sleep-client").unwrap();
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.loaded_sent = true;
        remote.entity_id = 42;
        remote.level_players.insert(66, 789);
        let kind = remote.player_type.as_ref().unwrap().clone();
        let authoritative = Vec3::new(8.5, 70.0, 8.5);
        game.player = Some(PlayerMotion::new(game.motion, authoritative).unwrap());
        game.mobs.push(Mob::new(66, kind, authoritative, 0.0));
        game.streaming
            .insert_remote(&mut game.world, Chunk::empty(0, 0));
        let registry = game.pack.assets.registry;
        let air = registry.default_state("minecraft:air").unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();

        // Straw's foot is 4/16 high; red's shape is 9/16. These expected
        // positions are independent of sleeping_position and its helpers.
        for (name, complete, head, expected) in [
            (
                "minecraft:straw_bed",
                true,
                [5, 64, 5],
                Some(Vec3::new(5.5, 64.375, 5.5)),
            ),
            (
                "minecraft:red_bed",
                true,
                [5, 64, 5],
                Some(Vec3::new(5.5, 64.6875, 5.5)),
            ),
            ("minecraft:straw_bed", false, [5, 64, 5], None),
            ("minecraft:stone", false, [5, 64, 5], None),
            ("minecraft:straw_bed", false, [37, 64, 5], None),
        ] {
            let case = (name, complete, head);
            game.world.set(5, 64, 5, air).unwrap();
            game.world.set(5, 64, 6, air).unwrap();
            if head[0] == 5 {
                let state = if name == "minecraft:stone" {
                    stone
                } else {
                    registry
                        .resolve(&horizon_world::chunk::BlockState {
                            name: name.into(),
                            properties: [
                                ("facing", "north"),
                                ("occupied", "false"),
                                ("part", "head"),
                            ]
                            .into_iter()
                            .map(|(k, v)| (k.into(), v.into()))
                            .collect(),
                        })
                        .unwrap()
                };
                game.world.set(5, 64, 5, state).unwrap();
                if complete {
                    let mut foot = registry.state(state).unwrap().clone();
                    foot.properties.insert("part".into(), "foot".into());
                    game.world
                        .set(5, 64, 6, registry.id(&foot).unwrap())
                        .unwrap();
                }
            } else {
                assert!(!game.world.chunks.contains_key(&(2, 0)));
            }
            deliver(game, teleport(authoritative));
            let other = game.mob_index.get_mut(&mut game.mobs, 66).unwrap();
            other.position = authoritative;
            other.previous = authoritative;
            other.head_pitch = 30.0;
            other.previous_head_pitch = 30.0;
            game.sleep_timer = 17;
            deliver(game, metadata(66, Some(head)));
            deliver(game, metadata(42, Some(head)));
            let at = expected.unwrap_or(authoritative);
            assert_eq!(
                game.player.as_ref().unwrap().position,
                at,
                "own metadata: {case:?}"
            );
            assert_eq!(game.previous_position, at, "own old position: {case:?}");
            let other = game.mob_index.get(&game.mobs, 66).unwrap();
            assert_eq!(other.position, at, "other metadata: {case:?}");
            assert_eq!(other.previous, at, "other old position: {case:?}");
            assert_eq!((other.head_pitch, other.previous_head_pitch), (0.0, 0.0));
            assert_eq!(game.sleeping, Some(head));
            assert_eq!(game.sleep_timer, 0);

            // A correct authoritative teleport cannot be overwritten by
            // the next sleeping tick, even while this client's bed is absent.
            deliver(game, teleport(at));
            game.player.as_mut().unwrap().velocity = Vec3::new(0.3, -0.4, 0.2);
            for timer in 1..=2 {
                game.tick_remote_player(true, false).unwrap();
                assert_eq!(game.sleep_timer, timer, "local tick must execute: {case:?}");
                assert_eq!(game.sleeping, Some(head));
                let player = game.player.as_ref().unwrap();
                assert_eq!(player.position, at, "own tick: {case:?}");
                assert_eq!(player.velocity, Vec3::ZERO);
                assert_eq!(player.pitch, 0.0);
                assert_eq!(
                    game.mob_index.get(&game.mobs, 66).unwrap().position,
                    at,
                    "other tick: {case:?}"
                );
            }

            deliver(game, metadata(66, None));
            deliver(game, metadata(42, None));
            assert_eq!(game.sleeping, None);
            assert_eq!(
                game.player.as_ref().unwrap().position,
                at,
                "clearing metadata does not relocate"
            );
            let other = game.mob_index.get(&game.mobs, 66).unwrap();
            assert_eq!(other.position, at);
            assert_eq!(other.extra.ints[1], 0);
            assert_eq!(other.extra.synced_pose, 0);
            deliver(game, C::Animate(play::Animate { id: 42, action: 0 }));
            assert_eq!(game.sleep_timer, 100);
            let wake = Vec3::new(9.5, 70.1, 9.5);
            deliver(game, teleport(wake));
            assert_eq!(game.player.as_ref().unwrap().position, wake);
            game.tick_remote_player(true, false).unwrap();
            assert_eq!(game.sleeping, None);
            assert_eq!(game.sleep_timer, 101);
            let player = game.player.as_ref().unwrap();
            assert_eq!(
                (player.position.x, player.position.z),
                (wake.x, wake.z),
                "wake tick must not snap to bed: {case:?}"
            );
        }
        server.shutdown();
    });
}

/// `ClientChatReceivedEvent`: a mod's replacement is the line the chat
/// shows, a cancelled system line never shows, and a cancelled message
/// held by Chat Delay lets the next one through in the same tick.
#[test]
#[ignore = "daily"]
fn mods_replace_and_cancel_received_chat() {
    use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
    use horizon_core::nbt::Tag;
    use horizon_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C},
    };

    struct Censor;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Censor {
        fn client_chat_received(
            &self,
            _: &mut dyn mod_api::ClientHost,
            event: &mut mod_api::ClientChatReceivedEvent<'_>,
        ) -> mod_api::Flow {
            use mod_api::text::TextContent;
            let text = match &event.message.content {
                TextContent::Literal(text) => text,
                // Player chat arrives decorated with the sender and message.
                TextContent::Translatable { key, arguments, .. } if key == "chat.type.text" => {
                    let TextContent::Literal(text) = &arguments[1].content else {
                        panic!("expected literal player message fixture");
                    };
                    text
                }
                _ => return mod_api::Flow::Continue,
            };
            if text.contains("drop") {
                return mod_api::Flow::Cancel;
            }
            // A system line carries no player message, so only it gets the tag.
            if event.system && event.player_message.is_none() {
                *event.message = mod_api::Text::literal(format!("[server] {text}"));
            }
            mod_api::Flow::Continue
        }
    }
    mod_platform::__private::install(std::sync::Arc::from([mod_api::ModEntry::new(
        "test-chat-received",
        Box::new(Censor),
    )]));

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "ChatTest").unwrap();
        let lines = |game: &super::super::Game| -> Vec<String> {
            let chat = &game.remote.as_ref().unwrap().chat;
            chat.iter().map(|(line, _)| line.clone()).collect()
        };
        let system = |content: &str| {
            C::SystemChat(play::SystemChat {
                content: Tag::String(content.into()),
                overlay: false,
            })
        };
        game.handle_packet(system("hello")).unwrap();
        game.handle_packet(system("drop me")).unwrap();
        assert_eq!(lines(game), ["[server] hello"]);

        game.settings.chat_delay = 1.0;
        let disguised = |message: &str| {
            C::DisguisedChat(play::DisguisedChat {
                message: Tag::String(message.into()),
                chat_type: play::ChatTypeBound {
                    chat_type: 0,
                    name: Tag::String("Steve".into()),
                    target: None,
                },
            })
        };
        game.handle_packet(disguised("drop this")).unwrap();
        game.handle_packet(disguised("hi")).unwrap();
        assert_eq!(lines(game).len(), 1, "Chat Delay holds both");
        game.tick_chat_queue();
        let shown = lines(game);
        assert_eq!(shown.len(), 2, "{shown:?}");
        assert!(
            shown[1].contains("hi") && !shown[1].contains("drop"),
            "{shown:?}"
        );
    });
}

#[test]
#[ignore = "daily"]
fn mod_chat_needs_a_joined_player_and_what_the_chat_option_allows() {
    use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
    use horizon_protocol::connection::Pipe;

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_err());
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "ChatTest").unwrap();
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_err());
        game.remote.as_mut().unwrap().joined = true;
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "  ").is_err());
        // Full, then Commands Only, then Hidden.
        game.settings.chat_visibility = 0;
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "hello").is_ok());
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_ok());
        game.settings.chat_visibility = 1;
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "hello").is_err());
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_ok());
        game.settings.chat_visibility = 2;
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_err());
    });
}

/// `ChatComponent.addClientSystemMessage`: the line joins this client's
/// chat history and no packet leaves for the server.
#[test]
#[ignore = "daily"]
fn a_mod_chat_line_shows_locally_and_sends_nothing() {
    use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
    use horizon_protocol::connection::{Connection, Pipe};

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        let (client, server) = Pipe::pair();
        game.connect_local(client, "ChatLineTest").unwrap();
        // Whatever connecting wrote is not this line's; drain it until the
        // client waits for the login answer, so the check below is about
        // `chat_line` alone.
        let (mut reader, _writer) = Connection::local(server).split();
        reader
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        while reader.recv().is_ok() {}
        mod_api::prelude::NetworkClientHost::chat_line(game, mod_api::Text::literal("local only"))
            .unwrap();
        let history: Vec<_> = game
            .remote
            .as_ref()
            .unwrap()
            .chat
            .iter()
            .map(|(line, _)| line.clone())
            .collect();
        assert_eq!(history, ["local only"]);
        assert!(
            reader.recv().unwrap_err().is_timed_out(),
            "a local chat line sends nothing to the server"
        );
    });
}

#[test]
#[ignore = "daily"]
fn rejected_menu_packets_retire_the_previous_client_menu() {
    use super::super::{
        Screen, execution_role::ExecutionRole, native_entity_refs::tests::with_game,
    };
    use horizon_client::items::ItemStack;
    use horizon_core::nbt::Tag;
    use horizon_protocol::{
        connection::Pipe,
        packets::play::{Clientbound as C, MountScreenOpen, OpenScreen},
    };

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "MenuTest").unwrap();
        for packet in [
            C::OpenScreen(OpenScreen {
                container: 7,
                menu: -1,
                title: Tag::String("unsupported".into()),
            }),
            C::MountScreenOpen(MountScreenOpen {
                container: 7,
                columns: 0,
                entity: i32::MAX,
            }),
        ] {
            let remote = game.remote.as_mut().unwrap();
            remote.menu.set(Some((7, "minecraft:generic_9x3".into())));
            let generation = remote.menu.generation;
            game.set_screen(Screen::Inventory);
            game.carried = Some(ItemStack::new("minecraft:stone", 1));
            game.handle_packet(packet).unwrap();
            let remote = game.remote.as_ref().unwrap();
            assert!(remote.menu.as_ref().is_none());
            assert_ne!(remote.menu.generation, generation);
            assert!(game.carried.is_none());
            assert!(game.screen == Screen::Playing);
        }
    });
}

/// `LivingEntity.handleEntityEvent(55)` is `swapHandItems`: the whole
/// stacks trade hands, not just their ids.
#[test]
#[ignore = "daily"]
fn the_hand_swap_event_keeps_whole_stacks() {
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::{items::ItemStack, mobs::Mob};
    use horizon_core::geometry::Vec3;
    use horizon_items::equipment::{MAINHAND, OFFHAND};
    use horizon_protocol::packets::play::{Clientbound as C, EntityEvent};

    with_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.damage = 40;
        sword.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("Biter".into()));
        let mut shield = ItemStack::new("minecraft:shield", 1);
        shield.damage = 12;
        let mut mob = Mob::new(31, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        mob.extra.set_slot_stack(MAINHAND, Some(sword.clone()));
        mob.extra.set_slot_stack(OFFHAND, Some(shield.clone()));
        game.mobs.push(mob);
        let swap = |game: &mut super::super::Game| {
            game.handle_packet(C::EntityEvent(EntityEvent { id: 31, event: 55 }))
                .unwrap();
        };
        swap(game);
        let m = game.mobs.iter().find(|m| m.id == 31).unwrap();
        assert_eq!(m.extra.slot_stack(MAINHAND), Some(&shield));
        assert_eq!(m.extra.slot_stack(OFFHAND), Some(&sword));
        swap(game);
        let m = game.mobs.iter().find(|m| m.id == 31).unwrap();
        assert_eq!(m.extra.slot_stack(MAINHAND), Some(&sword));
        assert_eq!(m.extra.slot_stack(OFFHAND), Some(&shield));
    });
}

#[test]
fn client_menu_generation_changes_on_reused_window_ids() {
    let mut menu = super::ClientMenu::default();
    let inventory = menu.generation;
    menu.set(Some((7, "minecraft:generic_9x3".to_owned())));
    let first = menu.generation;
    assert_ne!(inventory, first);
    menu.set(None);
    let closed = menu.generation;
    assert_ne!(first, closed);
    assert!(menu.as_ref().is_none());
    menu.set(Some((7, "minecraft:generic_9x3".to_owned())));
    assert_ne!(closed, menu.generation);
    assert_ne!(first, menu.generation);
    assert_eq!(menu.as_ref().map(|entry| entry.0), Some(7));
    let before_replacement = menu.generation;
    menu.set(Some((7, "minecraft:generic_9x3".to_owned())));
    assert_ne!(before_replacement, menu.generation);
}

#[test]
fn client_menu_slot_mapping_covers_inventory_and_containers() {
    for (name, count, first, last) in [
        ("inventory", 46, 200, 504),
        ("minecraft:crafting", 46, 200, 8),
        ("minecraft:generic_9x3", 63, 300, 8),
        ("minecraft:hopper", 41, 300, 8),
        ("minecraft:furnace", 39, 300, 8),
        ("minecraft:brewing_stand", 41, 300, 8),
    ] {
        assert_eq!(super::Game::inventory_screen_slot(name, 0), Some(first));
        assert_eq!(
            super::Game::inventory_screen_slot(name, count - 1),
            Some(last)
        );
        assert_eq!(super::Game::inventory_screen_slot(name, count), None);
    }
    assert_eq!(
        super::Game::inventory_screen_slot("minecraft:unknown", 0),
        None
    );
}

#[test]
#[ignore = "daily"]
fn transactional_bucket_hands_reach_the_owning_client() {
    use super::super::{
        native_entity_refs::tests::with_client_game, native_guest_turn_tests::with_guests,
    };
    use horizon_client::{
        items::ItemStack,
        lan::{Out, PlayerSender},
    };
    use horizon_protocol::{buf::Reader, packets::play::Clientbound as C};
    use mod_api::{HostTransaction, ITEM_FLUIDS, ItemPlace, ServerHost, move_resources};

    with_guests(|server| {
        let (tx, rx) = std::sync::mpsc::channel();
        server.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
            guest.sender = PlayerSender::new(tx, std::sync::Arc::default());
            guest.selected = 2;
            guest.inventory[2] = Some(ItemStack::new("minecraft:bucket", 1));
            guest.offhand = Some(ItemStack::new("minecraft:water_bucket", 1));
        });
        let main = ItemPlace::PlayerHand {
            player: 12,
            hand: horizon_client::first_person::Hand::Main,
        };
        let off = ItemPlace::PlayerHand {
            player: 12,
            hand: horizon_client::first_person::Hand::Off,
        };
        with_client_game(|client| {
            let (_reader, _writer) = connect_menu_test(client);
            spawn_menu_test(client);
            client.selected = 2;
            // Consume actual server packets through the client's ordinary handler.
            let deliver = |client: &mut super::Game<'_>| {
                let mut slots = Vec::new();
                for out in rx.try_iter() {
                    let bodies = match out {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        Out::Disconnect(_) => panic!("owner disconnected"),
                    };
                    for body in bodies {
                        let mut reader = Reader::new(&body);
                        let id = reader.varint().unwrap();
                        let packet = C::decode(id, reader.rest()).unwrap();
                        // Other tick packets concern entity/chunk state, not this contract.
                        if let C::ContainerSetSlot(ref slot) = packet {
                            slots.push(slot.slot);
                            client.handle_packet(packet).unwrap();
                        }
                    }
                }
                slots
            };
            server.tick_lan(true);
            deliver(client);
            assert_eq!(
                client.inventory[2],
                Some(ItemStack::new("minecraft:bucket", 1))
            );
            assert_eq!(
                client.offhand,
                Some(ItemStack::new("minecraft:water_bucket", 1))
            );

            for commit in [false, true] {
                let host: &mut dyn ServerHost = server;
                let result = host.transaction(|host, tx| {
                    let source = off.capability(host, &ITEM_FLUIDS).unwrap();
                    let target = main.capability(host, &ITEM_FLUIDS).unwrap();
                    assert_eq!(
                        move_resources(host, &source, &target, |_| true, 1000, tx).unwrap(),
                        1000
                    );
                    if commit { Ok(()) } else { Err(()) }
                });
                assert_eq!(result.is_ok(), commit);
                server.tick_lan(true);
                let slots = deliver(client);
                if commit {
                    assert!(slots.contains(&38), "selected hotbar slot published");
                    assert!(slots.contains(&45), "offhand published");
                } else {
                    assert!(slots.is_empty(), "abort publishes no tentative inventory");
                }
                let (held, other) = if commit {
                    ("minecraft:water_bucket", "minecraft:bucket")
                } else {
                    ("minecraft:bucket", "minecraft:water_bucket")
                };
                assert_eq!(
                    client.inventory[client.selected],
                    Some(ItemStack::new(held, 1))
                );
                assert_eq!(client.offhand, Some(ItemStack::new(other, 1)));
                let guest = &server.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(guest.inventory[2], client.inventory[2]);
                assert_eq!(guest.offhand, client.offhand);
            }
            server.tick_lan(true);
            assert!(
                deliver(client).is_empty(),
                "settled slots are not sent twice"
            );
        });
    });
}

/// A client connecting through a local pipe and through configuration, as
/// the client menu tests drive it: the server end's reader (what the
/// client sends) and writer.
pub(crate) fn connect_menu_test(
    game: &mut super::Game<'_>,
) -> (
    horizon_protocol::connection::ConnReader,
    horizon_protocol::connection::ConnWriter,
) {
    use super::super::execution_role::ExecutionRole;
    use horizon_protocol::{
        connection::{Connection, Pipe},
        packets::{Empty, configuration as cfg, login},
    };
    game.options.execution_role = ExecutionRole::Client;
    let (client, server) = Pipe::pair();
    game.connect_local(client, "MenuReadTest").unwrap();
    let (mut reader, mut writer) = Connection::local(server).split();
    reader
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    reader.recv().unwrap();
    reader.recv().unwrap();
    writer
        .send(
            &login::Clientbound::LoginFinished(login::LoginFinished {
                profile: login::GameProfile {
                    uuid: 42,
                    name: "MenuReadTest".into(),
                    properties: vec![],
                },
                session: 1,
            })
            .encode(),
        )
        .unwrap();
    let (id, body) = reader.recv().unwrap();
    assert!(matches!(
        login::Serverbound::decode(id, &body).unwrap(),
        login::Serverbound::LoginAcknowledged(_)
    ));
    reader.recv().unwrap();
    reader.recv().unwrap();
    writer
        .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
        .unwrap();
    let (id, body) = reader.recv().unwrap();
    assert!(matches!(
        cfg::Serverbound::decode(id, &body).unwrap(),
        cfg::Serverbound::FinishConfiguration(_)
    ));
    (reader, writer)
}

#[test]
#[ignore = "daily"]
fn dimension_transition_screen_is_selected_at_entry_drawn_and_dropped() {
    use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
    use horizon_client::{hud::HudBatch, level_loading::Reason};
    use mod_api::__private::game::RegistrationContextNew;
    use mod_api::{DimensionTransition, DimensionTransitionScreen};

    struct Arrival;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Arrival {
        fn register_dimension_transition_screen(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterDimensionTransitionScreenEvent<'_>,
        ) {
            event.screens.push(DimensionTransitionScreen {
                screen: "transition_test:arrival".into(),
                transition: DimensionTransition::Conditional {
                    to: "transition_test:b".into(),
                    from: "transition_test:a".into(),
                },
            });
        }
        fn dimension_transition_screen_draw(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut mod_api::DimensionTransitionScreenDrawEvent<'_>,
        ) {
            assert_eq!(event.screen, "transition_test:arrival");
            assert_eq!(event.reason, mod_api::DimensionTransitionReason::Other);
            assert_eq!(event.progress, Some(0.25));
            event.draw.fill([0.0, 0.0, 80.0, 10.0], 0xFF102030);
        }
    }
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("transition_test", Box::new(Arrival))].into(),
        || {
            let mut registrations = Vec::new();
            super::Game::native_register_dimension_transition_screen(
                &mod_api::RegistrationContext::new(),
                &mut registrations,
            );
            horizon_client_render::dimension_transition::register(registrations);
            with_game(|game| {
                game.options.execution_role = ExecutionRole::Client;
                let (client, _server) = horizon_protocol::connection::Pipe::pair();
                game.connect_local(client, "TransitionTest").unwrap();
                let mut spawn = horizon_protocol::packets::play::SpawnInfo {
                    dimension_type: 0,
                    dimension: "transition_test:a".into(),
                    seed: 0,
                    game_type: 0,
                    previous_game_type: None,
                    debug: false,
                    flat: true,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                };
                game.enter_remote_dimension(&spawn, false, Reason::Other);
                assert_eq!(
                    game.remote.as_ref().unwrap().level_load.transition_screen(),
                    None
                );
                spawn.dimension = "transition_test:b".into();
                game.enter_remote_dimension(&spawn, true, Reason::Other);
                let tracker = game.remote.as_ref().unwrap().level_load;
                assert!(tracker.transition_screen().is_some());
                let mut batch = HudBatch::scratch([320, 240]);
                assert!(game.draw_dimension_transition_screen(&mut batch, &tracker, Some(0.25)));
                assert!(!batch.vertices.is_empty());
                game.remote.as_mut().unwrap().level_load.finish();
                let tracker = game.remote.as_ref().unwrap().level_load;
                assert!(!game.draw_dimension_transition_screen(&mut batch, &tracker, None));
                spawn.dimension = "transition_test:c".into();
                game.enter_remote_dimension(&spawn, true, Reason::Other);
                let tracker = game.remote.as_ref().unwrap().level_load;
                let mut vanilla = HudBatch::scratch([320, 240]);
                assert!(!game.draw_dimension_transition_screen(&mut vanilla, &tracker, None));
                game.draw_level_loading_background(&mut vanilla, tracker.reason);
                assert!(!vanilla.vertices.is_empty());
            });
        },
    );
}

/// Spawns the connected client in the overworld as entity 7, profile 42.
fn spawn_menu_test(game: &mut super::Game<'_>) -> horizon_protocol::packets::play::SpawnInfo {
    use horizon_client::level_loading::Reason;
    use horizon_protocol::packets::play::SpawnInfo;
    let spawn = SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 0,
        game_type: 0,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    };
    game.enter_remote_dimension(&spawn, false, Reason::Other);
    let remote = game.remote.as_mut().unwrap();
    remote.logged_in = true;
    remote.entity_id = 7;
    remote.profile = 42;
    spawn
}

/// A right-click with an empty hand on the inventory's crafting result goes to the server
/// as a click on menu slot 0, whether the result is empty or a stack that could split.
#[test]
#[ignore = "daily"]
fn an_empty_hand_right_click_on_the_crafting_result_is_sent() {
    use super::super::{Screen, native_entity_refs::tests::with_game};
    use horizon_client::{items::ItemStack, menu::Input};
    use horizon_protocol::packets::play;

    with_game(|game| {
        let (mut reader, _writer) = connect_menu_test(game);
        spawn_menu_test(game);
        game.set_screen(Screen::Inventory);
        for result in [None, Some(ItemStack::new("minecraft:oak_planks", 4))] {
            let row = format!("{result:?}");
            game.carried = None;
            game.remote.as_mut().unwrap().craft_result = result;
            assert!(
                game.remote_menu_ops(&[(Some(200), 1, Input::Pickup)]),
                "{row}"
            );
            let sent = (0..32)
                .find_map(|_| {
                    let (id, body) = reader.recv().unwrap_or_else(|e| panic!("{row}: {e:?}"));
                    match play::Serverbound::decode(id, &body)
                        .unwrap_or_else(|e| panic!("{row}: {e:?}"))
                    {
                        play::Serverbound::ContainerClick(sent) => Some(sent),
                        _ => None,
                    }
                })
                .unwrap_or_else(|| panic!("{row}: no click sent"));
            assert_eq!(
                (sent.container, sent.slot, sent.button, sent.input),
                (0, 0, 1, 0),
                "{row}"
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn client_menu_reads_follow_packets_and_lifetimes() {
    use super::super::{
        Screen, execution_role::ExecutionRole, native_entity_refs::tests::with_game,
    };
    use horizon_client::level_loading::Reason;
    use horizon_core::nbt::Tag;
    use horizon_protocol::{
        connection::Pipe,
        item::Stack,
        packets::play::{self, Clientbound as C},
    };

    with_game(|game| {
        let (mut reader, _writer) = connect_menu_test(game);
        assert!(
            game.local_player()
                .and_then(|player| game.player_container_menu(player))
                .is_none()
        );
        let spawn = spawn_menu_test(game);
        let menu = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        assert_eq!(game.menu_slot_count(menu), Some(46));
        assert_eq!(game.menu_slot(menu, 46), None);
        assert_eq!(game.menu_slot(menu, 36), Some(None));
        let stone = Some(Stack::new(game.item_id("minecraft:stone"), 3));
        game.handle_packet(C::SetPlayerInventory(play::SetPlayerInventory {
            slot: 0,
            item: stone.clone(),
        }))
        .unwrap();
        assert_eq!(game.menu_slot(menu, 36).unwrap().unwrap().count, 3);
        use mod_api::{DragKind, MenuClick, MouseButton, SwapWith};
        let pickup = |slot| MenuClick::Pickup {
            slot: Some(slot),
            button: MouseButton::Left,
        };
        for invalid in [
            pickup(46),
            pickup(usize::MAX),
            MenuClick::Swap {
                slot: 36,
                with: SwapWith::Hotbar(9),
            },
            MenuClick::Clone { slot: 36 },
            MenuClick::Drag {
                kind: DragKind::Clone,
                slots: vec![36],
            },
            MenuClick::Drag {
                kind: DragKind::Split,
                slots: vec![36, usize::MAX],
            },
        ] {
            assert!(!menu.click(game, &[invalid]));
            assert_eq!(menu.slot(game, 36).unwrap().unwrap().count(), 3);
            assert_eq!(menu.carried(game), Some(None));
        }
        game.options.execution_role = ExecutionRole::IntegratedServer;
        assert!(!menu.click(game, &[pickup(36)]));
        game.options.execution_role = ExecutionRole::Client;
        let screen = game.screen;
        game.screen = Screen::Creative;
        assert!(!menu.click(game, &[pickup(36)]));
        game.screen = screen;
        game.remote.as_mut().unwrap().state_id = 17;
        assert!(menu.click(game, &[pickup(36)]));
        let (id, body) = reader.recv().unwrap();
        let play::Serverbound::ContainerClick(sent) = play::Serverbound::decode(id, &body).unwrap()
        else {
            panic!("expected the first valid click, not a refused request");
        };
        assert_eq!(
            (
                sent.container,
                sent.state,
                sent.slot,
                sent.button,
                sent.input
            ),
            (0, 17, 36, 0, 0)
        );
        assert_eq!(sent.changed.len(), 1);
        assert_eq!(sent.changed[0].0, 36);
        assert!(sent.changed[0].1.is_none());
        assert_eq!(sent.carried.unwrap().count, 3);
        assert_eq!(menu.slot(game, 36), Some(None));
        assert_eq!(menu.carried(game).unwrap().unwrap().count(), 3);
        assert!(!menu.click(game, &[pickup(36), pickup(usize::MAX),]));
        assert_eq!(menu.slot(game, 36), Some(None));
        assert_eq!(menu.carried(game).unwrap().unwrap().count(), 3);
        assert!(menu.click(
            game,
            &[MenuClick::Drag {
                kind: DragKind::Split,
                slots: vec![37]
            },]
        ));
        assert_eq!(menu.slot(game, 37).unwrap().unwrap().count(), 3);
        assert_eq!(menu.carried(game), Some(None));
        assert!(menu.click(game, &[pickup(37)]));
        assert!(menu.click(game, &[pickup(36)]));
        assert_eq!(menu.carried(game), Some(None));
        assert_eq!(menu.slot(game, 36).unwrap().unwrap().count(), 3);
        game.handle_packet(C::SetPlayerInventory(play::SetPlayerInventory {
            slot: 0,
            item: Some(Stack::new(game.item_id("minecraft:stone"), 2)),
        }))
        .unwrap();
        assert_eq!(menu.slot(game, 36).unwrap().unwrap().count(), 2);
        assert!(menu.click(game, &[MenuClick::QuickMove { slot: 36 }]));
        assert_eq!(menu.slot(game, 36), Some(None));
        assert_eq!(menu.slot(game, 9).unwrap().unwrap().count(), 2);
        assert!(menu.click(
            game,
            &[MenuClick::Swap {
                slot: 9,
                with: SwapWith::Hotbar(1)
            }]
        ));
        assert_eq!(menu.slot(game, 9), Some(None));
        assert_eq!(menu.slot(game, 37).unwrap().unwrap().count(), 2);
        assert!(menu.click(
            game,
            &[MenuClick::Throw {
                slot: 37,
                whole_stack: true
            }]
        ));
        assert_eq!(menu.slot(game, 37), Some(None));
        game.handle_packet(C::SetCursorItem(play::SetCursorItem {
            item: stone.clone(),
        }))
        .unwrap();
        assert_eq!(game.menu_carried(menu).unwrap().unwrap().count, 3);
        let crafting_id = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:crafting")
            .unwrap();
        let open = || {
            C::OpenScreen(play::OpenScreen {
                container: 7,
                menu: crafting_id,
                title: Tag::String("Crafting".into()),
            })
        };
        game.handle_packet(open()).unwrap();
        assert_eq!(game.menu_slot_count(menu), None);
        assert!(!menu.click(game, &[pickup(36)]));
        let crafting = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        assert_eq!(game.menu_slot_count(crafting), Some(46));
        game.handle_packet(C::ContainerSetSlot(play::ContainerSetSlot {
            container: 7,
            state: 0,
            slot: 1,
            item: stone.clone(),
        }))
        .unwrap();
        assert_eq!(game.menu_slot(crafting, 1).unwrap().unwrap().count, 3);
        game.handle_packet(C::ContainerSetSlot(play::ContainerSetSlot {
            container: 7,
            state: 0,
            slot: 0,
            item: stone,
        }))
        .unwrap();
        assert_eq!(game.menu_slot(crafting, 0).unwrap().unwrap().count, 3);
        game.handle_packet(open()).unwrap();
        assert_eq!(game.menu_slot_count(crafting), None);
        assert!(!crafting.click(game, &[pickup(1)]));
        let crafting = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn.clone(),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.menu_slot_count(crafting), None);
        assert!(!crafting.click(game, &[pickup(1)]));
        let inventory = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        assert_ne!(inventory, menu);
        let chest_id = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:generic_9x3")
            .unwrap();
        game.handle_packet(C::OpenScreen(play::OpenScreen {
            container: 7,
            menu: chest_id,
            title: Tag::String("Chest".into()),
        }))
        .unwrap();
        let chest = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        assert_eq!(game.menu_slot_count(chest), Some(63));
        let stone = Some(Stack::new(game.item_id("minecraft:stone"), 4));
        game.handle_packet(C::ContainerSetSlot(play::ContainerSetSlot {
            container: 7,
            state: 0,
            slot: 2,
            item: stone,
        }))
        .unwrap();
        assert_eq!(game.menu_slot(chest, 2).unwrap().unwrap().count, 4);
        assert_eq!(game.menu_slot(chest, 3), Some(None));
        game.handle_packet(C::SetCursorItem(play::SetCursorItem { item: None }))
            .unwrap();
        assert!(chest.click(game, &[pickup(2)]));
        assert_eq!(chest.slot(game, 2), Some(None));
        assert_eq!(chest.carried(game).unwrap().unwrap().count(), 4);
        assert!(chest.click(game, &[pickup(2)]));
        assert_eq!(chest.slot(game, 2).unwrap().unwrap().count(), 4);
        assert_eq!(chest.carried(game), Some(None));
        for expected in [
            (0, 17, -999, 0, 5),
            (0, 17, 37, 1, 5),
            (0, 17, -999, 2, 5),
            (0, 17, 37, 0, 0),
            (0, 17, 36, 0, 0),
            (0, 17, 36, 0, 1),
            (0, 17, 9, 1, 2),
            (0, 17, 37, 1, 4),
            (7, 0, 2, 0, 0),
            (7, 0, 2, 0, 0),
        ] {
            let sent = (0..32)
                .find_map(|_| {
                    let (id, body) = reader.recv().unwrap();
                    match play::Serverbound::decode(id, &body).unwrap() {
                        play::Serverbound::ContainerClick(sent) => Some(sent),
                        _ => None,
                    }
                })
                .expect("expected click packet");
            assert_eq!(
                (
                    sent.container,
                    sent.state,
                    sent.slot,
                    sent.button,
                    sent.input
                ),
                expected
            );
            if expected == (0, 17, -999, 2, 5) {
                assert!(sent.carried.is_none());
                assert_eq!(sent.changed.len(), 1);
                assert_eq!(sent.changed[0].0, 37);
                assert_eq!(sent.changed[0].1.as_ref().unwrap().count, 3);
            }
        }
        reader
            .set_read_timeout(Some(std::time::Duration::from_millis(20)))
            .unwrap();
        assert!(reader.recv().unwrap_err().is_timed_out());
        game.remote
            .as_mut()
            .unwrap()
            .menu
            .set(Some((7, "minecraft:unknown".into())));
        assert!(
            game.local_player()
                .and_then(|player| game.player_container_menu(player))
                .is_none()
        );
        assert_eq!(game.menu_slot_count(inventory), None);
        assert_eq!(game.menu_slot_count(chest), None);
        game.remote.as_mut().unwrap().menu.set(None);
        let before_replacement = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        let mut other = spawn;
        other.dimension = "test:other_overworld".into();
        game.enter_remote_dimension(&other, true, Reason::Other);
        assert_eq!(game.menu_slot_count(before_replacement), None);
        assert!(!before_replacement.click(game, &[pickup(36)]));
        let before_logout = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        game.client_logging_out();
        assert_eq!(game.menu_slot_count(before_logout), None);
        assert!(!before_logout.click(game, &[pickup(36)]));
        assert!(
            game.local_player()
                .and_then(|player| game.player_container_menu(player))
                .is_none()
        );
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "MenuReadTest").unwrap();
        game.enter_remote_dimension(&other, false, Reason::Other);
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        let reconnected = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        assert_eq!(game.menu_slot_count(before_logout), None);
        assert_eq!(game.menu_slot_count(reconnected), Some(46));
        assert_ne!(reconnected, before_logout);
    });
}

/// `MenuRef::close` and `click_button` go through the screen's own close
/// and the button packet, and refuse a menu that is not open or stale.
#[test]
#[ignore = "daily"]
fn client_menu_close_and_button_click_reach_the_server() {
    use super::super::{
        Screen, execution_role::ExecutionRole, native_entity_refs::tests::with_game,
    };
    use horizon_core::nbt::Tag;
    use horizon_protocol::packets::play::{self, Clientbound as C};

    with_game(|game| {
        let (mut reader, _writer) = connect_menu_test(game);
        spawn_menu_test(game);
        let sent = |reader: &mut horizon_protocol::connection::ConnReader| {
            (0..32).find_map(|_| {
                let (id, body) = reader.recv().ok()?;
                match play::Serverbound::decode(id, &body).unwrap() {
                    p @ (play::Serverbound::ContainerClose(_)
                    | play::Serverbound::ContainerButtonClick(_)) => Some(p),
                    _ => None,
                }
            })
        };
        // The inventory menu while playing: no screen to close, no buttons.
        let inventory = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        assert_eq!(game.screen, Screen::Playing);
        assert!(!inventory.close(game));
        assert!(!inventory.click_button(game, 0));
        let loom = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:loom")
            .unwrap();
        let open = |game: &mut super::Game<'_>| {
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 5,
                menu: loom,
                title: Tag::String("Loom".into()),
            }))
            .unwrap();
            game.local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap()
        };
        let menu = open(game);
        assert_eq!(game.screen, Screen::Inventory);
        // The old inventory handle is stale now that the loom replaced it.
        assert!(!inventory.close(game));
        assert!(!inventory.click_button(game, 0));
        game.options.execution_role = ExecutionRole::IntegratedServer;
        assert!(!menu.click_button(game, 3));
        assert!(!menu.close(game));
        game.options.execution_role = ExecutionRole::Client;
        assert_eq!(game.screen, Screen::Inventory);
        assert!(menu.click_button(game, 3));
        let Some(play::Serverbound::ContainerButtonClick(click)) = sent(&mut reader) else {
            panic!("expected the button click, not a refused request");
        };
        assert_eq!((click.container, click.button), (5, 3));
        assert!(menu.close(game));
        assert_eq!(game.screen, Screen::Playing);
        let Some(play::Serverbound::ContainerClose(close)) = sent(&mut reader) else {
            panic!("expected the close");
        };
        assert_eq!(close.container, 5);
        assert!(game.remote.as_ref().unwrap().menu.as_ref().is_none());
        // Closed: the handle is stale and nothing more is sent.
        assert!(!menu.close(game));
        assert!(!menu.click_button(game, 3));
        // A server-side close leaves a reopened menu's old handle stale too.
        let first = open(game);
        game.handle_packet(C::ContainerClose(play::ContainerClose { container: 5 }))
            .unwrap();
        let second = open(game);
        assert!(!first.close(game));
        assert!(!first.click_button(game, 3));
        assert!(second.close(game));
        reader
            .set_read_timeout(Some(std::time::Duration::from_millis(20)))
            .unwrap();
        let Some(play::Serverbound::ContainerClose(close)) = sent(&mut reader) else {
            panic!("expected the second close");
        };
        assert_eq!(close.container, 5);
        assert!(sent(&mut reader).is_none());
    });
}

/// A client names the level it shows by the server's key, even where `Dim` has
/// no name for it.
#[test]
#[ignore = "daily"]
fn client_handles_reach_a_custom_level() {
    use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
    use horizon_client::level_loading::Reason;
    use horizon_protocol::{connection::Pipe, packets::play::SpawnInfo};
    use mod_api::{DimensionKey, LevelRef};
    use std::sync::Arc;

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        let (client, _server) = Pipe::pair();
        use horizon_world::dimension_type::DimensionFacts;
        let stale_key = horizon_core::LevelKey::parse("test:stale_guest_facts").unwrap();
        let mut stale = DimensionFacts::overworld().clone();
        stale.coordinate_scale = 19.0;
        DimensionFacts::set_levels(vec![(stale_key, stale)]);
        game.connect_local(client, "CustomLevelTest").unwrap();
        assert_eq!(game.dimension_facts_for(stale_key).coordinate_scale, 1.0);
        let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
        let json = serde_json::json!({"min_y": 0, "height": 256,
            "coordinate_scale": 4.0, "ambient_light": 0.3,
            "attributes": {"minecraft:gameplay/fast_lava": true}});
        registries.registries.insert(
            "minecraft:dimension_type".into(),
            vec![(
                "test:type".into(),
                Some(horizon_client::scoreboard::json_to_tag(&json)),
            )],
        );
        game.set_registries(Arc::new(registries));
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "test:pocket".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, false, Reason::Other);
        game.remote.as_mut().unwrap().logged_in = true;

        let level = game.level("test:pocket").unwrap();
        assert_ne!(level.dimension(), DimensionKey::OVERWORLD);
        assert_eq!(level.dimension().id(game).as_deref(), Some("test:pocket"));
        assert!(game.level("minecraft:overworld").is_none());
        assert!(level.height(game).is_some());
        let facts = level.dimension_type(game).unwrap();
        assert_eq!(facts.id, "test:type");
        assert_eq!(facts.coordinate_scale, 4.0);
        assert_eq!(facts.ambient_light, 0.3);
        assert!(facts.fast_lava);
        assert_eq!(game.dimension_type().coordinate_scale, 4.0);
        let overworld = LevelRef::new(level.epoch(), DimensionKey::OVERWORLD);
        assert_eq!(overworld.height(game), None);
        // An equal JSON body with a different type name must not hit the old cache.
        let mut renamed = (*game.remote.as_ref().unwrap().registries).clone();
        renamed
            .registries
            .get_mut("minecraft:dimension_type")
            .unwrap()[0]
            .0 = "test:renamed".into();
        game.set_registries(Arc::new(renamed));
        game.enter_remote_dimension(&spawn, false, Reason::Other);
        assert_eq!(game.dimension_type().id, "test:renamed");
        // Reject invalid server numbers and enums without failing the join.
        for (field, value) in [
            ("coordinate_scale", serde_json::json!(0.0)),
            ("coordinate_scale", serde_json::json!("NaN")),
            ("coordinate_scale", serde_json::Value::Null),
            ("ambient_light", serde_json::json!("NaN")),
            ("ambient_light", serde_json::Value::Null),
            ("monster_spawn_block_light_limit", serde_json::json!(16)),
            ("monster_spawn_block_light_limit", serde_json::json!(-1)),
            ("skybox", serde_json::json!("invalid")),
        ] {
            let mut invalid = json.clone();
            invalid[field] = value;
            let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
            registries.registries.insert(
                "minecraft:dimension_type".into(),
                vec![(
                    "minecraft:the_nether".into(),
                    Some(horizon_client::scoreboard::json_to_tag(&invalid)),
                )],
            );
            game.set_registries(Arc::new(registries));
            game.enter_remote_dimension(&spawn, false, Reason::Other);
            assert_eq!(game.dimension_type().coordinate_scale, 8.0, "{field}");
            assert!(game.dimension_type().fast_lava);
        }
        let min = f64::from(1.0E-5_f32);
        let max = 30_000_000.0_f64;
        for (value, valid) in [
            (min.next_down(), false),
            (min, true),
            (min.next_up(), true),
            (max.next_down(), true),
            (max, true),
            (max.next_up(), false),
            (f64::from_bits(1), false),
        ] {
            let mut boundary = json.clone();
            boundary["coordinate_scale"] = serde_json::json!(value);
            // Vanilla uses Codec.FLOAT, not a unit-interval ambient-light codec.
            boundary["ambient_light"] = serde_json::json!(1.1);
            let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
            registries.registries.insert(
                "minecraft:dimension_type".into(),
                vec![(
                    "minecraft:the_nether".into(),
                    Some(horizon_client::scoreboard::json_to_tag(&boundary)),
                )],
            );
            game.set_registries(Arc::new(registries));
            game.enter_remote_dimension(&spawn, false, Reason::Other);
            assert_eq!(
                game.dimension_type().coordinate_scale,
                if valid { value } else { 8.0 }
            );
            if valid {
                assert_eq!(game.dimension_type().ambient_light, 1.1);
            }
        }
        for ambient in [1.5_f32, -0.5_f32] {
            let mut legal = json.clone();
            legal["ambient_light"] = serde_json::json!(ambient);
            let block_limit = if ambient > 0.0 { 15 } else { 0 };
            legal["monster_spawn_block_light_limit"] = serde_json::json!(block_limit);
            let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
            registries.registries.insert(
                "minecraft:dimension_type".into(),
                vec![(
                    "test:type".into(),
                    Some(horizon_client::scoreboard::json_to_tag(&legal)),
                )],
            );
            game.set_registries(Arc::new(registries));
            game.enter_remote_dimension(&spawn, false, Reason::Other);
            assert_eq!(game.dimension_type().coordinate_scale, 4.0);
            assert_eq!(game.dimension_type().ambient_light, ambient);
            assert_eq!(
                game.dimension_type().monster_spawn_block_light_limit,
                block_limit
            );
            let level = game.level("test:pocket").unwrap();
            assert_eq!(level.dimension_type(game).unwrap().ambient_light, ambient);
        }
        // Disconnect drops the connection-owned table; the server table is untouched.
        let lifetime = Arc::downgrade(game.remote.as_ref().unwrap().level_facts.as_ref().unwrap());
        game.leave_server(None).unwrap();
        assert!(
            lifetime.upgrade().is_none(),
            "connection facts must be released"
        );
        assert_eq!(DimensionFacts::of_level(stale_key).coordinate_scale, 19.0);
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "SecondFactsSession").unwrap();
        let key = horizon_core::LevelKey::parse("test:pocket").unwrap();
        assert_eq!(game.dimension_facts_for(key).coordinate_scale, 1.0);
        DimensionFacts::set_levels(Vec::new());
    });
}

/// The `both` handle reads answer on a client from its own copy: the local
/// player's health, effects, equipment and last death, the blocks and time
/// of day of the level it shows and the entities it tracks.
#[test]
#[ignore = "daily"]
fn client_handle_reads_answer_from_the_clients_copy() {
    use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
    use horizon_client::{
        chunk::Chunk, first_person::Hand, level_loading::Reason, motion::PlayerMotion,
    };
    use horizon_core::geometry::Vec3;
    use horizon_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C, SpawnInfo},
    };
    use mod_api::{EntityRef, EquipmentSlot, GlobalPos};
    use std::sync::Arc;

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "HandleReadTest").unwrap();
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: Some(play::GlobalPos {
                dimension: "minecraft:the_nether".into(),
                pos: [4, 70, -9],
            }),
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, false, Reason::Other);
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(1.5, 65.0, -2.5)).unwrap());
        game.yaw = 90.0;
        game.pitch = 10.0;

        // The local player and the level it shows.
        let player = game.local_player().unwrap();
        let me = player.entity();
        let living = game.living(me).unwrap();
        let level = game.level("minecraft:overworld").unwrap();
        assert!(game.level("minecraft:the_nether").is_none());
        assert_eq!(me.position(game), Some([1.5, 65.0, -2.5]));
        assert_eq!(me.rotation(game), Some([90.0, 10.0]));
        game.player.as_mut().unwrap().velocity = Vec3::new(0.0, -0.5, 0.25);
        assert_eq!(me.velocity(game), Some([0.0, -0.5, 0.25]));

        // Last death: the login/respawn packet's, as a GlobalPos.
        let died = player.last_death_location(game).unwrap();
        assert_eq!(died.dimension().to_string(), "minecraft:the_nether");
        let pos = died.pos();
        assert_eq!([pos.x, pos.y, pos.z], [4, 70, -9]);
        let _: GlobalPos = died;

        // Health, maximum health, effects and equipment come from the packets.
        game.handle_packet(C::SetHealth(play::SetHealth {
            health: 14.0,
            food: 20,
            saturation: 5.0,
        }))
        .unwrap();
        assert_eq!(living.health(game), Some(14.0));
        assert_eq!(living.max_health(game), Some(20.0));
        let speed = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:mob_effect")
            .unwrap()
            .id("minecraft:speed")
            .unwrap();
        assert_eq!(living.effects(game), Some(Vec::new()));
        assert!(
            living
                .effect(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed")
                )
                .is_none()
        );
        game.handle_packet(C::UpdateMobEffect(play::UpdateMobEffect {
            id: 7,
            effect: speed,
            amplifier: 1,
            duration: 600,
            flags: 2,
        }))
        .unwrap();
        let effect = living
            .effect(
                game,
                mod_platform::__private::part_tests::effect_key("minecraft:speed"),
            )
            .unwrap();
        assert_eq!((effect.duration, effect.amplifier), (600, 1));
        assert_eq!(living.effects(game).unwrap().len(), 1);
        let stone = Some(horizon_protocol::item::Stack::new(
            game.item_id("minecraft:stone"),
            3,
        ));
        game.handle_packet(C::SetPlayerInventory(play::SetPlayerInventory {
            slot: 0,
            item: stone,
        }))
        .unwrap();
        let held = living.hand_item(game, Hand::Main).unwrap();
        assert_eq!(
            (
                mod_api::__private::item_key_id(held.item()).unwrap(),
                held.count()
            ),
            ("minecraft:stone", 3)
        );
        assert_eq!(living.equipment(game, EquipmentSlot::Head), None);
        assert_eq!(living.hand_item(game, Hand::Off), None);

        // Blocks of the chunks the client holds; nothing loads another.
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let chunks = game.world.chunks.len();
        let at = [3, 64, 5];
        assert!(level.is_loaded(game, at));
        assert!(!level.is_loaded(game, [160, 64, 0]));
        game.handle_packet(C::BlockUpdate(play::BlockUpdate { pos: at, state: 1 }))
            .unwrap();
        assert_eq!(
            level
                .block(game, at)
                .and_then(mod_api::__private::block_key_id),
            Some("minecraft:stone")
        );
        assert!(level.block_state(game, at).is_some());
        assert_eq!(level.block(game, [160, 64, 0]), None);
        assert_eq!(level.block_property(game, [160, 64, 0], "facing"), None);
        assert_eq!(game.world.chunks.len(), chunks);

        // The time of day, as the server's clock last set it.
        game.set_registries({
            let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
            registries.registries.insert(
                "minecraft:world_clock".into(),
                vec![("minecraft:overworld".into(), None)],
            );
            Arc::new(registries)
        });
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 100,
            clocks: vec![play::ClockState {
                clock: 0,
                total_ticks: 30_000,
                partial_tick: 0.0,
                rate: 1.0,
            }],
        }))
        .unwrap();
        assert_eq!(level.day_time(game), Some(30_000));

        // An entity the server told the client of; others read None.
        let pig = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:pig")
            .unwrap();
        game.handle_packet(C::AddEntity(play::AddEntity {
            id: 31,
            uuid: 9,
            kind: pig,
            position: [10.0, 64.0, 12.0],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 64,
            head_yaw: 64,
            data: 0,
        }))
        .unwrap();
        let other = game.entity(31).unwrap();
        assert_eq!(other.position(game), Some([10.0, 64.0, 12.0]));
        assert!(other.velocity(game).is_some());
        assert!(other.rotation(game).is_some());
        assert!(game.living(other).is_none(), "only the local player");
        assert!(game.entity(99).is_none());
        let untracked = EntityRef::new(me.epoch(), 99);
        assert_eq!(untracked.position(game), None);
        assert_eq!(untracked.velocity(game), None);

        // A replaced level retires the handles.
        let mut other_level = spawn.clone();
        other_level.dimension = "test:other_overworld".into();
        game.enter_remote_dimension(&other_level, true, Reason::Other);
        assert_eq!(me.position(game), None);
        assert_eq!(living.health(game), None);
        assert_eq!(level.block(game, at), None);
        assert!(!level.is_loaded(game, at));
        assert_eq!(level.day_time(game), None);
        assert_eq!(player.last_death_location(game), None);

        // The server's role keeps its own reads: a client handle names nothing there.
        game.options.execution_role = ExecutionRole::IntegratedServer;
        assert_eq!(game.native_entity_target(untracked), None);
    });
}

#[test]
#[ignore = "daily"]
fn client_day_time_reads_overworld_clock_across_packets_and_levels() {
    use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
    use horizon_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C},
    };
    use mod_api::LevelRef;
    use std::sync::Arc;

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "ClockReadTest").unwrap();
        let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
        let dimensions = vec![
            ("minecraft:overworld".into(), None),
            ("minecraft:the_end".into(), None),
            ("minecraft:the_nether".into(), None),
        ];
        registries
            .registries
            .insert("minecraft:world_clock".into(), dimensions[..2].to_vec());
        registries
            .registries
            .insert("minecraft:dimension_type".into(), dimensions);
        let registries = Arc::new(registries);
        game.set_registries(registries.clone());
        let mut spawn = play::SpawnInfo {
            dimension_type: 1,
            dimension: "minecraft:the_end".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        let login = |spawn| {
            C::Login(play::Login {
                player_id: 7,
                hardcore: false,
                levels: vec!["minecraft:overworld".into(), "minecraft:the_end".into()],
                max_players: 1,
                chunk_radius: 2,
                simulation_distance: 2,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn,
                online_mode: false,
                enforces_secure_chat: false,
            })
        };
        let state = |clock, total_ticks, partial_tick, rate| play::ClockState {
            clock,
            total_ticks,
            partial_tick,
            rate,
        };
        game.handle_packet(login(spawn.clone())).unwrap();
        let end = game.level("minecraft:the_end").unwrap();
        let wrong_level = LevelRef::new(end.epoch(), mod_api::DimensionKey::OVERWORLD);
        assert_eq!(wrong_level.day_time(game), None);
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 100,
            clocks: vec![state(0, 30_000, 0.0, 1.0), state(1, 100, 0.0, 1.0)],
        }))
        .unwrap();
        assert_eq!(end.day_time(game), Some(30_000));
        assert_eq!(
            game.world.day_time, 100,
            "the sky follows the End default clock"
        );

        // An Overworld-only update must not select it as the End sky clock.
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 104,
            clocks: vec![state(0, 40_000, 0.75, 0.5)],
        }))
        .unwrap();
        assert_eq!(end.day_time(game), Some(40_000));
        assert_eq!(game.world.day_time, 104);
        game.tick_remote_level().unwrap();
        assert_eq!(end.day_time(game), Some(40_001));
        assert_eq!(game.world.day_time, 105);
        game.tick_remote_level().unwrap();
        assert_eq!(end.day_time(game), Some(40_001));
        assert_eq!(game.world.day_time, 106);

        // Empty time syncs advance every retained clock by the game-time delta.
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 110,
            clocks: vec![],
        }))
        .unwrap();
        assert_eq!(end.day_time(game), Some(40_003));
        assert_eq!(game.world.day_time, 110);
        // A paused clock stays put even while the client's game time runs.
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 110,
            clocks: vec![state(0, 50_000, 0.25, 0.0)],
        }))
        .unwrap();
        game.tick_remote_level().unwrap();
        assert_eq!(end.day_time(game), Some(50_000));
        assert_eq!(game.world.day_time, 111);

        // A frozen world ticks neither clock; packets still catch both up.
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 111,
            clocks: vec![state(0, 60_000, 0.0, 2.0)],
        }))
        .unwrap();
        game.handle_packet(C::TickingState(play::TickingState {
            rate: 20.0,
            frozen: true,
        }))
        .unwrap();
        game.tick_remote_level().unwrap();
        assert_eq!(game.world.game_time, 111);
        assert_eq!(end.day_time(game), Some(60_000));
        assert_eq!(game.world.day_time, 111);
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 114,
            clocks: vec![],
        }))
        .unwrap();
        assert_eq!(end.day_time(game), Some(60_006));
        assert_eq!(game.world.day_time, 114);
        // Backward corrections use the same signed delta and fractional state.
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 113,
            clocks: vec![],
        }))
        .unwrap();
        assert_eq!(end.day_time(game), Some(60_004));
        assert_eq!(game.world.day_time, 113);
        game.handle_packet(C::TickingState(play::TickingState {
            rate: 20.0,
            frozen: false,
        }))
        .unwrap();

        // Same-level respawn keeps the epoch and clocks, but replaces the
        // local player and its mood handler, with or without entity data.
        for keep in [0, 2] {
            game.ambience.moodiness = 0.375;
            game.handle_packet(C::Respawn(play::Respawn {
                spawn: spawn.clone(),
                keep,
            }))
            .unwrap();
            assert_eq!(game.ambience.moodiness, 0.0);
            assert_eq!(end.day_time(game), Some(60_004));
        }
        spawn.dimension_type = 0;
        spawn.dimension = "minecraft:overworld".into();
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn.clone(),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(end.day_time(game), None);
        let overworld = game.level("minecraft:overworld").unwrap();
        assert_eq!(overworld.day_time(game), Some(60_004));
        assert_eq!(
            game.world.day_time, 60_004,
            "select the new default from retained clocks"
        );
        game.tick_remote_level().unwrap();
        assert_eq!(overworld.day_time(game), Some(60_006));
        assert_eq!(game.world.day_time, 60_006);

        // A dimension without a default clock must not select the prior one.
        spawn.dimension_type = 2;
        spawn.dimension = "minecraft:the_nether".into();
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn.clone(),
            keep: 0,
        }))
        .unwrap();
        let nether = game.level("minecraft:the_nether").unwrap();
        assert_eq!(overworld.day_time(game), None);
        assert_eq!(nether.day_time(game), Some(60_006));
        assert_eq!(game.world.day_time, 0);

        // Login starts a new listener epoch, even on the same connection.
        game.handle_packet(login(spawn.clone())).unwrap();
        assert_eq!(nether.day_time(game), None);
        let fresh = game.level("minecraft:the_nether").unwrap();
        assert_eq!(fresh.day_time(game), Some(0));
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 1,
            clocks: vec![],
        }))
        .unwrap();
        assert_eq!(
            fresh.day_time(game),
            Some(1),
            "the new clock anchor and rate are fresh too"
        );
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 1,
            clocks: vec![state(0, 7_000, 0.0, 1.0)],
        }))
        .unwrap();
        assert_eq!(fresh.day_time(game), Some(7_000));
        assert_eq!(game.world.day_time, 0);
        game.client_logging_out();
        assert_eq!(fresh.day_time(game), None);
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "ClockReadTest").unwrap();
        assert_eq!(fresh.day_time(game), None);
        game.set_registries(registries);
        game.handle_packet(login(spawn)).unwrap();
        let reconnected = game.level("minecraft:the_nether").unwrap();
        assert_eq!(fresh.day_time(game), None);
        assert_eq!(reconnected.day_time(game), Some(0));
    });
}

#[test]
#[ignore = "daily"]
fn client_camera_entity_returns_to_player_after_removal() {
    use super::super::{
        Game, native_entity_refs::tests::with_client_game, native_lifecycle::NativeLifecycle,
    };
    use horizon_client::{level_loading::Reason, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;
    use horizon_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C, SpawnInfo},
    };
    use mod_api::ClientHost;

    with_client_game(|game| {
        let camera = |game: &Game<'_>| (game as &dyn ClientHost).camera_entity();
        assert!(camera(game).is_none(), "the fixture has no player");
        game.native_lifecycle = NativeLifecycle::default();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "CameraReadTest").unwrap();
        let mut spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 3,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, false, Reason::Other);
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        assert!(camera(game).is_none(), "a level alone has no player");
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let local = camera(game).unwrap();
        assert_eq!(
            local.id(),
            7,
            "use the remote identity, not PlayerMotion's default"
        );
        assert_eq!(local.position(game), Some([0.0; 3]));

        let creeper = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:creeper")
            .unwrap();
        game.handle_packet(C::AddEntity(play::AddEntity {
            id: 31,
            uuid: 9,
            kind: creeper,
            position: [10.0, 64.0, 12.0],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        }))
        .unwrap();
        game.remote_set_camera(31);
        assert!(!game.controls_camera());
        assert!(game.hides_as_camera(31));
        assert!(game.spectated_post_effect.is_some());
        let attached = camera(game).unwrap();
        game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![99] }))
            .unwrap();
        assert_eq!(camera(game), Some(attached));
        assert_eq!(attached.id(), 31);
        assert_eq!(attached.position(game), Some([10.0, 64.0, 12.0]));
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 99 }))
            .unwrap();
        assert_eq!(
            camera(game),
            Some(attached),
            "unknown packet targets are ignored"
        );
        game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![31] }))
            .unwrap();
        assert_eq!(camera(game), Some(local));
        assert!(
            game.controls_camera(),
            "movement input and position packets are enabled"
        );
        assert!(!game.hides_as_camera(31));
        assert!(game.spectated_post_effect.is_none());
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 7 }))
            .unwrap();
        assert_eq!(camera(game), Some(local));

        game.title = Some(game.new_title(game.save.root.clone()));
        assert!(
            camera(game).is_none(),
            "the title screen has no camera entity"
        );
        game.title = None;
        assert_eq!(camera(game), Some(local));

        spawn.dimension = "test:other_overworld".into();
        game.handle_packet(C::Respawn(play::Respawn { spawn, keep: 0 }))
            .unwrap();
        assert!(
            camera(game).is_none(),
            "replacement waits for its new player"
        );
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let fresh = camera(game).unwrap();
        assert_eq!(fresh.id(), local.id());
        assert_ne!(fresh.epoch(), local.epoch());
        assert_eq!(local.position(game), None, "old level handles stay stale");
        assert_eq!(fresh.position(game), Some([0.0; 3]));
        game.client_logging_out();
        assert!(camera(game).is_none());
        assert_eq!(fresh.position(game), None);
    });
}

/// The local player's own state reads answer on a client from what the
/// server sent it (`LocalPlayer`); another player reads None there.
#[test]
#[ignore = "daily"]
fn client_player_reads_answer_from_the_local_players_copy() {
    use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
    use horizon_client::{level_loading::Reason, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;
    use horizon_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C, SpawnInfo},
    };
    use mod_api::{EntityRef, GameMode};

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "PlayerReadTest").unwrap();
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, false, Reason::Other);
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 65.0, 0.5)).unwrap());
        let player = game.local_player().unwrap();

        // Survival, as the login packet left it: building allowed, no flight.
        assert_eq!(player.game_mode(game), Some(GameMode::Survival));
        assert_eq!(player.may_build(game), Some(true));
        assert_eq!(player.abilities(game).map(|a| a.may_fly), Some(false));

        // CHANGE_GAME_MODE to adventure: no building; then the abilities packet.
        game.handle_packet(C::GameEvent(play::GameEvent {
            event: 3,
            param: 2.0,
        }))
        .unwrap();
        assert_eq!(player.game_mode(game), Some(GameMode::Adventure));
        assert_eq!(player.is_creative(game), Some(false));
        assert_eq!(player.is_spectator(game), Some(false));
        assert_eq!(player.may_build(game), Some(false));
        game.handle_packet(C::PlayerAbilities(play::PlayerAbilities {
            flags: 4 | 2,
            flying_speed: 0.1,
            walking_speed: 0.2,
        }))
        .unwrap();
        let abilities = player.abilities(game).unwrap();
        assert!(abilities.may_fly && abilities.flying && !abilities.invulnerable);
        assert!(!abilities.may_build);
        assert_eq!((abilities.fly_speed, abilities.walk_speed), (0.1, 0.2));
        // A double jump toggles flight on the client first, and the server sends no
        // abilities back: the read follows the body (`LocalPlayer.getAbilities()`).
        game.player.as_mut().unwrap().flying = false;
        assert_eq!(player.abilities(game).map(|a| a.flying), Some(false));
        game.handle_packet(C::PlayerAbilities(play::PlayerAbilities {
            flags: 4,
            flying_speed: 0.1,
            walking_speed: 0.2,
        }))
        .unwrap();
        game.player.as_mut().unwrap().flying = true;
        assert_eq!(player.abilities(game).map(|a| a.flying), Some(true));

        // Food from SetHealth; the client keeps no exhaustion.
        game.handle_packet(C::SetHealth(play::SetHealth {
            health: 20.0,
            food: 5,
            saturation: 1.5,
        }))
        .unwrap();
        let food = player.food(game).unwrap();
        assert_eq!(
            (food.level, food.saturation, food.exhaustion),
            (5, 1.5, 0.0)
        );
        assert_eq!(player.needs_food(game), Some(true));
        assert_eq!(player.has_enough_food(game), Some(false));

        // The hotbar slot and the experience the server set.
        game.handle_packet(C::SetHeldSlot(play::SetHeldSlot { slot: 4 }))
            .unwrap();
        assert_eq!(player.selected_slot(game), Some(4));
        game.handle_packet(C::SetExperience(play::SetExperience {
            progress: 0.25,
            level: 3,
            total: 30,
        }))
        .unwrap();
        let xp = player.experience(game).unwrap();
        assert_eq!((xp.level, xp.progress, xp.total), (3, 0.25, 30));
        assert_eq!(player.xp_needed_for_next_level(game), Some(13));

        // The main inventory retains typed stacks and their damage, in slot order.
        let mut stack = horizon_client::items::ItemStack::new("minecraft:diamond_pickaxe", 1);
        stack.damage = 7;
        game.inventory.fill(None);
        game.inventory[3] = Some(stack.clone());
        game.inventory[8] = Some(horizon_client::items::ItemStack::new("minecraft:stone", 12));
        assert_eq!(
            player.inventory(game),
            Some(
                vec![
                    stack,
                    horizon_client::items::ItemStack::new("minecraft:stone", 12)
                ]
                .try_into_api()
                .unwrap()
            )
        );

        // The body's own movement state, which the client keeps itself (`LocalPlayer`).
        let me = player.entity();
        assert_eq!(me.is_sprinting(game), Some(false));
        assert_eq!(me.is_in_water(game), Some(false));
        let body = game.player.as_mut().unwrap();
        body.sprinting = true;
        body.fluids.was_touching_water = true;
        body.pose = horizon_client::pose::Pose::Crouching;
        assert_eq!(me.is_sprinting(game), Some(true));
        assert_eq!(me.is_in_water(game), Some(true));
        assert_eq!(me.is_in_lava(game), Some(false));
        assert_eq!(me.is_crouching(game), Some(true));

        // Another player: the client isn't sent their state.
        let epoch = player.entity().epoch();
        let other = mod_api::__private::game::player_ref(EntityRef::new(epoch, 8), 43);
        assert_eq!(other.entity().is_sprinting(game), None);
        assert_eq!(other.game_mode(game), None);
        assert_eq!(other.food(game), None);
        assert_eq!(other.selected_slot(game), None);
        assert_eq!(other.inventory(game), None);
    });
}

#[test]
#[ignore = "daily"]
fn client_epochs_follow_level_replacement_and_logout() {
    use super::super::{
        execution_role::ExecutionRole, native_entity_refs::tests::with_game,
        native_lifecycle::NativeLifecycle,
    };
    use horizon_client::{level_loading::Reason, mobs::Mob, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;
    use horizon_protocol::{
        connection::Pipe,
        packets::play::{Clientbound, Respawn, SpawnInfo},
    };
    use mod_api::{CrosshairTarget, EntityRef};

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        game.native_lifecycle = NativeLifecycle::default();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "EpochTest").unwrap();
        // A transport alone has no ClientLevel yet.
        assert!(game.native_lifecycle.epoch().is_none());
        let mut spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, false, Reason::Other);
        let joined = game.native_lifecycle.epoch().unwrap();
        let old = EntityRef::new(joined, 12);
        let remote = game.remote.as_mut().unwrap();
        remote.menu.set(Some((7, "minecraft:generic_9x3".into())));
        let open_generation = remote.menu.generation;
        game.handle_packet(Clientbound::Respawn(Respawn {
            spawn: spawn.clone(),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.native_lifecycle.epoch(), Some(joined));
        let remote = game.remote.as_mut().unwrap();
        assert!(remote.menu.as_ref().is_none());
        assert_ne!(remote.menu.generation, open_generation);
        let inventory_generation = remote.menu.generation;
        game.handle_packet(Clientbound::Respawn(Respawn {
            spawn: spawn.clone(),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.native_lifecycle.epoch(), Some(joined));
        let remote = game.remote.as_mut().unwrap();
        assert!(remote.menu.as_ref().is_none());
        assert_ne!(remote.menu.generation, inventory_generation);
        remote.menu.set(Some((7, "minecraft:generic_9x3".into())));
        assert_ne!(remote.menu.generation, open_generation);
        assert_ne!(remote.menu.generation, inventory_generation);

        // A different level key replaces the world even with the same
        // dimension type; IDs can be reused but old handles cannot.
        spawn.dimension = "test:other_overworld".into();
        game.enter_remote_dimension(&spawn, true, Reason::Other);
        let replaced = game.native_lifecycle.epoch().unwrap();
        assert_ne!(replaced, joined);
        let current = EntityRef::new(replaced, 12);
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        game.mobs
            .push(Mob::new(12, kind, Vec3::new(0.0, 1.0, 2.0), 0.0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.yaw = 0.0;
        game.pitch = 0.0;
        let Some(CrosshairTarget::Entity { entity, .. }) = game.crosshair_target() else {
            panic!("the crosshair hits the pig in front of the player");
        };
        assert_eq!(entity, current);
        assert!(
            game.native_entity_target(current).is_none(),
            "still server-only"
        );
        // Isolate the existing epoch check from the logical-server guard.
        game.options.execution_role = ExecutionRole::IntegratedServer;
        assert!(game.native_entity_target(current).is_some());
        assert!(game.native_entity_target(old).is_none());
        game.options.execution_role = ExecutionRole::Client;

        game.client_logging_out();
        assert!(game.native_lifecycle.epoch().is_none());
        assert!(game.crosshair_target().is_none());
        game.client_logging_out();
        assert!(game.native_lifecycle.epoch().is_none());
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "EpochTest").unwrap();
        assert!(game.native_lifecycle.epoch().is_none());
        game.enter_remote_dimension(&spawn, false, Reason::Other);
        assert_ne!(game.native_lifecycle.epoch(), Some(replaced));
        game.client_logging_out();
        assert!(game.native_lifecycle.epoch().is_none());
    });
}

#[test]
#[ignore = "daily"]
fn connection_and_dimension_replacement_clear_every_particle_group() {
    use super::super::{
        execution_role::ExecutionRole, native_entity_refs::tests::with_game,
        native_hooks::particle_count_tests::fill,
    };
    use horizon_client::level_loading::Reason;
    use horizon_protocol::{connection::Pipe, packets::play::SpawnInfo};
    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        fill(game);
        // Include pending particles as well as live groups before replacing the connection.
        game.particles.elder_guardian();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "ParticleReset").unwrap();
        assert_eq!(game.client_particle_count(), 0);
        assert!(game.pickups.live.is_empty());
        game.tick_particles();
        assert_eq!(game.client_particle_count(), 0);
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        fill(game);
        game.particles.elder_guardian();
        game.enter_remote_dimension(&spawn, false, Reason::Other);
        assert_eq!(game.client_particle_count(), 0);
        assert!(game.pickups.live.is_empty());
        game.tick_particles();
        assert_eq!(
            game.client_particle_count(),
            0,
            "pending groups cannot return in the new level"
        );
    });
}

#[test]
#[ignore = "daily"]
fn forgetting_and_closing_remote_columns_clear_their_data() {
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::{chunk::Chunk, containers::BlockEntity};

    with_game(|game| {
        for x in 0..2 {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, 0));
            game.world
                .block_entities
                .insert((x * 16, 64, 0), BlockEntity::Chest(vec![]));
        }
        // A same-position resend replaces the data without removing the column.
        game.streaming
            .replace_remote(&mut game.world, Chunk::empty(0, 0));
        assert_eq!(game.world.chunks.len(), 2);
        assert!(game.world.chunks.contains_key(&(0, 0)));
        assert!(game.streaming.is_lit(0, 0));
        let mut forgotten = [(0, 0)].into_iter().collect();
        game.forget_remote_columns(&mut forgotten, true);
        assert!(!game.world.chunks.contains_key(&(0, 0)));
        assert!(!game.world.block_entities.contains_key(&(0, 64, 0)));
        assert!(!game.streaming.is_lit(0, 0));
        assert!(game.world.chunks.contains_key(&(1, 0)));
        game.forget_remote_columns(&mut [(0, 0)].into_iter().collect(), true);
        for x in [7, 13] {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, 0));
            game.world
                .block_entities
                .insert((x * 16, 64, 0), BlockEntity::Chest(vec![]));
        }
        // Cache-centre packet: move to (15, 0), at radius 10.
        game.drop_remote_chunks_out_of_range((15, 0), 10);
        assert!(!game.world.chunks.contains_key(&(1, 0)));
        assert!(game.world.chunks.contains_key(&(7, 0)));
        assert!(game.world.chunks.contains_key(&(13, 0)));
        // Cache-radius packet: shrinking to 2 keeps radius 5 around (15, 0).
        game.drop_remote_chunks_out_of_range((15, 0), 2);
        assert!(!game.world.chunks.contains_key(&(7, 0)));
        assert!(game.world.chunks.contains_key(&(13, 0)));
        assert!(!game.world.block_entities.contains_key(&(7 * 16, 64, 0)));
        // A later forget for an evicted column is harmless.
        game.forget_remote_columns(&mut [(7, 0)].into_iter().collect(), true);
        game.forget_all_remote_columns();
        assert!(game.world.chunks.is_empty());
        assert!(game.world.block_entities.is_empty());
    });
}

/// The client chunk seam (game/network/remote_chunks.rs) across every free
/// route, as the freed set each route returns: a forget packet, the
/// eager centre-move drop (a cache-centre packet), the radius-shrink
/// drop (a cache-radius packet) and the client level going. One freed
/// entry per column, in the order the routes free them, and a repeat
/// forget for an already-freed column frees nothing: adopting a column
/// is a chunk packet's doing (`remote_chunk`), not the streaming
/// level's.
#[test]
#[ignore = "daily"]
fn every_free_route_returns_the_columns_it_freed_once() {
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::chunk::Chunk;

    with_game(|game| {
        for x in [0, 1] {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, 0));
        }
        // A forget packet frees its column once; the same forget again,
        // for a column already freed, frees nothing.
        let mut gone = [(0, 0)].into_iter().collect();
        assert_eq!(game.forget_remote_columns(&mut gone, true), [(0, 0)]);
        let mut gone = [(0, 0)].into_iter().collect();
        assert!(game.forget_remote_columns(&mut gone, true).is_empty());
        for x in [7, 13] {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, 0));
        }
        // The centre move to (15, 0) at radius 10 frees (1, 0) only.
        assert_eq!(game.drop_remote_chunks_out_of_range((15, 0), 10), [(1, 0)]);
        // Shrinking the radius to 2 frees (7, 0) only.
        assert_eq!(game.drop_remote_chunks_out_of_range((15, 0), 2), [(7, 0)]);
        // The client level going frees what is left.
        assert_eq!(game.forget_all_remote_columns(), [(13, 0)]);
        assert!(game.world.chunks.is_empty());
        assert!(game.world.block_entities.is_empty());
    });
}

/// The seam sits at the packet level: streaming adopting, replacing or
/// re-sending a column keeps it resident — a re-sent position loads
/// again (`remote_chunk`), never an unload and a load — so a forget
/// returns exactly it.
#[test]
#[ignore = "daily"]
fn streaming_adoption_and_resend_stay_resident_until_the_forget() {
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::chunk::Chunk;

    with_game(|game| {
        game.streaming
            .insert_remote(&mut game.world, Chunk::empty(0, 0));
        // A re-send of a resident, lit column takes replace_remote.
        game.streaming
            .replace_remote(&mut game.world, Chunk::empty(0, 0));
        assert!(game.world.chunks.contains_key(&(0, 0)));
        let mut gone = [(0, 0)].into_iter().collect();
        assert_eq!(game.forget_remote_columns(&mut gone, true), [(0, 0)]);
    });
}

#[test]
#[ignore = "daily"]
fn joining_a_server_empties_the_off_hand_too() {
    // ClientPacketListener.handleLogin makes a new LocalPlayer, whose whole Inventory is
    // empty: a server that never sends slot 45 must not leave the last server's item.
    use crate::game::{
        Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
    };
    use horizon_client::items::ItemStack;
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    let (first, _first_server) = horizon_protocol::connection::Pipe::pair();
    game.connect_local(first, "HopTest").unwrap();
    let stack = || Some(ItemStack::new("minecraft:shield", 1));
    game.inventory[0] = stack();
    game.armor[0] = stack();
    game.offhand = stack();
    let (second, _second_server) = horizon_protocol::connection::Pipe::pair();
    game.connect_local(second, "HopTest").unwrap();
    assert!(game.inventory[0].is_none() && game.armor[0].is_none());
    assert!(game.offhand.is_none());
}

#[test]
fn the_chunk_storage_takes_any_radius_and_centre_a_server_sends() {
    use super::in_storage_range;
    // Vanilla's storage: radius at least 2, plus 3, each way from the centre.
    assert!(in_storage_range(10, (0, 0), 13, -13));
    assert!(!in_storage_range(10, (0, 0), 14, 0));
    assert!(in_storage_range(-5, (100, 100), 105, 95));
    assert!(!in_storage_range(0, (100, 100), 106, 100));
    // Out of i32's range, as a server may send: no overflow, and the sums a wrap got wrong.
    assert!(in_storage_range(i32::MAX, (0, 0), i32::MAX, i32::MIN + 1));
    assert!(!in_storage_range(10, (i32::MIN, 0), i32::MAX, 0));
    assert!(in_storage_range(
        10,
        (i32::MIN, i32::MAX),
        i32::MIN + 13,
        i32::MAX - 13
    ));
}

use super::{
    boat_interact_passes, chat_after_event, client_has_level, hold_server_fire,
    item_frame_interact_passes, keep_client_state, open_hash_order, remote_object_swings,
    remote_use_swings, shape_meets, snap_target, synced_local_use,
};

#[test]
#[ignore = "daily"]
fn tracking_emitters_follow_the_packet_target() {
    use crate::game::{
        Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
        totem::TOTEM_EVENT,
    };
    use horizon_client::{motion::PlayerMotion, particles::ParticleType, pose::Pose};
    use horizon_core::geometry::Vec3;
    use horizon_protocol::{
        entity_data::{Value, serializer},
        packets::play::{self, Clientbound as C},
    };

    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    let (client, _server) = horizon_protocol::connection::Pipe::pair();
    game.connect_local(client, "EmitterTest").unwrap();
    game.remote.as_mut().unwrap().entity_id = 7;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap());
    game.particles.seed_randoms(42, 43, 44);

    let check = |game: &Game<'_>, ty, feet: [f64; 3], height: f32| {
        let mut seen = false;
        for (options, at, velocity) in game.particles.requested.as_ref().unwrap() {
            if options.ty != ty {
                continue;
            }
            seen = true;
            let y = feet[1] + f64::from(height) * (0.5 + (velocity[1] - 0.2) / 4.0);
            assert!((at[0] - feet[0]).abs() <= f64::from(0.6_f32) / 4.0);
            assert!(
                (at[1] - y).abs() < 1e-10,
                "{ty:?} at {at:?}, expected y {y}"
            );
            assert!((at[2] - feet[2]).abs() <= f64::from(0.6_f32) / 4.0);
        }
        assert!(seen, "no {ty:?} emissions");
    };
    let types = [
        ParticleType::Crit,
        ParticleType::EnchantedHit,
        ParticleType::TotemOfUndying,
    ];
    let target = 40;
    let start = [5.0, 64.0, 7.0];
    game.spawn_entity(target, "minecraft:player", start, [0.0; 3]);
    game.particles.requested = Some(Vec::new());
    for action in [1, 2] {
        game.handle_packet(C::Animate(play::Animate { id: target, action }))
            .unwrap();
    }
    game.handle_packet(C::EntityEvent(play::EntityEvent {
        id: target,
        event: TOTEM_EVENT,
    }))
    .unwrap();
    for ty in types {
        check(&game, ty, start, 1.8);
    }

    let moved = [15.0, 65.0, -2.0];
    game.particles.requested = Some(Vec::new());
    game.handle_packet(C::SetEntityData(play::SetEntityData {
        id: target,
        values: vec![(6, serializer::POSE, Value::Id(Pose::Crouching.id()))],
    }))
    .unwrap();
    game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
        id: target,
        path: play::PositionPath::Linear(moved),
        yaw: 0.0,
        pitch: 0.0,
        on_ground: true,
    }))
    .unwrap();
    for _ in 0..4 {
        game.tick_remote_entities(true);
    }
    game.tick_particles();
    for ty in types {
        check(&game, ty, moved, 1.5);
    }

    // The old entity moved again before unpairing. Re-pairing its id before
    // the next particle tick keeps emitting at the old entity's final body.
    game.particles.requested = Some(Vec::new());
    let last = [18.0, 65.0, -2.0];
    game.mob_index
        .get_mut(&mut game.mobs, target as u32)
        .unwrap()
        .position = Vec3::new(last[0], last[1], last[2]);
    game.handle_packet(C::RemoveEntities(play::RemoveEntities {
        ids: vec![target],
    }))
    .unwrap();
    game.spawn_entity(target, "minecraft:player", [28.0, 67.0, 1.0], [0.0; 3]);
    game.tick_particles();
    for ty in types {
        check(&game, ty, last, 1.5);
    }

    game.particles.clear();
    game.particles.requested = Some(Vec::new());
    game.handle_packet(C::EntityEvent(play::EntityEvent {
        id: 7,
        event: TOTEM_EVENT,
    }))
    .unwrap();
    check(&game, ParticleType::TotemOfUndying, [0.0, 64.0, 0.0], 1.8);
    game.particles.requested = Some(Vec::new());
    let player = game.player.as_mut().unwrap();
    player.position = Vec3::new(-4.0, 68.0, 8.0);
    player.set_pose(Pose::Swimming).unwrap();
    game.tick_particles();
    check(&game, ParticleType::TotemOfUndying, [-4.0, 68.0, 8.0], 0.6);
}

/// The vanilla 26.3 client's ServerboundContainerClickPacket for a shift-click from hotbar slot 37
/// into slot 10 listed 37 first (wire stream harness, `inventory`). The key 0 always leads.
#[test]
fn changed_slots_go_out_in_the_order_vanilla_s_hash_map_iterates() {
    assert_eq!(open_hash_order(&[10, 37]), [37, 10]);
    assert_eq!(open_hash_order(&[0, 1, 2, 36, 44])[0], 0);
    // Past 24 keys the table grows; every key is still listed once.
    let all: Vec<i16> = (0..46).collect();
    let mut out = open_hash_order(&all);
    out.sort_unstable();
    assert_eq!(out, all);
}

#[test]
fn typed_chat_is_sent_as_the_client_chat_event_leaves_it() {
    assert_eq!(chat_after_event("hello", |_, _| {}), Some("hello".into()));
    assert_eq!(
        chat_after_event("hello", |m, _| m.push_str(" world")),
        Some("hello world".into())
    );
    assert_eq!(chat_after_event("hello", |_, c| *c = true), None);
    assert_eq!(chat_after_event("hello", |m, _| m.clear()), None);
}

#[test]
#[ignore = "daily"]
fn block_alias_survival_consumption_and_client_prediction() {
    use crate::game::{Audio, Game, Options, Progress, Settings, World, load_resources};
    use horizon_client::{
        chunk::Chunk,
        items::ItemStack,
        mod_content::{ContentMod, build},
    };
    use horizon_mod_package::content::{Block, Content, Item};
    use std::{path::PathBuf, sync::Arc};
    let base = PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
    );
    let root = std::env::temp_dir().join(format!("horizon-alias-game-{}", std::process::id()));
    let mut content = Content::declared("ruby");
    content
        .declare(
            &[Block {
                id: "ruby:frame".into(),
                template: "minecraft:stone".into(),
                item: false,
                ..Default::default()
            }],
            &["empty", "filled"].map(|name| Item {
                id: format!("ruby:{name}"),
                template: "minecraft:stick".into(),
                block: Some("ruby:frame".into()),
                ..Default::default()
            }),
        )
        .unwrap();
    let mods = [ContentMod {
        id: "ruby".into(),
        content,
        jars: Vec::new(),
    }];
    let inventory =
        horizon_mod_package::enabled::EnabledMods::new([("ruby".to_owned(), mod_api::Side::Both)]);
    build(&base, std::path::Path::new(""), &mods, &root, &inventory).unwrap();
    let (registry, _) = horizon_world::level::data::load_registry_with_flags(&root).unwrap();
    crate::game::api_values::register_game_keys((0..registry.len()).map(|id| {
        registry
            .state(horizon_world::registry::BlockStateId(id as u32))
            .unwrap()
            .name
            .clone()
    }));
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.world = World::new(game.res.facts().air, 1);
    for x in -1..=1 {
        for z in -1..=1 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
    }
    game.sync_tick_chunks();
    game.set_mode_id(0);
    game.player = None;
    game.selected = 0;
    let target = game
        .pack
        .assets
        .registry
        .default_state("ruby:frame")
        .unwrap();
    for (i, name) in ["ruby:empty", "ruby:filled"].iter().enumerate() {
        let pos = [i as i32 * 2, 70, 0];
        let hit = crate::game::Hit {
            pos,
            face: [0, 1, 0],
            inside: false,
            point: [f64::from(pos[0]) + 0.5, 70.0, 0.5],
        };
        let stack = ItemStack::new(name, 2);
        assert_eq!(game.items().get(name).unwrap().block, Some(target));
        let preview = game
            .remote_can_place(&hit, &stack)
            .expect("client predicts the alias");
        assert_eq!([preview.0.x, preview.0.y, preview.0.z], pos);
        assert_eq!(
            game.world.block(pos[0], pos[1], pos[2]),
            game.res.facts().air,
            "prediction is dry"
        );
        *game.held_mut() = Some(stack);
        assert!(game.use_hand(Some(&hit)).unwrap());
        assert_eq!(game.world.block(pos[0], pos[1], pos[2]), target);
        let held = game.held().unwrap();
        assert_eq!((&*held.item, held.count), (*name, 1));
    }
    let blocked = crate::game::Hit {
        pos: [0, 70, 0],
        face: [0, 1, 0],
        inside: false,
        point: [0.5, 71.0, 0.5],
    };
    game.set_mode_id(2);
    let before = game.held().cloned();
    game.use_hand(Some(&blocked)).unwrap();
    assert_eq!(
        game.held(),
        before.as_ref(),
        "adventure refusal keeps the alias stack"
    );
    drop(game);
    std::fs::remove_dir_all(root).unwrap();
}

use horizon_client::{
    containers::{Beacon, BlockEntity},
    items::ItemStack,
};
use horizon_core::nbt::Tag;

/// #12040: a server's client moves at the speed the server's attributes give it, never its
/// effects' own. Vanilla's server removes a Speed that runs out in ServerPlayer.doTick, after
/// the tick's sendDirtyEntityData: ClientboundRemoveMobEffectPacket goes out at once and the
/// movement speed without the effect's modifier a tick later, so for that tick the client still
/// runs fast (a potion drunk starts the same way). A client a tick off is what GrimAC's
/// Simulation flagged.
#[test]
#[ignore = "daily"]
fn a_servers_client_takes_its_speed_from_the_attributes_not_the_effect() {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::motion::PlayerMotion;
    use horizon_core::geometry::Vec3;
    use horizon_protocol::packets::play::{
        AttributeModifier, AttributeSnapshot, Clientbound as C, RemoveMobEffect, UpdateAttributes,
        UpdateMobEffect,
    };
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.init_remote_for_replay().unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    let me = game.remote.as_ref().unwrap().entity_id;
    let speed = game.remote_effect_network_id("speed").unwrap();
    let attribute = {
        let p = game.player.as_ref().unwrap();
        let key = p.attributes.key("minecraft:movement_speed").unwrap();
        key.attribute().registry_id()
    };
    // Player.createAttributes: 0.1F.
    let walking = f64::from(0.1f32);
    // Speed II: 0.2 (a float) a level, multiplying the total.
    let amount = f64::from(0.2f32) * 2.0;
    let attributes = |sped: bool| {
        C::UpdateAttributes(UpdateAttributes {
            id: me,
            values: vec![AttributeSnapshot {
                attribute,
                base: walking,
                modifiers: sped
                    .then(|| AttributeModifier {
                        id: "minecraft:effect.speed".into(),
                        amount,
                        operation: 2,
                    })
                    .into_iter()
                    .collect(),
            }],
        })
    };
    // Each step: the packets of one server tick, then a client tick.
    let mut step = |packets: Vec<C>| {
        for packet in packets {
            game.handle_packet(packet).unwrap();
        }
        game.tick_client_effects();
        game.player
            .as_mut()
            .unwrap()
            .attribute("minecraft:movement_speed")
            .unwrap()
    };
    let effect = C::UpdateMobEffect(UpdateMobEffect {
        id: me,
        effect: speed,
        amplifier: 1,
        duration: 60,
        flags: 2 | 4,
    });
    let fast = walking * (1.0 + amount);
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let seen = [
        step(vec![effect]),
        step(vec![attributes(true)]),
        step(Vec::new()),
        step(vec![C::RemoveMobEffect(RemoveMobEffect {
            id: me,
            effect: speed,
        })]),
        step(vec![attributes(false)]),
    ];
    let expected = [walking, fast, fast, fast, walking];
    assert!(
        seen.iter().zip(expected).all(|(&s, e)| close(s, e)),
        "speed on each tick: {seen:?}, vanilla's client: {expected:?}"
    );
}

/// `ItemFrame.interact`, `AbstractBoat.interact`, and `Minecart.interact`:
/// interactions that pass leave the click to the held item without swinging;
/// vehicle boarding answers CONSUME and ends the click without a predicted swing (#9120).
#[test]
fn object_interactions_pass_and_swing_vanilla() {
    // ItemFrame: fixed frame or empty frame + empty hand passes.
    assert!(item_frame_interact_passes(true, true, true));
    assert!(item_frame_interact_passes(true, false, false));
    assert!(item_frame_interact_passes(false, false, false));
    // ItemFrame: inserting or rotating item does not pass.
    assert!(!item_frame_interact_passes(false, true, false)); // rotates
    assert!(!item_frame_interact_passes(false, false, true)); // inserts
    assert!(!item_frame_interact_passes(false, true, true)); // rotates

    // Boat: sneaking or out of control passes.
    assert!(boat_interact_passes(false, true, false, true)); // sneaking
    assert!(boat_interact_passes(false, false, true, true)); // out of control
    // Boat: the client answers SUCCESS, room or not (startRiding is the server's).
    assert!(!boat_interact_passes(false, false, false, true));
    assert!(!boat_interact_passes(false, false, false, false));

    // Chest boat: sneaking opens the chest; boarding answers SUCCESS.
    assert!(!boat_interact_passes(true, true, false, true)); // opens chest
    assert!(!boat_interact_passes(true, false, false, false)); // boards (client)
    // Chest boat out of control: passes with room, else opens the chest.
    assert!(boat_interact_passes(true, false, true, true));
    assert!(!boat_interact_passes(true, false, true, false));

    // Swings: boarding a rideable cart answers CONSUME (no swing).
    assert!(!remote_object_swings(None, true));
    // Swings: other object interactions, a boat's boarding too, answer SUCCESS.
    assert!(remote_object_swings(None, false));
    // Interaction entities honor response.
    assert!(remote_object_swings(Some(true), false));
    assert!(!remote_object_swings(Some(false), false));
}

/// A mod's pre-filled chat opens only over the world, with its text
/// typed in and nothing sent.
#[test]
#[ignore = "daily"]
fn mod_open_chat_fills_the_box_only_over_the_world() {
    use super::super::{
        Audio, Game, Options, Progress, Screen, Settings, execution_role::ExecutionRole,
        load_resources,
    };

    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!("no menu handoff"),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();

    // No player: the title screen.
    game.player = None;
    assert!(game.open_chat("/tp ").is_err());
    assert!(game.ui.chat_screen.is_none());
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, horizon_core::geometry::Vec3::ZERO)
            .unwrap(),
    );
    game.screen = Screen::Paused;
    assert!(game.open_chat("/tp ").is_err());
    assert!(game.ui.chat_screen.is_none());
    game.screen = Screen::Playing;
    assert!(game.open_chat("/tp ").is_ok());
    assert!(game.screen == Screen::Chat);
    assert_eq!(game.ui.chat_screen.as_ref().unwrap().input.value, "/tp ");
    // The chat is a screen too: a second call doesn't replace what's typed.
    assert!(game.open_chat("hello").is_err());
    assert_eq!(game.ui.chat_screen.as_ref().unwrap().input.value, "/tp ");
}

/// The packet riding graph must retain invisible intermediary entities. A custom
/// sit plugin can use an area-effect cloud, whose server position stops updating
/// while it rides a player, with a player or marker stand as its own passenger.
#[test]
#[ignore = "daily"]
fn plugin_player_stack_follows_network_moves_and_local_carriers() {
    use super::super::{
        Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
    };
    use horizon_client::{motion::PlayerMotion, sprint::Keys};
    use horizon_core::geometry::Vec3;
    use horizon_protocol::packets::play::{self, Clientbound as C};
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    let (client, _server) = horizon_protocol::connection::Pipe::pair();
    game.connect_local(client, "SeatTest").unwrap();
    game.remote.as_mut().unwrap().entity_id = 7;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
    game.spawn_entity(40, "minecraft:player", [2.0, 64.0, 3.0], [0.0; 3]);
    game.spawn_entity(41, "minecraft:player", [0.0, 64.0, 0.0], [0.0; 3]);
    game.spawn_entity(42, "minecraft:area_effect_cloud", [0.0; 3], [0.0; 3]);
    let set = |g: &mut Game<'_>, vehicle, passengers: Vec<i32>| {
        g.handle_packet(C::SetPassengers(play::SetPassengers {
            vehicle,
            passengers,
        }))
        .unwrap();
    };
    set(&mut game, 40, vec![42]);
    set(&mut game, 42, vec![7, 41]);
    let close = |at: Vec3, x: f64, y: f64, z: f64| {
        assert!(
            (at.x - x).abs() < 1e-5 && (at.y - y).abs() < 1e-5 && (at.z - z).abs() < 1e-5,
            "position {at:?}, expected [{x}, {y}, {z}]"
        );
    };
    assert!(game.remote_ride(Keys::default(), 0).unwrap());
    close(game.player.as_ref().unwrap().position, 2.0, 65.7, 3.0);
    close(
        game.mobs.iter().find(|m| m.id == 41).unwrap().position,
        2.0,
        65.7,
        3.0,
    );
    game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
        id: 40,
        path: play::PositionPath::Linear([12.0, 67.0, -4.0]),
        yaw: 45.0,
        pitch: 0.0,
        on_ground: true,
    }))
    .unwrap();
    for _ in 0..4 {
        game.tick_remote_entities(true);
        game.remote_ride(Keys::default(), 0).unwrap();
    }
    close(game.player.as_ref().unwrap().position, 12.0, 68.7, -4.0);
    close(
        game.mobs.iter().find(|m| m.id == 41).unwrap().position,
        12.0,
        68.7,
        -4.0,
    );
    // The intermediary's packet position must remain unchanged for relative decoding.
    assert_eq!(
        game.remote.as_ref().unwrap().tracked(42).unwrap().1,
        [0.0; 3]
    );
    // A remote player riding the local player must follow our own movement too.
    set(&mut game, 42, vec![41]);
    set(&mut game, 7, vec![42]);
    game.player.as_mut().unwrap().position = Vec3::new(20.0, 70.0, 25.0);
    game.position_riders();
    close(
        game.mobs.iter().find(|m| m.id == 41).unwrap().position,
        20.0,
        71.7,
        25.0,
    );
    // A direct stack has no intermediary's 0.5-height offset.
    set(&mut game, 40, vec![7, 41]);
    game.remote_ride(Keys::default(), 0).unwrap();
    close(game.player.as_ref().unwrap().position, 12.0, 68.2, -4.0);
    close(
        game.mobs.iter().find(|m| m.id == 41).unwrap().position,
        12.0,
        68.2,
        -4.0,
    );
    // Removing the carrier detaches passengers even without a final empty packet.
    game.remove_entity(40);
    assert_eq!(game.riding, None);
    assert_eq!(
        game.mobs.iter().find(|m| m.id == 41).unwrap().extra.mount,
        None
    );
}

/// `Entity.isColliding` and the snap `ClientLevel.syncBlockState` makes: a
/// block the server puts back where the player stands sends it to the
/// position the prediction remembered, and a block that comes back clear of
/// the player, an empty one, or one the player has been teleported since
/// leaves it where it is.
#[test]
fn a_block_the_server_puts_back_snaps_the_player_out_of_it() {
    use horizon_core::geometry::{Aabb, Vec3, VoxelShape};
    // A player standing in block (0, 64, 0): 0.6 wide, 1.8 tall.
    let player = Aabb::new(Vec3::new(0.2, 64.0, 0.2), Vec3::new(0.8, 65.8, 0.8));
    let cube = VoxelShape::block();
    assert!(
        shape_meets(&cube, [0, 64, 0], player),
        "the block the player is standing in"
    );
    assert!(!shape_meets(&cube, [0, 66, 0], player), "above its head");
    assert!(
        !shape_meets(&cube, [1, 64, 0], player),
        "the block beside it"
    );
    assert!(!shape_meets(&VoxelShape::empty(), [0, 64, 0], player));

    // The snap itself: the remembered position, unless a teleport came after
    // the prediction (`lastTeleportSequence < sequence`).
    let at = [0.5, 64.0, 0.5];
    assert_eq!(snap_target(at, -1, 3, true), Some(at));
    assert_eq!(snap_target(at, -1, 3, false), None, "a block it clears");
    assert_eq!(snap_target(at, 3, 3, true), None, "teleported at 3");
    assert_eq!(
        snap_target(at, 2, 3, true),
        Some(at),
        "teleported before it"
    );
}

/// `MultiPlayerGameMode.useItem`'s `shouldSwing`: the SUCCESS answers
/// swing, CONSUME's do not (#6425).
#[test]
fn a_thrown_item_swinging_on_a_server_and_a_charge_not() {
    let stack = |id: &str| ItemStack::new(id, 1);
    // SUCCESS: the throwables, a bucket, a boat, a bundle, a book, a map.
    for id in [
        "minecraft:ender_pearl",
        "minecraft:splash_potion",
        "minecraft:wind_charge",
        "minecraft:snowball",
        "minecraft:experience_bottle",
        "minecraft:bucket",
        "minecraft:oak_boat",
        "minecraft:bundle",
        "minecraft:writable_book",
        "minecraft:map",
    ] {
        assert!(remote_use_swings(&stack(id), false), "{id} swings");
    }
    // CONSUME: the charges and startUsingInstantly's uses, a drinkable
    // potion among them (`consumable` set).
    for id in [
        "minecraft:bow",
        "minecraft:crossbow",
        "minecraft:trident",
        "minecraft:spyglass",
        "minecraft:goat_horn",
        "minecraft:ender_eye",
    ] {
        assert!(!remote_use_swings(&stack(id), false), "{id} does not swing");
    }
    // The kinetic weapons charge like a trident.
    assert!(!remote_use_swings(&stack("minecraft:iron_spear"), false));
    // A consumable (food, a drink) starts its use without a swing.
    assert!(!remote_use_swings(&stack("minecraft:apple"), true));
}

/// Minecraft.tick: CLIENT_TICK_END only while a level exists. None before
/// the first Login (configuration has finished, the server has not placed
/// the player yet), and none from StartConfiguration until the next Login.
#[test]
fn tick_ends_need_the_level_the_login_makes() {
    assert!(!client_has_level(false, None), "joined, waiting for Login");
    assert!(client_has_level(true, None), "after Login");
    assert!(!client_has_level(true, Some(40)), "reconfiguring");
}

/// #12148: a proxy moves the player to its next server. The last server's
/// columns stay until the next Login, but Reconfigured already put in the
/// next server's biome table, shorter when only the last server had a
/// biome the game lacks: the columns hold an id past it ("the len is 67
/// but the index is 67"). Vanilla's clearClientLevel leaves no player, so
/// getSituationalMusic is MENU, and ServerReconfigScreen (no
/// LevelLoadingScreen) lets MusicManager.tick start it.
#[test]
#[ignore = "daily"]
fn a_reconfiguring_client_plays_menu_music_and_never_indexes_past_the_biomes() {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::{chunk::Chunk, music::Music};
    use horizon_core::geometry::Vec3;
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.init_remote_for_replay().unwrap();
    // The last server's column at the origin: its extra biome came after
    // the built-in ones, one past the table the next server leaves.
    let stale = world_biomes!(game).list.len() as u16;
    let mut column = Chunk::empty(0, 0);
    // Quart 16 above the column's bottom at -64: block y 0.
    column.set_biome_cell(0, 16, 0, stale);
    game.world
        .chunks
        .insert((0, 0), std::sync::Arc::new(column));
    assert_eq!(game.biome_at([0.0; 3]), "minecraft:plains");
    assert_eq!(
        game.biome_name_at(Vec3::new(0.5, 0.5, 0.5)).as_deref(),
        Some("minecraft:plains")
    );

    // StartConfiguration after a Login: no player, ServerReconfigScreen.
    let ticks = game.ticks;
    let r = game.remote.as_mut().unwrap();
    r.logged_in = true;
    r.reconfiguring = Some(ticks);
    game.player = None;
    assert_eq!(game.situational_music().0, Some(Music::menu()));
    // A new manager waits its first 100 ticks, then starts the menu track.
    let menu = (0..=100).any(|_| {
        game.tick_music();
        game.music.current(&game.audio.engine) == Some("minecraft:music.menu")
    });
    assert!(menu, "the menu music starts under ServerReconfigScreen");

    // With a level again the camera probe reads that column: plains' music.
    game.remote.as_mut().unwrap().reconfiguring = None;
    let (music, _) = game.situational_music();
    assert!(music.is_some_and(|m| m != Music::menu()));
}

/// #16040: a standalone replay has no configuration phase, so its registries come from the
/// local resources. Once the recording's Login enters the dimension the Overworld's clock is
/// resolved, and a recorded `SetTime` sets the time of day.
#[test]
#[ignore = "daily"]
fn a_standalone_replay_resolves_the_recorded_world_clock() {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_protocol::packets::play::{self, Clientbound as C};
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.init_remote_for_replay().unwrap();
    let spawn = play::SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 1,
        game_type: 0,
        previous_game_type: None,
        debug: false,
        flat: false,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    };
    game.enter_remote_dimension(&spawn, false, horizon_client::level_loading::Reason::Other);
    let overworld = game.remote.as_ref().unwrap().default_clock;
    assert!(
        overworld.is_some(),
        "the Overworld's default clock resolves"
    );
    // The afternoon of the issue's recording: day time 9155.
    game.handle_packet(C::SetTime(play::SetTime {
        game_time: 9155,
        clocks: vec![play::ClockState {
            clock: overworld.unwrap(),
            total_ticks: 9155,
            partial_tick: 0.0,
            rate: 1.0,
        }],
    }))
    .unwrap();
    assert_eq!(game.world.day_time, 9155);
}

/// The server's on-fire flag keeps the predicted countdown (which
/// `fluid_base_tick` runs down) from ending the fire; a clear flag leaves
/// the window's own contact fire alone.
#[test]
fn the_servers_fire_flag_holds_the_burning_state_through_the_countdown() {
    let mut ticks = 1;
    for _ in 0..5 {
        ticks = hold_server_fire(ticks - 1, true);
        assert!(ticks > 0, "still burning");
    }
    assert_eq!(hold_server_fire(300, true), 300, "a longer fire is not cut");
    assert_eq!(hold_server_fire(7, false), 7, "predicted lava fire stays");
    assert_eq!(hold_server_fire(0, false), 0);
}

#[test]
fn a_beacon_update_tag_keeps_the_beam_the_client_walked() {
    let old = BlockEntity::Beacon(Box::new(Beacon {
        levels: 1,
        primary: Some("minecraft:speed".into()),
        sections: vec![(0xF9FFFE, 2)],
        checking: vec![(0xF9FFFE, 1)],
        last_check_y: 70,
        ..Default::default()
    }));
    // BeaconBlockEntity.getUpdateTag: saveCustomOnly.
    let tag = Tag::compound([
        ("id", Tag::String("minecraft:beacon".into())),
        ("x", Tag::Int(0)),
        ("y", Tag::Int(64)),
        ("z", Tag::Int(0)),
        ("primary_effect", Tag::String("minecraft:haste".into())),
        (
            "secondary_effect",
            Tag::String("minecraft:regeneration".into()),
        ),
        ("Levels", Tag::Int(4)),
    ]);
    let (_, mut new) = horizon_client::save::decode_block_entity(&tag).unwrap();
    keep_client_state(Some(&old), &mut new);
    let BlockEntity::Beacon(b) = new else {
        panic!("not a beacon");
    };
    assert_eq!(b.sections, vec![(0xF9FFFE, 2)]);
    assert_eq!(b.checking, vec![(0xF9FFFE, 1)]);
    assert_eq!(b.last_check_y, 70);
    // The powers are the tag's; the levels stay the client's own (not read).
    assert_eq!(b.primary.as_deref(), Some("minecraft:haste"));
    assert_eq!(b.secondary.as_deref(), Some("minecraft:regeneration"));
    assert_eq!(b.levels, 1);
}

#[test]
fn the_server_stopping_the_use_lowers_the_shield() {
    // Player.disableShield's stopUsingItem: flags 0 or 2 while the
    // off-hand shield is raised (LocalPlayer.onSyncedDataUpdated →
    // stopUsingItem). stopUsingItem clears bit 0 and leaves bit 1, so a
    // main-hand stop is 0 and an off-hand stop is 2.
    let off = super::super::shield::OFFHAND_SLOT;
    assert_eq!(synced_local_use(Some((off, 12)), 0, 3, |_| true), None);
    assert_eq!(synced_local_use(Some((3, 5)), 0, 3, |_| true), None);
    assert_eq!(synced_local_use(Some((off, 12)), 2, 3, |_| true), None);
}

#[test]
fn the_server_starting_a_use_starts_it_in_that_hand() {
    let off = super::super::shield::OFFHAND_SLOT;
    assert_eq!(synced_local_use(None, 1, 4, |_| true), Some((4, 0)));
    assert_eq!(synced_local_use(None, 3, 4, |_| true), Some((off, 0)));
    // LivingEntity.startUsingItem needs an item in the hand.
    assert_eq!(synced_local_use(None, 1, 4, |_| false), None);
}

/// tooBigToInterpolate: a position sync measures from where the entity
/// is, a teleport from where its interpolation is headed.
#[test]
fn a_far_move_snaps_by_the_packets_own_measure() {
    use super::{MovePacket, Path, PosRot, SteppedInterpolation};
    let at = |x: f64| PosRot {
        position: [x, 0.0, 0.0],
        yaw: 0.0,
        pitch: 0.0,
    };
    let mut handler = SteppedInterpolation::new(3);
    // At x 0, headed for x 50.
    handler.interpolate_to(at(0.0), Some(Path::Linear([50.0, 0.0, 0.0])), None);
    let to = [100.0, 0.0, 0.0];
    assert!(MovePacket::PositionSync.snaps(&handler, at(0.0), to));
    assert!(!MovePacket::Teleport.snaps(&handler, at(0.0), to));
    assert!(!MovePacket::Move.snaps(&handler, at(0.0), to));
    // With nothing in flight, both measure from the entity.
    let idle = SteppedInterpolation::new(3);
    assert!(MovePacket::Teleport.snaps(&idle, at(0.0), to));
    assert!(!MovePacket::Teleport.snaps(&idle, at(0.0), [60.0, 0.0, 0.0]));
}

/// A far sync puts the entity there and drops the steps it was walking;
/// a rotation alone keeps the position it is headed for and turns there
/// over the update interval.
#[test]
fn a_living_entitys_move_snaps_or_steps() {
    use super::{MovePacket, Path, PosRot, SteppedInterpolation, stepped_move};
    let at = |x: f64, yaw: f32| PosRot {
        position: [x, 0.0, 0.0],
        yaw,
        pitch: 0.0,
    };
    let mut handler = SteppedInterpolation::new(3);
    // Walking to x 5 when the far sync comes.
    handler.interpolate_to(at(0.0, 0.0), Some(Path::Linear([5.0, 0.0, 0.0])), None);
    assert!(handler.active());
    let far = [100.0, 0.0, 0.0];
    let snap = stepped_move(
        &mut handler,
        MovePacket::PositionSync,
        at(0.0, 0.0),
        far,
        Some(Path::Linear(far)),
        Some((90.0, 0.0)),
    );
    assert_eq!(snap, Some(at(100.0, 90.0)));
    assert!(!handler.active());
    assert_eq!(handler.interpolate(), None, "no step back toward x 5");
    // A rotation-only move packet.
    let turn = stepped_move(
        &mut handler,
        MovePacket::Move,
        at(100.0, 90.0),
        far,
        None,
        Some((180.0, 0.0)),
    );
    assert_eq!(turn, None);
    let steps: Vec<_> = std::iter::from_fn(|| handler.interpolate()).collect();
    assert_eq!(
        steps.iter().map(|s| s.yaw).collect::<Vec<_>>(),
        [120.0, 150.0, 180.0]
    );
    assert!(steps.iter().all(|s| s.position == far));
}

#[test]
fn matching_flags_leave_a_running_use_alone() {
    assert_eq!(synced_local_use(Some((2, 7)), 1, 2, |_| true), Some((2, 7)));
    assert_eq!(synced_local_use(None, 0, 2, |_| true), None);
}

/// `ChatType.Bound.decorate` and `decorateNarration`: a private message
/// shows as "Steve whispers to you: hi" and is narrated "Steve says hi";
/// a chat type the registry lacks is `minecraft:chat`'s.
#[test]
fn chat_is_narrated_with_its_chat_types_narration_decoration() {
    use super::{Decoration, chat_type_decorate, play};
    use horizon_core::nbt::{JavaString, Tag, parse_snbt};
    use serde_json::json;
    let bound = play::ChatTypeBound {
        chat_type: 0,
        name: Tag::String(JavaString::from("Steve")),
        target: None,
    };
    let content = json!("hi");
    let (incoming, _) = parse_snbt(
        r#"{chat:{translation_key:"commands.message.display.incoming",parameters:["sender","content"],style:{color:"gray",italic:1b}},narration:{translation_key:"chat.type.text.narrate",parameters:["sender","content"]}}"#,
    )
    .unwrap();
    let chat = chat_type_decorate(Some(&incoming), Decoration::Chat, &bound, &content);
    assert_eq!(chat["translate"], "commands.message.display.incoming");
    assert_eq!(chat["color"], "gray");
    assert_eq!(chat["with"], json!(["Steve", "hi"]));
    assert_eq!(
        chat_type_decorate(Some(&incoming), Decoration::Narration, &bound, &content),
        json!({"translate": "chat.type.text.narrate", "with": ["Steve", "hi"]})
    );
    assert_eq!(
        chat_type_decorate(None, Decoration::Narration, &bound, &content),
        json!({"translate": "chat.type.text.narrate", "with": ["Steve", "hi"]})
    );
    assert_eq!(
        chat_type_decorate(None, Decoration::Chat, &bound, &content)["translate"],
        "chat.type.text"
    );
}

/// A cauldron's fill level is a use this client predicts, and a guest's
/// host forwards its sound: every cauldron state counts, not only the
/// doors and levers.
#[test]
fn every_cauldron_counts_as_hand_operated() {
    for name in [
        "minecraft:cauldron",
        "minecraft:water_cauldron",
        "minecraft:lava_cauldron",
        "minecraft:powder_snow_cauldron",
    ] {
        assert!(super::hand_operated(name), "{name}");
    }
    assert!(!super::hand_operated("minecraft:water"));
}

/// An NPC plugin lists the NPC, spawns it, and takes the entry off the list:
/// the entity keeps the skin it found; a player spawned before its entry
/// arrives takes the entry once it comes.
#[test]
fn a_player_entity_keeps_the_skin_its_entry_gave() {
    use horizon_client::skins::Textures;
    use std::collections::HashMap;
    let npc = Textures {
        skin: Some("http://textures.minecraft.net/texture/npc".into()),
        signed: true,
        ..Textures::default()
    };
    let (mut listed, mut kept) = (HashMap::from([(7u128, npc.clone())]), HashMap::new());
    let mut players = HashMap::from([(2i32, 7u128), (3, 8)]);
    assert_eq!(
        super::skin_entries(&players, &listed, &mut kept),
        [(7, npc.clone())]
    );
    listed.remove(&7);
    assert_eq!(
        super::skin_entries(&players, &listed, &mut kept),
        [(7, npc.clone())]
    );
    // Player 8's entry comes after its entity.
    let late = Textures {
        skin: Some("http://textures.minecraft.net/texture/late".into()),
        ..npc.clone()
    };
    listed.insert(8, late.clone());
    let mut entries = super::skin_entries(&players, &listed, &mut kept);
    entries.sort_by_key(|(u, _)| *u);
    assert_eq!(entries, [(7, npc), (8, late)]);
    // A removed entity forgets its profile with it.
    players.remove(&2);
    kept.remove(&2);
    listed.clear();
    assert_eq!(super::skin_entries(&players, &listed, &mut kept).len(), 1);
}

#[test]
fn tab_publication_fingerprint_tracks_all_inputs() {
    let fingerprint = |ids: &[u128], skins, atlas| {
        (
            ids.len(),
            ids.iter().fold(0, |xor, uuid| xor ^ uuid),
            skins,
            atlas,
        )
    };
    let published = fingerprint(&[7, 8], 1, 2);
    assert_eq!(published, fingerprint(&[8, 7], 1, 2));
    assert_ne!(published, fingerprint(&[7, 9], 1, 2));
    assert_ne!(published, fingerprint(&[7, 8], 2, 2));
    assert_ne!(published, fingerprint(&[7, 8], 1, 3));
}

#[test]
fn tab_object_skin_publication_tracks_changes_and_atlas_clear() {
    let mut published = std::collections::HashMap::new();
    let mut writes = 0;
    for skin in ["skins/first", "skins/first", "skins/second"] {
        for key in ["id:7", "name:player"] {
            if super::object_skin_changed(&published, key, skin) {
                published.insert(key.to_owned(), skin.to_owned());
                writes += 1;
            }
        }
    }
    assert_eq!(writes, 4, "unchanged skins do not publish either entry");
    published.clear();
    assert!(super::object_skin_changed(
        &published,
        "id:7",
        "skins/second"
    ));
    assert!(super::object_skin_changed(
        &published,
        "name:player",
        "skins/second"
    ));
}

/// An NPC plugin's scale: a base the plugin sets, or a modifier on 1.
#[test]
fn attribute_values_add_then_multiply_as_vanilla() {
    use horizon_protocol::packets::play::{AttributeModifier, AttributeSnapshot};
    let modifier = |amount, operation| AttributeModifier {
        id: "minecraft:proof".into(),
        amount,
        operation,
    };
    let snapshot = |base, modifiers| AttributeSnapshot {
        attribute: 0,
        base,
        modifiers,
    };
    assert_eq!(super::snapshot_value(&snapshot(2.0, vec![])), 2.0);
    assert_eq!(
        super::snapshot_value(&snapshot(1.0, vec![modifier(1.0, 0)])),
        2.0
    );
    // (1 + 1) * (1 + 0.5) from the base, then * (1 + 1) of the total.
    let all = vec![modifier(1.0, 0), modifier(0.5, 1), modifier(1.0, 2)];
    assert_eq!(super::snapshot_value(&snapshot(1.0, all)), 6.0);
}

use super::Game;

/// `Entity.lookAt`: yaw 0 faces +Z, -90 faces +X; pitch -90 looks up.
#[test]
fn look_at_turns_towards_the_target() {
    let close = |(yaw, pitch): (f32, f32), want: (f32, f32)| {
        assert!(
            (yaw - want.0).abs() < 1e-3 && (pitch - want.1).abs() < 1e-3,
            "{yaw} {pitch}, want {want:?}"
        );
    };
    let at = [10.0, 64.0, -5.0];
    let off = |d: [f64; 3]| [at[0] + d[0], at[1] + d[1], at[2] + d[2]];
    close(
        super::look_at_rotation(at, off([0.0, 0.0, 3.0])),
        (0.0, 0.0),
    );
    close(
        super::look_at_rotation(at, off([3.0, 0.0, 0.0])),
        (-90.0, 0.0),
    );
    close(
        super::look_at_rotation(at, off([-3.0, 0.0, 0.0])),
        (90.0, 0.0),
    );
    close(
        super::look_at_rotation(at, off([0.0, 0.0, -3.0])),
        (-180.0, 0.0),
    );
    close(
        super::look_at_rotation(at, off([0.0, 2.0, 2.0])),
        (0.0, -45.0),
    );
    close(
        super::look_at_rotation(at, off([0.0, -4.0, 4.0])),
        (0.0, 45.0),
    );
}

/// `handleMovePlayer` moves the player's pitch and the model's old pitch
/// (`xRotO`) alike: a relative change keeps the ease a tick began, each
/// end clamped on its own, and an absolute one snaps both (arismoko/horizon#3844).
#[test]
fn a_position_packet_moves_both_pitch_ends() {
    use super::position_packet_pitch as pitch;
    // The tick is laying the head down from 30 to 0; the packet turns 10.
    assert_eq!(
        (pitch(30.0, 10.0, true), pitch(0.0, 10.0, true)),
        (40.0, 10.0)
    );
    // Each end clamps on its own.
    assert_eq!(
        (pitch(80.0, 20.0, true), pitch(60.0, 20.0, true)),
        (90.0, 80.0)
    );
    // An absolute pitch ignores both ends.
    assert_eq!(
        (pitch(30.0, -45.0, false), pitch(0.0, -45.0, false)),
        (-45.0, -45.0)
    );
    assert_eq!(pitch(0.0, 120.0, false), 90.0);
}

/// The hurt sound of `ClientboundDamageEventPacket` follows its damage
/// type (`Player.getHurtSound`: the type's effects), with the codec's
/// `hurt` default for an unknown type (arismoko/horizon#4149).
#[test]
fn the_hurt_sound_follows_the_damage_events_type() {
    use super::hurt_sound_for;
    use horizon_protocol::registry::Dynamic;
    use horizon_world::damage_source::Catalog;
    let types = [
        ("minecraft:in_fire", "burning"),
        ("minecraft:drown", "drowning"),
        ("minecraft:sweet_berry_bush", "poking"),
        ("minecraft:freeze", "freezing"),
        ("minecraft:arrow", "hurt"),
    ];
    // The types the server synchronized, in registry order.
    let mut registries = Dynamic::default();
    registries.add(horizon_protocol::packets::configuration::RegistryData {
        registry: "minecraft:damage_type".into(),
        entries: types
            .iter()
            .map(
                |(id, _)| horizon_protocol::packets::configuration::RegistryEntry {
                    id: (*id).into(),
                    data: None,
                },
            )
            .collect(),
    });
    // The damage catalog: one entry per type above, each with its effects.
    let bits = |f: f32| format!("{:08x}", f.to_bits());
    let catalog = Catalog::from_json(
        &serde_json::json!({
            "data_version": i64::from(horizon_world::DATA_VERSION),
            "types": types.iter().map(|(id, effects)| serde_json::json!({
                "id": id,
                "message_id": id.trim_start_matches("minecraft:"),
                "exhaustion": bits(0.1),
                "scaling": "when_caused_by_living_non_player",
                "effects": effects,
                "death_message_type": "default",
            })).collect::<Vec<_>>(),
        }),
        &serde_json::json!({}),
    )
    .unwrap();
    for (id, (name, effects)) in types.iter().enumerate() {
        assert_eq!(
            hurt_sound_for(&registries, &catalog, id as i32),
            horizon_client::health::hurt_sound(effects),
            "{name}"
        );
    }
    // An unknown id falls back to the codec's default effects.
    assert_eq!(
        hurt_sound_for(&registries, &catalog, types.len() as i32),
        "minecraft:entity.player.hurt"
    );
    // A server-defined damage type with inline NBT effects needs no entry in
    // the client's built-in catalog.
    let mut custom = horizon_core::nbt::Compound::new();
    custom.insert(
        "effects".into(),
        horizon_core::nbt::Tag::String("burning".into()),
    );
    registries.add(horizon_protocol::packets::configuration::RegistryData {
        registry: "minecraft:damage_type".into(),
        entries: vec![horizon_protocol::packets::configuration::RegistryEntry {
            id: "custom:lava_splash".into(),
            data: Some(horizon_core::nbt::Tag::Compound(custom)),
        }],
    });
    assert_eq!(
        hurt_sound_for(&registries, &catalog, 0),
        "minecraft:entity.player.hurt_on_fire"
    );
}
#[test]
fn synchronized_damage_defaults_override_a_different_local_definition() {
    let local = horizon_world::damage_source::Catalog::from_json(&serde_json::json!({
        "data_version": horizon_world::DATA_VERSION,
        "types": [{"id":"demo:heat", "message_id":"heat", "exhaustion":"00000000", "scaling":"never", "effects":"burning", "death_message_type":"default"}]
    }), &serde_json::json!({})).unwrap();
    let mut registries = horizon_protocol::registry::Dynamic::default();
    registries.add(horizon_protocol::packets::configuration::RegistryData {
        registry: "minecraft:damage_type".into(),
        entries: vec![horizon_protocol::packets::configuration::RegistryEntry {
            id: "demo:heat".into(),
            data: Some(horizon_core::nbt::Tag::Compound(Default::default())),
        }],
    });
    assert_eq!(
        super::hurt_sound_for(&registries, &local, 0),
        "minecraft:entity.player.hurt"
    );
}
/// `ClientPacketListener.handleSetEntityPassengersPacket`: the vehicle
/// ejects its passengers, then each listed entity `startRiding`s it, so
/// `isVehicle` holds while the list is not empty, whatever the passenger's
/// type: a server's item display riding a player hides that player's name
/// tag like a text display does (#5666).
#[test]
fn any_tracked_passenger_makes_its_vehicle_a_vehicle() {
    use std::collections::{HashMap, HashSet};
    let tracked = |kind: &str| super::Tracked {
        kind: kind.to_owned(),
        position: [0.0, 64.0, 0.0],
        codec_base: [0.0, 64.0, 0.0],
        silent: false,
        vehicle: None,
    };
    let mut entities = HashMap::from([
        (42, tracked("minecraft:player")),
        (43, tracked("minecraft:player")),
        (9, tracked("minecraft:item_display")),
        (10, tracked("minecraft:block_display")),
    ]);
    let vehicles = |e: &HashMap<i32, super::Tracked>| -> HashSet<u32> {
        e.values().filter_map(|t| t.vehicle).collect()
    };
    super::seat_passengers(&mut entities, 42, &[9, 10]);
    assert_eq!(vehicles(&entities), HashSet::from([42]));
    // The list replaces the old one: 10 got off, 9 still rides.
    super::seat_passengers(&mut entities, 42, &[9]);
    assert_eq!(entities[&10].vehicle, None);
    assert_eq!(entities[&9].vehicle, Some(42));
    // startRiding leaves the old vehicle first.
    super::seat_passengers(&mut entities, 43, &[9]);
    assert_eq!(vehicles(&entities), HashSet::from([43]));
    // An empty list: nobody rides 43 any more.
    super::seat_passengers(&mut entities, 43, &[]);
    assert!(vehicles(&entities).is_empty());
}
/// A `SetPassengers` list is looked up as a set: the seats come out as the
/// scan of the list once per entity had them, duplicates, ids this client
/// doesn't track and entities on another vehicle included.
#[test]
fn a_passenger_set_seats_as_the_list_scan_did() {
    use std::collections::HashMap;
    // The seating before the set: `contains` on the list for every entity.
    fn scan(entities: &mut HashMap<i32, super::Tracked>, vehicle: u32, passengers: &[i32]) {
        for (id, t) in entities.iter_mut() {
            if passengers.contains(id) {
                t.vehicle = Some(vehicle);
            } else if t.vehicle == Some(vehicle) {
                t.vehicle = None;
            }
        }
    }
    let tracked = |vehicle: Option<u32>| super::Tracked {
        kind: "minecraft:item_display".to_owned(),
        position: [0.0, 64.0, 0.0],
        codec_base: [0.0, 64.0, 0.0],
        silent: false,
        vehicle,
    };
    // 1 rides 40 and stays listed, 2 rides 40 and is no longer listed, 3
    // rides 41, 4 and -5 ride nothing; 99 is not tracked.
    let start = || {
        HashMap::from([
            (1, tracked(Some(40))),
            (2, tracked(Some(40))),
            (3, tracked(Some(41))),
            (4, tracked(None)),
            (-5, tracked(None)),
        ])
    };
    for list in [
        &[1, 4, 4, 99, -5, 3][..],
        &[][..],
        &[2, 2, 2][..],
        &[99, -1, 0][..],
    ] {
        let mut by_set = start();
        let mut by_scan = start();
        super::seat_passengers(&mut by_set, 40, list);
        scan(&mut by_scan, 40, list);
        let seats = |e: &HashMap<i32, super::Tracked>| {
            let mut seats: Vec<_> = e.iter().map(|(id, t)| (*id, t.vehicle)).collect();
            seats.sort_unstable();
            seats
        };
        assert_eq!(seats(&by_set), seats(&by_scan), "list {list:?}");
    }
    // The first list, by hand: 3 left 41 for 40, 2 got off.
    let mut seated = start();
    super::seat_passengers(&mut seated, 40, &[1, 4, 4, 99, -5, 3]);
    assert_eq!(seated[&2].vehicle, None);
    assert!([1, 3, 4, -5].iter().all(|id| seated[id].vehicle == Some(40)));
}
/// `ClientboundSoundEntityPacket`: the sound is heard where the entity it
/// names is — the local player included, which is no tracked entity — and
/// an id this level doesn't have plays nothing at all.
#[test]
fn a_sound_entity_is_heard_where_its_entity_is() {
    use std::collections::HashMap;
    let mut tracked = HashMap::new();
    tracked.insert(
        7,
        super::Tracked {
            kind: "minecraft:cow".to_owned(),
            position: [8.5, 64.0, -3.5],
            codec_base: [8.5, 64.0, -3.5],
            silent: false,
            vehicle: None,
        },
    );
    let player = Some((2, [0.5, 70.0, 0.5], false));
    assert_eq!(
        super::entity_sound_state(2, player, None, &tracked, None)
            .filter(|s| !s.silent)
            .map(|s| s.position),
        Some([0.5, 70.0, 0.5]),
        "the local player is named by Remote.entity_id"
    );
    assert_eq!(
        super::entity_sound_state(7, player, None, &tracked, None)
            .filter(|s| !s.silent)
            .map(|s| s.position),
        Some([8.5, 64.0, -3.5]),
        "another entity the server spawned"
    );
    assert_eq!(
        super::entity_sound_state(9, player, None, &tracked, None)
            .filter(|s| !s.silent)
            .map(|s| s.position),
        None,
        "getEntity is null: handleSoundEntityEvent returns"
    );
    assert_eq!(
        super::entity_sound_state(2, None, None, &tracked, None)
            .filter(|s| !s.silent)
            .map(|s| s.position),
        None,
        "a player with no position yet"
    );
}
#[test]
fn a_sound_entity_retains_silence_for_players_and_non_mob_targets() {
    use std::collections::HashMap;
    let mut tracked = HashMap::new();
    tracked.insert(
        7,
        super::Tracked {
            kind: "minecraft:arrow".to_owned(),
            position: [8.5, 64.0, -3.5],
            codec_base: [8.5, 64.0, -3.5],
            silent: true,
            vehicle: None,
        },
    );
    let state = super::entity_sound_state(7, None, None, &tracked, None).unwrap();
    assert!(state.silent);
    assert!(!state.removed);
    assert_eq!(state.position, [8.5, 64.0, -3.5]);
    let player =
        super::entity_sound_state(2, Some((2, [1.0, 2.0, 3.0], true)), None, &tracked, None)
            .unwrap();
    assert!(player.silent);
    assert!(super::entity_sound_state(9, None, None, &tracked, None).is_none());
}
#[test]
fn a_dragon_sound_uses_live_movement_without_losing_mob_metadata() {
    use horizon_client::sound_engine::SoundEntityState;
    let stale = SoundEntityState {
        position: [0.0, 64.0, 0.0],
        silent: true,
        removed: false,
    };
    let tracked = std::collections::HashMap::new();
    for position in [[24.0, 80.0, -16.0], [25.5, 81.0, -18.0]] {
        let state =
            super::entity_sound_state(7, None, Some(stale), &tracked, Some((7, position))).unwrap();
        assert_eq!(state.position, position);
        assert!(state.silent);
        assert!(!state.removed);
    }
    let unrelated =
        super::entity_sound_state(7, None, Some(stale), &tracked, Some((8, [99.0; 3]))).unwrap();
    assert_eq!(unrelated.position, stale.position);
    assert!(super::entity_sound_state(7, None, None, &tracked, Some((7, [99.0; 3]))).is_none());
}

/// Q over a slot of the inventory on a server (arismoko/horizon#3357): the predicted THROW
/// drops the stack, so the player's hand swings, as `LivingEntity.drop`
/// does for `Prediction.PREDICTED`; a plain pick up drops nothing.
#[test]
fn a_predicted_throw_from_a_slot_swings_the_hand() {
    use horizon_client::{
        banner::Banners,
        brewing::Brewing,
        crafting::Recipes,
        creative::Names,
        enchant::Enchantments,
        items::{ItemStack, Items},
        menu::{Env, Input, Kind, Menu},
        stations::StationRecipes,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let none = &horizon_world::loot::PackOverlay::NONE;
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let items = Items::catalog(&root, none, &defaults).unwrap();
    let recipes = Recipes::load(&root, &items, none, &crate::game::test_server_load()).unwrap();
    let brewing = Brewing::load(&root, &items, none);
    let banners = Banners::load(&root, &defaults);
    let station_recipes =
        StationRecipes::load(&root, &items, none, &crate::game::test_server_load());
    let enchantments = Enchantments::default();
    let names = Names::default();
    let env = Env {
        crafting_remainder: None,
        player: None,
        player_entity_id: None,
        loop_context: &crate::game::test_server_load()
            .loop_context(horizon_world::native_load_context::LoopSide::Client, []),
        items: &items,
        recipes: &recipes,
        brewing: &brewing,
        banners: &banners,
        station_recipes: &station_recipes,
        enchantments: &enchantments,
        names: &names,
        trade: None,
        maps: None,
        limited_crafting: false,
        can_equip: None,
        known_recipes: None,
        slot_rules: None,
    };
    let mut menu = Menu::new(Kind::Inventory);
    let cobblestone = ItemStack {
        item: "minecraft:cobblestone".into(),
        count: 16,
        damage: 0,
        extra: None,
    };
    // The first hotbar slot of InventoryMenu.
    menu.set_slot(&env, 36, Some(cobblestone));
    let (sent, threw) = super::predict_clicks(&mut menu, &env, &[(36, 0, Input::Throw)]).unwrap();
    assert!(threw, "Q over a slot drops one and swings");
    assert_eq!(menu.slot(36).map(|s| s.count), Some(15));
    assert_eq!(sent.len(), 1);
    let (_, threw) = super::predict_clicks(&mut menu, &env, &[(36, 0, Input::Pickup)]).unwrap();
    assert!(!threw, "picking the stack up drops nothing");
    let (_, threw) = super::predict_clicks(&mut menu, &env, &[(-999, 1, Input::Pickup)]).unwrap();
    assert!(threw, "one of the carried stack outside the window");
}

#[test]
fn placement_preview_checks_obstruction_without_changing_the_level() {
    use super::DryLevel;
    use horizon_core::{position::BlockPos, random::LegacyRandom};
    use horizon_world::{
        level::{Behaviours, LevelAccess, PlaceContext, PlaceOutcome, Ticks, data::load_registry},
        registry::BlockStateId,
    };

    struct Level {
        air: BlockStateId,
        stone: BlockStateId,
        power: BlockStateId,
        client_side: bool,
        obstructed: bool,
        loaded: bool,
        ceiling: i32,
    }
    impl horizon_world::level::BlockEntityNeighbors for Level {}
    impl LevelAccess for Level {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            if pos == BlockPos::new(0, 0, 0) {
                self.stone
            } else if pos == BlockPos::new(-1, 1, 0) {
                self.power
            } else {
                self.air
            }
        }
        fn set_raw(&mut self, _: BlockPos, _: BlockStateId) -> Option<BlockStateId> {
            panic!("preview must not write to the live level");
        }
        fn is_loaded(&self, _: BlockPos) -> bool {
            self.loaded
        }
        fn is_client_side(&self) -> bool {
            self.client_side
        }
        fn max_y(&self) -> i32 {
            self.ceiling
        }
        fn is_unobstructed(&self, pos: BlockPos, _: BlockStateId) -> bool {
            assert_eq!(pos, BlockPos::new(0, 1, 0));
            !self.obstructed
        }
        fn random(&mut self) -> &mut LegacyRandom {
            panic!("preview must not advance the live level's random source");
        }
        fn game_time(&self) -> i64 {
            0
        }
    }

    // As with the LAN tests, prepared game resources are optional locally.
    let resources = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !resources.join("block-behaviours.json").exists() {
        return;
    }
    let registry = load_registry(&resources).unwrap();
    let behaviours = Behaviours::load(&registry, &resources).unwrap();
    let mut base = Level {
        air: registry.default_state("minecraft:air").unwrap(),
        stone: registry.default_state("minecraft:stone").unwrap(),
        power: registry.default_state("minecraft:redstone_block").unwrap(),
        client_side: true,
        obstructed: false,
        loaded: true,
        ceiling: 320,
    };
    for item in [
        "minecraft:stone",
        "minecraft:oak_slab",
        "minecraft:oak_door",
        "minecraft:tnt",
        "minecraft:piston",
        "minecraft:sticky_piston",
        // GrowingPlantHeadBlock.getStateForPlacement draws a random age.
        "minecraft:twisting_vines",
    ] {
        let ctx = PlaceContext {
            clicked_pos: BlockPos::new(0, 0, 0),
            face: horizon_core::geometry::Direction::Up,
            hit: horizon_core::geometry::Vec3::new(0.5, 1.0, 0.5),
            inside: false,
            yaw: 0.0,
            pitch: 45.0,
            sneaking: false,
            item: item.into(),
            can_use_game_master_blocks: false,
        };
        for (client_side, obstructed, loaded, ceiling) in [
            (true, false, true, 320),
            (true, true, true, 320),
            (true, false, false, 320),
            (true, false, true, 1),
            (false, false, true, 320),
        ] {
            base.client_side = client_side;
            base.obstructed = obstructed;
            base.loaded = loaded;
            base.ceiling = ceiling;
            let valid = !obstructed && loaded && ceiling > 1;
            let mut preview = DryLevel::new(&base);
            let mut ticks = Ticks::default();
            let outcome = behaviours.place_block(&mut preview, &mut ticks, &ctx);
            assert_eq!(
                matches!(outcome, PlaceOutcome::Placed { .. }),
                valid,
                "{item}, obstructed={obstructed}, loaded={loaded}, ceiling={ceiling}"
            );
            assert_eq!(preview.changed.is_empty(), !valid);
            if valid
                && matches!(
                    item,
                    "minecraft:tnt" | "minecraft:piston" | "minecraft:sticky_piston"
                )
            {
                assert_eq!(
                    registry
                        .state(preview.block_state(BlockPos::new(0, 1, 0)))
                        .unwrap()
                        .name,
                    if item == "minecraft:tnt" && !client_side {
                        "minecraft:air"
                    } else {
                        item
                    },
                    "powered {item}, client_side={client_side}"
                );
                assert_eq!(
                    ticks.block_events.is_empty(),
                    client_side || item == "minecraft:tnt",
                    "only the server starts a powered piston"
                );
            }
            assert_eq!(base.block_state(BlockPos::new(0, 1, 0)), base.air);
        }
    }
}

#[test]
fn the_creative_sync_covers_inventory_menu_slots_5_to_45_once_each() {
    // CreativeInventoryListener sends every InventoryMenu slot but the crafting ones: the
    // armour (5..=8), the inventory (9..=44) and the off hand (45).
    let stack = |item: &str| Some(horizon_client::items::ItemStack::new(item, 1));
    let inventory: Vec<_> = (0..36)
        .map(|i| stack(&format!("minecraft:item_{i}")))
        .collect();
    let offhand = stack("minecraft:shield");
    let armor = [
        stack("minecraft:diamond_helmet"),
        stack("minecraft:diamond_chestplate"),
        stack("minecraft:diamond_leggings"),
        stack("minecraft:diamond_boots"),
    ];
    let stacks = super::creative_sync_stacks(&inventory, &offhand, &armor);
    assert_eq!(stacks.len(), super::CREATIVE_SYNCED);
    let mut slots: Vec<i16> = (0..stacks.len()).map(super::creative_sync_slot).collect();
    // Each entry goes to the menu slot that holds that stack.
    for (i, &slot) in slots.iter().enumerate() {
        let game_slot = Game::inventory_screen_slot("inventory", i32::from(slot)).unwrap();
        let expected = match game_slot {
            0..36 => &inventory[game_slot],
            504 => &offhand,
            500..504 => &armor[game_slot - 500],
            _ => panic!("entry {i} went to game slot {game_slot}"),
        };
        assert_eq!(&stacks[i], expected, "entry {i}, menu slot {slot}");
    }
    slots.sort();
    assert_eq!(slots, (5..=45).collect::<Vec<i16>>());
}

#[test]
fn a_server_slot_update_leaves_an_unsent_creative_click_to_be_sent() {
    let stack = |item: &str| Some(horizon_client::items::ItemStack::new(item, 1));
    let none = || vec![None; super::CREATIVE_SYNCED];
    // Synced: nothing. A click puts a helmet on (entry 37, not sent yet), then the server
    // sets hotbar slot 0 (a pickup): only that entry counts as synced.
    let mut synced = none();
    let mut before = none();
    before[37] = stack("minecraft:diamond_helmet");
    let mut now = before.clone();
    now[0] = stack("minecraft:stick");
    super::mark_synced(&mut synced, &before, now.clone());
    assert_eq!(synced[0], now[0]);
    assert_eq!(
        synced[37], None,
        "the helmet still differs, so the next tick sends it"
    );
}

#[test]
fn every_inventory_menu_slot_is_a_game_slot_and_45_is_the_off_hand() {
    // InventoryMenu: result, 2x2 grid, armour, 27 + 9 inventory slots, then the off hand.
    assert_eq!(Game::inventory_screen_slot("inventory", 45), Some(504));
    let mut seen = std::collections::HashSet::new();
    for slot in 0..46 {
        let internal = Game::inventory_screen_slot("inventory", slot);
        assert!(internal.is_some_and(|i| seen.insert(i)), "slot {slot}");
    }
    assert_eq!(Game::inventory_screen_slot("inventory", 46), None);
}

#[test]
fn remote_clicks_find_slots_past_the_old_large_chest_cutoff() {
    for (menu, internal, index) in [
        ("minecraft:generic_9x6", 18, 63),
        ("minecraft:generic_9x6", 19, 64),
        ("minecraft:generic_9x6", 0, 81),
        ("minecraft:generic_9x6", 8, 89),
        ("minecraft:generic_9x6", 353, 53),
        ("minecraft:generic_9x5", 8, 80),
        ("minecraft:generic_9x4", 8, 71),
        ("minecraft:generic_9x3", 8, 62),
        ("inventory", 504, 45),
    ] {
        assert_eq!(
            Game::menu_slot_index(menu, internal),
            Some(index),
            "{menu}, game slot {internal}"
        );
    }
    assert_eq!(Game::menu_slot_index("minecraft:generic_9x6", 354), None);
    assert_eq!(Game::menu_slot_index("minecraft:generic_9x6", 504), None);
    assert_eq!(Game::menu_slot_index("unknown", 0), None);
}

/// `FireworkRocketItem.use` is SUCCESS while gliding: the main hand's
/// rocket ends the click and the off hand's shield is not raised (arismoko/horizon#3424).
/// On foot it passes, so the off hand still gets its turn.
#[test]
fn a_rocket_used_while_gliding_ends_the_click() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        eprintln!("skipping: no prepared resources in {}", root.display());
        return;
    }
    let registry = horizon_world::level::data::load_registry(&root).unwrap();
    let facts = horizon_client::blocks::BlockFacts::load(&root, &registry).unwrap();
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let items = horizon_client::items::Items::load(
        &root,
        &registry,
        &facts,
        &horizon_world::loot::PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    let rocket = items.get("minecraft:firework_rocket").unwrap();
    assert!(super::use_ends_click(rocket, true), "gliding");
    assert!(!super::use_ends_click(rocket, false), "on foot");
    // Other items answer the same whether or not the player glides.
    let shield = items.get("minecraft:shield").unwrap();
    let stick = items.get("minecraft:stick").unwrap();
    for gliding in [false, true] {
        assert!(super::use_ends_click(shield, gliding));
        assert!(!super::use_ends_click(stick, gliding));
        // FoodOnAStickItem.use passes on a client: no swing while the boost cools down.
        for rod in [
            "minecraft:carrot_on_a_stick",
            "minecraft:warped_fungus_on_a_stick",
        ] {
            let rod = items.get(rod).unwrap();
            assert!(!super::use_ends_click(rod, gliding), "{}", rod.id);
        }
        // A carved pumpkin has an equip_slot (head) but is not swappable, so it has no Item.use.
        let pumpkin = items.get("minecraft:carved_pumpkin").unwrap();
        assert!(!super::use_ends_click(pumpkin, gliding));
    }
}

/// `ClientPacketListener.handleGameEvent`: verify that the game event packet
/// types for arrow hit, pufferfish sting and limited crafting match vanilla.
#[test]
fn game_event_constants_and_types_match_vanilla() {
    use horizon_protocol::packets::play::GameEvent;
    // Event types from ClientboundGameEventPacket:
    // 6 = PLAY_ARROW_HIT_SOUND, 7 = RAIN_LEVEL_CHANGE, 8 = THUNDER_LEVEL_CHANGE,
    // 9 = PUFFER_FISH_STING, 11 = IMMEDIATE_RESPAWN, 12 = LIMITED_CRAFTING.
    let arrow = GameEvent {
        event: 6,
        param: 0.0,
    };
    let rain = GameEvent {
        event: 7,
        param: 0.5,
    };
    let thunder = GameEvent {
        event: 8,
        param: 0.2,
    };
    let puffer = GameEvent {
        event: 9,
        param: 0.0,
    };
    let respawn = GameEvent {
        event: 11,
        param: 1.0,
    };
    let crafting = GameEvent {
        event: 12,
        param: 1.0,
    };

    assert_eq!(arrow.event, 6);
    assert_eq!(rain.event, 7);
    assert_eq!(thunder.event, 8);
    assert_eq!(puffer.event, 9);
    assert_eq!(respawn.event, 11);
    assert_eq!(crafting.event, 12);
    assert!(crafting.param == 1.0);
}

/// #14773: Flint and steel and fire charge useOn on a client level ends the
/// click (returning true), so when held with an unusable off-hand item, the
/// click does not fall through to UseItem or try the off hand.
#[test]
#[ignore = "daily"]
fn flint_and_steel_use_on_ends_the_click() {
    use super::super::{
        Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
    };
    use crate::game::Hit;
    use horizon_client::{first_person::Hand, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;

    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    let (client, _server) = horizon_protocol::connection::Pipe::pair();
    game.connect_local(client, "FlintTest").unwrap();
    game.remote.as_mut().unwrap().entity_id = 7;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.inventory[0] = Some(ItemStack::new("minecraft:flint_and_steel", 1));
    game.offhand = Some(ItemStack::new("minecraft:stick", 1));
    game.selected = 0;
    game.forced_hit = Some(Hit {
        pos: [0, 63, 0],
        face: [0, 1, 0],
        point: [0.5, 64.0, 0.5],
        inside: false,
    });

    let done = game.remote_use_block(Hand::Main);
    assert!(done, "flint and steel useOn must end the click");
}

/// #19952: an unfocused window doesn't pause a world whose level loading
/// screen is still up over `Screen::Playing` (vanilla's screen is the
/// `LevelLoadingScreen` then); once the screen closes, it pauses as before.
#[test]
#[ignore = "daily"]
fn an_unfocused_window_waits_for_the_loading_screen_to_pause() {
    use horizon_client::{level_loading::Reason, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;
    super::super::native_entity_refs::tests::with_client_game(|game| {
        let (client, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(client, "focus-pause-test").unwrap();
        game.title = None;
        game.screen = crate::game::Screen::Playing;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        game.remote
            .as_mut()
            .unwrap()
            .level_load
            .start_client_load(0, Reason::Other);
        game.focus_lost = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        game.pause_if_inactive();
        assert_eq!(game.screen, crate::game::Screen::Playing, "loading screen up");
        game.remote.as_mut().unwrap().level_load.finish();
        game.pause_if_inactive();
        assert_eq!(game.screen, crate::game::Screen::Paused, "loading screen closed");
    });
}
