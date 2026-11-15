#![cfg(test)]
//! `client_chunk_load` and `client_chunk_unload` through a loopback client (arismoko/horizon#3425): a chunk
//! packet adopts a column and tells its load once the blocks are in; every route that frees a
//! resident column tells its unload once, with its blocks still readable.
use crate::game::Game;
use crate::game::execution_role::ExecutionRole;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_role};
use horizon_protocol::{
    chunk::{ChunkData, LightData, Paletted, Section, Strategy},
    packets::play::{LevelChunkWithLight, SpawnInfo},
};
use mod_api::{ClientChunkLoadEvent, ClientChunkUnloadEvent, ClientHost, Hooks};
use std::sync::Mutex;

/// What each event saw: its name, dimension, chunk and the block id at the chunk's corner
/// `(16 x, 64, 16 z)`, read through the client host inside the listener.
type Call = (&'static str, String, [i32; 2], Option<String>);
static CALLS: Mutex<Vec<Call>> = Mutex::new(Vec::new());

/// The block at `chunk`'s corner at y 64, as a listener reads it.
fn corner(host: &dyn ClientHost, dimension: &str, [x, z]: [i32; 2]) -> Option<String> {
    host.level(dimension)
        .and_then(|level| level.block(host, [x * 16, 64, z * 16]))
        .and_then(mod_api::__private::block_key_id)
        .map(str::to_owned)
}

struct Probe;
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn client_chunk_load(&self, host: &mut dyn ClientHost, event: &mut ClientChunkLoadEvent<'_>) {
        let block = corner(&*host, event.dimension, event.chunk);
        CALLS
            .lock()
            .unwrap()
            .push(("load", event.dimension.to_owned(), event.chunk, block));
    }
    fn client_chunk_unload(
        &self,
        host: &mut dyn ClientHost,
        event: &mut ClientChunkUnloadEvent<'_>,
    ) {
        let block = corner(&*host, event.dimension, event.chunk);
        CALLS
            .lock()
            .unwrap()
            .push(("unload", event.dimension.to_owned(), event.chunk, block));
    }
}

fn take_calls() -> Vec<Call> {
    std::mem::take(&mut *CALLS.lock().unwrap())
}

