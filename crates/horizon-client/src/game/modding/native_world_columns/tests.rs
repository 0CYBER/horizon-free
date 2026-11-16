use super::MAX_TERRAIN_COLUMNS;
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::streaming::ChunkGenerator;
use horizon_client::vanillagen::{TerrainResources, VanillaTerrain};

const OVERWORLD: &str = "minecraft:overworld";

#[test]
#[ignore = "daily"]
fn terrain_columns_are_the_generators_own_surface_and_load_no_chunk() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        // A world whose biome table is not the base one (a data pack adds biomes before the vanilla
        // ones): the generator's ids are this table's, and names are read through it.
        let mut world_biomes = (**game.res.biomes()).clone();
        world_biomes.list.rotate_left(1);
        for (id, biome) in world_biomes.list.iter().enumerate() {
            world_biomes.index.insert(biome.name.clone(), id as u16);
        }
        let directory = game.options.resources.join("worldgen");
        let terrain = VanillaTerrain::for_dimension(
            TerrainResources {
                data: Some(&game.pack.data_changes),
                dir: &directory,
                registry: game.pack.assets.registry,
                biomes: &world_biomes,
                item_data: &game.res.core.item_data,
            },
            42,
            OVERWORLD,
        )
        .unwrap();
        game.world_biomes = Some(std::sync::Arc::new(world_biomes.clone()));
        game.end.set_locator(Some(std::sync::Arc::new(terrain)));
        let terrain = game.end.locator().unwrap();
        // A grid across ocean and land, far from any loaded chunk.
        let columns: Vec<(i32, i32)> = (-4..4)
            .flat_map(|i| (-4..4).map(move |j| (i * 700 + 5, j * 700 + 9)))
            .collect();
        let loaded = game.world.chunks.len();
        let found = game.terrain_surface(OVERWORLD, &columns).unwrap();
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
        assert_eq!(game.world.chunks.len(), loaded, "no chunk was created");
        // Only the level being ticked, and a bounded call.
        assert!(
            game.terrain_surface("minecraft:the_nether", &columns)
                .is_err()
        );
        let many = vec![(0, 0); MAX_TERRAIN_COLUMNS + 1];
        assert!(game.terrain_surface(OVERWORLD, &many).is_err());
    });
}
