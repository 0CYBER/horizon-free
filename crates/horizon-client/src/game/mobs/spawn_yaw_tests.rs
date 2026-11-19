//! A new mob's yaw as its vanilla spawn path sets it (#12973): `/summon`
//! without NBT loads the mob from `{id:..}` and keeps yaw 0 with no
//! level-random draw (`SummonCommand.createEntity`), and `EntityType.create`
//! (spawn eggs, a dispenser's eggs, mob buckets) wraps its draw with
//! `Mth.wrapDegrees`.
use crate::game::creatures::SpawnReason;
use crate::game::creatures::SpawnSite;
use crate::game::*;
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
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    for command in [
        "forceload add 0 0",
        "fill 0 99 0 15 99 15 stone",
        "tick freeze",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game
}

fn yaw_of(game: &Game<'_>, id: u32) -> f32 {
    game.mobs.iter().find(|m| m.id == id).expect("spawned").yaw
}

#[test]
#[ignore = "daily"]
fn a_summoned_mob_without_nbt_faces_south() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let before = game.mobs.len();
    assert!(game.run_command("summon pig 4.5 100 8.5"));
    let pig = &game.mobs[before];
    assert_eq!(pig.yaw, 0.0);
}

#[test]
#[ignore = "daily"]
fn a_spawn_eggs_yaw_is_the_wrapped_level_draw() {
    let mut resources = None;
    let mut game = game(&mut resources);
    // Draws until one lands in [180, 360), where wrapping changes the value.
    let mut checked = false;
    for _ in 0..32 {
        let mut level = game.world.random.clone();
        let draw = level.next_float() * 360.0;
        let id = game
            .spawn_mob_spawned(
                "pig",
                [4.5, 100.0, 8.5],
                SpawnSite::Finalizing(SpawnReason::SpawnItemUse),
            )
            .expect("pig");
        let yaw = yaw_of(&game, id);
        assert_eq!(yaw, horizon_client::camera::wrap_degrees(draw));
        assert!((-180.0..180.0).contains(&yaw), "{yaw}");
        if draw >= 180.0 {
            checked = true;
            break;
        }
    }
    assert!(checked, "no draw of 180 or more in 32 spawns");
}

#[test]
#[ignore = "daily"]
fn a_dispensed_mob_takes_the_wrapped_draw_too() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let mut level = game.world.random.clone();
    let draw = level.next_float() * 360.0;
    let id = game.spawn_mob_at("pig", (4, 100, 8), false).expect("pig");
    assert_eq!(
        yaw_of(&game, id),
        horizon_client::camera::wrap_degrees(draw)
    );
}

/// `Shulker.finalizeSpawn` sets yaw 0 on the spawn path (a spawn egg draws a random one first); a shulker
/// loaded from NBT keeps its `Rotation`.
#[test]
#[ignore = "daily"]
fn a_new_shulker_faces_yaw_zero_and_a_loaded_one_keeps_its_rotation() {
    let mut resources = None;
    let mut game = game(&mut resources);
    for _ in 0..8 {
        let id = game
            .spawn_mob_spawned(
                "shulker",
                [4.5, 100.0, 8.5],
                SpawnSite::Finalizing(SpawnReason::SpawnItemUse),
            )
            .expect("shulker");
        let shulker = game.mobs.iter().find(|m| m.id == id).unwrap();
        assert_eq!([shulker.yaw, shulker.head_pitch], [0.0, 0.0]);
        // `setOldPosAndRot`: the previous and the head's rotations follow, whatever the creation path drew
        assert_eq!(
            [
                shulker.previous_yaw,
                shulker.head_yaw,
                shulker.previous_head_yaw
            ],
            [0.0; 3]
        );
    }
    let before = game.mobs.len();
    assert!(game.run_command("summon shulker 6.5 100 8.5"));
    assert_eq!(
        [game.mobs[before].yaw, game.mobs[before].head_pitch],
        [0.0, 0.0]
    );
    assert!(game.run_command("summon shulker 8.5 100 8.5 {Rotation:[73f,-24f]}"));
    assert_eq!(
        [game.mobs[before + 1].yaw, game.mobs[before + 1].head_pitch],
        [73.0, -24.0]
    );
}
