//! `LevelReader.getLightLevelDependentMagicValue` (the server side of
//! `Lightmap.getBrightness`) lerps the curved light level with the level's own
//! `DimensionType.ambientLight`: 0 in the overworld, 0.1 in the nether and
//! 0.25 in the end (`DimensionTypes.END`). At End light 11 the curve gives
//! 11/27 = 0.407, and 0.407 + 0.25 * (1 - 0.407) = 5/9 > 0.5, so
//! `Monster.updateNoActionTime` adds its 2 there.
use super::super::ai::tick;
use super::super::{Mob, MobWorld};
use super::with_built_test_world;
use crate::chunk::{Chunk, Light, Section};
use horizon_core::{LevelKey, geometry::Vec3, random::LegacyRandom};
use horizon_world::dimension_type::DimensionFacts;
use std::sync::Arc;

/// An empty column with sky light 11 in its section 8 (y 64..79) and none in
/// section 9 (y 80..95).
fn lit_column(world: &mut crate::world::World, _: &horizon_world::registry::BlockRegistry) {
    let mut chunk = Chunk::empty(0, 0);
    for (section, light) in [(8, 0xB0), (9, 0x00)] {
        chunk.sections[section] = Arc::new(Section {
            light: Light::Uniform(light),
            ..Default::default()
        });
    }
    world.chunks.insert((0, 0), Arc::new(chunk));
}

#[test]
fn the_end_ambient_light_reaches_mob_brightness_and_its_no_action_time() {
    let mut out = None;
    if !with_built_test_world(&[], lit_column, |w, types| {
        let of = |level: LevelKey| MobWorld {
            level,
            dimension_type: DimensionFacts::of_builtin_level(level),
            ..w.clone()
        };
        let (end, nether) = (of(LevelKey::END), of(LevelKey::NETHER));
        // A zombie hovering in the lit section, 40 blocks from the player so
        // `checkDespawn` keeps it (as `monster_no_action_tests` does).
        let mut m = Mob::new(
            1,
            Arc::new(types.types["minecraft:zombie"].clone()),
            Vec3::new(0.5, 64.0, 0.5),
            0.0,
        );
        m.no_gravity = true;
        let end = MobWorld {
            nearest_player: Some(Vec3::new(40.5, 64.0, 0.5)),
            ..end
        };
        let mut lr = horizon_core::random::LevelRandom::for_level(LegacyRandom::new(7));
        let mut events = Vec::new();
        for _ in 0..10 {
            assert!(
                tick(&mut m, &end, &mut lr, &mut events),
                "the zombie went away"
            );
        }
        out = Some((
            end.brightness(0, 65, 0),
            end.brightness(0, 81, 0),
            nether.brightness(0, 81, 0),
            w.brightness(0, 81, 0),
            m.ai.no_action_time,
        ));
    }) {
        return;
    }
    let Some((end_11, end_dark, nether_dark, overworld_dark, ticks)) = out else {
        return;
    };
    // `Mth.lerp(ambientLight, curvedV, 1.0F)` at light 11 in the end.
    assert!(
        (end_11 - 5.0 / 9.0).abs() < 1e-6,
        "End light 11 reads {end_11}, not 5/9"
    );
    // With no light the curved level is 0, so the brightness is the ambient
    // light itself.
    assert_eq!(end_dark, 0.25, "the end's ambient light");
    assert_eq!(nether_dark, 0.1, "the nether's ambient light");
    assert_eq!(overworld_dark, 0.0, "the overworld's ambient light");
    // `Mob.serverAiStep`'s 1 plus `Monster.updateNoActionTime`'s 2.
    assert_eq!(ticks, 30, "a monster at End light 11 gains 3 a tick");
}
