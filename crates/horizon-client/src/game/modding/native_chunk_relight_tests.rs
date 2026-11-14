//! `level.relight_chunks` and `chunk.resend` through the public handles, with a literal test for
//! each refusal.

use super::native_entity_refs::tests::{with_client_game, with_game};
use horizon_client::chunk::Chunk;
use mod_api::{ChunkRefusal, LevelRef, WorldEpoch};
use std::sync::Arc;

/// A stone roof over the chunk (100, 100), all eight neighbours loaded around it.
fn roofed(game: &mut super::Game<'_>) {
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for dz in -1..=1 {
        for dx in -1..=1 {
            let mut chunk = Chunk::empty(100 + dx, 100 + dz);
            if dx == 0 && dz == 0 {
                for (x, z) in (0..16).flat_map(|x| (0..16).map(move |z| (x, z))) {
                    chunk.set(x, 100, z, stone.0);
                }
            }
            game.world
                .chunks
                .insert((100 + dx, 100 + dz), Arc::new(chunk));
        }
    }
}

fn stale(level: LevelRef) -> LevelRef {
    LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension())
}

/// Blocks written straight into a chunk are lit at once, and only the chunk with all eight
/// neighbours counts; a chunk far from any loaded one is none.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_mod_relights_the_chunks_it_wrote() {
    with_game(|game| {
        roofed(game);
        let level = game.level(game.dim.id()).unwrap();
        let (x, z) = (100 * 16 + 8, 100 * 16 + 8);
        assert_eq!(level.relight_chunks(game, &[[100, 100]]), Ok(1));
        assert!(
            game.world.light(x, 90, z) >> 4 < 15,
            "shaded under the roof"
        );
        assert_eq!(game.world.light(x, 110, z) >> 4, 15, "open sky above it");
        assert_eq!(level.relight_chunks(game, &[[400, 400]]), Ok(0));
    });
}

/// 256 chunks are taken and 257 are refused, whole: nothing is lit.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn relight_chunks_refuses_more_than_256_chunks() {
    with_game(|game| {
        let level = game.level(game.dim.id()).unwrap();
        let chunks: Vec<[i32; 2]> = (0..257).map(|i| [400 + i, 400]).collect();
        assert_eq!(level.relight_chunks(game, &chunks[..256]), Ok(0));
        assert_eq!(
            level.relight_chunks(game, &chunks),
            Err(ChunkRefusal::TooManyChunks {
                asked: 257,
                max: 256
            })
        );
    });
}

/// A handle of an earlier open of the world is `Stale`, for the level and for a chunk of it.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_stale_handle_is_refused_by_name() {
    with_game(|game| {
        roofed(game);
        let level = game.level(game.dim.id()).unwrap();
        assert_eq!(
            stale(level).relight_chunks(game, &[[100, 100]]),
            Err(ChunkRefusal::Stale)
        );
        let chunk = mod_api::__private::game::chunk_ref(stale(level), [100, 100]);
        assert_eq!(chunk.resend(game), Err(ChunkRefusal::Stale));
    });
}

/// A level that is open but not the one being ticked is `NotInPlace`, and queues and lights
/// nothing.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn another_level_is_not_in_place() {
    with_game(|game| {
        roofed(game);
        let overworld = game.level("minecraft:overworld").unwrap();
        game.world.block_log = Some(Vec::new());
        game.enter_level(super::Dim::Nether).unwrap();
        assert_eq!(
            overworld.relight_chunks(game, &[[100, 100]]),
            Err(ChunkRefusal::NotInPlace)
        );
        let chunk = mod_api::__private::game::chunk_ref(overworld, [100, 100]);
        assert_eq!(chunk.resend(game), Err(ChunkRefusal::NotInPlace));
        assert!(game.world.bulk_log.is_empty());
    });
}

/// A chunk that is not loaded is `NotLoaded`; a loaded one is queued to go out whole.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_chunk_is_resent_only_when_loaded() {
    with_game(|game| {
        roofed(game);
        let level = game.level(game.dim.id()).unwrap();
        game.world.block_log = Some(Vec::new());
        game.world.bulk_log.clear();
        let loaded = game.chunk(level, [100, 100]).unwrap();
        assert_eq!(loaded.resend(game), Ok(()));
        assert!(game.world.bulk_log.contains(&(100, 100)));
        let unloaded = mod_api::__private::game::chunk_ref(level, [400, 400]);
        assert_eq!(unloaded.resend(game), Err(ChunkRefusal::NotLoaded));
        assert!(!game.world.bulk_log.contains(&(400, 400)));
    });
}

/// On a client both are `Client`.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_client_cannot_relight_or_resend() {
    with_client_game(|game| {
        roofed(game);
        let level = LevelRef::new(WorldEpoch::new(0), game.dim.key());
        assert_eq!(
            level.relight_chunks(game, &[[100, 100]]),
            Err(ChunkRefusal::Client)
        );
        let chunk = mod_api::__private::game::chunk_ref(level, [100, 100]);
        assert_eq!(chunk.resend(game), Err(ChunkRefusal::Client));
    });
}
