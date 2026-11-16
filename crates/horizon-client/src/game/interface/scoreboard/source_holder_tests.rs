//! A scoreboard command's source: a bare name finds the online player it
//! names (with their team's formatting), and `/trigger` completes the
//! objectives unlocked for the player asking, not for the host.
use super::*;
use crate::game::lan::LanSettings;
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig},
};
use horizon_protocol::{
    buf::{Reader, Writer},
    ids,
    packets::configuration::ClientInformation,
};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

/// A hosted world with `name` joined and placed at the world spawn.
fn game_with_guest<'a>(
    resources: &'a mut Option<Arc<crate::game::Resources>>,
    name: &str,
) -> (Game<'a>, mpsc::Receiver<Out>) {
    let mut game = crate::game::game_tests::tests::server_game(resources);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.build_registries();
    game.set_allow_commands(true);
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "scoreboard-source-test".into(), 2)
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
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 2,
            uuid: 331,
            name: name.into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        2,
        [4.5, 64.0, 4.5],
        2,
    );
    guest.dimension = "minecraft:overworld".into();
    guest.placed = true;
    game.lan.as_mut().unwrap().hub.guests.push(guest);
    (game, rx)
}

#[test]
#[ignore = "daily"]
fn a_score_for_another_player_uses_their_team_formatted_name() {
    let mut resources = None;
    let (mut game, _rx) = game_with_guest(&mut resources, "Alex");
    for command in [
        "team add red",
        "team modify red prefix {\"text\":\"[R] \"}",
        "team join red Alex",
        "scoreboard objectives add o dummy",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game.chat_capture = Some(Vec::new());
    assert!(game.run_command("scoreboard players set Alex o 5"));
    let lines = game.chat_capture.take().unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].plain(), "Set [o] for [R] Alex to 5");
}

/// The `/trigger` suggestions the server sent the guest for `trigger `.
fn guest_trigger_suggestions(game: &mut Game<'_>, rx: &mpsc::Receiver<Out>) -> Vec<String> {
    let mut w = Writer::new();
    w.varint(7);
    w.string("trigger ");
    game.guest_suggestions(2, &w.data);
    for out in rx.try_iter() {
        let bodies = match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut r = Reader::new(&body);
            if r.varint().unwrap() != ids::play::clientbound::COMMAND_SUGGESTIONS {
                continue;
            }
            let _id = r.varint().unwrap();
            let _start = r.varint().unwrap();
            let _length = r.varint().unwrap();
            let n = r.count(usize::MAX).unwrap();
            let mut names = Vec::new();
            for _ in 0..n {
                names.push(r.string(32767).unwrap());
                if r.bool().unwrap() {
                    let _ = r.nbt().unwrap();
                }
            }
            return names;
        }
    }
    panic!("no command suggestions packet");
}

#[test]
#[ignore = "daily"]
fn guest_trigger_completion_lists_the_guests_unlocked_objectives() {
    let mut resources = None;
    let (mut game, rx) = game_with_guest(&mut resources, "Alex");
    for command in [
        "scoreboard objectives add t1 trigger",
        "scoreboard objectives add t2 trigger",
        "scoreboard players enable Player t1",
        "scoreboard players enable Alex t2",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    assert_eq!(guest_trigger_suggestions(&mut game, &rx), ["t2"]);
}

#[test]
#[ignore = "daily"]
fn a_wildcard_reset_names_tracked_holders_without_their_team_formatting() {
    let mut resources = None;
    let (mut game, _rx) = game_with_guest(&mut resources, "Alex");
    for command in [
        "team add red",
        "team modify red prefix {\"text\":\"[R] \"}",
        "team join red Alex",
        "scoreboard objectives add o dummy",
        "scoreboard players set Alex o 5",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    // `Scoreboard.getTrackedPlayers` hands out `ScoreHolder.forNameOnly`, which has no display name.
    let swap = game.enter_guest(2).unwrap();
    game.chat_capture = Some(Vec::new());
    let permission = std::mem::replace(&mut game.ui.registries.permission, 4);
    let ok = game.run_command("scoreboard players reset *");
    game.ui.registries.permission = permission;
    let lines = game.chat_capture.take().unwrap();
    game.leave_guest(swap);
    assert!(ok);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].plain(), "Reset all scores for Alex");
}
