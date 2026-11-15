use super::*;

#[test]
fn death_reward_uses_levels_and_caps_the_recoverable_points() {
    for (level, reward) in [(0, 0), (1, 7), (14, 98), (15, 100), (30, 100)] {
        let xp = Experience {
            level,
            progress: 0.75,
            total: 10_000,
            score: 3,
        };
        assert_eq!(xp.death_reward(false), reward);
        assert_eq!(xp.death_reward(true), 0);
    }
}

/// `Player.giveExperiencePoints` walks a loss down through
/// `Player.giveExperienceLevels(-1)`, which zeroes the total when the level
/// would drop below 0: a point lost at level 0 leaves nothing of the 100.
#[test]
fn a_point_lost_at_level_zero_zeroes_the_total() {
    let mut xp = Experience {
        level: 0,
        progress: 0.0,
        total: 100,
        score: 10,
    };
    assert_eq!(xp.give(-1), None);
    assert_eq!((xp.level, xp.progress, xp.total), (0, 0.0, 0));
    assert_eq!(xp.score, 9);
}

/// The mirror: above level 0 the same loss only takes the point given and
/// walks the progress back a level (level 5 needs 17, so it lands on level 4).
#[test]
fn a_point_lost_above_level_zero_only_takes_that_point() {
    let mut xp = Experience {
        level: 5,
        progress: 0.0,
        total: 100,
        score: 10,
    };
    assert_eq!(xp.give(-1), None);
    assert_eq!(xp.total, 99);
    assert_eq!(xp.level, 4);
    assert!(
        (xp.progress - (1.0 - 1.0 / 15.0)).abs() < 1e-6,
        "{}",
        xp.progress
    );
}

#[test]
fn respawn_clears_all_xp_unless_it_is_kept() {
    let mut xp = Experience::default();
    xp.give(155);
    let before = (xp.level, xp.progress, xp.total, xp.score);
    xp.respawn(true);
    assert_eq!((xp.level, xp.progress, xp.total, xp.score), before);
    xp.respawn(false);
    assert_eq!((xp.level, xp.progress, xp.total, xp.score), (0, 0.0, 0, 0));
    xp.give(7);
    assert_eq!((xp.level, xp.progress, xp.total, xp.score), (1, 0.0, 7, 7));
}
