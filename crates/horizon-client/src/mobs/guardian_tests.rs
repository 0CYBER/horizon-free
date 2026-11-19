//! `Guardian.hurtServer`'s spikes and `Guardian.registerGoals`' priority-5
//! `MoveTowardsRestrictionGoal`: a still guardian's spikes hurt the living
//! thing that hit it at the instant of the hit, and a guardian taken out of
//! its home swims back into it. And its stroll: `randomStrollGoal.trigger()`
//! from the attack goal's stop and from a hurt, and the LOOK flag it keeps.
use super::*;
use crate::mobs::goals::{Cx, JUMP, MOVE};
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
#[ignore = "daily"]
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

/// A pool of water from x -8 to 70, z -4 to 4, y 60 to 66, and with `wall` a
/// stone wall at x = 20 across it (sight from one side to the other blocked).
fn pool_with(
    wall: bool,
) -> impl FnOnce(&mut crate::world::World, &horizon_world::registry::BlockRegistry) {
    move |world, registry| {
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
        if wall {
            let stone = registry.default_state("minecraft:stone").unwrap();
            for y in 60..=66 {
                for z in -4..=4 {
                    world.set(20, y, z, stone).unwrap();
                }
            }
        }
    }
}
fn pool() -> impl FnOnce(&mut crate::world::World, &horizon_world::registry::BlockRegistry) {
    pool_with(false)
}

/// One goal tick of a guardian (`GoalSelector.tick` alone, with the sight
/// cache `ai::tick` clears every tick).
fn goal_tick(
    goals: &mut crate::mobs::goals::GoalSelector,
    m: &mut Mob,
    w: &MobWorld,
    level: &mut crate::health::DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    m.ai.seen_cache.clear();
    goals.tick(m, &mut Cx { w, level, events });
}

/// `GuardianAttackGoal.stop`'s and `Guardian.hurtServer`'s
/// `randomStrollGoal.trigger()`: a guardian strolls on the very tick its beam
/// ends (its target gone from sight) and the tick it is hurt, instead of
/// waiting for the stroll's 1-in-40 interval draw.
#[test]
fn a_guardian_strolls_at_once_when_its_beam_ends_or_it_is_hurt() {
    // (seed, the beam fired, the stroll after the beam, its target, the stroll after a hurt)
    let mut out = Vec::new();
    let ran = with_built_test_world(&[], pool_with(true), |w, types| {
        let squid = Mob::new(
            100,
            Arc::new(types.types["minecraft:squid"].clone()),
            Vec3::new(12.5, 62.0, 0.5),
            0.0,
        );
        let others = [squid.seen(None, &crate::items::Items::empty())];
        let mut world = w.clone();
        world.others = &others;
        let kind = Arc::new(types.types["minecraft:guardian"].clone());
        for seed in 0..8 {
            let mut m = Mob::with_random(
                1,
                kind.clone(),
                Vec3::new(0.5, 62.0, 0.5),
                0.0,
                crate::mobs::EntityRandom::own(LegacyRandom::new(seed)),
            );
            register(&mut m);
            m.fluids.in_water = true;
            m.ai.target = Some(Who::Mob(100));
            let mut test_level = crate::health::TestLevel::new(seed);
            let mut events = Vec::new();
            // The beam charges for ten ticks, then fires.
            let mut beamed = false;
            for _ in 0..40 {
                crate::mobs::ai::tick(&mut m, &world, &mut test_level.level(), &mut events);
                if m.extra.flag {
                    beamed = true;
                    break;
                }
            }
            // Step out of sight behind the wall: the beam's goal stops.
            m.position = Vec3::new(30.5, 62.0, 0.5);
            m.previous = m.position;
            let mut strolled = false;
            for _ in 0..4 {
                crate::mobs::ai::tick(&mut m, &world, &mut test_level.level(), &mut events);
                strolled |= m.ai.goals.is_running("RandomStroll");
            }
            let target = m.ai.nav.target_pos;

            let mut hurt = Mob::with_random(
                1,
                kind.clone(),
                Vec3::new(0.5, 62.0, 0.5),
                0.0,
                crate::mobs::EntityRandom::own(LegacyRandom::new(seed)),
            );
            register(&mut hurt);
            hurt.fluids.in_water = true;
            hurt.damage(
                &mut test_level.level(),
                1.0,
                Damage::player([5.5, 62.0, 0.5]),
            );
            let mut after_hurt = false;
            for _ in 0..4 {
                crate::mobs::ai::tick(&mut hurt, &world, &mut test_level.level(), &mut events);
                after_hurt |= hurt.ai.goals.is_running("RandomStroll");
            }
            out.push((seed, beamed, strolled, target, after_hurt));
        }
    });
    if !ran {
        return;
    }
    for (seed, beamed, strolled, target, _) in &out {
        assert!(beamed, "seed {seed}: the beam fires at its target");
        assert!(
            strolled,
            "seed {seed}: the guardian strolls on the tick its beam ends"
        );
        assert!(target.is_some(), "seed {seed}: and swims somewhere");
    }
    for (seed, _, _, _, after_hurt) in &out {
        assert!(
            after_hurt,
            "seed {seed}: a hurt guardian strolls at once too"
        );
    }
}

