//! A mob landing on the edge of a lower block falls on the block that
//! supports it: `Entity.move` refreshes the supporting block
//! (`checkSupportingBlock`) before `checkFallDamage`, whose block is the
//! supporting one (`getOnPosLegacy`). A pig whose centre is over a honey block
//! (top 15/16) but whose box rests on the stone beside it takes the stone's
//! plain fall, not honey's fifth
//! (tools/test_mob_landing_support_parity.py measures the same on vanilla 26.3).
use super::*;
use horizon_client::chunk::Chunk;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

fn game<'a>(resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'a> {
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
        execution_role::ExecutionRole::IntegratedServer,
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
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.load_ticking_neighbourhood(0, 0);
    assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
    game
}

/// A pig with AI that cannot walk dropped 6 blocks at `x`, over honey at 8
/// and stone at 9; its Health, and the support it has, once it has landed.
fn drop_pig(x: f64) -> (f32, Option<[i32; 3]>, f64) {
    let mut resources = None;
    let mut game = game(&mut resources);
    for command in [
        "forceload add 0 0".to_owned(),
        "tick freeze".to_owned(),
        "fill 0 98 0 15 98 15 stone".to_owned(),
        "fill 0 99 0 15 110 15 air".to_owned(),
        "setblock 8 99 8 honey_block".to_owned(),
        "setblock 9 99 8 stone".to_owned(),
        format!(
            "summon pig {x} 106 8.5 {{PersistenceRequired:1b,Silent:1b,Health:10f,\
             attributes:[{{id:\"minecraft:movement_speed\",base:0.0d}}],UUID:[I;68173,0,0,1]}}"
        ),
    ] {
        assert!(game.run_command(&command), "{command}");
    }
    for _ in 0..24 {
        assert!(game.run_command("tick step 1"));
        game.tick_server(false).unwrap();
    }
    let pig = game
        .mobs
        .iter()
        .find(|m| m.species() == horizon_client::mobs::Species::Pig)
        .expect("the pig");
    assert!(pig.on_ground, "the pig has landed");
    (pig.health, pig.supporting, pig.position.y)
}

#[test]
#[ignore = "daily"]
fn a_pig_landing_on_the_stone_beside_honey_takes_the_stones_fall() {
    let (health, supporting, y) = drop_pig(8.9);
    assert_eq!(y, 100.0, "the box rests on the stone's top");
    assert_eq!(supporting, Some([9, 99, 8]), "the stone supports it");
    assert_eq!(health, 7.0, "the plain fall of 6 blocks");
}

#[test]
#[ignore = "daily"]
fn a_pig_landing_fully_on_honey_takes_a_fifth_of_the_fall() {
    let (health, supporting, y) = drop_pig(8.5);
    assert_eq!(y, 99.9375);
    assert_eq!(supporting, Some([8, 99, 8]));
    assert_eq!(health, 10.0);
}

#[path = "tests/mob_friction_tests.rs"]
mod mob_friction_tests;
