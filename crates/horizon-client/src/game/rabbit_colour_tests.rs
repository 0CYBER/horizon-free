use super::*;

/// `Rabbit.getBreedOffspring`: the baby starts as a fresh biome variant
/// (`Rabbit.getRandomRabbitVariant`: the level's `nextInt(100)`), and unless
/// the breeding parent's `nextInt(20)` comes up 0, it takes a parent's colour
/// by that parent's `nextBoolean`. A brown and a white rabbit therefore give
/// white about 47.5% of the time, brown 47.5%, and a fresh biome variant 5%.
#[test]
#[ignore = "needs prepared resources (HORIZON_TEST_RESOURCES)"]
fn a_baby_rabbit_takes_a_parents_colour_or_a_fresh_biome_variant() {
    use horizon_core::random::{EntityRandom, LegacyRandom, LevelRandom, RandomSource};
    let mut game = super::super::native_machines::tests::server_game();
    let kind = game
        .res
        .mob_types()
        .get("minecraft:rabbit")
        .expect("a rabbit")
        .clone();
    // What a fresh variant can be here comes from the biome's tags.
    let facts = game.biome_facts(8, 64, 8);
    let fresh = |roll: i32| {
        if facts.white_rabbit {
            if roll < 80 { 1 } else { 3 }
        } else if facts.gold_rabbit {
            4
        } else if roll < 50 {
            0
        } else if roll < 90 {
            5
        } else {
            2
        }
    };
    let (part, mate_id) = (7u32, 8u32);
    let n = 2_000u32;
    let mut counts = [0usize; 8];
    let mut fresh_drawn = 0usize;
    for i in 0..n {
        let (parent_seed, level_seed) = (
            1_000_003 + 7_919 * i64::from(i),
            4_242 + 104_729 * i64::from(i),
        );
        game.world.random = LevelRandom::for_level(LegacyRandom::new(level_seed));
        game.mobs.clear();
        game.orbs.clear();
        // Ids the spawner hands out stay clear of the pair's.
        game.next_entity = 10_000 + 4 * i;
        for (id, variant) in [(part, 0u8), (mate_id, 1)] {
            let mut m = Mob::new(id, kind.clone(), Vec3::new(8.5, 64.0, 8.5), 0.0);
            m.random = EntityRandom::own(LegacyRandom::new(parent_seed + i64::from(id)));
            m.extra.variant = variant;
            m.extra.in_love = 600;
            game.mobs.push(m);
        }
        game.breed(part, mate_id, None);
        let child = game
            .mobs
            .iter()
            .find(|m| m.id != part && m.id != mate_id)
            .expect("a baby rabbit");
        assert_eq!(child.extra.age, -24000, "{i}: a baby");
        // What vanilla's draws answer, in vanilla's order: the level's roll,
        // then the parent's one-in-twenty, then its coin toss.
        let mut level = LegacyRandom::new(level_seed);
        let biome_variant = fresh(level.next_int_bound(100).unwrap());
        let mut parent = LegacyRandom::new(parent_seed + i64::from(part));
        let expected = if parent.next_int_bound(20).unwrap() == 0 {
            fresh_drawn += 1;
            biome_variant
        } else if parent.next_boolean() {
            1
        } else {
            0
        };
        assert_eq!(
            child.extra.variant, expected,
            "{i}: the baby takes vanilla's draw"
        );
        // The parent's stream ends where vanilla's does: the coin, then
        // finalizeSpawnChildFromBreeding's 1-to-7 roll.
        if game.ui.rules.bool("mob_drops") {
            let _ = parent.next_int_bound(7);
        }
        let mut drawn = game
            .mobs
            .iter()
            .find(|m| m.id == part)
            .expect("the parent")
            .random
            .clone();
        for _ in 0..4 {
            assert_eq!(
                drawn.next_int(),
                parent.next_int(),
                "{i}: the parent's stream goes on where vanilla leaves it"
            );
        }
        counts[usize::from(child.extra.variant)] += 1;
    }
    // The one-in-twenty biome case was reached, and the fresh variants that
    // differ from both parents showed up: 5% of the babies.
    assert!(fresh_drawn > 50, "one in twenty: {fresh_drawn} of {n}");
    let fresh_share = |v: u8| {
        if facts.white_rabbit {
            match v {
                1 => 0.04,
                3 => 0.01,
                _ => 0.0,
            }
        } else if facts.gold_rabbit {
            if v == 4 { 0.05 } else { 0.0 }
        } else {
            match v {
                0 => 0.025,
                5 => 0.02,
                2 => 0.005,
                _ => 0.0,
            }
        }
    };
    for v in 0..8u8 {
        let vanilla = fresh_share(v) + if v == 0 || v == 1 { 0.475 } else { 0.0 };
        let seen = counts[usize::from(v)] as f64 / f64::from(n);
        assert!(
            (seen - vanilla).abs() < 0.03,
            "variant {v}: {seen:.3} of the babies, vanilla gives {vanilla:.3}"
        );
    }
    assert!(
        counts[2] + counts[3] + counts[4] + counts[5] > 0,
        "a fresh biome variant: {counts:?}"
    );
}

/// `Rabbit.getRandomRabbitVariant`: the biome's tags pick the split, and the
/// `nextInt(100)` is drawn even where a tag decides alone (the gold rabbit),
/// so a spawn's and a bred baby's level random move on by the same one draw.
#[test]
fn the_biome_tags_pick_the_rabbit_variant() {
    use horizon_core::random::{LegacyRandom, RandomSource};
    let mut white = [0usize; 8];
    let mut temperate = [0usize; 8];
    for seed in 0..2_000i64 {
        let mut r = LegacyRandom::new(seed);
        white[usize::from(horizon_client::mobs::rabbit::random_variant(
            true, false, &mut r,
        ))] += 1;
        let mut r = LegacyRandom::new(seed);
        temperate[usize::from(horizon_client::mobs::rabbit::random_variant(
            false, false, &mut r,
        ))] += 1;
    }
    assert_eq!(
        white[1] + white[3],
        2_000,
        "a white rabbit biome: {white:?}"
    );
    assert!(
        (1_550..1_650).contains(&white[1]),
        "80% white, 20% splotched: {white:?}"
    );
    assert_eq!(
        temperate[0] + temperate[2] + temperate[5],
        2_000,
        "elsewhere: {temperate:?}"
    );
    assert!(
        (900..1_100).contains(&temperate[0])
            && (700..900).contains(&temperate[5])
            && (100..300).contains(&temperate[2]),
        "50/40/10 brown, salt, black: {temperate:?}"
    );
    // The gold rabbit's tag decides alone, but its roll is still drawn.
    let mut a = LegacyRandom::new(7);
    assert_eq!(
        horizon_client::mobs::rabbit::random_variant(false, true, &mut a),
        4
    );
    let mut b = LegacyRandom::new(7);
    let _ = b.next_int_bound(100);
    assert_eq!(a.next_int(), b.next_int(), "the roll is drawn for gold");
}
