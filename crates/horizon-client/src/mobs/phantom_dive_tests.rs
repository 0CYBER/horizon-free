//! The dive's cadence and its bite, through the real AI tick:
//! `Phantom.PhantomAttackStrategyGoal` (`adjustedTickDelay(10)` on a goal that
//! does not ask for an every-tick update) and `PhantomSweepAttackGoal`'s
//! `levelEvent(1039, blockPosition(), 0)` for the bite, which a silent phantom
//! skips.
use super::*;
use crate::mobs::world::{test_player, with_built_test_world};
use horizon_core::random::LegacyRandom;
use std::sync::Arc;

/// Ages (ticks lived) at which the phantom's swoop begins, and the age its
/// chase goal started, with a player standing 10 blocks below it.
fn dive_ages(ticks: i32) -> Option<(i32, Vec<i32>)> {
    let mut out = None;
    let ran = with_built_test_world(
        &[],
        |_, _| {},
        |w, types| {
            let mut w = w.clone();
            w.player = Some(test_player(Vec3::new(0.5, 64.0, 0.5)));
            let kind = Arc::new(types.types["minecraft:phantom"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 74.0, 0.5), 0.0);
            register(&mut m);
            m.persistent = true;
            let mut lr = LegacyRandom::new(1);
            let mut events = Vec::new();
            let (mut started, mut swoops, mut was) = (None, Vec::new(), false);
            for _ in 0..ticks {
                crate::mobs::ai::tick(&mut m, &w, &mut lr, &mut events);
                if started.is_none() && m.ai.goals.is_running("AttackStrategy") {
                    started = Some(m.age as i32);
                }
                if m.extra.flag && !was {
                    swoops.push(m.age as i32);
                }
                was = m.extra.flag;
            }
            if let Some(started) = started {
                out = Some((started, swoops));
            }
        },
    );
    if !ran {
        return None;
    }
    out
}

/// `PhantomAttackStrategyGoal.start` halves `10` to five
/// (`adjustedTickDelay` = `reducedTickDelay` for a goal that does not require
/// an every-tick update) and the goal then ticks every second tick
/// (`Mob.serverAiStep`'s `(tickCount + id) % 2`), so the first dive comes eight
/// ticks after the chase starts. A goal that also asks for an every-tick
/// update ticks five times in five ticks instead, halving the dive's delay.
#[test]
fn the_first_dive_comes_eight_ticks_after_the_chase_starts() {
    let Some((started, swoops)) = dive_ages(60) else {
        return;
    };
    let first = swoops.first().copied().expect("the phantom should swoop");
    let gap = first - started;
    assert_eq!(
        gap, 8,
        "the first dive comes eight ticks after the chase starts: vanilla's \
         adjustedTickDelay(10) is five half-ticks, each two ticks. Started at \
         {started}, swooped at {first}"
    );
}

/// `PhantomSweepAttackGoal.tick`: a bite is `levelEvent(1039, blockPosition(), 0)`
/// at the phantom's block, played by the client at 0.3 volume with a 0.9-1.0
/// pitch — and skipped for a silent phantom (`if (!Phantom.this.isSilent())`).
#[test]
fn a_bite_is_level_event_1039_at_the_phantoms_block_unless_it_is_silent() {
    let mut outcomes = None;
    let ran = with_built_test_world(
        &[],
        |_, _| {},
        |w, types| {
            let mut w = w.clone();
            w.player = Some(test_player(Vec3::new(0.5, 64.0, 0.5)));
            let kind = Arc::new(types.types["minecraft:phantom"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            register(&mut m);
            m.ai.target = Some(Who::Player);
            m.extra.flag = true;
            let mut lr = LegacyRandom::new(0);
            let mut sweep = SweepAttack {
                scared: false,
                cat_search: 0,
            };
            let mut bite = |m: &mut Mob| {
                let mut events = Vec::new();
                let mut cx = Cx {
                    w: &w,
                    lr: &mut lr,
                    events: &mut events,
                };
                sweep.tick(m, &mut cx);
                events
            };
            let loud = bite(&mut m);
            m.extra.flag = true;
            m.silent = true;
            let quiet = bite(&mut m);
            let block = [
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ];
            outcomes = Some((loud, quiet, block));
        },
    );
    if !ran {
        return;
    }
    let (loud, quiet, block) = outcomes.unwrap();
    assert!(
        loud.iter()
            .any(|e| matches!(e, MobEvent::AttackPlayer(_, _))),
        "the bite should still hurt the player: {loud:?}"
    );
    assert!(
        loud.iter().any(|e| matches!(
            e,
            MobEvent::LevelEvent {
                id: 1039,
                pos,
                data: 0
            } if *pos == block
        )),
        "the bite should be level event 1039 at the phantom's block {block:?}, \
         not a full-volume sound: {loud:?}"
    );
    assert!(
        !quiet.iter().any(|e| matches!(
            e,
            MobEvent::LevelEvent { id: 1039, .. } | MobEvent::Sound("entity.phantom.bite", ..)
        )),
        "a silent phantom's bite makes no sound: {quiet:?}"
    );
}
