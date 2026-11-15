use crate::game::native_client_session::tests::logged_client;
use crate::game::native_entity_refs::tests::with_client_game;
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use horizon_core::geometry::Vec3;

/// The `minecraft:block.stone.fall` sounds the landing player's own client plays
/// for a 5-block fall onto stone, as (volume, pitch).
fn heard_stone_fall(silent: bool) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            game.remote.as_mut().unwrap().loaded_sent = true;
            game.remote.as_mut().unwrap().local_entity_silent = silent;
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(0, 0));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(1, 63, 1, stone).unwrap();
            let mut player = PlayerMotion::new(game.motion, Vec3::new(1.5, 64.1, 1.5)).unwrap();
            player.fall_distance = 5.0;
            player.velocity = Vec3::new(0.0, -0.2, 0.0);
            game.player = Some(player);
            game.audio.capture = Some(Vec::new());
            game.tick_remote().unwrap();
            out = game
                .audio
                .capture
                .as_ref()
                .unwrap()
                .iter()
                .filter(|s| s.event == "minecraft:block.stone.fall")
                .map(|s| (s.volume, s.pitch))
                .collect();
        })
    });
    out
}

#[test]
#[ignore = "daily"]
fn remote_five_block_fall_plays_stone_locally_once() {
    assert_eq!(heard_stone_fall(false), vec![(0.5, 0.75)]);
}

#[test]
#[ignore = "daily"]
fn a_silent_local_player_hears_no_block_fall_sound() {
    assert_eq!(heard_stone_fall(true), Vec::new());
}
