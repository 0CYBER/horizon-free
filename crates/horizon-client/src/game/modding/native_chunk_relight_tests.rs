//! `level.relight_chunks` (queued, not lit) and `chunk.resend` through the public handles, with a
//! literal test for each refusal.

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

/// A relight is queued at once, nothing is lit during the call, and the count is the columns
/// queued (the loaded ring included); a chunk far from any loaded one queues none.
#[test]
#[ignore = "daily"]
fn a_mod_queues_the_chunks_it_wrote() {
    with_game(|game| {
        roofed(game);
        let level = game.level(game.dim.id()).unwrap();
        game.world.dirty_light.clear();
        assert_eq!(level.relight_chunks(game, &[[100, 100]]), Ok(9));
        assert_eq!(game.world.dirty_light.len(), 9);
        assert!(game.world.dirty_light.contains(&(100, 100)));
        // Asked again before the light update: nothing new is queued.
        assert_eq!(level.relight_chunks(game, &[[100, 100]]), Ok(0));
        assert_eq!(level.relight_chunks(game, &[[400, 400]]), Ok(0));
        assert_eq!(game.world.dirty_light.len(), 9);
    });
}

/// 256 chunks are taken and 257 are refused, whole: nothing is queued.
#[test]
#[ignore = "daily"]
fn relight_chunks_refuses_more_than_256_chunks() {
    with_game(|game| {
        let level = game.level(game.dim.id()).unwrap();
        game.world.dirty_light.clear();
        let chunks: Vec<[i32; 2]> = (0..257).map(|i| [400 + i, 400]).collect();
        assert_eq!(level.relight_chunks(game, &chunks[..256]), Ok(0));
        assert_eq!(
            level.relight_chunks(game, &chunks),
            Err(ChunkRefusal::TooManyChunks {
                asked: 257,
                max: 256
            })
        );
        assert!(game.world.dirty_light.is_empty());
    });
}

/// A full light queue refuses a call whole, and `max` says what fits now.
#[test]
#[ignore = "daily"]
fn relight_chunks_refuses_a_call_that_overfills_the_queue() {
    with_game(|game| {
        roofed(game);
        let level = game.level(game.dim.id()).unwrap();
        game.world.dirty_light.clear();
        game.world
            .dirty_light
            .extend((0..4096 - 18).map(|i| (1000 + i, 1000)));
        assert_eq!(
            level.relight_chunks(game, &[[100, 100], [100, 100], [100, 100]]),
            Err(ChunkRefusal::TooManyChunks { asked: 3, max: 2 })
        );
        assert_eq!(level.relight_chunks(game, &[[100, 100]]), Ok(9));
        assert_eq!(game.world.dirty_light.len(), 4096 - 18 + 9);
    });
}

/// A handle of an earlier open of the world is `Stale`, for the level and for a chunk of it.
#[test]
#[ignore = "daily"]
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

/// A level that is open but not the one being ticked is `NotInPlace`, and queues nothing.
#[test]
#[ignore = "daily"]
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
#[ignore = "daily"]
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
#[ignore = "daily"]
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
