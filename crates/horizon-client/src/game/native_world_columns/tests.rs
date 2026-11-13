use super::super::native_chunk_tickets::native_chunk_load_tests::game;
use super::MAX_TERRAIN_COLUMNS;
use horizon_client::streaming::ChunkGenerator;

const OVERWORLD: &str = "minecraft:overworld";

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn terrain_columns_are_the_generators_own_surface_and_load_no_chunk() {
    let mut game = game();
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    // A world whose biome table is not the base one (a data pack adds biomes before the vanilla
    // ones): the generator's ids are this table's, and names are read through it.
    let mut world_biomes = (*game.res.core.biomes).clone();
    world_biomes.list.rotate_left(1);
    for (id, biome) in world_biomes.list.iter().enumerate() {
        world_biomes.index.insert(biome.name.clone(), id as u16);
    }
    let terrain = horizon_client::vanillagen::VanillaTerrain::new(
        &root.join("worldgen"),
        game.pack.assets.registry,
        &world_biomes,
        42,
    )
    .unwrap();
    game.world_biomes = Some(std::sync::Arc::new(world_biomes.clone()));
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
        assert_eq!(got.biome, world_biomes.name(want.biome));
    }
    assert!(
        found.iter().flatten().any(|c| c.water.is_some()),
        "an ocean"
    );
    assert!(
        found.iter().flatten().any(|c| c.water.is_none()),
        "dry land"
    );
    assert!(
        found.iter().flatten().any(|c| {
            let id = world_biomes.id(&c.biome);
            game.res.biomes().name(id) != c.biome
        }),
        "the world's table differs from the base one, so reading the base one would name another biome"
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
