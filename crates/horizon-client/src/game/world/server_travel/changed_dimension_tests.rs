//! `player_changed_dimension` is heard on every logical server: the integrated server that runs
//! single player and the LAN world opened from it, and a dedicated server (#3250).
//! All three tests are daily resource tests registered in `daily-horizon-client.txt`.
use super::Dim;
use crate::game::{
    Game, execution_role::ExecutionRole, lan::LanSettings, lan_actions::GuestCommand,
    native_entity_refs::tests::with_game_role,
};
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
};
use horizon_core::geometry::Vec3;
use horizon_protocol::packets::configuration::ClientInformation;
use mod_api::{Hooks, ModEntry, PlayerChangedDimensionEvent, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI32, AtomicUsize},
};

const CONN: u32 = 12;

type Heard = Arc<Mutex<Vec<(String, String)>>>;

struct Recorder(Heard);

#[mod_api::__subscriptions]
impl Hooks for Recorder {
    fn player_changed_dimension(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerChangedDimensionEvent<'_>,
    ) {
        self.0
            .lock()
            .unwrap()
            .push((event.from.to_owned(), event.to.to_owned()));
    }
}

/// A server of `role` with one survival player at (3.5, 64, 5.5) in the Overworld, its options as
/// `integrated.rs`'s server Game and a dedicated server set them.
fn with_server(role: ExecutionRole, test: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    with_game_role(root, role, |game| {
        game.options.dedicated = Some(Vec::new());
        let config = HubConfig {
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
            integrated: role == ExecutionRole::IntegratedServer,
            levels: game.level_shapes,
        };
        game.lan_host(
            LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            },
            config,
        )
        .unwrap();
        light(game, 0, 0);
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: CONN,
                uuid: 120,
                name: "Owner".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            CONN as i32,
            [3.5, 64.0, 5.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.placed = true;
        guest.loaded = true;
        guest.health = 20.0;
        guest.motion = Some(Box::new(
            horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(3.5, 64.0, 5.5))
                .unwrap(),
        ));
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        test(game);
    });
}

/// Lights the 5x5 chunks around chunk (`cx`, `cz`) of the level in place, as its
/// streaming does once they have loaded.
fn light(game: &mut Game<'_>, cx: i32, cz: i32) {
    for x in cx - 2..=cx + 2 {
        for z in cz - 2..=cz + 2 {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, z));
        }
    }
}

/// Runs the server's travel until the player's trip has ended in `dim`, loading
/// the destination around the trip's target.
fn finish_trip(game: &mut Game<'_>, dim: Dim) {
    let home = game.dim;
    game.enter_level(dim).unwrap();
    let at = game
        .server_trips
        .iter()
        .find(|trip| trip.conn == CONN)
        .expect("the player is on a trip")
        .target;
    light(game, (at.x.floor() as i32) >> 4, (at.z.floor() as i32) >> 4);
    for _ in 0..20 {
        if !game.on_server_trip(CONN) {
            break;
        }
        game.tick_server_travel();
    }
    assert!(
        !game.on_server_trip(CONN),
        "the trip into {dim:?} never ended"
    );
    game.enter_level(home).unwrap();
}

fn run_as_player(game: &mut Game<'_>, role: ExecutionRole, command: &str) {
    assert!(game.run_guest_command(GuestCommand {
        conn: CONN,
        command: command.into(),
        permission: 4,
        integrated: role == ExecutionRole::IntegratedServer,
        quiet: false,
    }));
    // The integrated server runs them at its tick's start (`integrated.rs`).
    game.run_level_teleports();
}

fn heard_on(role: ExecutionRole, test: impl FnOnce(&mut Game<'_>, &Heard)) {
    let heard = Heard::default();
    let mods: mod_api::Mods = vec![ModEntry::new(
        "dimension-recorder",
        Box::new(Recorder(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_server(role, |game| test(game, &heard));
    });
}

const SERVERS: [ExecutionRole; 2] = [
    ExecutionRole::IntegratedServer,
    ExecutionRole::DedicatedServer,
];

fn overworld_to_nether() -> Vec<(String, String)> {
    vec![("minecraft:overworld".into(), "minecraft:the_nether".into())]
}

#[test]
#[ignore = "daily"]
fn a_nether_portal_trip_is_heard_once_on_every_server() {
    for role in SERVERS {
        heard_on(role, |game, heard| {
            let portal = game
                .pack
                .assets
                .registry
                .default_state("minecraft:nether_portal")
                .unwrap();
            game.world.set(3, 64, 5, portal).unwrap();
            game.world.set(3, 65, 5, portal).unwrap();
            // `playersNetherPortalDefaultDelay` is 80 ticks for a survival player.
            for _ in 0..100 {
                if game.on_server_trip(CONN) {
                    break;
                }
                game.tick_server_travel();
            }
            assert!(
                game.on_server_trip(CONN),
                "{role:?}: the portal never took the player"
            );
            assert!(
                heard.lock().unwrap().is_empty(),
                "{role:?}: heard before the trip ended"
            );
            finish_trip(game, Dim::Nether);
            assert_eq!(*heard.lock().unwrap(), overworld_to_nether(), "{role:?}");
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_command_teleport_into_the_nether_is_heard_once_on_every_server() {
    for role in SERVERS {
        heard_on(role, |game, heard| {
            run_as_player(
                game,
                role,
                "execute in minecraft:the_nether run tp @s 0 64 0",
            );
            finish_trip(game, Dim::Nether);
            assert_eq!(*heard.lock().unwrap(), overworld_to_nether(), "{role:?}");
        });
    }
}

#[test]
#[ignore = "daily"]
fn an_end_gateway_jump_within_the_level_is_not_heard_on_any_server() {
    for role in SERVERS {
        heard_on(role, |game, heard| {
            game.build_registries();
            run_as_player(
                game,
                role,
                "setblock 3 64 5 minecraft:end_gateway{exit_portal:[I;40,64,40],ExactTeleport:0b}",
            );
            // Not exact: the player waits for the exit's chunks, then lands near it.
            game.start_gateway_trip(CONN, [3, 64, 5]);
            finish_trip(game, Dim::Overworld);
            let at = game.lan.as_ref().unwrap().hub.guest(CONN).unwrap().position;
            assert!(
                (at[0] - 40.5).hypot(at[2] - 40.5) < 16.0,
                "{role:?}: the jump landed at {at:?}"
            );
            assert!(heard.lock().unwrap().is_empty(), "{role:?}: {heard:?}");
        });
    }
}