fn with_client(f: impl FnOnce(&mut Game<'_>)) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("chunk_probe", Box::new(Probe))].into();
    mod_platform::testing::with_mods(mods, || with_loopback(f));
}

fn with_loopback(f: impl FnOnce(&mut Game<'_>)) {
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let root = crate::game::mods::content_root(&base, &Default::default()).unwrap();
    with_game_role(
        root.prepared().to_path_buf(),
        ExecutionRole::Client,
        |game| {
            game.native_lifecycle = crate::game::native_lifecycle::NativeLifecycle::default();
            let (client, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(client, "ChunkProbe").unwrap();
            game.enter_remote_dimension(
                &overworld(),
                false,
                horizon_client::level_loading::Reason::Other,
            );
            game.remote.as_mut().unwrap().logged_in = true;
            take_calls();
            f(game);
        },
    );
}

fn overworld() -> SpawnInfo {
    SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 0,
        game_type: 1,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    }
}

/// A chunk packet for column `(x, z)` with stone at its local `(0, 64, 0)`.
fn stone_column(game: &Game<'_>, x: i32, z: i32) -> LevelChunkWithLight {
    let air = game.res.facts().air.0 as i32;
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap()
        .0 as i32;
    // Section 8 holds y 64..80 in a world from y -64; index 0 is its local (0, 0, 0).
    let sections: Vec<_> = (0..24)
        .map(|i| {
            let mut states = vec![air; 4096];
            if i == 8 {
                states[0] = stone;
            }
            Section {
                non_empty: i16::from(i == 8),
                fluids: 0,
                states: Paletted::from_values(&states, Strategy::Blocks, 16),
                biomes: Paletted::from_values(&[0; 64], Strategy::Biomes, 2),
            }
        })
        .collect();
    LevelChunkWithLight {
        x,
        z,
        chunk: ChunkData {
            heightmaps: vec![],
            buffer: ChunkData::from_sections(&sections),
            block_entities: vec![],
        },
        light: LightData {
            sky_mask: vec![],
            block_mask: vec![],
            empty_sky_mask: vec![],
            empty_block_mask: vec![],
            sky: vec![],
            block: vec![],
        },
    }
}

fn load(chunk: [i32; 2], block: Option<&str>) -> Call {
    (
        "load",
        "minecraft:overworld".into(),
        chunk,
        block.map(str::to_owned),
    )
}

fn unload(chunk: [i32; 2], block: Option<&str>) -> Call {
    (
        "unload",
        "minecraft:overworld".into(),
        chunk,
        block.map(str::to_owned),
    )
}

/// Each call without the block read, for the routes whose columns are empty.
fn without_blocks(calls: Vec<Call>) -> Vec<(&'static str, String, [i32; 2])> {
    calls.into_iter().map(|(n, d, c, _)| (n, d, c)).collect()
}

fn unloaded(chunk: [i32; 2]) -> Vec<(&'static str, String, [i32; 2])> {
    vec![("unload", "minecraft:overworld".into(), chunk)]
}

/// A load once the column is in (its block reads through the host), again for a re-sent
/// column; an unload for a forget with the block still readable, and none for a second forget
/// of the same column; on a dimension change, an unload naming the old dimension.
#[test]
#[ignore = "daily"]
fn a_client_tells_a_chunk_packet_and_its_forget_once() {
    const NAME: &str = "game::native_hooks::client_chunk_hooks_tests::a_client_tells_a_chunk_packet_and_its_forget_once";
    if run_fresh_cases(NAME, "HORIZON_CLIENT_CHUNK_PACKET", &["game"]) {
        return;
    }
    with_client(|game| {
        let packet = stone_column(game, 3, 4);
        game.remote_chunk(&packet, None);
        assert_eq!(take_calls(), [load([3, 4], Some("minecraft:stone"))]);

        game.remote_chunk(&packet, None);
        assert_eq!(
            take_calls(),
            [load([3, 4], Some("minecraft:stone"))],
            "a re-sent column is told again"
        );

        let mut gone = [(3, 4)].into_iter().collect();
        game.forget_remote_columns(&mut gone, true);
        assert_eq!(take_calls(), [unload([3, 4], Some("minecraft:stone"))]);
        let mut gone = [(3, 4)].into_iter().collect();
        game.forget_remote_columns(&mut gone, true);
        assert!(take_calls().is_empty(), "a column already gone is not told");

        // A respawn into another level drops the old level's columns, told as the old
        // dimension's, before the client level changes.
        game.remote_chunk(&packet, None);
        take_calls();
        game.enter_remote_dimension(
            &SpawnInfo {
                dimension: "minecraft:the_nether".into(),
                ..overworld()
            },
            false,
            horizon_client::level_loading::Reason::Other,
        );
        assert_eq!(take_calls(), [unload([3, 4], Some("minecraft:stone"))]);
    });
}

/// The centre-move and radius drops tell one unload per dropped column and none for the
/// kept ones; the client level going tells the rest.
#[test]
#[ignore = "daily"]
fn every_route_that_frees_a_column_tells_its_unload() {
    const NAME: &str = "game::native_hooks::client_chunk_hooks_tests::every_route_that_frees_a_column_tells_its_unload";
    if run_fresh_cases(NAME, "HORIZON_CLIENT_CHUNK_ROUTES", &["game"]) {
        return;
    }
    with_client(|game| {
        for x in [1, 7, 13] {
            game.streaming
                .insert_remote(&mut game.world, horizon_client::chunk::Chunk::empty(x, 0));
        }
        assert_eq!(game.drop_remote_chunks_out_of_range((15, 0), 10), [(1, 0)]);
        assert_eq!(without_blocks(take_calls()), unloaded([1, 0]));
        assert_eq!(game.drop_remote_chunks_out_of_range((15, 0), 2), [(7, 0)]);
        assert_eq!(without_blocks(take_calls()), unloaded([7, 0]));
        assert_eq!(game.forget_all_remote_columns(), [(13, 0)]);
        assert_eq!(without_blocks(take_calls()), unloaded([13, 0]));
    });
}
