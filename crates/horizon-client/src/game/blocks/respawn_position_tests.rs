//! A block's parts decide where a player whose respawn point it is stands up
//! (`respawn_position`).
use super::respawn_anchor::RespawnConfig;
use crate::game::{Dim, native_entity_refs::tests::with_game};
use horizon_client::chunk::Chunk;
use horizon_core::geometry::Vec3;
use std::sync::Arc;

#[test]
#[ignore = "daily"]
fn a_block_part_moves_or_refuses_the_respawn_spot() {
    // The fixture's parts: glass puts the player two blocks above it, and a red bed refuses.
    mod_platform::__private::part_tests::respawn_position_game_thread();
    with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let point = |pos: [i32; 3]| RespawnConfig {
            dim: Dim::Overworld,
            pos,
            yaw: 45.0,
            pitch: 0.0,
            forced: true,
        };
        // Vanilla finds no spot inside glass; its part answers one above it.
        let glass = registry.default_state("minecraft:glass").unwrap();
        game.world.set(4, 64, 4, glass).unwrap();
        let spot = game.find_respawn_and_use(point([4, 64, 4]), true).unwrap();
        assert_eq!(spot, Some((Vec3::new(4.5, 66.0, 4.5), 10.0, 20.0)));
        // A bed whose part refuses gives no spot.
        let bed = registry.default_state("minecraft:red_bed").unwrap();
        game.world.set(8, 64, 8, bed).unwrap();
        let spot = game.find_respawn_and_use(point([8, 64, 8]), true).unwrap();
        assert_eq!(spot, None);
    });
}
