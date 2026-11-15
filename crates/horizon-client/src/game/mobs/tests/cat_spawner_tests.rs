//! `CatSpawner.spawnCat`: the cat is created and finalized for a natural
//! spawn, then snapped to the spawn block with `snapTo(spawnPos, 0.0F, 0.0F)`,
//! so it faces yaw 0 and its facing takes no draw from the level's random.
use crate::game::game_tests::tests::server_game;
use horizon_client::mobs::Species;
use horizon_core::random::RandomSource;

#[test]
#[ignore = "daily"]
fn a_cat_spawner_cat_faces_south_without_a_level_draw() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    let mut level = game.world.random.clone();
    for _ in 0..8 {
        // Mob.finalizeSpawn: triangle follow-range bonus and left hand;
        // Cat.finalizeSpawn: variant and sound. No additional yaw draw.
        level.next_double();
        level.next_double();
        level.next_float();
        level.next_int_bound(11).unwrap();
        level.next_int_bound(3).unwrap();
        let before = game.mobs.len();
        game.spawn_cat([4, 100, 8], false);
        assert_eq!(game.mobs.len(), before + 1, "one cat");
        let cat = game.mobs.last().expect("cat");
        assert_eq!(cat.species(), Species::Cat);
        assert_eq!(cat.yaw, 0.0);
        assert_eq!(cat.body_yaw, 0.0);
        assert_eq!(cat.head_yaw, 0.0);
        // `snapTo(BlockPos, ..)` is `Vec3.atBottomCenterOf`: x + 0.5, y, z + 0.5.
        assert_eq!(
            [cat.position.x, cat.position.y, cat.position.z],
            [4.5, 100.0, 8.5]
        );
    }
    // Finalization draws remain, but the fixed facing adds none.
    assert_eq!(game.world.random.state(), level.state());
}
