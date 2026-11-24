//! `/kick` and `/ban-ip` name their players as vanilla does
//! (`Player.getDisplayName`: the team's formatting, the `/tell` click, the
//! entity hover and the name a shift-click inserts), and `IpBanList.isBanned`
//! is `StoredUserList.contains`: an entry with an `expires` date that has
//! passed still bans its address until it is removed.
use super::*;
use crate::game::native_machines::tests::server_game;
use horizon_client::server::user_lists::{BanList, IpBanList, OpList, WhiteList};
use std::sync::{Arc, Mutex};

/// A hosted world with one guest, "Guest", at 1.2.3.4.
fn host_game() -> Game<'static> {
    let mut game = server_game();
    game.build_registries();
    game.set_allow_commands(true);
    let _rx = crate::game::lan::world_spawn_sync_tests::host_for(&mut game);
    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
        guest.name = "Guest".into();
        guest.address = "1.2.3.4:25565".parse().unwrap();
    });
    game
}

/// `DedicatedPlayerList`'s ban files in a scratch folder.
fn lists(dir: &std::path::Path) -> Arc<Mutex<crate::game::dedicated::Lists>> {
    Arc::new(Mutex::new(crate::game::dedicated::Lists {
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
    }))
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("horizon-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One dedicated command, parsed from the dedicated tree (the process-wide one
/// stays integrated for the other tests).
fn run_dedicated(game: &mut Game<'_>, command: &str) -> R<i32> {
    game.options.execution_role =
        crate::game::execution_role::ExecutionRole::DedicatedServer;
    let dispatcher = horizon_client::command::vanilla::build_for(false, true);
    game.ui.registries.permission = 4;
    let parsed = dispatcher.parse(command, &game.ui.registries);
    let (ctx, _) = parsed.executable().expect("a dedicated command");
    let (root, ctx) = (parsed.context.clone(), ctx.clone());
    game.run_chain(&root, &ctx)
}

fn on_a_red_team(game: &mut Game<'_>) {
    assert!(game.run_command("team add red"));
    assert!(game.run_command("team modify red color red"));
    assert!(game.run_command("team join red Guest"));
}

#[test]
#[ignore = "daily"]
fn kick_names_the_player_as_their_team_does() {
    let mut game = host_game();
    on_a_red_team(&mut game);
    game.chat_capture = Some(Vec::new());
    assert!(game.run_command("kick Guest"));
    let lines = game.chat_capture.take().unwrap();
    let line = lines
        .iter()
        .find(|l| l.translation_key() == Some("commands.kick.success"))
        .unwrap_or_else(|| panic!("no kick reply in {lines:?}"));
    // KickCommand.kickPlayers: player.getDisplayName().
    let shown = &line.component()["with"][0];
    assert_eq!(shown["color"], "red", "{shown}");
    // PlayerTeam.formatNameForTeam: the team's prefix, the name, its suffix.
    assert_eq!(shown["extra"][1]["text"], "Guest", "{shown}");
    assert_eq!(shown["click_event"]["command"], "/tell Guest ", "{shown}");
    assert_eq!(
        shown["hover_event"]["uuid"],
        horizon_protocol::crypto::uuid_string(1),
        "{shown}"
    );
}

#[test]
#[ignore = "daily"]
fn ban_ip_names_the_players_it_kicks_as_their_team_does() {
    let mut game = host_game();
    let dir = scratch("ban-ip-names");
    game.server_lists = Some(lists(&dir));
    on_a_red_team(&mut game);
    game.chat_capture = Some(Vec::new());
    assert_eq!(run_dedicated(&mut game, "ban-ip 1.2.3.4").unwrap(), 1);
    let lines = game.chat_capture.take().unwrap();
    let line = lines
        .iter()
        .find(|l| l.translation_key() == Some("commands.banip.info"))
        .unwrap_or_else(|| panic!("no ban-ip info line in {lines:?}"));
    // BanIpCommands.banIp: EntitySelector.joinNames(players), which is
    // ComponentUtils.formatList(players, Entity::getDisplayName): one player
    // is their display name itself, not a component wrapping it.
    let shown = &line.component()["with"][1];
    assert_eq!(shown["color"], "red", "{shown}");
    assert_eq!(shown["extra"][1]["text"], "Guest", "{shown}");
    assert_eq!(shown["click_event"]["command"], "/tell Guest ", "{shown}");
    assert_eq!(
        shown["hover_event"]["uuid"],
        horizon_protocol::crypto::uuid_string(1),
        "{shown}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "daily"]
fn an_expired_ip_ban_still_bans_the_address_until_it_is_removed() {
    let mut game = host_game();
    let dir = scratch("ban-ip-expired");
    std::fs::write(
        dir.join("banned-ips.json"),
        r#"[{"ip": "1.2.3.4", "created": "2020-01-01 00:00:00 +0000", "source": "Server", "expires": "2020-01-02 00:00:00 +0000"}]"#,
    )
    .unwrap();
    let lists = lists(&dir);
    lists.lock().unwrap().ip_bans.load().unwrap();
    game.server_lists = Some(lists);
    // BanIpCommands.banIp: `list.isBanned(ip)` is `StoredUserList.contains`,
    // which does not drop expired entries the way `get` does.
    let failed = run_dedicated(&mut game, "ban-ip 1.2.3.4").expect_err("already banned");
    assert_eq!(failed.key, "commands.banip.failed");
    let file = std::fs::read_to_string(dir.join("banned-ips.json")).unwrap();
    assert!(
        file.contains("2020-01-02"),
        "the entry is left as it was: {file}"
    );
    // PardonIpCommand.unban sees it too, and removes it.
    assert_eq!(run_dedicated(&mut game, "pardon-ip 1.2.3.4").unwrap(), 1);
    let file = std::fs::read_to_string(dir.join("banned-ips.json")).unwrap();
    assert!(!file.contains("1.2.3.4"), "{file}");
    std::fs::remove_dir_all(&dir).ok();
}
