//! `CommandSourceStack.broadcastToAdmins` (`CommandSourceStack.java:487-497`)
//! sends a command's `[name: ...]` line to the players `PlayerList.isOp`
//! names, and `PlayerList.isOp` (`PlayerList.java:587-593`) asks whether the
//! player's `ops.json` entry exists, never which level it carries: a level-0
//! operator hears another operator's command. On an integrated server the
//! owner is the only operator (`IntegratedServer.getProfilePermissions`),
//! whatever permission level `Allow Commands` gives the guests, and
//! splitscreen seats are no `ops.json` entries (`join_split`).
use crate::game::Game;
use crate::game::lan::LanSettings;
use crate::game::lan_actions::GuestCommand;
use horizon_client::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
use horizon_protocol::packets::configuration::ClientInformation;
use horizon_protocol::packets::play::Clientbound as C;
use horizon_core::nbt::Tag;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const ISSUER: u32 = 2;
const LEVEL_ZERO_OP: u32 = 3;
const BYSTANDER: u32 = 4;

/// A guest whose packets land in a channel, at `permission`.
fn guest(conn: u32, name: &str, permission: u8, at: [f64; 3]) -> (Guest, mpsc::Receiver<Out>) {
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn,
            uuid: 100 + u128::from(conn),
            name: name.into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        (1_000 + conn) as i32,
        at,
        2,
    );
    guest.placed = true;
    guest.loaded = true;
    guest.dimension = "minecraft:overworld".into();
    guest.permission = permission;
    (guest, rx)
}

fn string(tag: &Tag) -> Option<String> {
    match tag {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// The message of each `chat.type.admin` line a guest was sent: the `with`
/// argument after the command source's display name.
fn admin_messages(rx: &mpsc::Receiver<Out>) -> Vec<String> {
    let mut messages = Vec::new();
    for out in rx.try_iter() {
        let bodies = match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            if let C::SystemChat(packet) = C::decode(id, reader.rest()).unwrap()
                && packet.content.get("translate").and_then(string).as_deref()
                    == Some("chat.type.admin")
            {
                let Some(Tag::List(arguments)) = packet.content.get("with") else {
                    panic!("chat.type.admin without its arguments");
                };
                let inner = &arguments[1];
                messages.push(
                    inner
                        .get("translate")
                        .and_then(string)
                        .or_else(|| inner.get("text").and_then(string))
                        .expect("the message of an admin line"),
                );
            }
        }
    }
    messages
}

fn host(game: &mut Game<'_>, integrated: bool, max_players: usize, setup: &[&str]) {
    game.build_registries();
    game.set_allow_commands(true);
    game.block_ticks
        .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
    for line in setup {
        assert!(game.run_command(line), "{line}");
    }
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "admin-feedback-test".into(), max_players)
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
            max_players,
            integrated,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
}

/// `/gamemode`: a level-2 command whose success vanilla broadcasts.
const ADMIN_LINE: &str = "commands.gamemode.success.self";

fn change_game_mode(game: &mut Game<'_>, conn: u32, permission: u8, integrated: bool) {
    let ran = game
        .run_guest_command_told(GuestCommand {
            conn,
            command: "gamemode creative".into(),
            permission,
            integrated,
            quiet: false,
        })
        .expect("the guest is connected")
        .0;
    assert!(ran, "gamemode creative runs for a level-2 command source");
}

#[test]
#[ignore = "daily"]
fn a_level_zero_operator_hears_another_operators_command() {
    let mut game = crate::game::native_machines::tests::server_game();
    host(&mut game, false, 3, &[]);
    let mut host = game.lan.take().unwrap();
    // ops.json: the issuer at level 4, the second operator at level 0 (what
    // Horizon's own `/op` writes), and a player who is no operator.
    host.hub.ops.insert(100 + u128::from(ISSUER), 4);
    host.hub.ops.insert(100 + u128::from(LEVEL_ZERO_OP), 0);
    let (issuer, issuer_rx) = guest(ISSUER, "OperatorA", 4, [8.5, 200.0, 8.5]);
    let (level_zero, level_zero_rx) = guest(LEVEL_ZERO_OP, "OperatorB", 0, [9.5, 200.0, 9.5]);
    let (bystander, bystander_rx) = guest(BYSTANDER, "Bystander", 0, [10.5, 200.0, 10.5]);
    host.hub.guests.extend([issuer, level_zero, bystander]);
    game.lan = Some(host);
    change_game_mode(&mut game, ISSUER, 4, false);

    // PlayerList.isOp: the entry exists, so the level-0 operator hears it.
    assert_eq!(
        admin_messages(&level_zero_rx),
        [ADMIN_LINE.to_owned()],
        "a level-0 operator hears another operator's command"
    );
    // A player who is no operator hears nothing, and the issuer not their own.
    assert!(
        admin_messages(&bystander_rx).is_empty(),
        "a player who is no operator hears no admin line"
    );
    assert!(
        admin_messages(&issuer_rx).is_empty(),
        "the command's own source never hears its admin line"
    );
}

