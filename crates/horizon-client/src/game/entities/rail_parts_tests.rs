//! A rail's parts decide its track: the shape a cart reads (`rail_direction`), and as it
//! connects, whether it curves (`rail_flexible`), climbs (`rail_can_make_slopes`) or takes a
//! shape at all (`rail_valid_shape`).
use super::minecarts::Shape;
use crate::game::{Game, native_entity_refs::tests::with_game};
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use horizon_core::{geometry::Vec3, position::ChunkPos};
use std::sync::Arc;

/// A serving world with commands, a stone floor at y 63 and its chunk ticking. Returns the
/// scratch world folder to remove.
fn rail_world(game: &mut Game<'_>) -> std::path::PathBuf {
    game.options.dedicated = Some(Vec::new());
    let scratch = std::env::temp_dir().join(format!(
        "horizon-rail-parts-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    game.world_root = scratch.clone();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 12.5)).unwrap());
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("fill 0 63 0 15 63 15 minecraft:stone"));
    scratch
}

fn shape(game: &Game<'_>, [x, y, z]: [i32; 3]) -> String {
    let state = game.world.block(x, y, z);
    let state = game.pack.assets.registry.state(state).unwrap();
    state.properties["shape"].clone()
}

#[test]
#[ignore = "daily"]
fn rail_parts_decide_the_track_carts_and_connections_follow() {
    // The fixture's parts: a plain rail lays east-west for a cart and can't curve, a powered
    // rail can't climb, and an activator rail refuses east-west.
    mod_platform::__private::part_tests::rail_parts_game_thread();
    with_game(|game| {
        let scratch = rail_world(game);
        // A rail south and one east of a plain rail would curve it south-east; it stays
        // straight.
        assert!(game.run_command("setblock 2 64 3 minecraft:rail"));
        assert!(game.run_command("setblock 3 64 2 minecraft:rail"));
        assert!(game.run_command("setblock 2 64 2 minecraft:rail"));
        assert_ne!(shape(game, [2, 64, 2]), "south_east");
        // A cart reads the plain rail's part, a rail read for itself its stored shape.
        let (stored, carried) = (
            game.rail(None, 2, 64, 2).unwrap().shape,
            game.rail(Some(1), 2, 64, 2).unwrap().shape,
        );
        assert_eq!(carried, Shape::EastWest);
        assert_eq!(stored, Shape::parse(&shape(game, [2, 64, 2])));
        // A powered rail beside a raised one stays flat.
        assert!(game.run_command("setblock 6 64 8 minecraft:stone"));
        assert!(game.run_command("setblock 6 65 8 minecraft:powered_rail[shape=east_west]"));
        assert!(game.run_command("setblock 5 64 8 minecraft:powered_rail"));
        assert_eq!(shape(game, [5, 64, 8]), "east_west");
        // Two activator rails side by side east-west keep their north-south shape.
        assert!(game.run_command("setblock 2 64 12 minecraft:activator_rail"));
        assert!(game.run_command("setblock 3 64 12 minecraft:activator_rail"));
        assert_eq!(shape(game, [3, 64, 12]), "north_south");
        assert_eq!(shape(game, [2, 64, 12]), "north_south");
        // An activator rail that refuses east-west lays no connection toward the detector rail
        // east of it, so the detector keeps its own shape too.
        assert!(game.run_command("setblock 9 64 12 minecraft:detector_rail"));
        assert!(game.run_command("setblock 8 64 12 minecraft:activator_rail"));
        assert_eq!(shape(game, [8, 64, 12]), "north_south");
        assert_eq!(shape(game, [9, 64, 12]), "north_south");
        std::fs::remove_dir_all(scratch).ok();
    });
}
