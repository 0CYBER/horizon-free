//! The fish sounds go through `LivingEntity.makeSound`: `Entity.playSound(sound,
//! getSoundVolume(), getVoicePitch())`. `getVoicePitch` draws two floats, and it
//! is drawn before `Entity.playSound` looks at `isSilent`, so a silent fish
//! plays nothing and still moves its random stream by those two draws.
use super::*;
use crate::mobs::world::with_test_world;
use horizon_core::random::{EntityRandom, LegacyRandom};
use std::sync::Arc;

/// `LivingEntity.getVoicePitch` for an adult from the random as it stands, with
/// the random after its two draws.
fn pitch_then(before: &LegacyRandom) -> (f32, LegacyRandom) {
    let mut after = before.clone();
    let pitch = (after.next_float() - after.next_float()) * 0.2 + 1.0;
    (pitch, after)
}

/// A fish at (0.5, 64, 0.5) whose own random starts from a fixed seed.
fn fish(types: &crate::mobs::MobTypes, name: &str, silent: bool) -> Mob {
    let at = Vec3::new(0.5, 64.0, 0.5);
    let kind = Arc::new(types.types[name].clone());
    let mut m = Mob::with_random(
        1,
        kind,
        at,
        0.0,
        EntityRandom::own(LegacyRandom::new(424_242)),
    );
    m.silent = silent;
    m.no_gravity = true;
    m.persistent = true;
    m
}

/// The sounds a tick posted, as `(event, position, volume, pitch)`.
fn sounds(events: &[MobEvent]) -> Vec<(&'static str, [f64; 3], f32, f32)> {
    events
        .iter()
        .filter_map(|e| match e {
            MobEvent::Sound(event, at, volume, pitch) => Some((*event, *at, *volume, *pitch)),
            _ => None,
        })
        .collect()
}

/// `Pufferfish.tick` blows up and out with `makeSound`, so a silent fish puffs in
/// silence while its pitch is still drawn.
#[test]
fn a_pufferfish_puffs_with_its_voice_pitch_or_in_silence() {
    let ran = with_test_world(&[], |_, types| {
        for (counter, counter2, puff, puffed, event) in [
            (1, 0, 0, 1, "entity.puffer_fish.blow_up"),
            (41, 0, 1, 2, "entity.puffer_fish.blow_up"),
            (0, 61, 2, 1, "entity.puffer_fish.blow_out"),
            (0, 101, 1, 0, "entity.puffer_fish.blow_out"),
        ] {
            for silent in [false, true] {
                let mut m = fish(types, "minecraft:pufferfish", silent);
                (m.extra.size, m.extra.counter, m.extra.counter2) = (puff, counter, counter2);
                let (pitch, mut after) = pitch_then(&m.random);
                let mut events = Vec::new();
                tick(&mut m, &mut events);
                assert_eq!(m.extra.size, puffed, "the puff state still moves");
                let played = sounds(&events);
                let expected = if silent {
                    Vec::new()
                } else {
                    vec![(event, [0.5, 64.0, 0.5], 1.0, pitch)]
                };
                assert_eq!(played, expected, "makeSound at the feet, at getVoicePitch");
                assert_eq!(
                    m.random.next_float(),
                    after.next_float(),
                    "the pitch's two draws are made either way"
                );
            }
        }
    });
    let _ = ran;
}

/// `AbstractFish.aiStep` flops with `makeSound` after the two floats of its hop:
/// a silent cod hops in silence and still draws its pitch.
#[test]
fn a_flopping_fish_plays_at_its_voice_pitch_or_in_silence() {
    let ran = with_test_world(&[], |w, types| {
        for (name, event) in [
            ("minecraft:cod", "entity.cod.flop"),
            ("minecraft:salmon", "entity.salmon.flop"),
            ("minecraft:tadpole", "entity.tadpole.flop"),
        ] {
            for silent in [false, true] {
                let mut m = fish(types, name, silent);
                // `!isInWater() && onGround() && verticalCollision`.
                m.fluids.in_water = false;
                (m.on_ground, m.vertical_collision_below) = (true, true);
                let (_, hop) = pitch_then(&m.random);
                let mut events = Vec::new();
                ai_step(&mut m, w, &mut events);
                assert!(!m.on_ground && m.velocity.y > 0.0, "{name} hops");
                let played = sounds(&events);
                if silent {
                    assert!(played.is_empty(), "a silent {name} flops in silence");
                } else {
                    // The hop's two floats, then the pitch's two.
                    let (pitch, mut after) = pitch_then(&hop);
                    assert_eq!(
                        played,
                        vec![(event, [0.5, 64.0, 0.5], 1.0, pitch)],
                        "{name} flops at the feet, at getVoicePitch"
                    );
                    assert_eq!(m.random.next_float(), after.next_float());
                }
            }
        }
    });
    let _ = ran;
}

/// `Pufferfish.aiStep` plays no sting of its own for the player: `playerTouch`
/// runs in the player's tick, and its sting is the game event, not a sound of
/// the fish, so the fish's AI tick count cannot make it play.
#[test]
fn the_fish_ai_never_plays_the_players_sting() {
    let ran = with_test_world(&[], |w, types| {
        let p = crate::mobs::world::test_player(Vec3::new(0.5, 64.0, 0.5));
        let world = MobWorld {
            player: Some(p),
            ..w.clone()
        };
        for no_action_time in 0..20 {
            let mut m = fish(types, "minecraft:pufferfish", false);
            m.extra.size = 2;
            m.ai.no_action_time = no_action_time;
            let mut events = Vec::new();
            ai_step(&mut m, &world, &mut events);
            assert!(
                !events.iter().any(
                    |e| matches!(e, MobEvent::Sound(s, ..) if *s == "entity.puffer_fish.sting")
                ),
                "Pufferfish.aiStep plays no sting for the player (tick {no_action_time})"
            );
        }
    });
    let _ = ran;
}
