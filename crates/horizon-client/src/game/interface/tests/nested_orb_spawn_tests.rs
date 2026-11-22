#[test]
#[ignore = "daily"]
fn experience_pickup_keeps_nested_orbs_until_the_next_ordered_pass() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        game.block_ticks
            .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
            .unwrap();
        for (x, value) in [(7.5, 1), (9.5, 3), (11.5, 7)] {
            game.add_orb([x, 80.0, 3.5], value);
        }
        let _ = game.orbs.remove(1);
        let mut expected: Vec<_> = game.orbs.iter().map(|o| (o.id, o.join)).collect();
        crate::game::entity::taken_pass_tests::with_spawn(
            |g| g.add_orb([3.5, 80.0, 3.5], 1),
            || game.tick_experience(),
        );
        assert_eq!(game.orbs.len(), 3);
        let join = game.orbs[2].join;
        expected.push((game.orbs[2].id, join));
        assert_eq!(
            game.orbs.iter().map(|o| (o.id, o.join)).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(game.orbs[2].age, 0);
        game.ordered.announced = horizon_client::join_seq::JoinSeq::MAX;
        game.tickets.update(None);
        game.begin_ordered_ticks();
        game.run_ordered_ticks(horizon_client::join_seq::JoinSeq::MAX, false)
            .unwrap();
        let spawned = game.orbs.iter().find(|o| o.join == join).unwrap();
        assert_eq!(spawned.age, 1);
        assert_eq!(
            game.orbs.iter().map(|o| (o.id, o.join)).collect::<Vec<_>>(),
            expected
        );
    });
}
