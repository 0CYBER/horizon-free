//! Context and boundary regressions for the fire tick's individual query sites.
use super::*;
use crate::level::tests::{World, fixture};

const NEIGHBOURS: [(i32, i32, i32, Direction); 6] = [
    (0, -1, 0, Direction::Up),
    (0, 1, 0, Direction::Down),
    (0, 0, -1, Direction::South),
    (0, 0, 1, Direction::North),
    (-1, 0, 0, Direction::East),
    (1, 0, 0, Direction::West),
];

#[test]
fn survival_shape_and_spread_ask_the_neighbours_position_and_inward_face() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f);
    let at = BlockPos::new(4, 8, 4);
    let glass = w.id("glass", &[]);
    // Zero answers prevent short-circuiting and let every direction be observed.
    w.level.fire_odds.insert((glass, None), (0, 0));
    for (x, y, z, _) in NEIGHBOURS {
        w.put(at.offset(x, y, z), glass);
    }
    let expected: Vec<_> = NEIGHBOURS
        .map(|(x, y, z, face)| (glass, at.offset(x, y, z), face))
        .into();
    let sim = w.b().sim(&mut w.level, &mut w.ticks);
    assert!(!sim.is_valid_fire_location(at));
    assert_eq!(*sim.level.fire_asks.borrow(), expected);
    sim.level.fire_asks.borrow_mut().clear();
    assert_eq!(sim.fire_ignite_odds_at(at), 0);
    assert_eq!(*sim.level.fire_asks.borrow(), expected);
    // Air below forces shape attachment queries after below/Up.
    let air = sim.b.default_of(sim.b.named.air);
    sim.level.set_raw(at.offset(0, -1, 0), air);
    sim.level.fire_asks.borrow_mut().clear();
    let shape = sim.fire_state_for(at);
    let mut shape_expected = expected.clone();
    shape_expected[0].0 = air;
    assert_eq!(*sim.level.fire_asks.borrow(), shape_expected);
    for direction in [
        Direction::Up,
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ] {
        assert!(!sim.b.table.bool(shape, sim.direction_prop(direction)));
    }
    sim.level
        .fire_odds
        .insert((glass, Some(Direction::West)), (7, 0));
    let shape = sim.fire_state_for(at);
    assert!(sim.b.table.bool(shape, sim.direction_prop(Direction::East)));
    assert!(!sim.b.table.bool(shape, sim.direction_prop(Direction::West)));
    assert_eq!(sim.fire_ignite_odds_at(at), 7);
}

#[test]
fn each_burn_roll_asks_its_target_and_notifies_before_replacement() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f);
    let at = BlockPos::new(4, 8, 4);
    let glass = w.id("glass", &[]);
    for (x, y, z, face) in NEIGHBOURS {
        let target = at.offset(x, y, z);
        w.put(target, glass);
        let direction = face.opposite();
        let chance = if y == 0 { 300 } else { 250 };
        w.level.fire_odds.insert((glass, None), (0, chance));
        w.level.fire_asks.borrow_mut().clear();
        w.b()
            .sim(&mut w.level, &mut w.ticks)
            .fire_check_burn_out(at, direction, chance, 0);
        assert_eq!(w.level.fire_asks.borrow()[0], (glass, target, face));
        assert_ne!(w.get(target), glass);
        let notice = w.level.caught_fire.last().unwrap();
        assert_eq!((notice.0, notice.1, notice.2), (target, glass, face));
        assert_eq!(w.level.caught_states.last(), Some(&glass));
    }
    assert_eq!(w.level.caught_fire.len(), 6);
}

#[test]
fn odds_boundaries_and_maximum_spread_speed_do_not_overflow() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f);
    let at = BlockPos::new(4, 8, 4);
    let glass = w.id("glass", &[]);
    for (answer, expected) in [
        ((-1, -1), (0, 0)),
        ((250, 250), (250, 250)),
        ((300, 300), (300, 300)),
        ((i32::MAX, i32::MAX), (i32::MAX, 300)),
    ] {
        w.level.fire_odds.insert((glass, None), answer);
        let sim = w.b().sim(&mut w.level, &mut w.ticks);
        assert_eq!(sim.ignite_odds(glass, at, Direction::Up), expected.0);
        assert_eq!(sim.burn_odds(glass, at, Direction::Up), expected.1);
    }
    w.put(at.offset(0, -1, 0), w.id("netherrack", &[]));
    // Unburnable glass supplies maximum spread speed to the air above it.
    w.level.fire_odds.insert((glass, None), (i32::MAX, 0));
    w.put(at.offset(1, 0, 0), glass);
    let fire = w.id("fire", &[]);
    w.put(at, fire);
    w.b().sim(&mut w.level, &mut w.ticks).fire_tick(at, fire);
    assert!(w.b().is_block(w.get(at.offset(1, 1, 0)), w.b().named.fire));
    assert_eq!(
        *w.level.source_asks.borrow(),
        vec![(w.id("netherrack", &[]), at.offset(0, -1, 0))]
    );
}

#[test]
fn age_fifteen_rechecks_below_at_its_top_face() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f);
    let at = BlockPos::new(4, 8, 4);
    let glass = w.id("glass", &[]);
    let stone = w.id("stone", &[]);
    let fire = w.id("fire", &[("age", "15")]);
    w.put(at.offset(0, -1, 0), stone);
    w.put(at.offset(1, 0, 0), glass);
    w.level.fire_odds.insert((glass, None), (1, 0));
    // Repeat under the fixture's fixed RNG until vanilla's one-in-four draw runs.
    for _ in 0..64 {
        w.put(at, fire);
        w.level.fire_asks.borrow_mut().clear();
        w.b().sim(&mut w.level, &mut w.ticks).fire_tick(at, fire);
        if w.b().is_air(w.get(at)) {
            assert!(w.level.fire_asks.borrow().contains(&(
                stone,
                at.offset(0, -1, 0),
                Direction::Up
            )));
            return;
        }
    }
    panic!("age-15 check never ran under the fixed seed");
}
