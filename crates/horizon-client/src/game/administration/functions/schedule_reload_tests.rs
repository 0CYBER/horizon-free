//! The schedule is live state, not pack content: `TimerQueue` is an in-memory
//! `SavedData` whose firing (`TimerQueue.tick`) and `clear` (`TimerQueue.remove`)
//! drop the event from the queue, and a reload only replaces the function
//! library, so neither a fired nor a cleared event may come back. A pending
//! one is still read from the world's saved file when the world is opened.
use super::*;

/// A server game whose world is a fresh folder with one pack holding
/// `hs:hello`, which counts its runs in the `hs` objective.
fn schedule_game<'a>(
    label: &str,
    resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>,
) -> (Game<'a>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "horizon-schedule-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    let pack = dir.join("datapacks/hs");
    let functions = pack.join("data/hs/function");
    std::fs::create_dir_all(&functions).unwrap();
    let (major, minor) = horizon_core::version::DATA_PACK_FORMAT;
    std::fs::write(
        pack.join("pack.mcmeta"),
        serde_json::json!({"pack": {
            "description": "Scheduled events reload test", "pack_format": major,
            "min_format": [major, minor], "max_format": [major, minor],
        }})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        functions.join("hello.mcfunction"),
        "scoreboard players add fired hs 1\n",
    )
    .unwrap();
    let mut game = crate::game::game_tests::tests::server_game(resources);
    game.world_root = dir.clone();
    game.build_registries();
    game.set_allow_commands(true);
    game.reload_datapacks(false, &game.level.enabled_features.clone())
        .unwrap();
    assert!(game.fns.enabled.contains(&"file/hs".to_owned()));
    assert!(
        game.fns.store.errors.is_empty(),
        "{:?}",
        game.fns.store.errors
    );
    game.run_command("scoreboard objectives add hs dummy");
    (game, dir)
}

fn run(game: &mut Game<'_>, line: &str) {
    game.exec = ExecState::default();
    let source = game.player_source();
    game.execute_line(line, source).unwrap();
}

fn fired(game: &Game<'_>) -> i32 {
    game.score.board.score("fired", "hs").map_or(0, |s| s.value)
}

#[test]
#[ignore = "daily"]
fn a_fired_or_cleared_event_stays_gone_across_a_reload() {
    let mut resources = None;
    let (mut game, dir) = schedule_game("gone", &mut resources);
    // A fired event: saved while pending, then run, then reloaded.
    run(&mut game, "schedule function hs:hello 5t");
    game.save_schedule();
    let due = game.world.game_time + 5;
    game.world.game_time = due;
    game.run_scheduled_functions();
    assert_eq!(fired(&game), 1);
    assert!(game.fns.schedule.is_empty());
    run(&mut game, "reload");
    game.run_scheduled_functions();
    assert_eq!(fired(&game), 1, "a fired event must not come back");
    // A cleared event: saved while pending, then `/schedule clear`-ed.
    run(&mut game, "schedule function hs:hello 100t");
    game.save_schedule();
    run(&mut game, "schedule clear hs:hello");
    assert!(game.fns.schedule.is_empty());
    run(&mut game, "reload");
    game.world.game_time = due + 100;
    game.run_scheduled_functions();
    assert_eq!(fired(&game), 1, "a cleared event must not come back");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn a_pending_event_survives_a_save_and_a_reload() {
    let mut resources = None;
    let (mut game, dir) = schedule_game("pending", &mut resources);
    run(&mut game, "schedule function hs:hello 40t");
    game.save_schedule();
    let due = game.world.game_time + 40;
    // A reload keeps the live queue: the event still fires at its time.
    run(&mut game, "reload");
    assert_eq!(game.fns.schedule.len(), 1);
    game.world.game_time = due;
    game.run_scheduled_functions();
    assert_eq!(fired(&game), 1);
    assert!(game.fns.schedule.is_empty());
    // A pending event of a saved world is read back when a session opens it.
    run(&mut game, "schedule function hs:hello 60t");
    game.save_schedule();
    let due = game.world.game_time + 60;
    game.fns = Default::default();
    game.ensure_datapacks();
    let loaded: Vec<_> = game
        .fns
        .schedule
        .iter()
        .map(|(id, time, _)| (id.to_owned(), time))
        .collect();
    assert_eq!(loaded, vec![("hs:hello".to_owned(), due)]);
    game.world.game_time = due;
    game.run_scheduled_functions();
    assert_eq!(fired(&game), 2);
    std::fs::remove_dir_all(dir).unwrap();
}
