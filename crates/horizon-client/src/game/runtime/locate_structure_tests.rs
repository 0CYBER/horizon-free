//! `/locate structure` with an id or tag that is no structure: vanilla's `LocateCommand.getHolders`
//! throws `commands.locate.structure.invalid` before any search runs, so the player is not told that a
//! search came up empty. A structure the registry has, and no search can find, is the other error.
use super::*;

fn game(resources: &mut Option<std::sync::Arc<Resources>>) -> Game<'_> {
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

/// What a failing `command` says first.
fn failure(game: &mut Game<'_>, command: &str) -> String {
    game.chat_capture = Some(Vec::new());
    assert!(!game.run_command(command), "{command}");
    let said = game.chat_capture.take().unwrap_or_default();
    said.first()
        .map(horizon_client::text::Text::plain)
        .unwrap_or_default()
}

#[test]
#[ignore = "daily"]
fn an_unknown_structure_or_tag_is_invalid_and_a_known_one_is_only_not_found() {
    let mut game_resources = None;
    let mut game = game(&mut game_resources);
    assert_eq!(
        failure(&mut game, "locate structure nothing"),
        "There is no structure with type \"minecraft:nothing\""
    );
    assert_eq!(
        failure(&mut game, "locate structure #nothing:nothing"),
        "There is no structure with type \"#nothing:nothing\""
    );
    assert_eq!(
        failure(&mut game, "locate structure minecraft:village_plains"),
        "Could not find a structure of type \"minecraft:village_plains\" nearby"
    );
    assert_eq!(
        failure(&mut game, "locate structure #minecraft:village"),
        "Could not find a structure of type \"#minecraft:village\" nearby"
    );
}
