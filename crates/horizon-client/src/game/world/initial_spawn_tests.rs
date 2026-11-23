//! A world's spawn search follows its level seed: two searches on one seed
//! walk the same candidates, the ones the level random's first draw names.
use super::*;
use horizon_client::world_spawn::{PlayerSpawnFinder, SpawnCandidate};
use horizon_core::random::{LegacyRandom, RandomSource};

/// The first eight column candidates of a spawn search on `seed`, in order.
fn candidates(game: &mut Game<'_>, seed: i64, suggestion: [i32; 3]) -> Vec<(i32, i32)> {
    game.world.random.set_seed(seed);
    let mut state = None;
    // The suggestion's chunk is not loaded, so the search keeps the finder it
    // built and asks for that candidate's chunk.
    assert!(game.search_spawn(suggestion, &mut state).is_err());
    let mut finder = state.expect("the search keeps its finder");
    let mut columns = Vec::new();
    while columns.len() < 8 {
        match finder.next_candidate() {
            Some(SpawnCandidate::Column(x, z)) => columns.push((x, z)),
            _ => break,
        }
    }
    columns
}

#[test]
#[ignore = "daily"]
fn a_spawn_search_follows_the_level_seed() {
    native_entity_refs::tests::with_game(|game| {
        assert!(game.ui.rules.set_string("respawn_radius", "10"));
        let suggestion = [100, 70, -40];
        let seed = 19_426;

        let first = candidates(game, seed, suggestion);
        let second = candidates(game, seed, suggestion);
        assert_eq!(
            first, second,
            "the same level seed must walk the same spawn candidates"
        );

        // PlayerSpawnFinder.findSpawn draws its start offset from the level
        // random, over the square of the respawn radius (10, the default).
        let mut level_random = LegacyRandom::new(seed);
        let offset = level_random.next_int_bound(21 * 21).unwrap();
        let mut expected = PlayerSpawnFinder::new(suggestion, 10, |_| offset);
        let mut expected_columns = Vec::new();
        while expected_columns.len() < 8 {
            match expected.next_candidate() {
                Some(SpawnCandidate::Column(x, z)) => expected_columns.push((x, z)),
                _ => break,
            }
        }
        assert_eq!(
            first, expected_columns,
            "the candidates come from the level random's first draw"
        );
    });
}
