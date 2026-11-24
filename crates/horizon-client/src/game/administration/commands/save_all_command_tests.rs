//! `SaveAllCommand.saveAll`: `commands.save.success` reaches the source and
//! the other operators (`sendSuccess(..., true)`), and a save that fails is
//! the `ERROR_FAILED` its source alone hears, with the command's result 0.
use super::*;
use crate::game::{dedicated, execution_role::ExecutionRole, lan::LanSettings};
use horizon_client::{
    lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig},
    server::user_lists::{BanList, IpBanList, OpList, WhiteList},
};
use horizon_protocol::packets::{configuration::ClientInformation, play::Clientbound};
use serde_json::Value;
use std::sync::{Arc, Mutex, atomic::AtomicI32, mpsc};

/// The components of the system chat lines a player's connection received.
fn chats(rx: &mpsc::Receiver<Out>) -> Vec<Value> {
    let mut lines = Vec::new();
    for sent in rx.try_iter() {
        let bodies = match sent {
            Out::Packet(p) => vec![p],
            Out::Packets(p) => p,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            if let Clientbound::SystemChat(p) = Clientbound::decode(id, reader.rest()).unwrap() {
                lines.push(horizon_client::scoreboard::tag_to_json(&p.content));
            }
        }
    }
    lines
}

/// The `[name: message]` lines `broadcastToAdmins` sends the operators.
fn admin_lines(lines: &[Value]) -> Vec<&Value> {
    lines
        .iter()
        .filter(|l| l["translate"] == "chat.type.admin")
        .collect()
}

/// A dedicated server of the world in `dir`, with the operators Alice (conn
/// 1) and Bob (conn 7): each one's connection, as its packets are sent.
fn server_with_two_operators(
    dir: &std::path::Path,
) -> (Game<'static>, mpsc::Receiver<Out>, mpsc::Receiver<Out>) {
    let mut game = crate::game::native_machines::tests::server_game();
    game.options.execution_role = ExecutionRole::DedicatedServer;
    game.world_root = dir.to_path_buf();
    game.save = Arc::new(horizon_client::save::SaveDir::new(
        dir.to_path_buf(),
        horizon_client::save::LevelShape::OVERWORLD,
    ));
    game.server_lists = Some(Arc::new(Mutex::new(dedicated::Lists {
        ops: OpList::new(dir.join("ops.json")),
        white: WhiteList::new(dir.join("whitelist.json")),
        bans: BanList::new(dir.join("banned-players.json")),
        ip_bans: IpBanList::new(dir.join("banned-ips.json")),
        use_whitelist: false,
        enforce_whitelist: false,
        op_level: 4,
        player_idle_timeout: 0,
        stop: false,
        autosave: true,
        whitelist_bypass: Vec::new(),
    })));
    game.set_allow_commands(true);
    // A dedicated server's tree holds `/save-all` (`Commands.includeDedicated`),
    // without changing the process-wide tree of unrelated tests.
    game.ui.dispatcher = crate::game::chat::SessionDispatcher(Arc::new(
        horizon_client::command::vanilla::build_for(false, true),
    ));
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "save-all-command".into(), 4)
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
            max_players: 4,
            integrated: true,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let mut receivers = Vec::new();
    for (conn, name) in [(1_u32, "Alice"), (7_u32, "Bob")] {
        let (tx, rx) = mpsc::channel();
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
            (1_000 + conn) as i32,
            [0.5, 80.0, 0.5],
            2,
        );
        guest.placed = true;
        guest.dimension = "minecraft:overworld".into();
        guest.permission = 4;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        receivers.push(rx);
    }
    let bob = receivers.pop().unwrap();
    let alice = receivers.pop().unwrap();
    (game, alice, bob)
}

fn save_all(game: &mut Game<'_>, conn: u32, quiet: bool) -> (bool, Vec<Text>) {
    game.run_guest_command_told(crate::game::lan_actions::GuestCommand {
        conn,
        command: "save-all".into(),
        permission: 4,
        integrated: true,
        quiet,
    })
    .expect("the operator is connected")
}