#[test]
#[ignore = "daily"]
fn only_the_owner_hears_an_integrated_servers_guest_command() {
    let mut game = crate::game::native_machines::tests::server_game();
    host(&mut game, true, 3, &[]);
    let mut host = game.lan.take().unwrap();
    // IntegratedServer.getProfilePermissions: the owner is the operator while
    // the world allows commands; the guests' command access (GAMEMASTER) does
    // not list them in `ops`.
    let owner = 100 + u128::from(ISSUER);
    host.hub.ops.insert(owner, 4);
    host.hub.singleplayer_owner = Some(owner);
    let (issuer, issuer_rx) = guest(ISSUER, "Owner", 4, [8.5, 200.0, 8.5]);
    let (level_two, level_two_rx) = guest(LEVEL_ZERO_OP, "GuestA", 2, [9.5, 200.0, 9.5]);
    let (other, other_rx) = guest(BYSTANDER, "GuestB", 2, [10.5, 200.0, 10.5]);
    host.hub.guests.extend([issuer, level_two, other]);
    game.lan = Some(host);
    change_game_mode(&mut game, LEVEL_ZERO_OP, 2, true);

    // The owner hears it, being the only operator of an integrated server.
    assert_eq!(
        admin_messages(&issuer_rx),
        [ADMIN_LINE.to_owned()],
        "the owner hears a guest's command"
    );
    // A guest with command access is no operator: no admin line reaches it.
    assert!(
        admin_messages(&other_rx).is_empty(),
        "a guest with command access hears no admin line"
    );
    assert!(
        admin_messages(&level_two_rx).is_empty(),
        "the command's own source never hears its admin line"
    );
}

#[test]
#[ignore = "daily"]
fn an_integrated_server_without_commands_tells_no_one() {
    let mut game = crate::game::native_machines::tests::server_game();
    // `/trigger` is a level-0 command whose success vanilla broadcasts
    // (`TriggerCommand.sendSuccess(..., true)`), so a guest without command
    // access can queue an admin line.
    host(
        &mut game,
        true,
        2,
        &[
            "scoreboard objectives add t trigger",
            "scoreboard players enable GuestA t",
        ],
    );
    let mut host = game.lan.take().unwrap();
    // IntegratedServer.setWorldAllowCommands(false): the owner keeps an
    // `ops` entry, at level 0, and the guests drop to ALL. `PlayerList.isOp`
    // names no operator then (its owner clause needs allowCommands), so
    // nobody hears the line.
    let owner = 100 + u128::from(ISSUER);
    host.hub.ops.insert(owner, 0);
    host.hub.singleplayer_owner = Some(owner);
    host.hub.config.guest_commands = false;
    let (owner_guest, owner_rx) = guest(ISSUER, "Owner", 0, [8.5, 200.0, 8.5]);
    let (level_zero, level_zero_rx) = guest(LEVEL_ZERO_OP, "GuestA", 0, [9.5, 200.0, 9.5]);
    host.hub.guests.extend([owner_guest, level_zero]);
    game.lan = Some(host);
    let ran = game
        .run_guest_command_told(GuestCommand {
            conn: LEVEL_ZERO_OP,
            command: "trigger t set 5".into(),
            permission: 0,
            integrated: true,
            quiet: false,
        })
        .expect("the guest is connected")
        .0;
    assert!(ran, "a level-0 command runs without command access");

    assert!(
        admin_messages(&owner_rx).is_empty(),
        "a world without commands has no operator to tell"
    );
    assert!(
        admin_messages(&level_zero_rx).is_empty(),
        "the command's own source never hears its admin line"
    );
}

#[test]
#[ignore = "daily"]
fn a_level_zero_splitscreen_seat_hears_no_admin_line() {
    let mut game = crate::game::native_machines::tests::server_game();
    // One splitscreen seat runs a level-0 command (`/trigger`) while another
    // seat, both at the owner's level 0 without commands, listens.
    host(
        &mut game,
        true,
        3,
        &[
            "scoreboard objectives add t trigger",
            "scoreboard players enable SeatA t",
        ],
    );
    let mut host = game.lan.take().unwrap();
    let owner = 100 + u128::from(ISSUER);
    let issuer = 100 + u128::from(LEVEL_ZERO_OP);
    let listener = 100 + u128::from(BYSTANDER);
    // IntegratedServer.setWorldAllowCommands(false) and join_split: the owner
    // and both seats have level-0 `ops` entries, but only the owner's comes
    // from the world; the seats' entries are no `ops.json` entries, so
    // `PlayerList.isOp` names neither.
    host.hub.ops.insert(owner, 0);
    host.hub.singleplayer_owner = Some(owner);
    host.hub.ops.extend([(issuer, 0), (listener, 0)]);
    host.hub.split_players.extend([issuer, listener]);
    host.hub.config.guest_commands = false;
    let (issuer_guest, issuer_rx) = guest(LEVEL_ZERO_OP, "SeatA", 0, [8.5, 200.0, 8.5]);
    let (listener_guest, listener_rx) = guest(BYSTANDER, "SeatB", 0, [9.5, 200.0, 9.5]);
    host.hub.guests.extend([issuer_guest, listener_guest]);
    game.lan = Some(host);
    let ran = game
        .run_guest_command_told(GuestCommand {
            conn: LEVEL_ZERO_OP,
            command: "trigger t set 5".into(),
            permission: 0,
            integrated: true,
            quiet: false,
        })
        .expect("the guest is connected")
        .0;
    assert!(ran, "a level-0 command runs without command access");

    assert!(
        admin_messages(&listener_rx).is_empty(),
        "a level-zero splitscreen seat hears no admin line"
    );
    assert!(
        admin_messages(&issuer_rx).is_empty(),
        "the command's own source never hears its admin line"
    );
}
