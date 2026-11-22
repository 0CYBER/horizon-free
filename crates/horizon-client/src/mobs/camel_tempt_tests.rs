//! `CamelAi.initIdleActivity`'s `FollowTemptation(camel -> 2.5F, camel ->
//! camel.isBaby() ? 2.5 : 3.5)`: an adult camel stops 3.5 blocks from the
//! player holding its food, a baby 2.5.
use super::*;
use crate::mobs::world::{test_player, with_built_test_world};
use horizon_core::geometry::Vec3;
use std::sync::Arc;

/// Whether the tempt goal of a `kind` of `age` runs and walks toward a player
/// `distance` blocks away holding `hand`, after one tick of its goal list on a
/// stone floor. `None` without the prepared vanilla data.
fn tempted(kind: &str, hand: &str, age: i32, distance: f64) -> Option<(bool, bool)> {
    let mut out = None;
    let ran = with_built_test_world(
        &[],
        |world, registry| {
            for cx in -1..2 {
                for cz in -1..2 {
                    world
                        .chunks
                        .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                }
            }
            let stone = registry.default_state("minecraft:stone").unwrap();
            for bx in -4..16 {
                for bz in -4..5 {
                    world.set(bx, 64, bz, stone).unwrap();
                }
            }
        },
        |w, types| {
            let mut world = w.clone();
            let mut p = test_player(Vec3::new(0.5 + distance, 65.0, 0.5));
            p.main_hand = Some(hand.into());
            world.player = Some(p);
            let kind = Arc::new(types.types[kind].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 65.0, 0.5), 0.0);
            m.extra.age = age;
            register(&mut m);
            m.on_ground = true;
            let mut lr = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            let mut cx = Cx {
                w: &world,
                level: &mut lr.level(),
                events: &mut events,
            };
            let mut goals = std::mem::take(&mut m.ai.goals);
            goals.tick(&mut m, &mut cx);
            let running = goals.is_running("Tempt");
            m.ai.goals = goals;
            out = Some((running, m.ai.nav.path.is_some()));
        },
    );
    let _ = ran;
    out
}

/// A player exactly at the close enough distance still walks the camel in
/// (vanilla compares `<`), one a little closer is inside and it stays put:
/// an adult stops at 3.5 blocks, a baby at 2.5.
#[test]
fn an_adult_camel_stops_further_off_than_a_baby() {
    let (Some(adult_far), Some(adult_near), Some(baby_far), Some(baby_near)) = (
        tempted("minecraft:camel", "minecraft:cactus", 0, 3.5),
        tempted("minecraft:camel", "minecraft:cactus", 0, 3.0),
        tempted("minecraft:camel", "minecraft:cactus", -24000, 2.5),
        tempted("minecraft:camel", "minecraft:cactus", -24000, 2.0),
    ) else {
        return;
    };
    assert_eq!(
        [adult_far.0, adult_near.0, baby_far.0, baby_near.0],
        [true; 4],
        "a cactus tempts a camel of either age"
    );
    assert_eq!(
        [adult_far.1, adult_near.1, baby_far.1, baby_near.1],
        [true, false, true, false],
        "an adult camel walks in from 3.5 blocks and stays put at 3, a baby \
         walks in from 2.5 and stays put at 2"
    );
}

/// `CamelHusk.canBeABaby` is false, so a husk keeps the adult's 3.5 whatever
/// its age says; its food is the rabbit's foot (`#minecraft:camel_husk_food`).
#[test]
fn a_camel_husk_keeps_the_adult_distance() {
    let (Some(far), Some(near)) = (
        tempted("minecraft:camel_husk", "minecraft:rabbit_foot", -24000, 3.5),
        tempted("minecraft:camel_husk", "minecraft:rabbit_foot", -24000, 2.5),
    ) else {
        return;
    };
    assert!(far.0 && near.0, "a rabbit's foot tempts a camel husk");
    assert_eq!(
        [far.1, near.1],
        [true, false],
        "a camel husk is never a baby, so it walks in from 3.5 blocks and \
         stays put at 2.5 whatever its age says"
    );
}
