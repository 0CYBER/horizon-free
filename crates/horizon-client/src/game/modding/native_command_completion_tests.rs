//! A mod command's path follows the sender's permission everywhere it is visible: the command tree
//! a player receives keeps only the children they may run, the server's completions answer the same
//! test, and a registered argument type is asked with the sender, so a provider that completes a
//! mod's own registry offers only what that sender may use (arismoko/horizon#4307).

use super::Game;
use super::native_server::{CommandArguments, command_tree, register_dispatcher};
use crate::game::lan::LanSettings;
use horizon_client::{
    chunk::Chunk,
    command::{MOD_COMMAND, Registries},
    lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig},
};
use horizon_protocol::{
    buf::{Codec, Reader, Writer},
    ids,
    packets::{configuration::ClientInformation, play},
};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

/// The warps the test mod publishes, with the level each one is for: `spawn` for everyone,
/// `secret` for an owner.
const WARPS: [(&str, u8); 2] = [("spawn", 0), ("secret", 4)];

/// The test mod's argument type: it completes the warps this sender may use, which is its own
/// policy — the game keeps the answer as it comes.
struct WarpNames;

impl mod_api::CommandArgumentType for WarpNames {
    fn kind(&self) -> mod_api::CommandArgumentKind {
        mod_api::CommandArgumentKind::Word
    }

    fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
        Ok(input.to_owned())
    }

    fn suggest(
        &self,
        sender: Option<&mod_api::CommandSender>,
        prefix: &str,
    ) -> Vec<mod_api::CommandSuggestion> {
        let permission = sender.map_or(0, |sender| sender.permission().level());
        WARPS
            .into_iter()
            .filter(|(_, level)| *level <= permission)
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, _)| mod_api::CommandSuggestion::new(name))
            .collect()
    }
}

/// A hosted world with `name` joined, placed and at the world spawn, the way
/// `game/interface/scoreboard/source_holder_tests.rs` builds one.
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
            ..NetConfig::loopback(0, "native-completion-test".into(), 2)
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

/// The warps command in the server's dispatcher and in every player's tree: `/warp` for everyone,
/// `/warp go <name>` for everyone, `/warp admin <name>` for owners.
fn install_warps(game: &mut Game<'_>) {
    let arguments: CommandArguments = std::collections::HashMap::from([(
        "test:warp".to_owned(),
        Arc::new(WarpNames) as Arc<dyn mod_api::CommandArgumentType>,
    )]);
    let commands = [
        mod_api::Command::new("warp"),
        mod_api::Command::new("warp go <name:test:warp>"),
        mod_api::Command::op("warp admin <name:test:warp>", 4),
    ];
    let (tree, _, problems) = command_tree(&commands, |_| false, &arguments);
    assert!(problems.is_empty(), "{problems:?}");
    let mut dispatcher = (*game.ui.dispatcher.0).clone();
    register_dispatcher(&mut dispatcher, &tree, MOD_COMMAND, &arguments);
    game.ui.dispatcher.0 = Arc::new(dispatcher);
    let host = game.lan.as_mut().unwrap();
    host.hub.set_mod_commands(0, tree);
}

/// The last command tree the guest was sent.
fn last_tree(rx: &mpsc::Receiver<Out>) -> play::Commands {
    let mut last = None;
    for out in rx.try_iter() {
        let bodies = match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut r = Reader::new(&body);
            if r.varint().unwrap() != ids::play::clientbound::COMMANDS {
                continue;
            }
            last = Some(play::Commands::read(&mut r).unwrap());
        }
    }
    last.expect("no command tree")
}

/// The children of the node named `name`, in the tree's order.
fn children(packet: &play::Commands, name: &str) -> Vec<String> {
    let node = packet
        .nodes
        .iter()
        .find(|node| node.name == name)
        .unwrap_or_else(|| panic!("no `{name}` in the tree"));
    node.children
        .iter()
        .map(|&child| packet.nodes[child as usize].name.clone())
        .collect()
}

/// The completions the server answered the guest's request for `command` with.
fn suggestions(game: &mut Game<'_>, rx: &mpsc::Receiver<Out>, command: &str) -> Vec<String> {
    let mut w = Writer::new();
    w.varint(7);
    w.string(command);
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
            let count = r.count(usize::MAX).unwrap();
            let mut names = Vec::new();
            for _ in 0..count {
                names.push(r.string(32767).unwrap());
                if r.bool().unwrap() {
                    let _ = r.nbt().unwrap();
                }
            }
            return names;
        }
    }
    panic!("no command suggestions packet for `{command}`");
}

