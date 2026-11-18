use super::*;
use std::sync::atomic::Ordering;

#[test]
#[ignore = "daily"]
fn ground_friction_averages_intersecting_blocks_and_only_builds_a_kind_for_listeners() {
    let calls = mod_platform::__private::part_tests::friction_game_thread();
    crate::entry::install_client_hooks();
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        game.world.set(5, 64, 5, stone).unwrap();
        // This box rests on two full blocks; the stone beside it is outside the slab.
        game.world.set(7, 64, 5, stone).unwrap();
        let bounds = Aabb::new(Vec3::new(5.0, 65.0, 5.0), Vec3::new(7.0, 65.5, 6.0));
        let boat = Boat::new(12, "oak".into(), Vec3::new(6.0, 65.0, 5.5), 0.0);
        for (block, expected, reads) in [
            ("minecraft:stone", 0.6, 0),
            ("minecraft:white_wool", 0.79, 1),
        ] {
            let state = registry.default_state(block).unwrap();
            game.world.set(6, 64, 5, state).unwrap();
            calls.store(0, Ordering::SeqCst);
            TYPE_BUILDS.with(|n| n.set(0));
            assert!(
                (game.boat_ground_friction(&boat, bounds) - expected).abs() < 1.0e-7,
                "{block}"
            );
            assert_eq!(calls.load(Ordering::SeqCst), reads);
            TYPE_BUILDS
                .with(|n| assert_eq!(n.get(), reads, "only subscribed blocks build a boat kind"));
        }
    });
}
