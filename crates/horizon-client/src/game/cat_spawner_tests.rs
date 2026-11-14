//! `CatSpawner.spawnCat`: the cat is created and finalized for a natural
//! spawn, then snapped to the spawn block with `snapTo(spawnPos, 0.0F, 0.0F)`,
//! so it faces yaw 0 and its facing takes no draw from the level's random.
use crate::game::native_machines::tests::server_game;
use horizon_client::mobs::Species;

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_cat_spawner_cat_faces_south_without_a_level_draw() {
    let mut game = server_game();
    let level = game.world.random.state();
    for _ in 0..8 {
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
    // Eight cats and not one draw: the facing is fixed, not drawn.
    assert_eq!(game.world.random.state(), level);
}
