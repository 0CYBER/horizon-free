//! `Entity.doWaterSplashEffect` plays `getSwimSplashSound` below a volume of
//! 0.25 and `getSwimHighSpeedSplashSound` above it. Only `Player` overrides the
//! high-speed one; a monster's, the hoglin's, the dolphin's and the axolotl's
//! splash overrides are the slow sound only, so a fast entry splashes
//! generically for every one of them.
use super::*;
use std::sync::Arc;

/// What a mob entering still water posts: its splash sound with its volume,
/// and the SPLASH game event with its position and source.
struct Entry {
    sound: Option<(&'static str, f32)>,
    event: Option<(&'static str, [f64; 3], u32)>,
}

/// `id` at the surface of the test world's pool, entering it at `velocity`
/// (`Entity.updateFluidInteraction` after its first tick).
fn enter(id: &str, velocity: Vec3, w: &MobWorld, types: &crate::mobs::MobTypes) -> Entry {
    let mut mob = Mob::new(
        7,
        Arc::new(types.types[id].clone()),
        Vec3::new(2.5, 3.0, 2.5),
        0.0,
    );
    mob.first_tick = false;
    mob.velocity = velocity;
    let mut events = Vec::new();
    mob.update_fluids(w, &mut events);
    let mut entry = Entry {
        sound: None,
        event: None,
    };
    for event in &events {
        match event {
            MobEvent::Sound(name, _, volume, _) => entry.sound = Some((*name, *volume)),
            MobEvent::GameEvent {
                event, at, source, ..
            } => entry.event = Some((*event, *at, *source)),
            _ => {}
        }
    }
    entry
}

#[test]
fn a_fast_water_entry_splashes_generically_for_every_mob() {
    let ran = crate::mobs::world::with_built_test_world(
        &[],
        |world, registry| {
            world
                .chunks
                .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            let water = registry.default_state("minecraft:water").unwrap();
            for x in 0..=4 {
                for z in 0..=4 {
                    for y in 0..=8 {
                        world.set(x, y, z, water).unwrap();
                    }
                }
            }
        },
        |w, types| {
            // (id, slow splash, fast splash): a monster, the hoglin, the
            // dolphin and the axolotl are the splash overrides of the tree,
            // and a cow and a squid answer the generic one either way.
            const SPLASHES: [(&str, &str, &str); 6] = [
                (
                    "minecraft:cow",
                    "entity.generic.splash",
                    "entity.generic.splash",
                ),
                (
                    "minecraft:squid",
                    "entity.generic.splash",
                    "entity.generic.splash",
                ),
                (
                    "minecraft:zombie",
                    "entity.hostile.splash",
                    "entity.generic.splash",
                ),
                (
                    "minecraft:hoglin",
                    "entity.hostile.splash",
                    "entity.generic.splash",
                ),
                (
                    "minecraft:dolphin",
                    "entity.dolphin.splash",
                    "entity.generic.splash",
                ),
                (
                    "minecraft:axolotl",
                    "entity.axolotl.splash",
                    "entity.generic.splash",
                ),
            ];
            for (id, slow, fast) in SPLASHES {
                // Half a block a tick: a volume of 0.1, below the threshold.
                let gentle = enter(id, Vec3::new(0.0, -0.5, 0.0), w, types);
                assert_eq!(gentle.sound, Some((slow, 0.1)), "{id} entering slowly");
                // Two blocks a tick: a volume of 0.4, above the threshold.
                let quick = enter(id, Vec3::new(0.0, -2.0, 0.0), w, types);
                assert_eq!(quick.sound, Some((fast, 0.4)), "{id} entering fast");
                // Either way the SPLASH game event comes from the mob.
                assert_eq!(
                    quick.event,
                    Some(("minecraft:splash", [2.5, 3.0, 2.5], 7)),
                    "{id} entering fast"
                );
                assert_eq!(
                    gentle.event,
                    Some(("minecraft:splash", [2.5, 3.0, 2.5], 7)),
                    "{id} entering slowly"
                );
            }
        },
    );
    // Without the vanilla data under .local/horizon-client the world cannot
    // be built: the test has nothing to check.
    let _ = ran;
}
