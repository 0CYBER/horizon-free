//! `Monster.updateNoActionTime` (`Monster.aiStep`): +2 a tick while
//! `getLightLevelDependentMagicValue` is above 0.5. A blaze and a vex always
//! report 1.0, so with `Mob.serverAiStep`'s own +1 they gain 3 a tick whatever
//! the light and pass the despawn threshold of 600 after 201 ticks.
use super::*;
use crate::mobs::ai::tick;
use crate::mobs::world::with_test_world;
use horizon_core::{geometry::Vec3, random::LegacyRandom};
use std::sync::Arc;

/// A fully ticking mob of that type, hovering so the empty test world's floor
/// (there is none) cannot matter.
fn mob(types: &crate::mobs::MobTypes, name: &str) -> Mob {
    let mut m = Mob::new(
        1,
        Arc::new(types.types[name].clone()),
        Vec3::new(0.5, 64.0, 0.5),
        0.0,
    );
    m.no_gravity = true;
    m
}

/// `no_action_time` after 200 and after 201 full ticks: the tick that passes
/// the threshold is the 201st.
fn no_action_times(types: &crate::mobs::MobTypes, w: &MobWorld<'_>, name: &str) -> (i32, i32) {
    let mut m = mob(types, name);
    let mut lr = LegacyRandom::new(7);
    let mut events = Vec::new();
    let mut seen = Vec::with_capacity(201);
    for _ in 0..201 {
        assert!(tick(&mut m, w, &mut lr, &mut events), "{name} went away");
        seen.push(m.ai.no_action_time);
    }
    (seen[199], seen[200])
}

#[test]
fn a_blaze_and_a_vex_keep_their_extra_no_action_time_in_the_dark() {
    let mut out = None;
    if !with_test_world(&[], |w, types| {
        // Still dark at midnight, with the player 40 blocks off, past the
        // 32 blocks inside which `checkDespawn` resets the count.
        let dark = MobWorld {
            daylight: false,
            nearest_player: Some(Vec3::new(40.5, 64.0, 0.5)),
            ..w.clone()
        };
        let lit = MobWorld {
            daylight: true,
            ..dark.clone()
        };
        assert!(dark.brightness(0, 65, 0) < 0.5, "the fixture is dark");
        assert!(lit.brightness(0, 65, 0) > 0.5, "the fixture is lit");
        out = Some(
            ["minecraft:blaze", "minecraft:vex", "minecraft:zombie"].map(|name| {
                (
                    no_action_times(types, &dark, name),
                    no_action_times(types, &lit, name),
                )
            }),
        );
    }) {
        return;
    }
    let Some(
        [
            (blaze_dark, blaze_lit),
            (vex_dark, vex_lit),
            (zombie_dark, zombie_lit),
        ],
    ) = out
    else {
        return;
    };
    // A blaze and a vex add 2 in any light (their magic value is a constant 1.0).
    assert_eq!(blaze_dark, (600, 603), "a blaze in the dark gains 3 a tick");
    assert_eq!(blaze_lit, (600, 603), "a blaze in the light");
    assert_eq!(vex_dark, (600, 603), "a vex in the dark gains 3 a tick");
    assert_eq!(vex_lit, (600, 603), "a vex in the light");
    // Any other monster only adds its 2 in light above 0.5.
    assert_eq!(
        zombie_dark,
        (200, 201),
        "a zombie in the dark gains 1 a tick"
    );
    assert_eq!(
        zombie_lit,
        (600, 603),
        "a zombie in the light gains 3 a tick"
    );
}
