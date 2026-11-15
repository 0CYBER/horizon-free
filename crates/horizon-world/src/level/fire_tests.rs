//! Fire and lava over an in-memory level, answered by `TestLevel` as a mod's block parts would
//! answer `fire_odds`, `is_fire_source`, `ignited_by_lava` and `on_caught_fire`. Needs the
//! prepared client resources (skipped when absent), as `tests` does.
use super::tests::{World, fixture};
use super::*;

fn world() -> Option<World<'static>> {
    fixture().map(World::new)
}

fn is_fire(w: &World, at: BlockPos) -> bool {
    w.b().block(w.get(at)) == w.b().block(w.id("fire", &[]))
}

#[test]
fn a_level_can_answer_how_a_block_burns() {
    let Some(mut w) = world() else { return };
    w.floor(0, 10, "stone");
    let glass = w.id("glass", &[]);
    let planks = w.id("oak_planks", &[]);
    let burn = |w: &mut World, fire: BlockPos| {
        w.set(fire.offset(0, -1, 0), w.id("netherrack", &[]));
        w.set(fire.offset(1, 0, 0), glass);
        w.set(fire.offset(-1, 0, 0), planks);
        w.set(fire, w.id("fire", &[]));
        for _ in 0..4000 {
            w.tick();
        }
    };
    // Vanilla's table: glass never burns, planks do.
    let fire = BlockPos::new(0, 1, 0);
    burn(&mut w, fire);
    assert_eq!(w.get(fire.offset(1, 0, 0)), glass, "glass doesn't burn");
    assert_ne!(w.get(fire.offset(-1, 0, 0)), planks, "planks burn");

    // The level's answer wins over the table, either way.
    w.level.fire_odds.insert((glass, None), (30, 60));
    w.level.fire_odds.insert((planks, None), (0, 0));
    let fire = BlockPos::new(0, 1, 6);
    burn(&mut w, fire);
    assert_ne!(w.get(fire.offset(1, 0, 0)), glass, "flammable glass burns");
    assert_eq!(
        w.get(fire.offset(-1, 0, 0)),
        planks,
        "fireproof planks don't"
    );

    // Fire in the air beside a block stays only while the block's ignite
    // odds are above zero, and burn odds of zero never destroy the block.
    let lit_beside = |w: &mut World, at: BlockPos, ticks: usize| {
        w.set(at.offset(1, 0, 0), glass);
        w.set(at, w.id("fire", &[]));
        for _ in 0..ticks {
            w.tick();
        }
        w.get(at)
    };
    let fire = w.id("fire", &[]);
    w.level.fire_odds.clear();
    let at = BlockPos::new(-6, 5, 0);
    assert_ne!(
        w.b().block(lit_beside(&mut w, at, 45)),
        w.b().block(fire),
        "fire beside plain glass goes out"
    );
    w.level.fire_odds.insert((glass, None), (5, 0));
    let at = BlockPos::new(-6, 5, 6);
    assert_eq!(
        w.b().block(lit_beside(&mut w, at, 45)),
        w.b().block(fire),
        "fire stands beside glass that ignites"
    );
    for _ in 0..4000 {
        w.tick();
    }
    assert_eq!(w.get(at.offset(1, 0, 0)), glass, "and never burns it away");
}

#[test]
fn a_level_can_answer_fire_odds_for_one_face() {
    let Some(mut w) = world() else { return };
    let (glass, fire) = (w.id("glass", &[]), w.id("fire", &[]));
    // The glass ignites from its top only.
    w.level
        .fire_odds
        .insert((glass, Some(Direction::Up)), (5, 0));
    // Fire on top of it: a valid location, and the age-15 burn-out needs a
    // block below that can't burn.
    let on_top = BlockPos::new(0, 2, 0);
    w.set(on_top.offset(0, -1, 0), glass);
    w.set(on_top, fire);
    // Fire in the air beside the glass's west face, with air below it.
    let beside = BlockPos::new(6, 6, 0);
    w.set(beside.offset(1, 0, 0), glass);
    w.set(beside, fire);
    for _ in 0..45 {
        w.tick();
    }
    assert!(!is_fire(&w, beside), "the west face isn't flammable");
    for _ in 0..4000 {
        w.tick();
    }
    assert!(is_fire(&w, on_top), "fire stands on the flammable top");
    assert_eq!(w.get(on_top.offset(0, -1, 0)), glass, "burn odds 0 keep it");
}

