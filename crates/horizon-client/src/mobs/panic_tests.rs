//! `AnimalPanic` (the brain's panic of the frog, goat, allay, camel, nautilus
//! and armadillo) and the Manhattan water search it shares with `PanicGoal`.
use super::*;
use crate::mobs::ai;
use crate::mobs::world::with_built_test_world;
use crate::mobs::{Damage, DamageKind, MobEvent, MobTypes};
use horizon_core::geometry::Vec3;
use std::sync::Arc;

/// A stone floor at y 63 around the origin: chunks -1..1, blocks -12..12.
fn floor(world: &mut crate::world::World, registry: &horizon_world::registry::BlockRegistry) {
    for cx in -1..=1 {
        for cz in -1..=1 {
            world
                .chunks
                .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
        }
    }
    let stone = registry.default_state("minecraft:stone").unwrap();
    for x in -12..=12 {
        for z in -12..=12 {
            world.set(x, 63, z, stone).unwrap();
        }
    }
}

/// A mob of `id` at `at` with its own goals, hurt inside the 40 tick window
/// `getLastDamageSource` lasts, and its goal selector ready to tick.
fn hurt_mob(types: &MobTypes, id: &str, at: [f64; 3], mob_id: u32) -> Mob {
    let mut m = Mob::new(
        mob_id,
        Arc::new(types.types[id].clone()),
        Vec3::new(at[0], at[1], at[2]),
        0.0,
    );
    ai::register(&mut m);
    m.on_ground = true;
    m.last_damage = Some((Damage::of(DamageKind::MobAttack), i64::from(m.age)));
    m
}

/// `AnimalPanic` runs 100 to 120 ticks and re-picks a spot every time the
/// navigation is done; `PanicGoal`, which the goat is not given, gives up
/// after the 40 ticks its damage stays `getLastDamageSource`.
#[test]
fn a_panicking_goat_flees_for_five_to_six_seconds() {
    let mut outcome = None;
    let ran = with_built_test_world(&[], floor, |w, types| {
        let mut m = hurt_mob(types, "minecraft:goat", [0.5, 64.0, 0.5], 1);
        let mut goals = std::mem::take(&mut m.ai.goals);
        let mut test_level = crate::health::TestLevel::new(0);
        let mut events: Vec<MobEvent> = Vec::new();
        let mut cx = Cx {
            w,
            level: &mut test_level.level(),
            events: &mut events,
        };
        let mut panicked = 0;
        let mut retargeted = false;
        for tick in 0..200 {
            m.age += 1;
            goals.tick(&mut m, &mut cx);
            if m.ai.panicking {
                panicked += 1;
            }
            // Well past the damage window: the goat reaches its spot and has
            // to pick another one.
            if tick == 60 {
                for _ in 0..10 {
                    assert!(
                        m.ai.panicking,
                        "the goat gave up {tick} ticks after the hit"
                    );
                    m.ai.nav.stop();
                    goals.tick(&mut m, &mut cx);
                    if !m.ai.nav.is_done() {
                        retargeted = true;
                        break;
                    }
                }
            }
        }
        m.ai.goals = goals;
        outcome = Some((panicked, retargeted));
    });
    let _ = ran;
    let Some((panicked, retargeted)) = outcome else {
        return;
    };
    // The start tick plus the drawn 100 to 120 (`Behavior.tryStart` and
    // `tickOrStop` both run on it).
    assert!(
        (101..=121).contains(&panicked),
        "a panicking goat flees 100 to 120 ticks, not {panicked}"
    );
    assert!(
        retargeted,
        "a panicking goat that reaches its spot picks another one"
    );
}

