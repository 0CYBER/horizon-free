//! The melee cadence of the three mobs whose vanilla brain arms
//! `ATTACK_COOLING_DOWN` with `MeleeAttack.create(n)` (`Zoglin`,
//! `PiglinAi.initFightActivity`, `PiglinBruteAi.initFightActivity`):
//! `MemorySlot.tick` counts that memory down and erases it a tick after it
//! reaches zero, so vanilla's hits come n + 1 ticks apart. Horizon runs those
//! mobs on a goal that counts its own cooldown down before it tests it, so the
//! goal's interval has to be one tick more than vanilla's number.
use super::*;
use crate::mobs::MobEvent;
use crate::mobs::world::{test_player, with_built_test_world};
use std::sync::Arc;

/// The AI ticks at which the mob's hit lands (its `AttackPlayer` event), with a
/// player standing in its reach.
fn hit_ticks(kind: &str, baby: bool, ticks: i32) -> Option<Vec<i32>> {
    let kind = kind.to_string();
    let mut out = None;
    let ran = with_built_test_world(
        &[],
        |world, registry| {
            let stone = registry.default_state("minecraft:stone").unwrap();
            for cx in -1..1 {
                for cz in -1..1 {
                    world.chunks.insert(
                        (cx, cz),
                        std::sync::Arc::new(crate::chunk::Chunk::empty(cx, cz)),
                    );
                }
            }
            for x in -4..5 {
                for z in -4..5 {
                    world.set(x, 63, z, stone).unwrap();
                }
            }
        },
        |w, types| {
            let mut w = w.clone();
            w.player = Some(test_player(Vec3::new(1.5, 64.0, 0.5)));
            let mut m = Mob::new(
                1,
                Arc::new(types.types[&kind].clone()),
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            );
            crate::mobs::ai::register(&mut m);
            crate::mobs::finalize::set_baby(&mut m, baby);
            m.persistent = true;
            m.ai.target = Some(Who::Player);
            let mut test_level = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            let mut hits = Vec::new();
            for tick in 0..ticks {
                events.clear();
                crate::mobs::ai::tick(&mut m, &w, &mut test_level.level(), &mut events);
                if events
                    .iter()
                    .any(|e| matches!(e, MobEvent::AttackPlayer(..)))
                {
                    hits.push(tick);
                }
            }
            out = Some(hits);
        },
    );
    ran.then_some(out).flatten()
}

fn gaps(hits: &[i32]) -> Vec<i32> {
    hits.windows(2).map(|p| p[1] - p[0]).collect()
}

/// `Zoglin.initFightActivity`: `MeleeAttack.create(40)` for adults. The memory
/// is erased on the 41st brain tick, so an adult zoglin hits every 41 ticks.
#[test]
fn an_adult_zoglin_hits_every_41_ticks() {
    let Some(hits) = hit_ticks("minecraft:zoglin", false, 200) else {
        return;
    };
    assert!(hits.len() >= 3, "the zoglin should keep hitting: {hits:?}");
    assert_eq!(
        gaps(&hits),
        vec![41; hits.len() - 1],
        "vanilla's adult zoglin hits every 41 ticks (create(40) plus the tick \
         `MemorySlot.tick` needs to erase it): {hits:?}"
    );
}

/// `Zoglin.initFightActivity`: `MeleeAttack.create(15)` for babies, so 16.
#[test]
fn a_baby_zoglin_hits_every_16_ticks() {
    let Some(hits) = hit_ticks("minecraft:zoglin", true, 100) else {
        return;
    };
    assert!(
        hits.len() >= 3,
        "the baby zoglin should keep hitting: {hits:?}"
    );
    assert_eq!(
        gaps(&hits),
        vec![16; hits.len() - 1],
        "vanilla's baby zoglin hits every 16 ticks (create(15) plus the \
         erasing tick): {hits:?}"
    );
}

/// `PiglinBruteAi.initFightActivity`: `MeleeAttack.create(20)`, so 21.
#[test]
fn a_piglin_brute_hits_every_21_ticks() {
    let Some(hits) = hit_ticks("minecraft:piglin_brute", false, 120) else {
        return;
    };
    assert!(hits.len() >= 3, "the brute should keep hitting: {hits:?}");
    assert_eq!(
        gaps(&hits),
        vec![21; hits.len() - 1],
        "vanilla's piglin brute hits every 21 ticks (create(20) plus the \
         erasing tick): {hits:?}"
    );
}

/// `PiglinAi.initFightActivity`: `MeleeAttack.create(20)`, so 21.
#[test]
fn a_piglin_hits_every_21_ticks() {
    let Some(hits) = hit_ticks("minecraft:piglin", false, 120) else {
        return;
    };
    assert!(hits.len() >= 3, "the piglin should keep hitting: {hits:?}");
    assert_eq!(
        gaps(&hits),
        vec![21; hits.len() - 1],
        "vanilla's piglin hits every 21 ticks (create(20) plus the erasing \
         tick): {hits:?}"
    );
}
