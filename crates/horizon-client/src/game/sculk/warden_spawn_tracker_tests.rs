//! `/warden_spawn_tracker` (`WardenSpawnTrackerCommand`): it edits the tracker
//! `ServerPlayer.addAdditionalSaveData` stores as `warden_spawn_tracker`
//! (`ServerPlayer.java:421`, codec `WardenSpawnTracker.java:18-24`), it edits
//! only the source player (`getPlayerOrException`) and it names that player
//! (`player.getDisplayName()`).
use super::*;
use crate::game::commands::{R, execute::Source};
use crate::game::native_machines::tests::server_game;
use horizon_client::{command::vanilla, motion::PlayerMotion};
use horizon_world::nbt::Tag;

/// A server's game with the window's own player standing at the world spawn,
/// or `None` without prepared resources.
fn game() -> Option<Game<'static>> {
    std::env::var_os("HORIZON_TEST_RESOURCES")?;
    let mut game = server_game();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.build_registries();
    game.set_allow_commands(true);
    game.ui.registries.permission = 4;
    Some(game)
}

/// Runs a line of the dev tree (`/warden_spawn_tracker` is registered only
/// there, `SharedConstants.DEBUG_DEV_COMMANDS`) with `source`.
fn run_with(game: &mut Game<'_>, line: &str, source: Source) -> R<i32> {
    let d = vanilla::build_with(true);
    let parse = d.parse(line, &game.ui.registries);
    let (ctx, _) = parse.executable().expect(line);
    let (ctx, root) = (ctx.clone(), parse.context.clone());
    game.with_source(source, |g| g.run_chain(&root, &ctx))
}

/// The same, as the player's own command (no `execute` source).
fn run(game: &mut Game<'_>, line: &str) -> R<i32> {
    let d = vanilla::build_with(true);
    let parse = d.parse(line, &game.ui.registries);
    let (ctx, _) = parse.executable().expect(line);
    let (ctx, root) = (ctx.clone(), parse.context.clone());
    game.run_chain(&root, &ctx)
}

/// `warden_spawn_tracker` as `/data get entity` reads it and as the player's
/// file is written from: the tracker after `set 2`, then after `clear`.
#[test]
fn the_warden_spawn_tracker_is_in_the_players_saved_data() {
    let Some(mut game) = game() else { return };
    assert_eq!(run(&mut game, "warden_spawn_tracker set 2").unwrap(), 1);
    let set = WardenSpawnTracker {
        ticks_since_last_warning: 0,
        warning_level: 2,
        cooldown_ticks: 0,
    }
    .to_nbt();
    // `NbtPredicate.getEntityTagToCompare`: what `/data get entity @s` and
    // `nbt=` read.
    let tag = game.player_nbt().expect("a player");
    assert_eq!(tag.get("warden_spawn_tracker"), Some(&set));
    // The save the game makes before it writes the player's file.
    game.save_warden_tracker();
    assert_eq!(
        game.level.player_tag().get("warden_spawn_tracker"),
        Some(&set)
    );
    // `clear` stores the zeroed tracker, as vanilla stores it always.
    assert_eq!(run(&mut game, "warden_spawn_tracker clear").unwrap(), 1);
    let cleared = Tag::compound([
        ("ticks_since_last_warning", Tag::Int(0)),
        ("warning_level", Tag::Int(0)),
        ("cooldown_ticks", Tag::Int(0)),
    ]);
    assert_eq!(
        game.player_nbt().unwrap().get("warden_spawn_tracker"),
        Some(&cleared)
    );
}

/// Only the source player's tracker: a mob has none, so `execute as <mob>`
/// drops the failure (`CommandSourceStack.handleError` forked) and the server
/// source reports it.
#[test]
fn the_warden_spawn_tracker_edits_only_the_source_player() {
    let Some(mut game) = game() else { return };
    assert_eq!(run(&mut game, "warden_spawn_tracker set 3").unwrap(), 1);
    assert!(run(&mut game, "summon minecraft:pig 4.5 64 4.5 {NoAI:1b}").is_ok());
    assert_eq!(
        run(
            &mut game,
            "execute as @e[type=minecraft:pig] run warden_spawn_tracker clear"
        )
        .unwrap(),
        0
    );
    assert_eq!(game.sculk.tracker.warning_level, 3);
    // The server's own source is not forked: the failure is reported.
    let source = game.server_source();
    let err = run_with(&mut game, "warden_spawn_tracker clear", source)
        .expect_err("the server is not a player");
    assert!(
        format!("{err:?}").contains("permissions.requires.player"),
        "{err:?}"
    );
    assert_eq!(game.sculk.tracker.warning_level, 3);
}

/// The reply carries the source player's `getDisplayName`, not a literal: the
/// LAN owner's name, with the `/tell` suggestion, hover and insertion.
#[test]
fn the_warden_spawn_tracker_names_the_actor() {
    let Some(mut game) = game() else { return };
    // The LAN owner Alice, as `as_guest` puts them in place.
    game.lan_acting_uuid = Some(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
    game.lan_acting_name = Some("Alice".into());
    game.chat_capture = Some(Vec::new());
    assert_eq!(run(&mut game, "warden_spawn_tracker set 1").unwrap(), 1);
    let said = game.chat_capture.take().unwrap();
    let message = said[0].component();
    assert_eq!(
        message["translate"],
        "commands.warden_spawn_tracker.set.success.single"
    );
    assert_eq!(message["with"][0]["text"], "Alice", "{message}");
    assert_eq!(message["with"][0]["insertion"], "Alice", "{message}");
    // A silent source (`/function`, `run_command_quietly`) gets nothing, as
    // `sendSuccess` drops it (`CommandSourceStack.sendSuccess`'s `silent`).
    game.chat_capture = Some(Vec::new());
    let quiet = Source {
        silent: true,
        ..game.player_source()
    };
    assert_eq!(
        run_with(&mut game, "warden_spawn_tracker set 2", quiet).unwrap(),
        1
    );
    assert!(game.chat_capture.as_ref().unwrap().is_empty());
}
