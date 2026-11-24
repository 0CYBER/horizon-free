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

#[test]
#[ignore = "daily"]
fn a_client_reads_the_far_terrains_surface_past_its_chunks() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        use horizon_client::lod::{Column, Lod, Span, Surface, TileKey};
        let tiles = [(-3, 2, 3), (-6, 4, 2), (i32::MAX, 0, u8::MAX)];
        let read_tiles = |host: &dyn mod_api::ClientHost, tiles: &[(i32, i32, u8)]| {
            host.client_lod_columns(tiles)
        };
        assert!(read_tiles(game, &tiles).is_err(), "no client world");
        // A tile of 8-block cells over x -384..-256, z 256..384, no chunk of it loaded.
        let columns = [(-300, 300), (-257, 383), (-384, 256), (1000, 1000)];
        assert!(
            game.client_far_surface(&columns).is_err(),
            "no client world"
        );
        let (client, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(client, "far-surface-client").unwrap();
        game.remote.as_mut().unwrap().logged_in = true;
        game.lod = None;
        assert_eq!(
            game.client_far_surface(&columns).unwrap(),
            [None, None, None, None],
            "far terrain off"
        );
        assert_eq!(read_tiles(game, &tiles).unwrap(), [None, None, None]);
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let water = registry.default_state("minecraft:water").unwrap();
        let surface = Surface {
            height: 61,
            state: stone,
            water: Some((63, water)),
            biome: game.res.biomes().id("minecraft:ocean"),
        };
        let mut samples = vec![Column::from_surface(Some(surface)); 16 * 16];
        // After detail replaces a cell, it can have a renderable top without
        // the separately stored surface metadata for ground picking.
        let detail = registry.default_state("minecraft:sand").unwrap();
        samples[17].surface = None;
        samples[17].push(Span {
            min: 64,
            max: 70,
            state: detail,
            biome: surface.biome,
            light: 0x6090,
        });
        samples[18] = Column::default();
        let mut lod = Lod::new(game.streaming.lod_source(), game.worker_context());
        lod.insert_ready_for_test(
            TileKey {
                x: -3,
                z: 2,
                level: 3,
            },
            samples,
        );
        game.lod = Some(lod);
        let got = game.client_far_surface(&columns).unwrap();
        for (column, at) in got.iter().zip(&columns).take(3) {
            let column = column
                .as_ref()
                .unwrap_or_else(|| panic!("a far column at {at:?}"));
            assert_eq!(column.height, 61);
            assert_eq!(column.block, "minecraft:stone");
            assert_eq!(column.water, Some(63));
            assert_eq!(column.biome, "minecraft:ocean");
        }
        assert_eq!(got[3], None, "no tile there");
        let many = vec![(0, 0); crate::game::block_edits::MAX_COLUMNS + 1];
        assert!(game.client_far_surface(&many).is_err());

        let resident_count = game.world.chunks.len();
        let tiles_read = read_tiles(game, &tiles).unwrap();
        let tile = tiles_read[0].as_ref().unwrap();
        assert_eq!(tile.cell_width, 8);
        assert_eq!(tile.columns.len(), 256);
        let wet = tile.columns[0].as_ref().unwrap();
        assert_eq!((wet.min_y, wet.top_y), (61, 63));
        assert_eq!(wet.state, mod_api::BlockStateId(water.0));
        let edited = tile.columns[17].as_ref().unwrap();
        assert_eq!((edited.min_y, edited.top_y), (64, 70));
        assert_eq!(edited.state, mod_api::BlockStateId(detail.0));
        assert_eq!(edited.biome, "minecraft:ocean");
        assert_eq!((edited.sky_light, edited.block_light), (9, 6));
        assert_eq!(tile.columns[18], None, "empty cell in a retained tile");
        assert_eq!(tiles_read[1], None, "do not substitute the coarser tile");
        assert_eq!(tiles_read[2], None, "impossible key does not overflow");
        assert!(read_tiles(game, &[tiles[0]; 257]).is_err());
        assert_eq!(game.world.chunks.len(), resident_count);
    });
}
