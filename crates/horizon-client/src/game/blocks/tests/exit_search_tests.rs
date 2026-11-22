//! `PortalForcer.findClosestPortalPosition` may read a portal from a saved
//! chunk, but `getLargestRectangleAround` reads the level's blocks: a portal
//! whose rectangle reaches a chunk that has not loaded must wait for it, not
//! be read as air.

use super::*;
use crate::game::game_tests::tests::server_game;
use horizon_client::{
    chunk::Chunk,
    fast_hash::FxHashSet,
    save::{LevelShape, SaveDir},
};
use std::sync::Arc;

fn temp_save(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-exit-search-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn plan_at(at: BlockPos) -> Plan {
    Plan {
        dim: Dim::Overworld,
        approximate: at,
        entry_x_axis: true,
        offset: Vec3::new(0.5, 0.0, 0.0),
        width: 0.6,
        height: 1.8,
        end: false,
    }
}

#[test]
#[ignore = "daily"]
fn an_exit_portal_in_an_unloaded_chunk_waits_for_it() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    let dir = temp_save("one-chunk");
    game.save = Arc::new(SaveDir::new(dir.clone(), LevelShape::OVERWORLD));
    let portal = game
        .pack
        .assets
        .registry
        .default_state("minecraft:nether_portal")
        .unwrap();
    // A 2 by 3 portal in chunk (2, 0), saved, then unloaded.
    game.streaming
        .insert_remote(&mut game.world, Chunk::empty(2, 0));
    for width in 0..2 {
        for height in 0..3 {
            game.world.set(33 + width, 64 + height, 8, portal).unwrap();
        }
    }
    game.save_level(true).unwrap();
    let saved = game.world.chunks.remove(&(2, 0)).unwrap();
    assert!(!game.world.chunks.contains_key(&(2, 0)));
    let plan = plan_at(BlockPos::new(33, 64, 8));
    // The search reads the portal from the saved chunk, so it cannot read
    // its rectangle from the level yet.
    assert_eq!(
        game.search_exit_portal(&plan),
        ExitSearch::Unloaded(BlockPos::new(33, 64, 8), vec![(2, 0)])
    );
    // The exit waits for that chunk, with its portal ticket, and builds
    // nothing (`ensureLoadedAndValid`).
    assert_eq!(game.resolve_exit_portal(&plan), Exit::Waiting);
    assert!(
        game.tickets.portal_tickets().iter().any(|t| t.chunk == (2, 0)),
        "the found portal's chunk has a ticket"
    );
    // The methods mods call keep their old answers: nothing while it waits.
    assert_eq!(game.find_exit_portal(&plan), None);
    assert_eq!(game.nether_exit_portal(&plan), None);
    // Once its chunk is in the level the rectangle is the portal's, not a
    // sheet of air.
    game.world.chunks.insert((2, 0), saved);
    let ExitSearch::Found(found, rectangle) = game.search_exit_portal(&plan) else {
        panic!("the loaded portal is read");
    };
    assert_eq!(found, BlockPos::new(33, 64, 8));
    assert_eq!((rectangle.axis1_size, rectangle.axis2_size), (2, 3));
    assert_eq!(game.find_exit_portal(&plan), Some((found, rectangle)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The rectangle of a portal spanning the x=31/32 chunk boundary reaches the
/// neighbour chunk even when the found block is in the loaded one. Measuring
/// it before the neighbour loads would stop at x=31 and land the traveller on
/// half the portal.
#[test]
#[ignore = "daily"]
fn an_exit_portal_spanning_two_chunks_waits_for_both() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    let dir = temp_save("two-chunks");
    game.save = Arc::new(SaveDir::new(dir.clone(), LevelShape::OVERWORLD));
    let portal = game
        .pack
        .assets
        .registry
        .default_state("minecraft:nether_portal")
        .unwrap();
    // A 2 by 3 portal at x=31..32: chunk (1, 0) holds x=31, chunk (2, 0)
    // holds x=32.
    game.streaming
        .insert_remote(&mut game.world, Chunk::empty(1, 0));
    game.streaming
        .insert_remote(&mut game.world, Chunk::empty(2, 0));
    for width in 0..2 {
        for height in 0..3 {
            game.world.set(31 + width, 64 + height, 8, portal).unwrap();
        }
    }
    game.save_level(true).unwrap();
    // Only the chunk holding the portal's origin stays in the level; the
    // neighbour is saved and its portal blocks are still found from disk.
    let saved = game.world.chunks.remove(&(2, 0)).unwrap();
    let gone: FxHashSet<(i32, i32)> = [(2, 0)].into_iter().collect();
    game.streaming.forget_columns(&mut game.world, &gone);
    assert!(!game.world.chunks.contains_key(&(2, 0)));
    let plan = plan_at(BlockPos::new(31, 64, 8));
    // The rectangle can reach into the saved neighbour, whose blocks answer
    // air until it loads: both chunks wait.
    assert_eq!(
        game.search_exit_portal(&plan),
        ExitSearch::Unloaded(BlockPos::new(31, 64, 8), vec![(1, 0), (2, 0)])
    );
    assert_eq!(game.resolve_exit_portal(&plan), Exit::Waiting);
    for chunk in [(1, 0), (2, 0)] {
        assert!(
            game.tickets.portal_tickets().iter().any(|t| t.chunk == chunk),
            "chunk {chunk:?} has a ticket"
        );
    }
    // Load the neighbour: the rectangle is the whole 2x3 portal, not the
    // loaded half, and the arrival uses it.
    game.streaming
        .insert_remote(&mut game.world, (*saved).clone());
    let ExitSearch::Found(found, rectangle) = game.search_exit_portal(&plan) else {
        panic!("the fully loaded portal is read");
    };
    assert_eq!(found, BlockPos::new(31, 64, 8));
    assert_eq!(rectangle.min_corner, BlockPos::new(31, 64, 8));
    assert_eq!((rectangle.axis1_size, rectangle.axis2_size), (2, 3));
    assert_eq!(game.resolve_exit_portal(&plan), Exit::Found(rectangle));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The walk stops at the first position missing from `portals`, and that
/// position is read by `largest_rectangle_around` to end the rectangle. When
/// the portal-block scan was cut short, the position can be portal in the
/// level, so its chunk has to load before the rectangle is measured.
#[test]
fn the_chunk_of_the_first_non_portal_position_waits() {
    let found = BlockPos::new(30, 64, 8);
    // A run the scan cut at x=47: the rectangle continues to x=50 once
    // chunk (3, 0) is loaded.
    let portals: Vec<_> = (31..=47)
        .map(|x| BlockPos::new(x, 64, 8))
        .collect();
    assert_eq!(
        exit_rectangle_chunks(&portals, found),
        vec![(1, 0), (2, 0), (3, 0)]
    );
}