struct IdentifierCompletion;

#[mod_api::__subscriptions]
impl mod_api::Hooks for IdentifierCompletion {
    fn tab_complete(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::TabCompleteEvent<'_>,
    ) {
        if event.command == "probe ex" {
            assert_eq!(event.range, [6, 8]);
            event
                .suggestions
                .push(mod_api::Suggestion::new((6, 8), "example:stone"));
        }
    }
}

#[test]
#[ignore = "daily"]
fn an_identifier_listener_replies_on_the_wire_and_leaves_room_for_the_next_argument() {
    let mods = vec![mod_api::ModEntry::new(
        "identifier_completion",
        Box::new(IdentifierCompletion),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut resources = None;
        let (mut game, rx) = game_with_guest(&mut resources, "Alex");
        let arguments = CommandArguments::new();
        let (tree, _, problems) = command_tree(
            &[mod_api::Command::new("probe <value:id> <count:integer>")],
            |_| false,
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = (*game.ui.dispatcher.0).clone();
        register_dispatcher(&mut dispatcher, &tree, MOD_COMMAND, &arguments);
        game.ui.dispatcher.0 = Arc::new(dispatcher);
        game.lan.as_mut().unwrap().hub.set_mod_commands(0, tree);
        last_tree(&rx);

        let typed = "probe ex";
        let mut request = Writer::new();
        request.varint(7);
        request.string(typed);
        game.guest_suggestions(2, &request.data);
        let mut answer = None;
        for out in rx.try_iter() {
            let bodies = match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            };
            for body in bodies {
                let mut reader = Reader::new(&body);
                if reader.varint().unwrap() != ids::play::clientbound::COMMAND_SUGGESTIONS {
                    continue;
                }
                assert_eq!(reader.varint().unwrap(), 7);
                let start = reader.varint().unwrap();
                let length = reader.varint().unwrap();
                assert_eq!(reader.varint().unwrap(), 1);
                let text = reader.string(32767).unwrap();
                assert!(!reader.bool().unwrap());
                assert!(answer.replace((start, length, text)).is_none());
            }
        }
        let (start, length, text) = answer.expect("no suggestion packet");
        assert_eq!((start, length, text.as_str()), (6, 2, "example:stone"));
        let mut applied = typed.to_owned();
        applied.replace_range(start as usize..(start + length) as usize, &text);
        assert_eq!(applied, "probe example:stone");
        let completed = format!("{applied} 3");
        let parsed = game.ui.dispatcher.0.parse(&completed, &game.ui.registries);
        let (context, _) = parsed.executable().unwrap();
        assert_eq!(
            context.get("value"),
            Some(&horizon_client::command::Arg::Str("example:stone".into()))
        );
    });
}

/// The `warp` tree and completions an operator gets, and a plain player does not: the admin child
/// literal and the private warp are absent until the level arrives, and running `warp admin` is
/// refused for the same reason in both states.
#[test]
#[ignore = "daily"]
fn a_plain_players_tree_and_completions_leave_out_the_paths_they_may_not_run() {
    let mut resources = None;
    let (mut game, rx) = game_with_guest(&mut resources, "Alex");
    install_warps(&mut game);
    assert_eq!(children(&last_tree(&rx), "warp"), ["go"]);
    assert_eq!(suggestions(&mut game, &rx, "warp "), ["go"]);
    assert_eq!(suggestions(&mut game, &rx, "warp go "), ["spawn"]);
    let at_player = Registries {
        permission: 0,
        ..Registries::default()
    };
    assert!(
        game.ui
            .dispatcher
            .0
            .parse("warp admin secret", &at_player)
            .executable()
            .is_err(),
        "running the admin path is refused, as it always was"
    );

    // `/op`: the operator list changes and the hub sends the trees again.
    let uuid = game.lan.as_ref().unwrap().hub.guests[0].uuid;
    {
        let host = game.lan.as_mut().unwrap();
        host.hub.ops.insert(uuid, 4);
        host.hub.resend_commands();
    }
    assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].permission, 4);
    assert_eq!(children(&last_tree(&rx), "warp"), ["admin", "go"]);
    assert_eq!(suggestions(&mut game, &rx, "warp "), ["admin", "go"]);
    assert_eq!(suggestions(&mut game, &rx, "warp go "), ["secret", "spawn"]);
    let at_owner = Registries {
        permission: 4,
        ..Registries::default()
    };
    assert!(
        game.ui
            .dispatcher
            .0
            .parse("warp admin secret", &at_owner)
            .executable()
            .is_ok(),
        "with the level, the path parses and runs"
    );
}
