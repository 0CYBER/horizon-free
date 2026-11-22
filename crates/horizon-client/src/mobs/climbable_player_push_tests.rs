//! `LivingEntity.isPushable` vetoes a player on a climbable block: a mob that
//! overlaps one gives the pair no push at all — neither to the player nor to
//! itself (`getPushableEntities` leaves the player out, so `doPush` never runs)
//! — where beside a player on the ground the same mob pushes both ways.
use super::*;
use crate::mobs::ai::push_entities;
use crate::mobs::world::with_test_world;
use crate::mobs::{MobEvent, Who};
use std::sync::Arc;

/// A pig at x = 1.0 whose box overlaps the player's at x = 0.6, its own
/// `pushEntities` run with a player that is (`on_climbable`) or is not
/// standing on a ladder: the pushes it gave the player, and its velocity
/// after. None without the vanilla data.
fn pig_against_player(on_climbable: bool) -> Option<(Vec<[f64; 3]>, Vec3)> {
    let mut out = None;
    with_test_world(&[], |w, types| {
        let mut w = w.clone();
        let mut pig = Mob::new(
            1,
            Arc::new(types.types["minecraft:pig"].clone()),
            Vec3::new(1.0, 64.0, 0.5),
            0.0,
        );
        let mut player = crate::mobs::world::test_player(Vec3::new(0.6, 64.0, 0.5));
        // What `player_view` copies from the player's own tick: whether a
        // ladder (or another climbable block) stands at its feet.
        player.on_climbable = on_climbable;
        w.player = Some(player);
        let mut events = Vec::new();
        let mut level = crate::health::TestLevel::new(0);
        push_entities(&mut pig, &w, &mut level.level(), &mut events);
        let pushes = events
            .iter()
            .filter_map(|e| match e {
                MobEvent::Push {
                    target: Who::Player,
                    velocity,
                } => Some(*velocity),
                _ => None,
            })
            .collect();
        out = Some((pushes, pig.velocity));
    });
    out
}

#[test]
fn a_player_on_a_ladder_is_pushed_by_nothing() {
    // `Player.onClimbable` is true: `isPushable` is false, so the pig has no
    // pushable entity in its box and neither of them moves.
    let Some((pushes, velocity)) = pig_against_player(true) else {
        return;
    };
    assert_eq!(
        pushes,
        Vec::<[f64; 3]>::new(),
        "a player on a ladder was pushed"
    );
    assert_eq!(
        velocity,
        Vec3::ZERO,
        "the pig took a push back from a player on a ladder"
    );
    // The control: off the ladder the pair pushes both ways, each by the same
    // 0.05 `Entity.push(Entity)` gives them.
    let Some((pushes, velocity)) = pig_against_player(false) else {
        return;
    };
    assert_eq!(pushes.len(), 1, "the pig pushed the player once");
    let [x, y, z] = pushes[0];
    assert!(
        x < 0.0 && y == 0.0 && z == 0.0,
        "the player is pushed away from the pig: {pushes:?}"
    );
    assert_eq!(
        velocity,
        Vec3::new(-x, 0.0, -z),
        "the pig takes the same push the other way"
    );
}
