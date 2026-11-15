//! `EnderDragonFight.updateDragon`: the fight's boss bar takes the dragon's
//! custom name (`dragon.getDisplayName()`), and keeps the type's name until
//! it has one.
use super::*;
use horizon_client::chunk::Chunk;
use horizon_core::geometry::Vec3;
use horizon_core::nbt::Tag;

fn end_game() -> Game<'static> {
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
    assert_eq!(game.enter_command_level(Dim::End), Some(Dim::Overworld));
    assert!(game.dimension_type().has_ender_dragon_fight, "the End");
    game
}

#[test]
#[ignore = "daily"]
fn the_dragon_boss_bar_takes_the_dragons_custom_name() {
    let mut game = end_game();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 128.0, 0.5)).unwrap());
    // The chunks `EnderDragonFight.isArenaLoaded` waits for, lit as received
    // chunks are, so the fight's next player scan adopts or makes its dragon.
    {
        let streaming = &mut game.streaming;
        let world = &mut game.world;
        for z in -2..=1 {
            for x in -2..=1 {
                streaming.insert_remote(world, Chunk::empty(x, z));
            }
        }
    }
    // `updatePlayers` scans every 20 ticks; then `findOrCreateDragon` makes
    // the fight's dragon.
    for _ in 0..20 {
        game.tick_fight();
    }
    let id = game.dragon.as_ref().expect("the fight's dragon").id;
    game.boss_overlay.reset();
    game.publish_boss_bars();
    let default = game
        .boss_overlay
        .bars(horizon_client::boss_overlay::now_millis())
        .pop()
        .expect("the dragon's bar");
    assert_eq!(
        default.name.translation_key(),
        Some("entity.minecraft.ender_dragon")
    );
    // The name a summoned dragon carries: `remember_tags` keeps its
    // `CustomName` for `/data get`, and `updateDragon` reads it.
    game.persist.raw.insert(
        id,
        Tag::compound([("CustomName", Tag::String("Bob".into()))]),
    );
    game.publish_boss_bars();
    let named = game
        .boss_overlay
        .bars(horizon_client::boss_overlay::now_millis())
        .pop()
        .expect("the dragon's bar");
    assert_eq!(named.name.plain(), "Bob");
    // A living NoAI dragon's `aiStep` does not call `updateDragon`, so the
    // bar keeps the name it had.
    game.dragon.as_mut().expect("the fight's dragon").no_ai = true;
    game.persist.raw.insert(
        id,
        Tag::compound([("CustomName", Tag::String("Not Yet".into()))]),
    );
    game.publish_boss_bars();
    let still = game
        .boss_overlay
        .bars(horizon_client::boss_overlay::now_millis())
        .pop()
        .expect("the dragon's bar");
    assert_eq!(still.name.plain(), "Bob");
}
