//! A rider's double jump toggles creative flight, a refused toggle re-sends the
//! server's own flying speed, and Ctrl-pick with `include_data` prints an
//! avatar's profile.
use crate::game::*;
use mod_api::{self, Hooks, PlayerToggleFlightEvent, ServerHost};
use horizon_client::{
    items::ItemStack,
    lan::{
        Joined, NetConfig, Out, PlayerSender,
        guest::{CREATIVE, Guest},
        hub::HubConfig,
    },
    mobs::Mob,
    motion::PlayerMotion,
    sprint::Keys,
};
use horizon_core::geometry::Vec3;
use horizon_protocol::{
    buf::Reader,
    connection::Pipe,
    packets::{
        configuration::ClientInformation,
        play::{self, Clientbound as C, Serverbound as S},
    },
};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

fn output(rx: &mpsc::Receiver<Out>) -> Vec<C> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(reason) => panic!("unexpected disconnect: {reason:?}"),
        })
        .map(|body| {
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            C::decode(id, reader.rest()).unwrap()
        })
        .collect()
}

fn guest(
    conn: u32,
    uuid: u128,
    name: &str,
    entity_id: i32,
    at: [f64; 3],
) -> (Guest, mpsc::Receiver<Out>) {
    let (tx, rx) = mpsc::channel();
    let mut g = Guest::new(
        Joined {
            conn,
            uuid,
            name: name.into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        entity_id,
        at,
        2,
    );
    g.placed = true;
    g.loaded = true;
    (g, rx)
}

/// A server with a creative game-master picker (conn 1) and another player's
/// avatar (conn 2) beside it, both in the Overworld.
fn lan_game(name: &str) -> (Game<'static>, mpsc::Receiver<Out>) {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::loopback(0, name.into(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let (mut picker, rx) = guest(1, 1, "Picker", 21, [0.5, 100.0, 0.5]);
    picker.game_mode = CREATIVE;
    picker.permission = 2;
    picker.abilities.flying_speed = 2.5;
    let (mut target, _) = guest(2, 2, "Target", 22, [1.5, 100.0, 0.5]);
    target.properties = vec![horizon_protocol::packets::login::Property {
        name: "textures".into(),
        value: "skin".into(),
        signature: None,
    }];
    let hub = &mut game.lan.as_mut().unwrap().hub;
    hub.guests.push(picker);
    hub.guests.push(target);
    (game, rx)
}

fn pass(game: &mut Game<'_>, conn: u32, packet: S) {
    game.lan_guest_packet(conn, packet);
}

#[test]
#[ignore = "daily"]
fn a_riding_player_double_jump_toggles_creative_flight() {
    let mut game = crate::game::native_machines::tests::server_game();
    let (client, _server) = Pipe::pair();
    game.connect_local(client, "FlightTest").unwrap();
    game.remote.as_mut().unwrap().joined = true;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());

    let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
    let mut horse = Mob::new(7, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    horse.extra.tame = true;
    game.mobs.clear();
    game.mobs.push(horse);
    game.riding = Some(7);
    game.remote.as_mut().unwrap().first_passenger = true;
    let player = game.player.as_mut().unwrap();
    player.may_fly = true;
    player.flying = false;

    let press = |game: &mut Game<'_>, jump: bool| {
        game.remote_ride(
            Keys {
                jump,
                ..Default::default()
            },
            0,
        )
        .unwrap()
    };
    // A saddled horse is a `jumpableVehicle`: a double tap toggles flight.
    game.mobs[0].set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
    assert!(press(&mut game, true));
    assert!(
        !game.player.as_ref().unwrap().flying,
        "the first press only opens the double-tap window"
    );
    assert!(press(&mut game, false));
    assert!(!game.player.as_ref().unwrap().flying);
    assert!(press(&mut game, true));
    assert!(
        game.player.as_ref().unwrap().flying,
        "the second press toggles creative flight while riding"
    );

    // An unsaddled horse cannot jump, so it is no `jumpableVehicle` and the
    // toggle stays away (`getVehicle() == null || jumpableVehicle() != null`).
    game.mobs[0].set_saddle(None);
    assert!(press(&mut game, false));
    assert!(press(&mut game, true));
    assert!(press(&mut game, false));
    assert!(press(&mut game, true));
    assert!(
        game.player.as_ref().unwrap().flying,
        "no toggle on a mount that cannot jump"
    );
}

