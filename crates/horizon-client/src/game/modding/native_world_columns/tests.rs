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

#[test]
#[ignore = "daily"]
fn client_map_reads_retained_surfaces_and_chunk_deltas() {
    use crate::game::{
        api_values::TryIntoApi,
        native_entity_refs::tests::with_client_game,
        remote_chunks::ChunkFormat,
        viewed_worlds::tests::{layered, spawn},
    };
    use horizon_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C},
        registry::DimensionType,
    };
    use mod_api::{ChunkDeltaCursor, ChunkDeltas, ClientHost, Heightmap};
    use std::sync::Arc;

    with_client_game(|game| {
        let columns = [(-1, -1), (32, -16), (-15, -16), (-16, -16), (-1, -1)];
        let heights = Heightmap::WorldSurface;
        assert!(
            (game as &dyn ClientHost)
                .client_surface_columns(&columns, heights)
                .is_err()
        );

        // Real client packet handlers, on the same local session kind as an integrated world.
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "ClientMapTest").unwrap();
        let dimension = DimensionType::vanilla(OVERWORLD);
        let biomes = game.res.biomes();
        let biome_map = vec![biomes.id("minecraft:plains"), biomes.id("minecraft:desert")];
        game.remote.as_mut().unwrap().chunk_format =
            Arc::new(ChunkFormat::new(&dimension, biome_map, 1));
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec![OVERWORLD.into(), "minecraft:the_nether".into()],
            max_players: 1,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: spawn(OVERWORLD),
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        let level = (game as &dyn ClientHost).level(OVERWORLD).unwrap();
        let poll = |host: &dyn ClientHost, since| host.level_chunk_deltas(level, since).unwrap();
        let initial = poll(game, None);
        assert!(initial.full && initial.changes.is_empty());
        assert_eq!(
            (game as &dyn ClientHost)
                .client_surface_columns(&columns, heights)
                .unwrap(),
            [None, None, None, None, None]
        );

        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let water = registry.default_state("minecraft:water").unwrap();
        let air = registry.default_state("minecraft:air").unwrap();
        let log = registry
            .resolve(&horizon_world::chunk::BlockState {
                name: "minecraft:oak_log".into(),
                properties: [("axis".into(), "x".into())].into(),
            })
            .unwrap();

        game.handle_packet(layered(1, -1, 0)).unwrap();
        game.handle_packet(map_surface_packet([air.0, stone.0, water.0, log.0]))
            .unwrap();
        let order = [([-1, -1], true), ([1, -1], true)];
        let roster = |deltas: &ChunkDeltas| {
            deltas
                .changes
                .iter()
                .map(|change| (change.pos, change.loaded))
                .collect::<Vec<_>>()
        };
        let arrivals = poll(game, Some(initial.cursor));
        assert!(!arrivals.full);
        assert_eq!(roster(&arrivals), order, "reverse arrival order is sorted");
        let snapshot = poll(game, None);
        assert!(snapshot.full);
        assert_eq!(roster(&snapshot), order);

        let host: &dyn ClientHost = game;
        let surface = host.client_surface_columns(&columns, heights).unwrap();
        assert_eq!(
            host.client_surface_heights(&columns, heights).unwrap(),
            [Some(320), None, Some(65), Some(-64), Some(320)]
        );
        assert_eq!(
            surface
                .iter()
                .map(|s| s.as_ref().map(|s| s.height))
                .collect::<Vec<_>>(),
            [Some(320), None, Some(65), Some(-64), Some(320)]
        );
        let top = surface[0].as_ref().unwrap();
        assert_eq!(
            top.block,
            log.try_into_api().unwrap(),
            "non-default state at the build ceiling"
        );
        let states = mod_api::HostBlockStates::block_states(host);
        assert_eq!(
            (
                mod_api::BlockMetadata::name(&states, top.block.block()),
                top.block.property(&states, "axis")
            ),
            (Ok("minecraft:oak_log"), Ok(Some("x")))
        );
        assert_eq!(
            (&*top.biome, top.sky_light, top.block_light),
            ("minecraft:desert", 9, 3)
        );
        let wet = surface[2].as_ref().unwrap();
        assert_eq!(wet.block, water.try_into_api().unwrap());
        let floor = host
            .client_surface_columns(&[columns[2]], Heightmap::OceanFloor)
            .unwrap();
        let floor = floor[0].as_ref().unwrap();
        assert_eq!(
            (floor.height, floor.block),
            (61, stone.try_into_api().unwrap())
        );
        let empty = surface[3].as_ref().unwrap();
        assert_eq!(empty.block, air.try_into_api().unwrap());
        assert_eq!(
            (&*empty.biome, empty.sky_light, empty.block_light),
            ("minecraft:plains", 6, 2)
        );
        assert_eq!(
            surface[0], surface[4],
            "duplicate columns stay in input order"
        );
        let limit = vec![columns[0]; 65_536];
        assert!(host.client_surface_columns(&limit, heights).is_ok());
        let too_many = vec![columns[0]; 65_537];
        assert!(host.client_surface_columns(&too_many, heights).is_err());
        let idle = poll(host, Some(arrivals.cursor));
        assert!(
            !idle.full && idle.changes.is_empty(),
            "reads leave residency unchanged"
        );
        assert_eq!(idle.cursor, arrivals.cursor, "reads do not touch chunks");

        // Repeated writes to one key and a removal, after a transported cursor.
        let cursor = wire_round_trip(&arrivals).cursor;
        for state in [stone, air] {
            game.handle_packet(C::BlockUpdate(play::BlockUpdate {
                pos: [-1, 319, -1],
                state: state.0 as i32,
            }))
            .unwrap();
        }
        game.handle_packet(C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [1, -1] }))
            .unwrap();
        let changed = poll(game, Some(cursor));
        assert!(!changed.full);
        assert_eq!(roster(&changed), [order[0], ([1, -1], false)]);
        let last = (game as &dyn ClientHost)
            .client_surface_columns(&[(-1, -1), (16, -16)], heights)
            .unwrap();
        assert_eq!(
            last[0].as_ref().unwrap().height,
            -64,
            "an in-place edit invalidates height"
        );
        assert_eq!(last[1], None, "a forgotten column is no longer readable");
        assert_eq!(wire_round_trip(&surface), surface);

        // More actual writes than the existing chunk journal retains.
        for i in 0..9_000 {
            let state = if i % 2 == 0 { stone } else { air };
            game.world.set(-1, 319, -1, state).unwrap();
        }
        let old = poll(game, Some(changed.cursor));
        assert!(old.full);
        assert_eq!(roster(&old), order[..1]);
        let (map, _) = old.cursor.for_level(level).unwrap();
        let unknown = poll(game, Some(ChunkDeltaCursor::new(level, (map, u64::MAX))));
        assert!(unknown.full);
        assert_eq!(unknown.changes, old.changes);

        game.world.chunks.clear();
        let cleared = poll(game, Some(old.cursor));
        assert!(cleared.full && cleared.changes.is_empty());

        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:the_nether"),
            keep: 0,
        }))
        .unwrap();
        let host: &dyn ClientHost = game;
        let new_level = host.level("minecraft:the_nether").unwrap();
        assert_eq!(host.level_chunk_deltas(level, Some(cleared.cursor)), None);
        let switched = host
            .level_chunk_deltas(new_level, Some(cleared.cursor))
            .unwrap();
        assert!(switched.full && switched.changes.is_empty());
    });
}

