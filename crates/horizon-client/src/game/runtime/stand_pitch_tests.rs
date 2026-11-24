//! `@e[x_rotation=...]` on an armor stand: the selector reads `Entity.getXRot`, which the stand
//! keeps from its `Rotation` (the pitch the kind never turns by itself), as `/data get` shows it.
//! Vanilla 26.3 selects exactly the stands whose pitch lies in the range.
use super::*;
use horizon_client::chunk::Chunk;
use horizon_core::position::ChunkPos;
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
fn x_rotation_selects_an_armor_stand_by_its_pitch() {
    let mut game_resources = None;
    let mut game = game(&mut game_resources);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    for (i, pitch) in [-40, 0, 40].into_iter().enumerate() {
        assert!(game.run_command(&format!(
            "summon armor_stand {}.5 64 1.5 {{NoGravity:1b,Marker:1b,Tags:[\"s{i}\"],Rotation:[0f,{pitch}f]}}",
            i + 1
        )));
    }
    let mut count = |selector: &str| {
        (0..3)
            .filter(|i| game.run_command(&format!("execute if entity @e[tag=s{i},{selector}]")))
            .count()
    };
    assert_eq!(count("x_rotation=40"), 1);
    assert_eq!(count("x_rotation=0"), 1);
    assert_eq!(count("x_rotation=-40"), 1);
    assert_eq!(count("x_rotation=-40..0"), 2);
}
