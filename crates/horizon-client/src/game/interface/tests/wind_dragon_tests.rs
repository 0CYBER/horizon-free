use crate::game::{Game, creatures::SpawnSite, wind::Source};
use horizon_client::chunk::Chunk;
use horizon_client::mobs::spawn_rules::Reason;
use horizon_core::geometry::Vec3;
use std::sync::Arc;

fn end_game<'a>(resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'a> {
    let mut game = crate::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    assert_eq!(
        game.enter_command_level(crate::game::Dim::End),
        Some(crate::game::Dim::Overworld)
    );
    assert!(game.dimension_type().has_ender_dragon_fight, "the End");
    for x in -1..2 {
        for z in -1..2 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
    }
    assert!(game.run_command("forceload add 0 0"));
    game
}

#[test]
#[ignore = "daily"]
fn a_wind_burst_pushes_the_dragon_without_damage() {
    let mut resources = None;
    let mut game = end_game(&mut resources);
    game.spawn_dragon_spawned(
        [0.0, 70.0, 0.0],
        SpawnSite::Finalizing(Reason::SpawnItemUse),
    )
    .expect("spawned dragon");
    game.dragon.as_mut().expect("dragon").velocity = Vec3::ZERO;

    game.wind_burst([0.0, 66.0, 0.0], 5.0, 1.0, Source::WIND_BURST)
        .unwrap();

    let dragon = game.dragon.as_ref().expect("dragon");
    assert_eq!(dragon.health, 200.0);
    assert!(
        dragon.velocity.y > 0.0,
        "Wind Burst did not push the dragon"
    );
}

#[test]
#[ignore = "daily"]
fn a_wind_burst_scales_the_dragons_push_by_its_multiplier() {
    let mut resources = None;
    let mut game = end_game(&mut resources);
    game.spawn_dragon_spawned(
        [0.0, 70.0, 0.0],
        SpawnSite::Finalizing(Reason::SpawnItemUse),
    )
    .expect("spawned dragon");
    game.dragon.as_mut().expect("dragon").velocity = Vec3::ZERO;
    game.wind_burst([0.0, 66.0, 0.0], 5.0, 1.22, Source::WIND_BURST)
        .unwrap();

    let push = game.dragon.as_ref().expect("dragon").velocity.y;
    assert!(push > 0.0, "Wind Burst did not push the dragon");
    assert!(push > 0.6, "Wind Burst multiplier was not applied: {push}");
}
