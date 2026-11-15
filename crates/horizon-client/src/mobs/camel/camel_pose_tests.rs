//! A sitting camel that gets a walk target: `Camel.CamelMoveControl` stands it
//! up (and the 52-tick stand-up holds it in place before it walks), while
//! `Camel.tickRidden`'s forward key waits for a pose transition to finish.
use super::*;
use crate::mobs::equine::Rider;
use crate::mobs::world::{test_player, with_built_test_world, with_test_world};
use crate::mobs::{Mob, MobEvent};
use horizon_core::geometry::Vec3;
use horizon_core::random::LegacyRandom;
use std::sync::Arc;

/// A flat grass shelf the camel can path across.
fn shelf(world: &mut crate::world::World, registry: &horizon_world::registry::BlockRegistry) {
    let grass = registry.default_state("minecraft:grass_block").unwrap();
    for cx in -3..3 {
        for cz in -3..3 {
            world
                .chunks
                .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
        }
    }
    for x in -8..=8 {
        for z in -8..=8 {
            world.set(x, 63, z, grass).unwrap();
        }
    }
}

/// A rider holding the forward key.
fn forward_rider() -> Rider {
    Rider {
        strafe: 0.0,
        forward: 1.0,
        yaw: 0.0,
        pitch: 0.0,
        jump: 0.0,
        steer: false,
        sprint: false,
        jumping: false,
        client: false,
    }
}

/// `Camel.CamelMoveControl.tick`: a walk target (`MOVE_TO`) stands a sitting
/// camel up, with its stand sound, before the move control carries the target
/// out; `Camel.travel` then keeps it in place for the 52 ticks of the
/// stand-up, and it walks once the pose settles.
#[test]
fn a_walk_target_stands_a_sitting_camel_up_and_then_it_walks() {
    let mut outcome = None;
    let ran = with_built_test_world(&[], shelf, |w, types| {
        let kind = Arc::new(types.types["minecraft:camel"].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        let mut world = w.clone();
        let mut player = test_player(Vec3::new(6.5, 64.0, 0.5));
        player.main_hand = Some("minecraft:cactus".into());
        world.player = Some(player);
        let mut lr = horizon_core::random::LevelRandom::for_level(LegacyRandom::new(0));
        let mut events = Vec::new();
        // Settle on the grass.
        for _ in 0..5 {
            crate::mobs::ai::tick(&mut m, &world, &mut lr, &mut events);
        }
        // Sit it down and let the 40-tick kneel finish.
        sit_down(&mut m, &mut events);
        m.age += 40;
        assert!(sitting(&m) && !in_transition(&m), "a settled sitting camel");
        events.clear();
        let before = m.position.x;
        // The tempt goal starts on a selector tick, then its next tick asks
        // for a walk target.
        let mut ticks = 0;
        while sitting(&m) && ticks < 4 {
            crate::mobs::ai::tick(&mut m, &world, &mut lr, &mut events);
            ticks += 1;
        }
        let stood = !sitting(&m);
        let sound = events
            .iter()
            .any(|e| matches!(e, MobEvent::Sound("entity.camel.stand", _, _, _)));
        // The stand-up's 52 ticks hold it where it is...
        while pose_time(&m) < 51 {
            crate::mobs::ai::tick(&mut m, &world, &mut lr, &mut events);
        }
        let held = (m.position.x - before).abs();
        // ...then it walks toward the player.
        for _ in 0..25 {
            crate::mobs::ai::tick(&mut m, &world, &mut lr, &mut events);
        }
        let walked = m.position.x - before;
        outcome = Some((stood, sound, held, walked));
    });
    if !ran {
        return;
    }
    let (stood, sound, held, walked) = outcome.unwrap();
    assert!(
        stood && sound,
        "the walk target stands the sitting camel up with its stand sound"
    );
    assert!(
        held < 0.01,
        "the stand-up transition holds it in place, moved {held}"
    );
    assert!(
        walked > 0.5,
        "then it walks toward the tempting player, moved {walked}"
    );
}

/// `Camel.tickRidden`: `controller.zza > 0.0F && isCamelSitting() &&
/// !isInPoseTransition()` - pressing forward during the 40-tick kneel leaves
/// the camel sitting until the transition is over.
#[test]
fn pressing_forward_during_the_kneel_waits_for_the_transition() {
    let mut outcome = None;
    let ran = with_test_world(&[], |_, types| {
        let kind = Arc::new(types.types["minecraft:camel"].clone());
        let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
        m.age = 100;
        let mut events = Vec::new();
        sit_down(&mut m, &mut events);
        m.rider = Some(forward_rider());
        // 10 ticks into the 40-tick kneel.
        m.age += 10;
        crate::mobs::equine::ridden_step(&mut m);
        let during_kneel = sitting(&m);
        // The kneel ends at 40: now the forward key stands it up.
        m.age += 30;
        crate::mobs::equine::ridden_step(&mut m);
        outcome = Some((during_kneel, sitting(&m)));
    });
    if !ran {
        return;
    }
    assert_eq!(
        outcome,
        Some((true, false)),
        "forward during the kneel leaves it sitting; once the kneel is over it stands up"
    );
}

/// `CamelAi` gates `BabyFollowAdult` with `Predicate.not(Camel::refuseToMove)`:
/// a sitting baby camel does not follow its parent, whose walk target would
/// otherwise stand it up.
#[test]
fn a_sitting_baby_camel_does_not_follow_its_parent() {
    use crate::mobs::Species;
    use crate::mobs::sensor_scan_tests::kind as cow_kind;
    let mut parent_kind = (*cow_kind()).clone();
    parent_kind.id = "minecraft:camel".into();
    parent_kind.species = Species::Camel;
    let parent = Mob::new(2, Arc::new(parent_kind), Vec3::new(0.5, 64.0, 5.5), 0.0);
    let seen = parent.seen(None);
    let mut outcome = None;
    let ran = with_built_test_world(&[seen], shelf, |w, types| {
        let kind = Arc::new(types.types["minecraft:camel"].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        m.extra.age = -24_000;
        let mut lr = horizon_core::random::LevelRandom::for_level(LegacyRandom::new(0));
        let mut events = Vec::new();
        // Sit it before it ever follows the parent, and let the kneel finish.
        sit_down(&mut m, &mut events);
        m.age += 40;
        // A camel that sat after 20 idle seconds has no stroll to take.
        m.ai.no_action_time = 100;
        assert!(sitting(&m) && !in_transition(&m), "a settled sitting camel");
        let mut stood = false;
        for _ in 0..20 {
            crate::mobs::ai::tick(&mut m, w, &mut lr, &mut events);
            stood |= !sitting(&m);
        }
        outcome = Some(stood);
    });
    if !ran {
        return;
    }
    assert_eq!(
        outcome,
        Some(false),
        "a sitting baby camel does not follow its parent"
    );
}