#[test]
fn a_level_can_answer_what_keeps_a_fire_burning() {
    let Some(mut w) = world() else { return };
    let (stone, netherrack) = (w.id("stone", &[]), w.id("netherrack", &[]));
    let (on_stone, on_netherrack) = (BlockPos::new(0, 1, 0), BlockPos::new(8, 1, 0));
    let light = |w: &mut World, at: BlockPos, ground: BlockStateId| {
        w.set(at.offset(0, -1, 0), ground);
        w.set(at, w.id("fire", &[]));
        for _ in 0..4000 {
            w.tick();
        }
    };
    // Stone is sturdy but no valid location: its fire goes out once its age
    // passes 3. The overworld's infiniburn tag keeps netherrack's.
    light(&mut w, on_stone, stone);
    light(&mut w, on_netherrack, netherrack);
    assert!(!is_fire(&w, on_stone), "fire on plain stone goes out");
    assert!(is_fire(&w, on_netherrack), "netherrack is a fire source");

    // The level's answer wins over the tag, either way.
    w.level.fire_sources.insert(stone, true);
    w.level.fire_sources.insert(netherrack, false);
    light(&mut w, on_stone, stone);
    assert!(is_fire(&w, on_stone), "stone answering yes keeps its fire");
    assert!(
        !is_fire(&w, on_netherrack),
        "netherrack answering no loses it"
    );
}

#[test]
fn a_level_can_answer_what_lava_sets_alight() {
    let Some(mut w) = world() else { return };
    let lava = w.id("lava", &[]);
    // Lava ringed at its own level by `ring`, with air above, random-ticked
    // under the fixed seed: lava lights the air over the ring (whose top faces
    // it) and the empty cell on top of the ring. Whether any fire appeared.
    let lights = |w: &mut World, at: BlockPos, ring: &str| {
        let ring = w.id(ring, &[]);
        w.put(at, lava);
        for x in -1..=1 {
            for z in -1..=1 {
                if (x, z) != (0, 0) {
                    w.put(at.offset(x, 0, z), ring);
                }
            }
        }
        for _ in 0..200 {
            w.b().random_tick(&mut w.level, &mut w.ticks, at);
        }
        (-2..=2).any(|x| (-2..=2).any(|z| (1..=2).any(|y| is_fire(w, at.offset(x, y, z)))))
    };
    assert!(
        lights(&mut w, BlockPos::new(0, 5, 0), "oak_planks"),
        "vanilla flags planks"
    );
    assert!(
        !lights(&mut w, BlockPos::new(10, 5, 0), "stone"),
        "and not stone"
    );

    let (stone, planks) = (w.id("stone", &[]), w.id("oak_planks", &[]));
    w.level.lava_ignites.insert(stone, true);
    w.level.lava_ignites.insert(planks, false);
    assert!(
        lights(&mut w, BlockPos::new(20, 5, 0), "stone"),
        "stone answering yes"
    );
    assert!(
        !lights(&mut w, BlockPos::new(30, 5, 0), "oak_planks"),
        "planks answering no"
    );
}

/// Fire on netherrack at `fire` with `block` east of it, for 4000 ticks. The fire spreads nowhere,
/// so only it can burn the block, from the block's west face.
fn burn_east(w: &mut World, fire: BlockPos, block: BlockStateId) -> BlockPos {
    let east = fire.offset(1, 0, 0);
    w.set(fire.offset(0, -1, 0), w.id("netherrack", &[]));
    w.set(east, block);
    for x in -1..=1 {
        for y in -1..=4 {
            for z in -1..=1 {
                let at = fire.offset(x, y, z);
                if at != east {
                    w.level.guarded.insert(at);
                }
            }
        }
    }
    w.set(fire, w.id("fire", &[]));
    for _ in 0..4000 {
        w.tick();
    }
    east
}