/// `Guardian.registerGoals`' `randomStrollGoal.setFlags(MOVE, LOOK)`: while a
/// guardian strolls the look-at-player goal cannot turn its head to a player
/// standing by (the stroll keeps the head with the body).
#[test]
fn a_strolling_guardian_keeps_its_head_with_its_body() {
    // (seed, still strolling, the furthest the head left the body, in degrees)
    let mut out = Vec::new();
    let ran = with_built_test_world(&[], pool(), |w, types| {
        let mut world = w.clone();
        world.player = Some(crate::mobs::world::test_player(Vec3::new(5.5, 62.0, 0.5)));
        let kind = Arc::new(types.types["minecraft:guardian"].clone());
        for seed in 0..200 {
            let mut m = Mob::with_random(
                1,
                kind.clone(),
                Vec3::new(0.5, 62.0, 0.5),
                0.0,
                crate::mobs::EntityRandom::own(LegacyRandom::new(seed)),
            );
            register(&mut m);
            m.fluids.in_water = true;
            let mut test_level = crate::health::TestLevel::new(seed);
            let mut events = Vec::new();
            let mut goals = std::mem::take(&mut m.ai.goals);
            // The stroll's first draw is 1 in 40: take a seed where it strolls at once.
            goal_tick(
                &mut goals,
                &mut m,
                &world,
                &mut test_level.level(),
                &mut events,
            );
            if !goals.is_running("RandomStroll") {
                continue;
            }
            let mut turned = 0.0_f32;
            for _ in 0..300 {
                goal_tick(
                    &mut goals,
                    &mut m,
                    &world,
                    &mut test_level.level(),
                    &mut events,
                );
                m.look_control_tick();
                // How far the head left the body it is carried by, in degrees.
                let off = (m.head_yaw - m.body_yaw + 180.0).rem_euclid(360.0) - 180.0;
                turned = turned.max(off.abs());
            }
            out.push((seed, goals.is_running("RandomStroll"), turned));
            m.ai.goals = goals;
        }
    });
    if !ran {
        return;
    }
    assert!(!out.is_empty(), "some seed strolls on its first goal tick");
    for (seed, strolling, turned) in out {
        assert!(strolling, "seed {seed}: the stroll runs on");
        assert!(
            turned < 0.5,
            "seed {seed}: a strolling guardian does not turn its head to the player ({turned} degrees)"
        );
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
#[ignore = "daily"]
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

/// Whether a `me` guardian at (0.5, 62, 0.5) turns its look to a `other` one
/// ten blocks east within 1000 goal ticks, with only its LOOK goals free to
/// run (so only a look-at goal can aim it): the eye position it aimed at.
fn looks_at_guardian(types: &super::MobTypes, w: &MobWorld<'_>, me: &str, other: &str) -> bool {
    let mut m = Mob::new(
        1,
        Arc::new(types.types[me].clone()),
        Vec3::new(0.5, 62.0, 0.5),
        0.0,
    );
    register(&mut m);
    let seen = [Mob::new(
        2,
        Arc::new(types.types[other].clone()),
        Vec3::new(10.5, 62.0, 0.5),
        0.0,
    )
    .seen(None, &crate::items::Items::empty())];
    let world = MobWorld {
        others: &seen,
        ..w.clone()
    };
    let eye = [seen[0].position.x, seen[0].eye_y, seen[0].position.z];
    m.ai.goals.disabled = MOVE | JUMP;
    let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
    let mut goals = std::mem::take(&mut m.ai.goals);
    let mut looked = false;
    for _ in 0..1000 {
        let mut cx = Cx {
            w: &world,
            level: &mut test_level.level(),
            events: &mut events,
        };
        goals.tick(&mut m, &mut cx);
        if goals.is_running("LookAtPlayer") && m.ai.look.wanted == eye {
            looked = true;
            break;
        }
    }
    m.ai.goals = goals;
    looked
}

/// `Guardian.registerGoals`' `addGoal(8, new LookAtPlayerGoal(this,
/// Guardian.class, 12.0F, 0.01F))`: a guardian looks at another guardian ten
/// blocks away, and `Guardian.class` takes the elder guardians, which inherit
/// the goal.
#[test]
fn a_guardian_looks_at_another_guardian_elder_included() {
    let mut out = Vec::new();
    let ran = with_test_world(&[], |w, types| {
        for (me, other) in [
            ("minecraft:guardian", "minecraft:elder_guardian"),
            ("minecraft:elder_guardian", "minecraft:guardian"),
        ] {
            out.push((me, other, looks_at_guardian(types, w, me, other)));
        }
    });
    if !ran {
        return;
    }
    for (me, other, looked) in out {
        assert!(looked, "a {me} never looked at the {other} 10 blocks off");
    }
}
