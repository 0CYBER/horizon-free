//! `Frog` registers no goals at all (its `FrogAi` swim activity only walks and
//! swims, and `Frog.travelInWater` adds no buoyancy), `HoglinAi.initCoreActivity`
//! holds `LookAtTargetSink` and `MoveToTargetSink` only, and nothing under
//! `monster/piglin/` registers a goal: none of them has a `FloatGoal`, so their
//! navigation's `canFloat` stays false and a jump in water is
//! `Mob.jumpInLiquid`'s 0.3 bounce, not a floater's 0.04. They stay under the
//! surface: the frog holds its depth (`Frog.travelInWater` has no gravity) and
//! the others sink, and the piglins and hoglins drown there (the frog and the
//! zoglin are in `#minecraft:can_breathe_under_water`, through
//! `#minecraft:undead` for the zoglin).
use crate::mobs::{Mob, Species, goals::Cx};
use horizon_core::geometry::Vec3;
use std::sync::Arc;

/// The five, in the order the test reports them.
const SINKERS: [(&str, Species); 5] = [
    ("minecraft:frog", Species::Frog),
    ("minecraft:piglin", Species::Piglin),
    ("minecraft:piglin_brute", Species::PiglinBrute),
    ("minecraft:hoglin", Species::Hoglin),
    ("minecraft:zoglin", Species::Zoglin),
];

/// A `FloatGoal` sets the navigation floating and jumps 0.04 a tick in water,
/// which is what surfaces a mob. Vanilla gives these five none, so forty ticks
/// in a deep pool leave them under the surface: a frog where it started, the
/// other four lower. The piglins and the hoglin lose air there.
#[test]
fn frogs_piglins_hoglins_and_zoglins_do_not_float() {
    let ran = crate::mobs::world::with_built_test_world(
        &[],
        |world, registry| {
            world
                .chunks
                .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            let stone = registry.default_state("minecraft:stone").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            for x in 0..=7 {
                for z in 0..=7 {
                    world.set(x, 59, z, stone).unwrap();
                    for y in 60..=70 {
                        world.set(x, y, z, water).unwrap();
                    }
                }
            }
        },
        |w, types| {
            for (id, species) in SINKERS {
                let kind = Arc::new(types.types[id].clone());
                assert_eq!(kind.species, species);
                let mut m = Mob::new(1, kind, Vec3::new(4.5, 64.0, 4.5), 0.0);
                let mut events = Vec::new();
                m.update_fluids(w, &mut events);
                assert!(
                    m.fluids.in_water && m.fluids.eye_in_water,
                    "{id}: spawned under the surface"
                );
                let start = m.position.y;
                let mut test_level = crate::health::TestLevel::new(0);
                let mut goals = std::mem::take(&mut m.ai.goals);
                for _ in 0..40 {
                    // `LivingEntity.baseTick`: the fluids, then the air supply
                    // and drowning (`base_tick` owns both now).
                    super::base_tick(&mut m, w, &mut test_level.level(), &mut events);
                    {
                        let mut cx = Cx {
                            w,
                            level: &mut test_level.level(),
                            events: &mut events,
                        };
                        goals.tick(&mut m, &mut cx);
                    }
                    // Mob.serverAiStep's order for the jump: the goals' jump
                    // flag, JumpControl, `jumpInLiquid`, then travel.
                    m.jump_control_tick();
                    m.handle_jumping(w);
                    m.travel(w, &mut test_level.level(), Vec3::ZERO, &mut events);
                    m.update_fluids(w, &mut events);
                }
                m.ai.goals = goals;
                if species == Species::Frog {
                    // `Frog.travelInWater` adds neither lift nor gravity: a frog
                    // that stops in water keeps its depth.
                    assert!(
                        (m.position.y - start).abs() < 1.0e-9,
                        "{id}: moved off its depth in the water, from {start} to {}",
                        m.position.y
                    );
                } else {
                    // `Mob.jumpInLiquid` without a `FloatGoal` adds no 0.04, and
                    // `LivingEntity.travelInWater` sinks them.
                    assert!(
                        m.position.y < start,
                        "{id}: did not sink in the water, from {start} to {}",
                        m.position.y
                    );
                }
                // The frog is in `#minecraft:can_breathe_under_water` and the
                // zoglin in `#minecraft:undead`, which it lists; the piglins and
                // the hoglin are in neither and drown.
                if matches!(species, Species::Frog | Species::Zoglin) {
                    assert!(m.kind.flags.breathes_water, "{id}: breathes under water");
                    assert_eq!(m.air, 300, "{id}: holds its breath under water");
                } else {
                    // `LivingEntity.decreaseAirSupply` takes one a tick from the
                    // 300 a mob starts with.
                    assert_eq!(m.air, 260, "{id}: loses air under water");
                }
                assert!(
                    !m.ai.goals.has("Float"),
                    "{id}: vanilla registers no FloatGoal"
                );
                assert!(
                    !m.ai.nav.can_float,
                    "{id}: without a FloatGoal the navigation cannot float"
                );
            }
        },
    );
    let _ = ran;
}