/// Where `TurtlePanicGoal` flees to (`lookForWater(level, mob, 7)`), or `None`
/// without the vanilla data (CI).
fn turtle_water_pick(water: &[[i32; 3]]) -> Option<Option<[i32; 3]>> {
    let water = water.to_vec();
    let mut picked: Option<[i32; 3]> = None;
    let ran = with_built_test_world(
        &[],
        |world, registry| {
            floor(world, registry);
            let state = registry.default_state("minecraft:water").unwrap();
            for p in water {
                world.set(p[0], p[1], p[2], state).unwrap();
            }
        },
        |w, types| {
            let mut m = hurt_mob(types, "minecraft:turtle", [8.5, 64.0, 8.5], 1);
            let mut goals = std::mem::take(&mut m.ai.goals);
            let mut test_level = crate::health::TestLevel::new(0);
            let mut events: Vec<MobEvent> = Vec::new();
            goals.tick(
                &mut m,
                &mut Cx {
                    w,
                    level: &mut test_level.level(),
                    events: &mut events,
                },
            );
            m.ai.goals = goals;
            picked = m.ai.nav.target_pos;
        },
    );
    ran.then_some(picked)
}

/// `TurtlePanicGoal.lookForWater` walks `BlockPos.manhattanOrdered` by
/// depth, then x, then y, then +z before -z, so of two spots as near it
/// takes the one to its +z (and, at one depth, +y over +x).
#[test]
fn the_turtle_flees_to_the_manhattan_first_pool() {
    // The same pool on both sides: +z before -z.
    let Some(pick) = turtle_water_pick(&[[8, 64, 9], [8, 64, 7]]) else {
        return;
    };
    assert_eq!(pick, Some([8, 64, 9]));
    // x before y: +y is nearer the scan's start than +x.
    let Some(pick) = turtle_water_pick(&[[8, 65, 8], [9, 64, 8]]) else {
        return;
    };
    assert_eq!(pick, Some([8, 65, 8]));
    // The nearer depth wins over the axis order.
    let Some(pick) = turtle_water_pick(&[[8, 64, 10], [8, 64, 7]]) else {
        return;
    };
    assert_eq!(pick, Some([8, 64, 7]));
}

/// A burning camel at `(0.5, 64, 0.5)` on the floor with `water` around it;
/// where `CamelAi.CamelPanic` flees to, or `None` without the vanilla data.
fn burning_camel_pick(water: &[[i32; 3]]) -> Option<Option<[i32; 3]>> {
    let water = water.to_vec();
    let mut picked: Option<[i32; 3]> = None;
    let ran = with_built_test_world(
        &[],
        |world, registry| {
            floor(world, registry);
            let state = registry.default_state("minecraft:water").unwrap();
            for p in water {
                world.set(p[0], p[1], p[2], state).unwrap();
            }
        },
        |w, types| {
            let mut m = hurt_mob(types, "minecraft:camel", [0.5, 64.0, 0.5], 1);
            m.fire_ticks = 100;
            let mut goals = std::mem::take(&mut m.ai.goals);
            let mut test_level = crate::health::TestLevel::new(0);
            let mut events: Vec<MobEvent> = Vec::new();
            goals.tick(
                &mut m,
                &mut Cx {
                    w,
                    level: &mut test_level.level(),
                    events: &mut events,
                },
            );
            m.ai.goals = goals;
            picked = m.ai.nav.target_pos;
        },
    );
    ran.then_some(picked)
}

/// `AnimalPanic.lookForWater` asks a mob whose bounding box rounds up to two
/// blocks wide (`Mth.ceil(getBbWidth()) == 2`, the camel) for the blocks
/// south, east and south-east of the spot too: one water block is no pool for
/// a burning camel, a 2x2 one is.
#[test]
fn a_burning_camel_needs_a_two_by_two_pool() {
    // A lone water block at its +z: no pool, so the camel falls back to a
    // land spot (`LandRandomPos` never picks water).
    let Some(pick) = burning_camel_pick(&[[0, 64, 1]]) else {
        return;
    };
    assert_ne!(pick, Some([0, 64, 1]), "a lone water block is no pool");
    // The same block with south, east and south-east water: a pool.
    let Some(pick) = burning_camel_pick(&[[0, 64, 1], [0, 64, 2], [1, 64, 1], [1, 64, 2]]) else {
        return;
    };
    assert_eq!(pick, Some([0, 64, 1]));
}
