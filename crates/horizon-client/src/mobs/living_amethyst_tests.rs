//! `Entity.walkingStepSound` and `Entity.playAmethystStepSound`: a step on a
//! `#minecraft:crystal_sound_blocks` block rings `block.amethyst_block.chime`
//! after the footfall, at most once every 20 ticks, swelling by 0.07 a step
//! and pitched by the entity's own random.
use super::*;
use crate::mobs::world::with_built_test_world;
use horizon_core::random::{EntityRandom, LegacyRandom};
use mod_api::SoundSource;
use std::sync::Arc;

/// A mob of `kind` at (0.5, 1, 0.5) standing on the block at (0, 0, 0), its
/// own random from a fixed seed.
fn mob(types: &crate::mobs::MobTypes, kind: &str, silent: bool) -> Mob {
    let kind = Arc::new(types.types[kind].clone());
    let mut m = Mob::with_random(
        1,
        kind,
        Vec3::new(0.5, 1.0, 0.5),
        0.0,
        EntityRandom::own(LegacyRandom::new(424_242)),
    );
    m.on_ground = true;
    m.supporting = Some([0, 0, 0]);
    m.silent = silent;
    m.persistent = true;
    m
}

fn sheep(types: &crate::mobs::MobTypes, silent: bool) -> Mob {
    mob(types, "minecraft:sheep", silent)
}

/// The sounds one step posted, as `(event, source, position, volume, pitch)`;
/// a plain `Sound` has no source of its own.
fn sounds(events: &[MobEvent]) -> Vec<(&'static str, Option<SoundSource>, [f64; 3], f32, f32)> {
    events
        .iter()
        .filter_map(|e| match e {
            MobEvent::Sound(event, at, volume, pitch) => Some((*event, None, *at, *volume, *pitch)),
            MobEvent::SourcedSound(event, source, at, volume, pitch) => {
                Some((*event, Some(*source), *at, *volume, *pitch))
            }
            _ => None,
        })
        .collect()
}

/// `Entity.playAmethystStepSound`'s volume and pitch from the swell
/// `intensity` after `elapsed` ticks without a chime, on the draw `r`:
/// `intensity = min(1, intensity * 0.997^elapsed + 0.07)`,
/// `pitch = 0.5 + intensity * r * 1.2`, `volume = 0.1 + intensity * 1.2`.
fn chime(intensity: f32, elapsed: u32, r: f32) -> (f32, f32, f32) {
    let decayed = intensity * horizon_core::jmath::pow(0.997, f64::from(elapsed)) as f32;
    let intensity = 1.0_f32.min(decayed + 0.07);
    (intensity, 0.5 + intensity * r * 1.2, 0.1 + intensity * 1.2)
}