fn world(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("horizon-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `SaveAllCommand.saveAll`'s `source.sendSuccess(() ->
/// Component.translatable("commands.save.success"), true)`: the operator who
/// ran it and the other operators both hear that the game was saved.
#[test]
#[ignore = "daily"]
fn a_guests_save_all_reaches_the_other_operators() {
    let (mut game, alice, bob) = server_with_two_operators(&world("save-all-success"));
    let mut ran = None;
    // `broadcastToAdmins`: a player's source logs the line to a dedicated
    // server's console too (`log_admin_commands`).
    let console = dedicated::capture_log(|| ran = Some(save_all(&mut game, 1, false)));
    let (ok, lines) = ran.unwrap();
    assert!(ok, "the save went through: {lines:?}");
    assert!(
        lines
            .iter()
            .any(|l| l.translation_key() == Some("commands.save.success")),
        "the source keeps its own copy: {lines:?}"
    );
    assert!(
        admin_lines(&chats(&alice)).is_empty(),
        "the source is not also told its own command as an admin line"
    );
    let bob = chats(&bob);
    let [line] = admin_lines(&bob)[..] else {
        panic!("the other operator hears the save, once: {bob:?}");
    };
    assert_eq!(line["with"][0]["text"], "Alice", "{line}");
    assert_eq!(
        line["with"][1]["translate"], "commands.save.success",
        "{line}"
    );
    assert_eq!(console, ["System chat: [Alice: Saved the game]"]);
}

/// `SaveAllCommand.saveAll`'s `throw ERROR_FAILED.create()`: a save that does
/// not go through fails the command (result 0) and is the source's red line
/// alone, not a message every operator gets.
#[test]
#[ignore = "daily"]
fn a_failed_save_fails_at_the_source_only() {
    let dir = world("save-all-failure");
    let (mut game, alice, bob) = server_with_two_operators(&dir);
    // level.dat cannot be written: its folder is a file.
    let blocked = dir.join("blocked");
    std::fs::write(&blocked, b"").unwrap();
    game.world_root = blocked;
    let (ok, lines) = save_all(&mut game, 1, false);
    assert!(!ok, "a failed save is a failed command: {lines:?}");
    assert!(
        lines
            .iter()
            .any(|l| l.translation_key() == Some("commands.save.failed")),
        "the command's own lines hold the failure: {lines:?}"
    );
    let alice_lines = chats(&alice);
    assert!(
        alice_lines
            .iter()
            .any(|l| l["translate"] == "commands.save.failed"),
        "the source hears the failure: {alice_lines:?}"
    );
    let bob_lines = chats(&bob);
    assert!(
        bob_lines
            .iter()
            .all(|l| l["translate"] != "commands.save.failed"),
        "no other operator is told of the failure: {bob_lines:?}"
    );
    assert!(admin_lines(&bob_lines).is_empty(), "{bob_lines:?}");
    // A silent source (`withSuppressedOutput`, a mod's quiet batch) hears
    // nothing and tells nobody.
    let (ok, lines) = save_all(&mut game, 1, true);
    assert!(!ok, "a failed save is a failed command: {lines:?}");
    assert!(lines.is_empty(), "a silent source hears nothing: {lines:?}");
    let (quiet_alice, quiet_bob) = (chats(&alice), chats(&bob));
    assert!(
        quiet_alice.is_empty() && quiet_bob.is_empty(),
        "a silent source tells nobody: {quiet_alice:?} {quiet_bob:?}"
    );
    // The same at a source that is not a guest: the command returns 0.
    let failed = game.cmd_save_all();
    let message = failed.expect_err("a save that fails throws ERROR_FAILED");
    assert_eq!(
        message.message.translation_key(),
        Some("commands.save.failed")
    );
}

/// `SaveAllCommand.saveAll` runs once per source: `/execute as @a run
/// save-all` in a guest's turn saves once, and every execution that asked for
/// it keeps its own success line and its own admin line.
#[test]
#[ignore = "daily"]
fn every_source_of_a_guests_save_all_keeps_its_feedback() {
    let (mut game, _alice, bob) = server_with_two_operators(&world("save-all-two-sources"));
    let mut ran = None;
    let console = dedicated::capture_log(|| {
        ran = Some(
            game.run_guest_command_told(crate::game::lan_actions::GuestCommand {
                conn: 1,
                command: "execute as @a run save-all".into(),
                permission: 4,
                integrated: true,
                quiet: false,
            })
            .expect("the operator is connected"),
        );
    });
    let (ok, lines) = ran.unwrap();
    assert!(ok, "the save went through: {lines:?}");
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.translation_key() == Some("commands.save.success"))
            .count(),
        2,
        "each source keeps its own copy: {lines:?}"
    );
    let bob_lines = chats(&bob);
    let admins = admin_lines(&bob_lines);
    assert_eq!(
        admins.len(),
        2,
        "each source's save reaches the other operator: {bob_lines:?}"
    );
    assert_eq!(admins[0]["with"][0]["text"], "Alice", "{}", admins[0]);
    assert_eq!(admins[1]["with"][0]["text"], "Bob", "{}", admins[1]);
    assert_eq!(
        console,
        [
            "System chat: [Alice: Saved the game]",
            "System chat: [Bob: Saved the game]"
        ]
    );
}

/// `CommandSourceStack.sendSuccess` and `broadcastToAdmins` read the feedback
/// rules when the command runs, not when its deferred save finishes: a
/// `/save-all` that the same turn then turns `send_command_feedback` off still
/// tells its source and the other operators that the game was saved.
#[test]
#[ignore = "daily"]
fn a_save_all_keeps_the_feedback_rule_of_its_command() {
    let (mut game, _alice, bob) = server_with_two_operators(&world("save-all-feedback-rule"));
    // What `/save-all` records while it runs, with the rule on...
    let deferred = game.deferred_save_all();
    // ...and the rest of the turn turning it off.
    assert!(
        game.ui.rules.set(
            "send_command_feedback",
            horizon_client::gamerules::Value::Bool(false)
        )
    );
    assert!(!game.accepts_success(), "the rule is off at the turn's end");
    let mut lines = Vec::new();
    assert!(
        game.finish_deferred_save_all(vec![deferred], &mut lines),
        "the save went through: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.translation_key() == Some("commands.save.success")),
        "the source still hears the save: {lines:?}"
    );
    game.send_admin_feedback(None);
    let bob_lines = chats(&bob);
    let [line] = admin_lines(&bob_lines)[..] else {
        panic!("the other operator still hears the save, once: {bob_lines:?}");
    };
    assert_eq!(
        line["with"][1]["translate"], "commands.save.success",
        "{line}"
    );
    // A `/save-all` that runs with the rule off reports nothing: the decision
    // is the command's.
    let deferred = game.deferred_save_all();
    let mut lines = Vec::new();
    assert!(game.finish_deferred_save_all(vec![deferred], &mut lines));
    assert!(
        lines.is_empty(),
        "the rule was off when the command ran: {lines:?}"
    );
    game.send_admin_feedback(None);
    assert!(
        admin_lines(&chats(&bob)).is_empty(),
        "the rule was off when the command ran"
    );
}
