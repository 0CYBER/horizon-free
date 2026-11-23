//! `execute store ... entity` of a number into `Tags` or `CustomName`: vanilla 26.3 fails to
//! decode the field and drops it, so the entity ends up with no tags and no name.
use super::*;
use horizon_client::chunk::Chunk;
use std::sync::Arc;

fn game(resources: &mut Option<Arc<Resources>>) -> Game<'_> {
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
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let res = resources.insert(loaded.resources);
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game
}

#[test]
#[ignore = "daily"]
fn a_number_stored_into_tags_or_custom_name_is_dropped() {
    let mut resources = None;
    let mut game = game(&mut resources);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    assert!(game.run_command("forceload add 0 0 15 15"));
    assert!(game.run_command("scoreboard objectives add t dummy"));
    assert!(game.run_command("summon marker 8.5 80 8.5 {Tags:[\"m1\"],CustomName:\"keep\"}"));
    let found = |game: &mut Game, field: &str| {
        let cmd = format!(
            "execute store success score #r t run data get entity @e[type=marker,limit=1] {field}"
        );
        game.run_command(&cmd);
        game.score.board.score("#r", "t").unwrap().value
    };
    assert_eq!(found(&mut game, "Tags"), 1);
    assert_eq!(found(&mut game, "CustomName"), 1);

    assert!(game.run_command(
        "execute store result entity @e[type=marker,limit=1] Tags[0] int 1 run say hi"
    ));
    assert_eq!(found(&mut game, "Tags"), 0);

    assert!(game.run_command(
        "execute store result entity @e[type=marker,limit=1] CustomName int 1 run say hi"
    ));
    assert_eq!(found(&mut game, "CustomName"), 0);

    // The elements that are still strings stay: ["a","b"] with a number in the
    // first slot ends as ["b"].
    assert!(game.run_command("kill @e[type=marker]"));
    assert!(game.run_command("summon marker 8.5 80 8.5 {Tags:[\"a\",\"b\"]}"));
    assert!(game.run_command(
        "execute store result entity @e[type=marker,limit=1] Tags[0] int 1 run say hi"
    ));
    assert_eq!(found(&mut game, "Tags[1]"), 0);
    game.run_command(
        "execute store success score #r t if data entity @e[type=marker,limit=1] {Tags:[\"b\"]}",
    );
    assert_eq!(game.score.board.score("#r", "t").unwrap().value, 1);
    game.run_command(
        "execute store success score #r t if data entity @e[type=marker,limit=1] {Tags:[\"a\"]}",
    );
    assert_eq!(game.score.board.score("#r", "t").unwrap().value, 0);
}
