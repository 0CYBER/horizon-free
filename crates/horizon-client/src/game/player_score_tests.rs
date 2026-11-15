//! `Player.getScore`: `giveExperiencePoints` adds to it, setting the xp bar
//! does not, and it is saved; `ServerPlayer.restoreFrom` keeps it only with
//! keep-inventory (or a spectator).
use super::super::*;
use crate::game::native_machines::tests::server_game;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use horizon_world::nbt::Tag;

fn score(game: &mut Game<'_>) -> Option<i32> {
    match game.player_nbt()?.get("Score") {
        Some(Tag::Int(score)) => Some(*score),
        _ => None,
    }
}

/// The `Score` in the player file a save writes and a load reads back.
fn saved_score(game: &mut Game<'_>) -> i32 {
    game.sync_level();
    let dir = std::env::temp_dir().join(format!(
        "horizon-player-score-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    game.level.write(&dir).unwrap();
    let back = horizon_client::save::Level::read(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    match back.player_tag().get("Score") {
        Some(Tag::Int(score)) => *score,
        other => panic!("saved Score missing: {other:?}"),
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn experience_points_raise_the_score_that_set_points_and_a_normal_respawn_drop() {
    let mut game = server_game();
    game.options.dedicated = None;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 4.5)).unwrap());
    game.set_allow_commands(true);
    assert_eq!(score(&mut game), Some(0));
    assert!(game.run_command("xp add @s 10 points"));
    assert_eq!(score(&mut game), Some(10));
    assert_eq!(saved_score(&mut game), 10);
    // ExperienceCommand.Type.POINTS.set calls setExperiencePoints, not
    // giveExperiencePoints: the bar changes, the score does not.
    assert!(game.run_command("xp set @s 0 points"));
    assert_eq!(score(&mut game), Some(10));
    // ServerPlayer.transferInventoryXpAndScore, which restoreFrom calls only
    // with KEEP_INVENTORY or for a spectator: a normal respawn drops the score.
    game.experience.respawn(false);
    assert_eq!(score(&mut game), Some(0));
    assert_eq!(saved_score(&mut game), 0);
    assert!(game.run_command("xp add @s 10 points"));
    game.experience.respawn(true);
    assert_eq!(score(&mut game), Some(10));
    assert_eq!(saved_score(&mut game), 10);
}
