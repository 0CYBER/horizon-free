use super::super::native_chunk_tickets::native_chunk_load_tests::game;
use super::MAX_TERRAIN_COLUMNS;
use horizon_client::streaming::ChunkGenerator;

const OVERWORLD: &str = "minecraft:overworld";

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn terrain_columns_are_the_generators_own_surface_and_load_no_chunk() {
    let game = game();
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let terrain = horizon_client::vanillagen::VanillaTerrain::new(
        &root.join("worldgen"),
        game.pack.assets.registry,
        &game.res.core.biomes,
        42,
    )
    .unwrap();
    game.end.set_locator(Some(std::sync::Arc::new(terrain)));
    let terrain = game.end.locator().unwrap();
    // A grid across ocean and land, far from any loaded chunk.
    let columns: Vec<(i32, i32)> = (-4..4)
        .flat_map(|i| (-4..4).map(move |j| (i * 700 + 5, j * 700 + 9)))
        .collect();
    let found = game.native_terrain_surface(OVERWORLD, &columns).unwrap();
    let mut worker = terrain.worker();
    for (&(x, z), got) in columns.iter().zip(&found) {
        let want = terrain.lod_surface(&mut worker, x, z).expect("sampled");
        let got = got.as_ref().expect("a column");
        assert_eq!(got.height, want.height, "height at {x} {z}");
        assert_eq!(
            got.water,
            want.water.map(|(top, _)| top),
            "water at {x} {z}"
        );
        assert_eq!(&*got.block, &*game.res.facts().get(want.state).name);
        assert_eq!(got.biome, game.res.biomes().name(want.biome));
    }
    assert!(
        found.iter().flatten().any(|c| c.water.is_some()),
        "an ocean"
    );
    assert!(
        found.iter().flatten().any(|c| c.water.is_none()),
        "dry land"
    );
    assert!(game.world.chunks.is_empty(), "no chunk was created");
    // Only the level being ticked, and a bounded call.
    assert!(
        game.native_terrain_surface("minecraft:the_nether", &columns)
            .is_err()
    );
    let many = vec![(0, 0); MAX_TERRAIN_COLUMNS + 1];
    assert!(game.native_terrain_surface(OVERWORLD, &many).is_err());
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_tp_to_chunks_not_lit_holds_the_player_once() {
    let mut game = game();
    // Far from the spawn, so nothing there has loaded.
    game.hold_for_chunks(
        7,
        horizon_core::geometry::Vec3::new(96_000.5, 150.0, 96_000.5),
    );
    assert!(game.on_server_trip(7));
    // Another player is not held, and a second tp replaces the first's wait.
    assert!(!game.on_server_trip(8));
    game.hold_for_chunks(7, horizon_core::geometry::Vec3::new(-96_000.5, 150.0, 0.5));
    assert_eq!(game.server_trips.iter().filter(|t| t.is(7)).count(), 1);
}
