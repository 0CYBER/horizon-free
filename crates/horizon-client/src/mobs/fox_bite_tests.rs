//! `Fox.FoxMeleeAttackGoal.checkAndPerformAttack`'s bite: `Fox.this.playSound(FOX_BITE, 1.0F, 1.0F)`,
//! the `Entity.playSound(sound, volume, pitch)` that `Entity.isSilent` mutes. It is the plain
//! `playSound`, not `Mob.playSound(event)` with `getVoicePitch`, so it draws no random and is
//! heard at pitch 1.
use super::*;
use crate::mobs::{
    goals::GoalSelector,
    world::{test_player, with_test_world},
};
use std::sync::Arc;

/// The `entity.fox.bite` sounds one tick of the fox's melee goal plays, as (position, volume,
/// pitch); `None` without the prepared vanilla data.
fn bite_sounds(silent: bool) -> Option<Vec<([f64; 3], f32, f32)>> {
    let mut out = None;
    let ran = with_test_world(&[], |w, types| {
        let kind = Arc::new(types.types["minecraft:fox"].clone());
        let at = Vec3::new(0.5, 64.0, 0.5);
        let mut m = Mob::new(1, kind, at, 0.0);
        m.silent = silent;
        m.ai.target = Some(Who::Player);
        let mut world = w.clone();
        world.player = Some(test_player(at));
        let mut goals = GoalSelector::default();
        goals.add(1, MeleeAttack::new(1.2, true).bite("entity.fox.bite"));
        let mut fixture = crate::health::TestLevel::new(0);
        let mut events = Vec::new();
        goals.tick(
            &mut m,
            &mut Cx {
                w: &world,
                level: &mut fixture.level(),
                events: &mut events,
            },
        );
        out = Some(
            events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::Sound(name, at, volume, pitch) if *name == "entity.fox.bite" => {
                        Some((*at, *volume, *pitch))
                    }
                    _ => None,
                })
                .collect(),
        );
    });
    ran.then_some(out).flatten()
}

/// A fox's bite is `playSound(FOX_BITE, 1.0F, 1.0F)` through `Entity.playSound`, which
/// `isSilent` mutes, so a `Silent` fox bites in silence; the sound of the others sits at the
/// fox's feet, at volume 1 and pitch 1.
#[test]
fn a_silent_fox_bites_in_silence() {
    let Some(silent) = bite_sounds(true) else {
        return;
    };
    assert!(
        silent.is_empty(),
        "a Silent fox plays no bite sound: {silent:?}"
    );
    let heard = bite_sounds(false).expect("the vanilla data ran for the silent fox too");
    assert_eq!(
        heard,
        vec![([0.5, 64.0, 0.5], 1.0, 1.0)],
        "one bite at the fox's feet, volume 1, pitch 1"
    );
}
