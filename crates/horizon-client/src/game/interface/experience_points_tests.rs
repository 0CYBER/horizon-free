//! `Player.giveExperiencePoints` with a negative amount, through the game's own walk:
//! the loss goes down a level at a time through `Player.giveExperienceLevels(-1)`.
use crate::game::game_tests::tests::server_game;
use horizon_client::entities::Experience;

/// A point lost at level 0 leaves nothing of the 100: `giveExperienceLevels(-1)` zeroes the
/// total when the level would drop below 0.
#[test]
#[ignore = "daily"]
fn a_point_lost_at_level_zero_zeroes_the_total() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.experience = Experience {
        level: 0,
        progress: 0.0,
        total: 100,
    };
    game.give_experience_points(-1);
    let xp = &game.experience;
    assert_eq!((xp.level, xp.progress, xp.total), (0, 0.0, 0));
}

/// The mirror: above level 0 the same loss only takes the point given and walks the progress
/// back a level (level 5 needs 17, so it lands on level 4).
#[test]
#[ignore = "daily"]
fn a_point_lost_above_level_zero_only_takes_that_point() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.experience = Experience {
        level: 5,
        progress: 0.0,
        total: 100,
    };
    game.give_experience_points(-1);
    let xp = &game.experience;
    assert_eq!(xp.total, 99);
    assert_eq!(xp.level, 4);
    assert!(
        (xp.progress - (1.0 - 1.0 / 15.0)).abs() < 1e-6,
        "{}",
        xp.progress
    );
}
