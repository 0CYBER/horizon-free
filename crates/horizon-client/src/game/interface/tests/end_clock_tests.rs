//! `/time` addresses the clock the End runs on: the one `World.end_clock`
//! ticks and `world_clocks.dat` saves (`ServerClockManager.getInstance`).
use crate::game::{Audio, Dim, Game, Options, Progress, Settings, execution_role, load_resources};
use crate::save::Level;

fn game() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
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
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
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
    // The player stands in the End, so `/time` without `of` names its clock.
    game.dim = Dim::End;
    game
}

/// `/time <line>`; the feedback it printed.
fn run(game: &mut Game<'_>, line: &str) -> String {
    game.chat_capture = Some(Vec::new());
    assert!(game.run_command(&format!("time {line}")), "/time {line}");
    let lines = game.chat_capture.take().unwrap();
    lines.last().expect("feedback").plain()
}

#[test]
#[ignore = "daily"]
fn time_moves_the_clock_the_end_runs_on() {
    let mut game = game();
    let world = std::env::temp_dir().join(format!("horizon-end-clock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    game.world_root = world.clone();
    // `DimensionTypes`: the End's dimension type's default clock.
    assert_eq!(
        game.dimension_type().default_clock.as_deref(),
        Some("minecraft:the_end")
    );
    assert_eq!(
        run(&mut game, "set 5000"),
        "Set minecraft:the_end to 5000 tick(s)"
    );
    // `/time` and the End's own clock are one instance, as in vanilla.
    assert_eq!(game.world.end_clock, 5_000);
    assert_eq!(
        run(&mut game, "query time"),
        "Clock minecraft:the_end is at 5000 tick(s)"
    );
    // It grows each tick the End's clock advances.
    game.advance_clock();
    game.advance_clock();
    assert_eq!(game.world.end_clock, 5_002);
    assert_eq!(
        run(&mut game, "query time"),
        "Clock minecraft:the_end is at 5002 tick(s)"
    );
    assert_eq!(
        run(&mut game, "add 1200"),
        "Set minecraft:the_end to 6202 tick(s)"
    );
    // `/time pause` stops that same clock, and `resume` starts it again.
    assert_eq!(run(&mut game, "pause"), "Paused clock minecraft:the_end");
    for _ in 0..5 {
        game.advance_clock();
    }
    assert_eq!(game.world.end_clock, 6_202);
    assert_eq!(run(&mut game, "resume"), "Resumed clock minecraft:the_end");
    game.advance_clock();
    assert_eq!(game.world.end_clock, 6_203);
    // The clock and its pause survive a save: vanilla packs every clock into
    // world_clocks.dat, and the world opens with them.
    assert_eq!(
        run(&mut game, "set 400"),
        "Set minecraft:the_end to 400 tick(s)"
    );
    assert_eq!(run(&mut game, "pause"), "Paused clock minecraft:the_end");
    game.sync_level();
    game.level.write(&world).unwrap();
    game.save_world_state();
    let level = Level::read(&world).unwrap();
    assert_eq!(level.end_clock, Some(400));
    game.level = level;
    game.world.end_clock = game.level.end_clock.unwrap_or(game.level.time);
    game.load_world_state().unwrap();
    assert_eq!(
        run(&mut game, "query time"),
        "Clock minecraft:the_end is at 400 tick(s)"
    );
    game.advance_clock();
    assert_eq!(game.world.end_clock, 400, "the pause came back too");
    assert_eq!(run(&mut game, "resume"), "Resumed clock minecraft:the_end");
    game.advance_clock();
    assert_eq!(game.world.end_clock, 401);
    let _ = std::fs::remove_dir_all(&world);
}
