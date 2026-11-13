//! A `tp` inside a level to chunks not lit yet holds the player (their client's moves are ignored,
//! `Guest::landing`) until the chunks are lit; however the hold ends, the moves are heard again (#15727).
use crate::game::{Game, lan::LanHost, lan::LanSettings};
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig},
};
use horizon_core::geometry::Vec3;
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const CONN: u32 = 7;
const FAR: f64 = 96_000.5;

fn light(game: &mut Game<'_>, around: i32) {
    for x in around - 2..=around + 2 {
        for z in around - 2..=around + 2 {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, z));
        }
    }
}

/// A server with one guest standing on lit land at the origin.
fn server() -> (Game<'static>, LanHost, mpsc::Receiver<Out>) {
    let mut game = crate::game::native_machines::tests::server_game();
    light(&mut game, 0);
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::loopback(0, String::new(), 2)
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
        },
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: CONN,
            uuid: 77,
            name: "Alice".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:7".parse().unwrap(),
            expected: false,
            configured: Default::default(),
        },
        CONN as i32,
        [0.5, 100.0, 0.5],
        2,
    );
    guest.dimension = "minecraft:overworld".into();
    guest.placed = true;
    guest.loaded = true;
    guest.health = 20.0;
    game.lan.as_mut().unwrap().hub.guests.push(guest);
    let host = game.lan.take().unwrap();
    (game, host, rx)
}

fn landing(host: &LanHost) -> bool {
    host.hub.guests[0].landing
}

#[test]
#[ignore = "prepared resources in HORIZON_TEST_RESOURCES"]
fn a_hold_ignores_the_clients_moves_until_a_tp_to_lit_land_replaces_it() {
    let (mut game, mut host, _rx) = server();
    game.hold_for_chunks(CONN, Vec3::new(FAR, 150.0, FAR));
    game.tick_server_travel(&mut host);
    assert!(landing(&host), "held while the far chunks are not lit");
    // A second `tp`, to land that is lit, ends the wait: the moves are heard again.
    game.hold_for_chunks(CONN, Vec3::new(0.5, 100.0, 0.5));
    game.tick_server_travel(&mut host);
    assert!(!game.on_server_trip(CONN));
    assert!(!landing(&host), "no hold left, so no moves ignored");
}

#[test]
#[ignore = "prepared resources in HORIZON_TEST_RESOURCES"]
fn a_hold_ended_by_a_trip_elsewhere_hears_the_client_again() {
    let (mut game, mut host, _rx) = server();
    game.hold_for_chunks(CONN, Vec3::new(FAR, 150.0, FAR));
    game.tick_server_travel(&mut host);
    assert!(landing(&host));
    // What a death, a respawn or a `tp` to another level does to the trip waiting for the player.
    game.server_trips.retain(|t| !t.is(CONN));
    game.tick_server_travel(&mut host);
    assert!(!landing(&host));
}

#[test]
#[ignore = "prepared resources in HORIZON_TEST_RESOURCES"]
fn a_hold_places_the_player_once_the_chunks_are_lit() {
    let (mut game, mut host, _rx) = server();
    game.hold_for_chunks(CONN, Vec3::new(FAR, 150.0, FAR));
    game.tick_server_travel(&mut host);
    assert!(landing(&host));
    light(&mut game, (FAR as i32) >> 4);
    game.tick_server_travel(&mut host);
    let guest = &host.hub.guests[0];
    assert!(!game.on_server_trip(CONN));
    assert!(!guest.landing);
    assert_eq!(guest.position, [FAR, 150.0, FAR]);
    // A `tp` inside a level sets no portal cooldown.
    assert_eq!(host.play.entry(CONN).or_default().portal.cooldown, 0);
}

#[test]
#[ignore = "prepared resources in HORIZON_TEST_RESOURCES"]
fn a_hold_follows_the_destination_a_mod_chose() {
    let (mut game, mut host, _rx) = server();
    game.hold_for_chunks(CONN, Vec3::new(FAR, 150.0, FAR));
    // A mod redirected the move to another unlit place: the hub put the player there, and the wait is for it.
    host.hub.guests[0].position = [-FAR, 150.0, 0.5];
    game.tick_server_travel(&mut host);
    let trips: Vec<_> = game.server_trips.iter().filter(|t| t.is(CONN)).collect();
    assert_eq!(trips.len(), 1);
    assert_eq!(trips[0].target, Vec3::new(-FAR, 150.0, 0.5));
    // A mod canceled it: the player stays on lit land, so there is nothing to wait for.
    host.hub.guests[0].position = [0.5, 100.0, 0.5];
    game.tick_server_travel(&mut host);
    game.tick_server_travel(&mut host);
    assert!(!game.on_server_trip(CONN));
    assert!(!landing(&host));
}
