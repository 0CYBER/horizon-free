//! `/bossbar get <id> players` names its players as vanilla does
//! (`BossBarCommands.getPlayers` → `ComponentUtils.formatList(players,
//! Player::getDisplayName)`): the team's formatting, the `/tell` click, the
//! entity hover and the name a shift-click inserts.
use super::*;
use crate::game::{lan::LanSettings, lan_actions::GuestCommand};
use horizon_client::{
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
};
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

/// A host game with a window player and two other online players.
fn game_with_two_guests() -> Game<'static> {
    use crate::game::{Audio, Options, Progress, Settings, load_resources};
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("set HORIZON_TEST_RESOURCES"),
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
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.player = Some(
        PlayerMotion::new(
            game.motion,
            horizon_core::geometry::Vec3::new(10.5, 64.0, -3.25),
        )
        .unwrap(),
    );
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::lan(0, "bossbar-names-test".into(), 3)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 3,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    for (conn, name) in [(2, "Guest"), (3, "Other")] {
        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn),
                name: name.into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            conn as i32,
            [8.5, 64.0, 8.5],
            0,
        );
        guest.placed = true;
        guest.loaded = true;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
    }
    game
}

/// A command typed by guest 2, as `LanHub` queues it: the feedback it got.
fn guest_2_command(game: &mut Game<'_>, command: &str) -> Vec<Text> {
    let (ok, lines) = game
        .run_guest_command_told(GuestCommand {
            conn: 2,
            command: command.into(),
            permission: 4,
            integrated: false,
            quiet: false,
        })
        .expect("guest 2 is online");
    assert!(ok, "{command}");
    lines
}

#[test]
#[ignore = "daily"]
fn a_boss_bars_player_list_is_decorated_like_other_player_lists() {
    let mut game = game_with_two_guests();
    assert!(game.run_command("bossbar add demo:bar Demo"));
    assert!(game.run_command("team add demo_team"));
    assert!(game.run_command("team modify demo_team color red"));
    assert!(game.run_command("team join demo_team Guest"));
    assert!(game.run_command("team join demo_team Other"));
    guest_2_command(&mut game, "bossbar set demo:bar players @a");

    let lines = guest_2_command(&mut game, "bossbar get demo:bar players");
    assert_eq!(lines.len(), 1, "{lines:?}");
    let message = lines[0].component();
    // commands.bossbar.get.players.some: the bar's name, the count, the list.
    let names = message["with"][2]["extra"]
        .as_array()
        .expect("the player list");
    for (uuid, name) in [(2, "Guest"), (3, "Other")] {
        let listed = names
            .iter()
            .find(|n| n["insertion"] == name)
            .unwrap_or_else(|| panic!("no decorated name for {name} in {message}"));
        // Player::getDisplayName: the team's colour, the `/tell` click, the
        // entity hover and the name a shift-click inserts.
        assert_eq!(listed["color"], "red", "{listed}");
        assert_eq!(
            listed["click_event"]["command"],
            format!("/tell {name} "),
            "{listed}"
        );
        assert_eq!(listed["hover_event"]["action"], "show_entity", "{listed}");
        assert_eq!(listed["hover_event"]["id"], "minecraft:player", "{listed}");
        assert_eq!(listed["hover_event"]["name"], name, "{listed}");
        assert_eq!(
            listed["hover_event"]["uuid"],
            horizon_protocol::crypto::uuid_string(uuid),
            "{listed}"
        );
    }
}
