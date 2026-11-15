//! A block's parts answer the state the camera sees of it (`state_at_viewpoint`), and the
//! camera's fluid follows the answer.
use crate::game::native_entity_refs::tests::with_client_game;
use horizon_client::{chunk::Chunk, fog::FogType};
use std::sync::Arc;

#[test]
#[ignore = "daily"]
fn the_camera_reads_the_state_a_block_part_says_it_sees() {
    // The fixture's part: glass is seen as water below half its height.
    mod_platform::__private::part_tests::state_at_viewpoint_game_thread();
    with_client_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let glass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:glass")
            .unwrap();
        game.world.set(4, 64, 4, glass).unwrap();
        assert_eq!(game.camera_fog_type([4.5, 64.3, 4.5]), FogType::Water);
        assert_eq!(game.camera_fog_type([4.5, 64.7, 4.5]), FogType::Atmospheric);
    });
}
