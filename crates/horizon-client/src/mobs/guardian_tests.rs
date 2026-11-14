//! `Guardian.hurtServer`'s spikes and `Guardian.registerGoals`' priority-5
//! `MoveTowardsRestrictionGoal`: a still guardian's spikes hurt the living
//! thing that hit it at the instant of the hit, and a guardian taken out of
//! its home swims back into it.
use super::*;
use crate::mobs::goals::Cx;
use crate::mobs::guardian::{elder_server_ai_step, register};
use crate::mobs::world::{with_built_test_world, with_test_world};
use horizon_core::geometry::Vec3;
use std::sync::Arc;

/// The damage events in an outbox (a hurt also leaves its sound there).
fn spikes(events: &[MobEvent]) -> Vec<&MobEvent> {
    events
        .iter()
        .filter(|e| {
            matches!(
                e,
                MobEvent::DamagePlayer { .. } | MobEvent::DamageMob { .. }
            )
        })
        .collect()
}

/// `Guardian.hurtServer`: a guardian that is not moving hurts a direct living
/// cause for 2 thorns damage, before `LivingEntity.hurtServer`'s guards, so a
/// blow it refuses draws the spikes too. Thorns, magic and a projectile's
/// arrow (a direct entity that is not living) are exempt.
#[test]
fn a_still_guardian_spikes_whoever_hits_it() {
    let by_mob = |kind| Damage {
        kind,
        ..Damage::mob(7, [3.0; 3])
    };
    let by_player = |kind| Damage {
        kind,
        ..Damage::player([3.0; 3])
    };
    // (case, source, DATA_ID_MOVING, permanently invulnerable).
    let cases = [
        ("a player's sword", Damage::player([3.0; 3]), false, false),
        ("a mob's blow", Damage::mob(7, [3.0; 3]), false, false),
        (
            "a warden's boom",
            by_mob(DamageKind::SonicBoom),
            false,
            false,
        ),
        ("a refused blow", Damage::player([3.0; 3]), false, true),
        ("thorns", by_mob(DamageKind::Thorns), false, false),
        ("an arrow", by_player(DamageKind::Arrow), false, false),
        ("magic", by_player(DamageKind::Magic), false, false),
        ("a swimming guardian", Damage::player([3.0; 3]), true, false),
    ];
    let mut out = Vec::new();
    let ran = with_test_world(&[], |_, types| {
        let kind = Arc::new(types.types["minecraft:guardian"].clone());
        for (case, source, moving, invulnerable) in cases {
            let mut m = Mob::new(1, kind.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
            register(&mut m);
            // DATA_ID_MOVING, which `GuardianMoveControl` sets while it swims.
            m.extra.flag2 = moving;
            m.invulnerable_flag = invulnerable;
            let mut test_level = crate::health::TestLevel::new(0);
            let landed = m.damage(&mut test_level.level(), 2.0, source);
            out.push((case, landed, std::mem::take(&mut m.outbox)));
        }
    });
    if !ran {
        return;
    }
    let hit = |case: &str| {
        let (_, landed, events) = out.iter().find(|(c, ..)| *c == case).unwrap();
        (*landed, spikes(events))
    };

    let (landed, spiked) = hit("a player's sword");
    assert!(landed);
    match spiked[..] {
        [MobEvent::DamagePlayer { amount, from, kind }] => {
            assert_eq!((*amount, *kind), (2.0, "minecraft:thorns"));
            assert_eq!(*from, [0.5, 64.0, 0.5]);
        }
        _ => panic!("a player's hit must spike the player: {spiked:?}"),
    }
    let (landed, spiked) = hit("a mob's blow");
    assert!(landed);
    match spiked[..] {
        [
            MobEvent::DamageMob {
                target,
                amount,
                source,
            },
        ] => {
            assert_eq!((*target, *amount), (7, 2.0));
            assert_eq!(source.kind, DamageKind::Thorns);
            assert_eq!(source.attacker, Some(Who::Mob(1)));
            assert_eq!(source.from, Some([0.5, 64.0, 0.5]));
        }
        _ => panic!("a mob's hit must spike the mob: {spiked:?}"),
    }
    // The direct entity is the living attacker: a bee's sting or a warden's boom
    // draws the spikes as a blow does.
    let (_, spiked) = hit("a warden's boom");
    assert_eq!(spiked.len(), 1, "a warden's boom spikes: {spiked:?}");
    // The spikes come before `super.hurtServer`: a guardian its own
    // invulnerability refuses still answers.
    let (landed, spiked) = hit("a refused blow");
    assert!(!landed, "an invulnerable guardian takes nothing");
    assert_eq!(spiked.len(), 1, "and still spikes: {spiked:?}");
    for case in ["thorns", "an arrow", "magic", "a swimming guardian"] {
        assert!(hit(case).1.is_empty(), "{case} draws no spikes");
    }
}

/// A pool of water from x -8 to 70, z -4 to 4, y 60 to 66.
fn pool() -> impl FnOnce(&mut crate::world::World, &horizon_world::registry::BlockRegistry) {
    |world, registry| {
        let water = registry.default_state("minecraft:water").unwrap();
        for cx in -1..=4 {
            for cz in -1..=0 {
                world
                    .chunks
                    .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
            }
        }
        for x in -8..=70 {
            for y in 60..=66 {
                for z in -4..=4 {
                    world.set(x, y, z, water).unwrap();
                }
            }
        }
    }
}

/// An elder guardian that took its home where it first stood (block (0, 62, 0),
/// radius 16), then stood at x = `x`, one `ai::tick` later: whether
/// `MoveTowardsRestrictionGoal` steers it and where its path leads.
fn displaced_elders(xs: &[f64]) -> Option<Vec<(bool, Option<[i32; 3]>)>> {
    let mut out = Vec::new();
    let ran = with_built_test_world(&[], pool(), |w, types| {
        let kind = Arc::new(types.types["minecraft:elder_guardian"].clone());
        for &x in xs {
            let mut m = Mob::new(1, kind.clone(), Vec3::new(0.5, 62.0, 0.5), 0.0);
            register(&mut m);
            m.fluids.in_water = true;
            let mut test_level = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            elder_server_ai_step(
                &mut m,
                &mut Cx {
                    w,
                    level: &mut test_level.level(),
                    events: &mut events,
                },
            );
            m.position = Vec3::new(x, 62.0, 0.5);
            m.previous = m.position;
            crate::mobs::ai::tick(&mut m, w, &mut test_level.level(), &mut events);
            out.push((
                m.ai.goals.is_running("MoveTowardsRestriction"),
                m.ai.nav.target_pos,
            ));
        }
    });
    if !ran {
        return None;
    }
    Some(out)
}

/// `Guardian.registerGoals`' `addGoal(5, new MoveTowardsRestrictionGoal(this, 1.0))`:
/// outside its 16-block home the goal starts and picks a spot toward it;
/// inside it the goal does not run at all.
#[test]
fn a_displaced_elder_guardian_swims_back_toward_its_home() {
    let Some(out) = displaced_elders(&[60.5, 5.5]) else {
        return;
    };
    let (running, target) = out[0];
    assert!(
        running,
        "the restriction goal steers a guardian 60 blocks out"
    );
    let target = target.expect("a path into the home");
    let from_home = |p: [i32; 3]| {
        let (dx, dz) = (f64::from(p[0]), f64::from(p[2]));
        (dx * dx + dz * dz).sqrt()
    };
    assert!(
        from_home(target) < 60.0,
        "the path leads home, not further out: {target:?}"
    );
    assert!(
        !out[1].0,
        "inside its home the guardian has no goal to go home"
    );
}