#[test]
#[ignore = "daily"]
fn a_camel_second_passenger_cannot_toggle_creative_flight() {
    let mut game = crate::game::native_machines::tests::server_game();
    let (client, _server) = Pipe::pair();
    game.connect_local(client, "FlightTest").unwrap();
    game.remote.as_mut().unwrap().joined = true;
    game.remote.as_mut().unwrap().first_passenger = true;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.player.as_mut().unwrap().may_fly = true;
    let kind = game.res.mob_types().get("minecraft:camel").unwrap().clone();
    let mut camel = Mob::new(7, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    camel.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
    game.mobs.clear();
    game.mobs.push(camel);
    game.riding = Some(7);
    assert!(game.jumpable_mount(), "the saddled camel must be jumpable");
    game.remote.as_mut().unwrap().first_passenger = false;
    for jump in [true, false, true] {
        game.remote_ride(
            Keys {
                jump,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    }
    assert!(!game.player.as_ref().unwrap().flying);
}

/// A mod that refuses every flight toggle.
struct RefuseFlight;
#[mod_api::__subscriptions]
impl Hooks for RefuseFlight {
    fn player_toggle_flight(
        &self,
        _: &mut dyn ServerHost,
        _: &mut PlayerToggleFlightEvent<'_>,
    ) -> mod_api::Flow {
        mod_api::Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn a_refused_flight_toggle_keeps_the_servers_flying_speed() {
    crate::game::in_own_process(
        "game::remote_vehicles::creative_flight_tests::a_refused_flight_toggle_keeps_the_servers_flying_speed",
        a_refused_flight_toggle_keeps_the_servers_flying_speed_in_own_process,
    );
}

fn a_refused_flight_toggle_keeps_the_servers_flying_speed_in_own_process() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("refuse-flight", Box::new(RefuseFlight))].into(),
        || {
            let (mut game, rx) = lan_game("flight-abilities-test");
            game.lan_flight_toggled(1, true);
            let abilities = output(&rx)
                .into_iter()
                .find_map(|packet| match packet {
                    C::PlayerAbilities(abilities) => Some(abilities),
                    _ => None,
                })
                .expect("a refused toggle re-sends the guest's abilities");
            assert_eq!(
                abilities.flying_speed, 2.5,
                "the server's own flying speed, not the default 0.05"
            );
            assert_eq!(abilities.flags & 2, 0, "the refused flight stays off");
        },
    );
}

#[test]
#[ignore = "daily"]
fn ctrl_pick_with_include_data_prints_another_players_profile() {
    let (mut game, rx) = lan_game("flight-profile-test");
    pass(
        &mut game,
        1,
        S::PickItemFromEntity(play::PickItemFromEntity {
            id: 22,
            include_data: false,
        }),
    );
    assert!(
        !output(&rx)
            .iter()
            .any(|packet| matches!(packet, C::SystemChat(_))),
        "no profile without include_data"
    );

    pass(
        &mut game,
        1,
        S::PickItemFromEntity(play::PickItemFromEntity {
            id: 22,
            include_data: true,
        }),
    );
    let chats: Vec<_> = output(&rx)
        .into_iter()
        .filter_map(|packet| match packet {
            C::SystemChat(chat) => Some(chat.content),
            _ => None,
        })
        .collect();
    assert_eq!(chats.len(), 1, "one profile report");
    let text = horizon_client::snbt::to_snbt(&chats[0]);
    assert!(
        text.contains("commands.fetchprofile.entity.success"),
        "{text}"
    );
    assert!(text.contains("Target"), "{text}");
    assert!(text.contains("textures"), "{text}");
}

#[test]
#[ignore = "daily"]
fn a_sitting_camel_first_passenger_cannot_toggle_creative_flight() {
    let mut game = crate::game::native_machines::tests::server_game();
    let (client, _server) = Pipe::pair();
    game.connect_local(client, "FlightTest").unwrap();
    game.remote.as_mut().unwrap().joined = true;
    game.remote.as_mut().unwrap().first_passenger = true;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.player.as_mut().unwrap().may_fly = true;
    let kind = game.res.mob_types().get("minecraft:camel").unwrap().clone();
    let mut camel = Mob::new(7, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    camel.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
    game.mobs.clear();
    game.mobs.push(camel);
    game.riding = Some(7);
    assert!(
        game.jumpable_mount(),
        "a saddled camel that moves is jumpable"
    );
    // `Camel.canJump` is `!refuseToMove() && super.canJump()`: a sitting or
    // standing-up camel takes no jump, so it is no `jumpableVehicle` either.
    game.mobs[0].extra.camel_sitting = true;
    assert!(
        !game.jumpable_mount(),
        "a sitting camel refuses to move and cannot jump"
    );
    for jump in [true, false, true] {
        game.remote_ride(
            Keys {
                jump,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    }
    assert!(
        !game.player.as_ref().unwrap().flying,
        "no toggle on a mount that refuses to move"
    );
}

#[test]
#[ignore = "daily"]
fn a_pending_auto_jump_from_walking_does_not_block_the_riders_toggle() {
    let mut game = crate::game::native_machines::tests::server_game();
    let (client, _server) = Pipe::pair();
    game.connect_local(client, "FlightTest").unwrap();
    game.remote.as_mut().unwrap().joined = true;
    game.remote.as_mut().unwrap().first_passenger = true;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
    let mut horse = Mob::new(7, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    horse.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
    game.mobs.clear();
    game.mobs.push(horse);
    game.riding = Some(7);
    let player = game.player.as_mut().unwrap();
    player.may_fly = true;
    player.flying = false;
    // The last walking tick started an auto-jump (`LocalPlayer.autoJumpTime`
    // is 1 and its `wasAutoJump` was set); the player boards with it pending.
    player.auto_jump_time = 1;
    player.was_auto_jump = true;

    let press = |game: &mut Game<'_>, jump: bool| {
        game.remote_ride(
            Keys {
                jump,
                ..Default::default()
            },
            0,
        )
        .unwrap()
    };
    // `LocalPlayer.aiStep` still counts it down in the vehicle and its
    // `makeJump` presses jump, which an auto-jump never toggles flight with.
    assert!(press(&mut game, false));
    assert!(
        !game.player.as_ref().unwrap().flying,
        "an auto-jump's own press never toggles flight"
    );
    // The countdown is spent: a manual double tap toggles again.
    assert!(press(&mut game, false));
    assert!(press(&mut game, true));
    assert!(press(&mut game, false));
    assert!(press(&mut game, true));
    assert!(
        game.player.as_ref().unwrap().flying,
        "a value left by the last walking tick does not block the rider's toggle"
    );
}

/// What the client sent since the last call, as play packets.
fn sent_packets(reader: &mut horizon_protocol::connection::ConnReader) -> Vec<S> {
    reader
        .set_read_timeout(Some(std::time::Duration::from_millis(200)))
        .unwrap();
    let mut packets = Vec::new();
    while let Ok((id, body)) = reader.recv() {
        packets.extend(S::decode(id, &body).ok());
    }
    packets
}

#[test]
#[ignore = "daily"]
fn a_rider_flight_toggle_sends_one_ability_packet_from_the_client_tick() {
    use crate::game::native_entity_refs::tests::with_client_game;
    use crate::game::remote::tests::connect_menu_test;
    use horizon_client::{chunk::Chunk, held::Source};
    use horizon_protocol::connection::ConnReader;
    use winit::keyboard::KeyCode;

    with_client_game(|game| {
        let (mut reader, _writer) = connect_menu_test(game);
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: play::SpawnInfo {
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
            },
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [0.5, 64.0, 0.5],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives: 0,
        }))
        .unwrap();
        sent_packets(&mut reader);
        for x in -1..=1 {
            for z in -1..=1 {
                game.streaming
                    .insert_remote(&mut game.world, Chunk::empty(x, z));
            }
        }
        let remote = game.remote.as_mut().unwrap();
        remote.joined = true;
        remote.level_load.finish();
        let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap();
        player.may_fly = true;
        player.flying = false;
        game.player = Some(player);
        game.screen = Screen::Playing;
        let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
        let mut horse = Mob::new(7, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        horse.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
        game.mobs.clear();
        game.mobs.push(horse);
        game.riding = Some(7);
        game.remote.as_mut().unwrap().first_passenger = true;
        game.keys.clear();

        let tick = |game: &mut Game<'_>, reader: &mut ConnReader, jump: bool| -> Vec<i8> {
            if jump {
                game.keys.press(KeyCode::Space, Source::Keyboard);
            } else {
                game.keys.release(&KeyCode::Space, Source::Keyboard);
            }
            game.tick_remote().unwrap();
            sent_packets(reader)
                .into_iter()
                .filter_map(|packet| match packet {
                    S::PlayerAbilities(abilities) => Some(abilities.flags),
                    _ => None,
                })
                .collect()
        };
        assert!(
            tick(game, &mut reader, true).is_empty(),
            "the first press only opens the double-tap window"
        );
        assert!(tick(game, &mut reader, false).is_empty());
        assert_eq!(
            tick(game, &mut reader, true),
            [2],
            "the second press toggles flight and the tick tells the server"
        );
        assert!(game.player.as_ref().unwrap().flying);
    });
}

