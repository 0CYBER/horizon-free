//! `/random`'s roll line: who hears it and who it names, and the names
//! `/random reset` completes (RandomCommand).
use super::*;
use crate::game::{Dim, game_tests::tests::server_game};
use horizon_client::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
use horizon_protocol::packets::{configuration::ClientInformation, play::Clientbound};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

/// The packets a guest's connection was sent, encoded.
fn packets(receiver: &mpsc::Receiver<Out>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for sent in receiver.try_iter() {
        match sent {
            Out::Packet(p) => out.push(p),
            Out::Packets(p) => out.extend(p),
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        }
    }
    out
}

/// The system lines a guest's connection was sent (`ClientboundSystemChatPacket`).
fn chats(receiver: &mpsc::Receiver<Out>) -> Vec<String> {
    let mut out = Vec::new();
    for body in packets(receiver) {
        let mut reader = horizon_protocol::buf::Reader::new(&body);
        let id = reader.varint().unwrap();
        if let Clientbound::SystemChat(p) = Clientbound::decode(id, reader.rest()).unwrap() {
            // The client's own read of the line it is sent.
            out.push(
                horizon_client::scoreboard::comp_text(&horizon_client::scoreboard::tag_to_json(
                    &p.content,
                ))
                .plain(),
            );
        }
    }
    out
}

/// The completions a guest was sent (`ClientboundCommandSuggestionsPacket`).
fn suggestions(receiver: &mpsc::Receiver<Out>) -> Vec<String> {
    let mut out = Vec::new();
    for body in packets(receiver) {
        let mut r = horizon_protocol::buf::Reader::new(&body);
        if r.varint().ok() != Some(horizon_protocol::ids::play::clientbound::COMMAND_SUGGESTIONS) {
            continue;
        }
        let _ = (r.varint(), r.varint(), r.varint());
        for _ in 0..r.count(usize::MAX).unwrap_or(0) {
            out.extend(r.string(32767).ok());
            if r.bool().unwrap_or(false) {
                let _ = r.nbt();
            }
        }
    }
    out
}

/// A server with two guests on it (`conn` 2 and 7).
fn server_with_guests() -> (Game<'static>, Vec<mpsc::Receiver<Out>>) {
    let resources = Box::leak(Box::new(None));
    let mut game = server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "random-roll".into(), 4)
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
            max_players: 4,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let mut receivers = Vec::new();
    for conn in [2, 7] {
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn),
                name: format!("Roller{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            (1_000 + conn) as i32,
            [8.5, 200.0, 8.5],
            2,
        );
        guest.placed = true;
        guest.loaded = true;
        guest.permission = 2;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        receivers.push(rx);
    }
    (game, receivers)
}

/// PlayerList.broadcastSystemMessage: a guest's roll reaches every player,
/// the rolling guest once (its copy is the command's captured output).
#[test]
#[ignore = "daily"]
fn a_guests_roll_reaches_every_player_once() {
    let (mut game, receivers) = server_with_guests();
    let (ok, lines) = game
        .run_guest_command_told(crate::game::lan_actions::GuestCommand {
            conn: 2,
            command: "random roll 1..2".into(),
            permission: 2,
            integrated: false,
            quiet: false,
        })
        .expect("the guest is on");
    assert!(ok);
    let mine: Vec<String> = lines.iter().map(Text::plain).collect();
    assert_eq!(mine.len(), 1, "{mine:?}");
    assert_eq!(
        lines[0].translation_key(),
        Some("commands.random.roll"),
        "{mine:?}"
    );
    assert!(mine[0].starts_with("Roller2 rolled "), "{mine:?}");
    // The other player hears it through the hub, once, with the same line.
    let theirs = chats(&receivers[1]);
    assert_eq!(theirs.len(), 1, "{theirs:?}");
    assert_eq!(theirs[0], mine[0]);
    assert!(theirs[0].ends_with(" (from 1 to 2)"), "{theirs:?}");
    // The rolling guest's own connection was skipped by the broadcast: the one
    // copy it gets is the command's captured output.
    let own = chats(&receivers[0]);
    assert_eq!(
        own.iter().filter(|line| *line == &mine[0]).count(),
        1,
        "{own:?}"
    );
}

/// CommandSourceStack.getDisplayName: a command block names itself, the
/// console says "Server".
#[test]
#[ignore = "daily"]
fn a_blocks_roll_names_the_block_and_the_console_says_server() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command(
        r#"setblock 0 200 0 minecraft:command_block{Command:"random roll 1..2",CustomName:{text:"Dice"}}"#
    ));
    let roll = |game: &mut Game<'_>, source: crate::game::commands::execute::Source| {
        let outer = game.command_source.replace(source);
        game.chat_capture = Some(Vec::new());
        assert!(game.run_command("random roll 1..2"));
        game.command_source = outer;
        let lines = game.chat_capture.take().unwrap();
        assert_eq!(lines.len(), 1, "{:?}", lines[0].plain());
        lines[0].plain()
    };
    let block = crate::game::commands::execute::Source {
        entity: None,
        block: Some((0, 200, 0)),
        ..game.server_source()
    };
    let named = roll(&mut game, block);
    assert!(named.starts_with("Dice rolled "), "{named}");
    assert!(named.ends_with(" (from 1 to 2)"), "{named}");
    let console = game.server_source();
    let named = roll(&mut game, console);
    assert!(named.starts_with("Server rolled "), "{named}");
    assert!(named.ends_with(" (from 1 to 2)"), "{named}");
}

/// RandomSequences.forAllSequences: the server's answer to the chat box's
/// completion of `/random reset ` lists the sequences the world has made.
#[test]
#[ignore = "daily"]
fn the_reset_argument_offers_the_sequences_the_world_has() {
    let (mut game, receivers) = server_with_guests();
    assert!(game.run_command("random value 1..10 minecraft:test"));
    // ServerGamePacketListenerImpl.handleCustomCommandSuggestions: the
    // `ask_server` request the window's chat box sends for `/random reset `.
    let mut w = horizon_protocol::buf::Writer::new();
    w.varint(1);
    w.string("random reset ");
    game.guest_suggestions(2, &w.data);
    let offered = suggestions(&receivers[0]);
    assert!(offered.iter().any(|s| s == "minecraft:test"), "{offered:?}");
}