fn map_surface_packet(
    [air, stone, water, log]: [u32; 4],
) -> horizon_protocol::packets::play::Clientbound {
    use horizon_protocol::{
        buf::set_bit,
        chunk::{ChunkData, LightData, Paletted, Section, Strategy},
        packets::play::{Clientbound, LevelChunkWithLight},
    };

    let sections: Vec<_> = (0..24)
        .map(|section| {
            let mut states = [air as i32; 4096];
            let mut biomes = [0; 64];
            let (non_empty, fluids) = match section {
                7 => {
                    states[12 << 8 | 1] = stone as i32; // (-15, 60, -16)
                    (1, 0)
                }
                8 => {
                    states[1] = water as i32; // (-15, 64, -16)
                    (1, 1)
                }
                23 => {
                    states[4095] = log as i32; // (-1, 319, -1)
                    biomes[63] = 1;
                    (1, 0)
                }
                _ => (0, 0),
            };
            Section {
                non_empty,
                fluids,
                states: Paletted::from_values(&states, Strategy::Blocks, 16),
                biomes: Paletted::from_values(&biomes, Strategy::Biomes, 1),
            }
        })
        .collect();
    let mut mask = Vec::new();
    set_bit(&mut mask, 1); // first block section
    set_bit(&mut mask, 24); // last block section
    Clientbound::LevelChunkWithLight(LevelChunkWithLight {
        x: -1,
        z: -1,
        chunk: ChunkData {
            heightmaps: Vec::new(),
            buffer: ChunkData::from_sections(&sections),
            block_entities: Vec::new(),
        },
        light: LightData {
            sky_mask: mask.clone(),
            block_mask: mask,
            empty_sky_mask: Vec::new(),
            empty_block_mask: Vec::new(),
            sky: vec![vec![0x66; 2048], vec![0x99; 2048]],
            block: vec![vec![0x22; 2048], vec![0x33; 2048]],
        },
    })
}

fn wire_round_trip<T: mod_api::wire::Wire>(value: &T) -> T {
    let mut writer = mod_api::wire::Writer::new();
    writer.put(value);
    mod_api::wire::Reader::decode(writer.bytes()).unwrap()
}
