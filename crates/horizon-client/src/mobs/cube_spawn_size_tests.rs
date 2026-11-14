//! `AbstractCubeMob.setSpawnSize`: the scale is `nextInt(3)`, and a scale
//! below 2 gets one more when `nextFloat() < 0.5 * getSpecialMultiplier()`.
//! So a full multiplier (effective difficulty above 4) gives size 1 one time
//! in six, size 2 one in three and size 4 one in two, where a zero one keeps
//! a flat third each. The float itself is drawn whenever the scale is below
//! 2, whatever the multiplier (vanilla's `&&` tests it after the `nextInt`),
//! and never at scale 2.
use super::super::spawning::{self, BiomeFacts, LocalDifficulty};
use super::super::world::with_test_world;
use super::super::{Mob, MobTypes};
use horizon_core::{
    geometry::Vec3,
    random::{EntityRandom, LegacyRandom, RandomSource},
};
use std::sync::Arc;

fn cube(types: &MobTypes, id: &str, seed: i64) -> Mob {
    let mut m = Mob::new(1, Arc::new(types.types[id].clone()), Vec3::ZERO, 0.0);
    m.random = EntityRandom::own(LegacyRandom::new(seed));
    m
}
/// Hard, a long-played area, full moon: `calculateDifficulty` gives
/// `3 * (0.75 + 0.25 + 1.0 + 0.25)`, so the multiplier is its 1.0 cap.
fn full() -> LocalDifficulty {
    LocalDifficulty::new(3, 2_000_000, 3_600_000, 1.0)
}
/// Easy, a fresh world: effective difficulty 0.75, below 2, multiplier 0.
fn none() -> LocalDifficulty {
    LocalDifficulty::new(1, 0, 0, 0.0)
}

#[test]
fn the_difficulty_bonus_makes_bigger_slimes_and_magma_cubes() {
    let ran = with_test_world(&[], |_w, types| {
        assert_eq!(full().special(), 1.0);
        assert_eq!(none().special(), 0.0);
        for id in ["minecraft:slime", "minecraft:magma_cube"] {
            let (mut one, mut two, mut four) = (0, 0, 0);
            let trials = 3000;
            for seed in 0..i64::from(trials) {
                let mut m = cube(types, id, seed);
                spawning::finalize(
                    &mut m,
                    &BiomeFacts::default(),
                    &full(),
                    &mut LegacyRandom::new(seed),
                );
                match m.extra.size {
                    1 => one += 1,
                    2 => two += 1,
                    4 => four += 1,
                    size => panic!("{id} seed {seed}: size {size}"),
                }
            }
            let all = f64::from(trials);
            let (one, two, four) = (
                f64::from(one) / all,
                f64::from(two) / all,
                f64::from(four) / all,
            );
            // 1/6, 1/3 and 1/2, within three standard deviations.
            assert!((0.14..0.20).contains(&one), "{id}: size 1 at {one}");
            assert!((0.30..0.37).contains(&two), "{id}: size 2 at {two}");
            assert!((0.46..0.53).contains(&four), "{id}: size 4 at {four}");

            let (mut one, mut two, mut four) = (0, 0, 0);
            for seed in 0..i64::from(trials) {
                let mut m = cube(types, id, seed);
                spawning::finalize(
                    &mut m,
                    &BiomeFacts::default(),
                    &none(),
                    &mut LegacyRandom::new(seed),
                );
                match m.extra.size {
                    1 => one += 1,
                    2 => two += 1,
                    4 => four += 1,
                    size => panic!("{id} seed {seed}: size {size}"),
                }
            }
            for (size, count) in [(1, one), (2, two), (4, four)] {
                let share = f64::from(count) / all;
                assert!(
                    (0.30..0.37).contains(&share),
                    "{id}: size {size} at {share} with no multiplier"
                );
            }
        }
    });
    let _ = ran;
}

#[test]
fn the_bonus_float_is_drawn_once_at_the_end_and_only_below_scale_two() {
    let ran = with_test_world(&[], |_w, types| {
        let mut bumped = 0;
        for seed in 0..64 {
            for multiplier in [0.0f32, 1.0] {
                // Replay the two draws on a bare random: the `nextInt(3)`, then
                // `nextFloat()` when the scale is below 2 (vanilla's `&&`
                // tests it whatever the multiplier), and nothing else.
                let mut reference = LegacyRandom::new(seed);
                let scale = reference.next_int_bound(3).unwrap();
                let roll = (scale < 2).then(|| reference.next_float());
                // `setSpawnSize` draws from the level's random.
                let mut m = cube(types, "minecraft:slime", seed);
                let mut level = LegacyRandom::new(seed);
                super::finalize(&mut m, multiplier, &mut level);
                assert_eq!(
                    level.state(),
                    reference.state(),
                    "seed {seed}, multiplier {multiplier}"
                );
                let bumped_size = 2 << scale;
                let expected = match roll {
                    Some(roll) if roll < 0.5 * multiplier => {
                        bumped += 1;
                        bumped_size
                    }
                    _ => 1 << scale,
                };
                assert_eq!(
                    m.extra.size, expected,
                    "seed {seed}, multiplier {multiplier}, scale {scale}, roll {roll:?}"
                );
            }
        }
        assert!((8..32).contains(&bumped), "bumped {bumped} of 64");
    });
    let _ = ran;
}
