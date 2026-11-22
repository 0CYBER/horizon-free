use super::*;
use std::sync::atomic::Ordering;

#[test]
#[ignore = "daily"]
fn ice_beside_bottom_slabs_keeps_vanilla_boat_speed() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let slab = registry.default_state("minecraft:oak_slab").unwrap();
        let ice = registry.default_state("minecraft:ice").unwrap();
        for (z, state) in [(5, slab), (6, ice), (7, slab)] {
            game.world.set(10, 64, z, state).unwrap();
        }

        // Vanilla 26.3 AbstractBoat.tick with forward held, starting at ice height
        // with the bow over the ice. The hull crosses both lower slabs.
        let vanilla_ticks = [
            (5.758_000_005_782, 0.383_000_005_782),
            (6.173_340_017_859, 0.415_340_012_077),
            (6.620_373_236_722, 0.447_033_218_863),
            (7.098_465_798_841, 0.478_092_562_119),
            (7.606_996_517_942, 0.508_530_719_101),
            (8.145_356_631_466, 0.538_360_113_524),
        ];
        let mut boat = Boat::new(12, "oak".into(), Vec3::new(10.5, 65.0, 5.375), 0.0);
        boat.velocity = Vec3::new(0.0, 0.0, 0.35);
        boat.input = [false, false, true, false];
        for (tick, (z, speed)) in vanilla_ticks.into_iter().enumerate() {
            boat = game.tick_boat(boat, true);
            assert_eq!(boat.position.y, 65.0, "tick {}", tick + 1);
            assert!(
                (boat.position.z - z).abs() < 1e-10 && (boat.velocity.z - speed).abs() < 1e-10,
                "tick {}: position {}, speed {}; vanilla position {z}, speed {speed}",
                tick + 1,
                boat.position.z,
                boat.velocity.z,
            );
        }

        // At slab height the face of the higher ice block stops the hull.
        let mut boat = Boat::new(12, "oak".into(), Vec3::new(10.5, 64.5, 5.125), 0.0);
        boat.velocity = Vec3::new(0.0, 0.0, 0.35);
        boat.input = [false, false, true, false];
        let boat = game.tick_boat(boat, true);
        assert_eq!(boat.position, Vec3::new(10.5, 64.5, 5.3125));
        assert_eq!(boat.velocity, Vec3::ZERO);
    });
}

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
