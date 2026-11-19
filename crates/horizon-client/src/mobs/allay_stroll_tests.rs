//! `AllayAi`'s idle `RandomStroll.fly(1.0F)`, whose picker is
//! `getTargetFlyPos`: `AirAndWaterRandomPos.getPos(body, 10, 7, -2, view.x,
//! view.z, PI / 2)`, so an allay wanders to an air or water spot up to ten
//! blocks out (a `WaterAvoidingRandomFlyingGoal`, which Horizon ran here,
//! tries a hover spot first and then reaches eight at most).
use super::*;
use crate::mobs::EntityRandom;
use crate::mobs::goals::Cx;
use crate::mobs::world::with_test_world;
use horizon_core::geometry::Vec3;
use horizon_core::random::LegacyRandom;
use std::sync::Arc;

/// The furthest block, horizontally, that a wandering allay walked to: the
/// stroll's target while it runs (nothing else of an allay with no item and
/// no liked player moves it). `10 by 7` reaches ten blocks, `8 by 7`/`8 by 4`
/// eight.
#[test]
fn an_allay_wanders_up_to_ten_blocks_away() {
    let mut farthest = 0.0_f64;
    let ran = with_test_world(&[], |w, types| {
        let kind = Arc::new(types.types["minecraft:allay"].clone());
        for seed in 0..40 {
            let mut m = Mob::with_random(
                1,
                kind.clone(),
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
                EntityRandom::own(LegacyRandom::new(seed)),
            );
            register(&mut m);
            let mut test_level = crate::health::TestLevel::new(seed);
            let mut events = Vec::new();
            let mut goals = std::mem::take(&mut m.ai.goals);
            let mut cx = Cx {
                w,
                level: &mut test_level.level(),
                events: &mut events,
            };
            // The stroll's first draw is 1 in 60: give each allay 300 goal ticks.
            for _ in 0..300 {
                goals.tick(&mut m, &mut cx);
                if let Some(p) = m.ai.nav.target_pos {
                    // `nav_move_to` floors the wanted spot: the block it walks to,
                    // from the allay's block (0, 64, 0).
                    farthest = farthest.max(f64::from(p[0]).abs().max(f64::from(p[2]).abs()));
                    break;
                }
            }
            m.ai.goals = goals;
        }
    });
    if !ran {
        return;
    }
    assert!(
        (9.0..=10.0).contains(&farthest),
        "an allay wanders to spots up to ten blocks out, not eight: {farthest}"
    );
}
