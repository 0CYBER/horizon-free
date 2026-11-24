//! `@e[dx=...]` and a marker armor stand: its box is empty (`MARKER_DIMENSIONS`), a point, so a
//! stand standing on the lower face of the selector's box is not inside it (the boxes meet only if
//! they overlap, a strict test), while a point a little inside it is. The answers are vanilla
//! 26.3's, for a marker at (1.5, 64, 1.5).
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
fn a_marker_stand_is_a_point_for_the_selector_box() {
    let mut game_resources = None;
    let mut game = game(&mut game_resources);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command(
        "summon armor_stand 1.5 64 1.5 {NoGravity:1b,Marker:1b,Tags:[\"m\"]}"
    ));
    let mut inside = |selector: &str| game.run_command(&format!("execute if entity @e[tag=m,{selector}]"));
    assert!(!inside("x=1,y=64,z=1,dx=0,dy=0,dz=0"), "on the lower face");
    assert!(!inside("x=0,y=64,z=0,dx=1,dy=1,dz=1"), "on the lower face of a larger box");
    assert!(!inside("x=1.5,y=64,z=1.5,dx=0,dy=0,dz=0"), "on the lower face, at the stand");
    assert!(!inside("x=1,y=63,z=1,dx=0,dy=0,dz=0"), "on the upper face");
    assert!(inside("x=1.4,y=63.9,z=1.4,dx=0,dy=0,dz=0"), "inside");
}
