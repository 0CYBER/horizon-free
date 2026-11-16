use super::*;

/// `Game::give_experience_points` without the mods: the points go to the bar, then one level at a time
/// for each level the bar crosses (`Player.giveExperiencePoints`).
fn give(xp: &mut Experience, points: i32) {
    xp.add_points(points);
    while xp.progress < 0.0 {
        let lost = xp.progress * Experience::needed(xp.level) as f32;
        let had_level = xp.level > 0;
        xp.add_levels(-1);
        xp.progress = if had_level {
            1.0 + lost / Experience::needed(xp.level) as f32
        } else {
            0.0
        };
    }
    while xp.progress >= 1.0 {
        xp.progress = (xp.progress - 1.0) * Experience::needed(xp.level) as f32;
        xp.add_levels(1);
        xp.progress /= Experience::needed(xp.level) as f32;
    }
}

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
    give(&mut xp, -1);
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
    give(&mut xp, -1);
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
    give(&mut xp, 155);
    let before = (xp.level, xp.progress, xp.total, xp.score);
    xp.respawn(true);
    assert_eq!((xp.level, xp.progress, xp.total, xp.score), before);
    xp.respawn(false);
    assert_eq!((xp.level, xp.progress, xp.total, xp.score), (0, 0.0, 0, 0));
    give(&mut xp, 7);
    assert_eq!((xp.level, xp.progress, xp.total, xp.score), (1, 0.0, 7, 7));
}