/// A step on amethyst chimes on tick 20, 40 and 60 and nowhere between: the
/// chime waits 20 ticks, the swell decays as `0.997^elapsed`, and each chime
/// draws the mob's own random once, after its footfall.
#[test]
fn a_mob_stepping_on_amethyst_chimes_once_every_20_ticks() {
    let build = |world: &mut crate::world::World,
                 registry: &horizon_world::registry::BlockRegistry| {
        world
            .chunks
            .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
        let id = registry.default_state("minecraft:amethyst_block").unwrap();
        for x in 0..=8 {
            for z in 0..=8 {
                world.set(x, 0, z, id).unwrap();
            }
        }
    };
    let ran = with_built_test_world(&[], build, |w, types| {
        let mut m = sheep(types, false);
        // A copy of the mob's own random: the chime's draws and nothing else.
        let mut draws = m.random.clone();
        let (mut intensity, mut last) = (0.0_f32, 0_u32);
        let mut chimed = Vec::new();
        for tick in 0..=60 {
            m.age = tick;
            // One step's worth of walking (2 blocks times the 0.6 factor) over
            // the 1.0 the next step waits for, every tick.
            m.ai.steps.move_dist = 1.0;
            m.ai.steps.next_step = 1.0;
            let mut events = Vec::new();
            m.movement_emission(w, Vec3::new(2.0, 0.0, 0.0), &mut events);
            let heard = sounds(&events);
            let footfall = ("entity.sheep.step", None, [0.5, 1.0, 0.5], 0.15, 1.0);
            if tick >= last + 20 {
                let (swell, pitch, volume) = chime(intensity, tick - last, draws.next_float());
                intensity = swell;
                last = tick;
                chimed.push(tick);
                assert_eq!(
                    heard,
                    vec![
                        footfall,
                        (
                            "block.amethyst_block.chime",
                            Some(SoundSource::Neutral),
                            [0.5, 1.0, 0.5],
                            volume,
                            pitch
                        )
                    ],
                    "tick {tick}: the footfall, then the chime at the mob's feet"
                );
            } else {
                assert_eq!(heard, vec![footfall], "tick {tick}: no chime");
            }
        }
        assert_eq!(chimed, vec![20, 40, 60]);
        assert_eq!(
            m.random.next_float(),
            draws.next_float(),
            "one draw per chime, none on the silent ticks"
        );
    });
    // Without the vanilla data under .local/horizon-client the world cannot be
    // built: the test has nothing to check.
    let _ = ran;
}

/// The chime carries the mob's own source, not the event's block category: a
/// sheep chimes neutral and a zombie hostile.
#[test]
fn a_chime_carries_the_mobs_sound_source() {
    let build = |world: &mut crate::world::World,
                 registry: &horizon_world::registry::BlockRegistry| {
        world
            .chunks
            .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
        let id = registry.default_state("minecraft:amethyst_block").unwrap();
        world.set(0, 0, 0, id).unwrap();
    };
    let ran = with_built_test_world(&[], build, |w, types| {
        for (kind, source) in [
            ("minecraft:sheep", SoundSource::Neutral),
            ("minecraft:zombie", SoundSource::Hostile),
        ] {
            let mut m = mob(types, kind, false);
            m.age = 20;
            m.ai.steps.move_dist = 1.0;
            m.ai.steps.next_step = 1.0;
            let mut events = Vec::new();
            m.movement_emission(w, Vec3::new(2.0, 0.0, 0.0), &mut events);
            let chime = sounds(&events)
                .into_iter()
                .find(|s| s.0 == "block.amethyst_block.chime")
                .unwrap_or_else(|| panic!("{kind}: a chime"));
            assert_eq!(chime.1, Some(source), "{kind}");
        }
    });
    let _ = ran;
}

/// `Entity.playSound` looks at `isSilent` after the swell and the draw, so a
/// silent mob rings nothing and still moves its own random.
#[test]
fn a_silent_mob_still_swells_and_draws_without_ringing() {
    let build = |world: &mut crate::world::World,
                 registry: &horizon_world::registry::BlockRegistry| {
        world
            .chunks
            .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
        let id = registry.default_state("minecraft:amethyst_block").unwrap();
        world.set(0, 0, 0, id).unwrap();
    };
    let ran = with_built_test_world(&[], build, |w, types| {
        let mut m = sheep(types, true);
        let mut draws = m.random.clone();
        m.age = 20;
        m.ai.steps.move_dist = 1.0;
        m.ai.steps.next_step = 1.0;
        let mut events = Vec::new();
        m.movement_emission(w, Vec3::new(2.0, 0.0, 0.0), &mut events);
        assert_eq!(sounds(&events), Vec::new(), "a silent mob rings nothing");
        let (swell, _, _) = chime(0.0, 20, draws.next_float());
        assert_eq!(m.ai.steps.crystal_sound_intensity, swell);
        assert_eq!(m.ai.steps.last_crystal_sound_play_tick, 20);
        assert_eq!(m.random.next_float(), draws.next_float());
    });
    let _ = ran;
}