#[test]
fn a_level_hears_each_block_fire_burns() {
    let Some(mut w) = world() else { return };
    let (wool, tnt) = (w.id("white_wool", &[]), w.id("tnt", &[]));
    // What the level heard for `at`: the old state, the face and the level events before it.
    let heard = |w: &World, at: BlockPos| {
        let heard = w.level.caught_fire.iter().filter(|heard| heard.0 == at);
        heard
            .map(|&(_, state, face, before)| (state, face, before))
            .collect::<Vec<_>>()
    };

    // `block_burn` cancels: the wool stays, and the level hears nothing.
    w.level.unburnable.insert(BlockPos::new(1, 1, 10));
    let kept = burn_east(&mut w, BlockPos::new(0, 1, 10), wool);
    assert_eq!(w.get(kept), wool);
    assert!(w.level.caught_fire.is_empty());

    // Wool east of the fire burns once, heard with its old state and its west face.
    let burned = burn_east(&mut w, BlockPos::new(0, 1, 0), wool);
    assert_ne!(w.get(burned), wool);
    let wool_heard = heard(&w, burned);
    let [(state, face, _)] = wool_heard[..] else {
        panic!("the burned wool is heard once, not {wool_heard:?}");
    };
    assert_eq!((state, face), (wool, Direction::West));

    // The level hears TNT while it is present, before it primes.
    let primed = burn_east(&mut w, BlockPos::new(0, 1, 20), tnt);
    let tnt_heard = heard(&w, primed);
    let [(state, face, before)] = tnt_heard[..] else {
        panic!("the burned TNT is heard once, not {tnt_heard:?}");
    };
    assert_eq!((state, face), (tnt, Direction::West));
    let primed_first = w.level.events[..before]
        .iter()
        .any(|event| matches!(event, LevelEvent::PrimeTnt { pos, .. } if *pos == primed));
    assert!(!primed_first, "the TNT primes after the notice");
    assert!(
        w.level.events[before..]
            .iter()
            .any(|event| matches!(event, LevelEvent::PrimeTnt { pos, .. } if *pos == primed))
    );
    assert_eq!(w.level.caught_states, vec![wool, tnt]);
}

#[test]
fn lava_queries_exact_positions_and_faces_in_both_random_tick_branches() {
    let Some(mut w) = world() else { return };
    let at = BlockPos::new(4, 8, 4);
    let air = w.id("air", &[]);
    // Seed 0 selects the lower branch: three draws at (0,0), (1,1), (1,1).
    *w.level.random() = LegacyRandom::new(0);
    w.b().sim(&mut w.level, &mut w.ticks).lava_random_tick(at);
    assert_eq!(
        *w.level.lava_asks.borrow(),
        vec![
            (air, BlockPos::new(4, 8, 4), Direction::Up),
            (air, BlockPos::new(5, 8, 5), Direction::Up),
            (air, BlockPos::new(5, 8, 5), Direction::Up),
        ]
    );
    w.level.lava_asks.borrow_mut().clear();
    // Seed 2 selects one upward pass to (3,9,5); air never short-circuits.
    *w.level.random() = LegacyRandom::new(2);
    w.b().sim(&mut w.level, &mut w.ticks).lava_random_tick(at);
    assert_eq!(
        *w.level.lava_asks.borrow(),
        vec![
            (air, BlockPos::new(3, 8, 5), Direction::Up),
            (air, BlockPos::new(3, 10, 5), Direction::Down),
            (air, BlockPos::new(3, 9, 4), Direction::South),
            (air, BlockPos::new(3, 9, 6), Direction::North),
            (air, BlockPos::new(2, 9, 5), Direction::East),
            (air, BlockPos::new(4, 9, 5), Direction::West),
        ]
    );
}
