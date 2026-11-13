use super::*;
use crate::items::ItemStack;
use crate::merchant::{ItemCost, MerchantOffer, Trader};
use crate::mobs::world::with_built_test_world;
use horizon_world::registry::BlockRegistry;
use std::sync::Arc;

const FLOOR: i32 = 63;
const SPOT: Vec3 = Vec3::new(0.5, 64.0, 0.5);

fn villager(types: &crate::mobs::MobTypes, id: u32) -> Mob {
    Mob::new(
        id,
        Arc::new(types.types["minecraft:villager"].clone()),
        SPOT,
        0.0,
    )
}

/// A stone floor with a 1×1 cell at the origin: stone on its four sides, two high. `sides` replaces
/// the block east of the cell (x = 1) at feet and head height, `lid` is put over its head.
fn build(
    world: &mut crate::world::World,
    registry: &BlockRegistry,
    walls: bool,
    east: Option<&str>,
    lid: Option<&str>,
) {
    for cx in -1..=0 {
        for cz in -1..=0 {
            world
                .chunks
                .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
        }
    }
    let state = |name: &str| registry.default_state(name).unwrap();
    for x in -4..=4 {
        for z in -4..=4 {
            world.set(x, FLOOR, z, state("minecraft:stone")).unwrap();
        }
    }
    if walls {
        for (x, z) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            for y in [64, 65] {
                world.set(x, y, z, state("minecraft:stone")).unwrap();
            }
        }
    }
    if let Some(block) = east {
        for y in [64, 65] {
            world.set(1, y, 0, state("minecraft:air")).unwrap();
        }
        if block != "minecraft:air" {
            world.set(1, 64, 0, state(block)).unwrap();
        }
    }
    if let Some(lid) = lid {
        world.set(0, 65, 0, state(lid)).unwrap();
    }
}

fn stuck_in(
    walls: bool,
    east: Option<&str>,
    lid: Option<&str>,
    prepare: impl FnOnce(&mut Mob),
) -> Option<bool> {
    let mut result = None;
    with_built_test_world(
        &[],
        |world, registry| build(world, registry, walls, east, lid),
        |w, types| {
            let mut m = villager(types, 1);
            prepare(&mut m);
            result = Some(is_stuck(&m, w));
        },
    )
    .then_some(())?;
    result
}

#[test]
fn a_villager_in_a_one_by_one_cell_is_stuck_and_one_in_the_open_is_not() {
    // `None`: the prepared vanilla data is missing here, as in a fresh checkout.
    let Some(cell) = stuck_in(true, None, None, |_| {}) else {
        return;
    };
    assert!(cell, "walled in");
    assert_eq!(
        stuck_in(true, None, Some("minecraft:oak_trapdoor"), |_| {}),
        Some(true)
    );
    assert_eq!(
        stuck_in(true, None, Some("minecraft:stone_slab"), |_| {}),
        Some(true)
    );
    assert_eq!(
        stuck_in(false, None, None, |_| {}),
        Some(false),
        "open field"
    );
    // One side open, a step up to a half block (a slab, jumpable), or a door villagers open.
    assert_eq!(
        stuck_in(true, Some("minecraft:air"), None, |_| {}),
        Some(false)
    );
    assert_eq!(
        stuck_in(true, Some("minecraft:oak_door"), None, |_| {}),
        Some(false)
    );
    // A fence one and a half blocks high is no way out.
    assert_eq!(
        stuck_in(true, Some("minecraft:oak_fence"), None, |_| {}),
        Some(true)
    );
}

#[test]
fn a_rider_is_stuck_in_the_open_and_a_sleeper_is_not_in_a_cell() {
    let Some(rider) = stuck_in(false, None, None, |m| m.extra.mount = Some(7)) else {
        return;
    };
    assert!(rider);
    assert_eq!(
        stuck_in(true, None, None, |m| m.extra.ints[1] = 1),
        Some(false),
        "the brain has to wake a sleeper"
    );
}

fn used_librarian_offer() -> Trader {
    Trader {
        offers: vec![MerchantOffer {
            base_cost_a: ItemCost {
                stack: ItemStack::new("minecraft:emerald", 4),
            },
            cost_b: None,
            result: ItemStack::new("minecraft:iron_boots", 1),
            uses: 12,
            max_uses: 12,
            reward_exp: true,
            special_price_diff: 0,
            demand: 0,
            price_multiplier: 0.2,
            xp: 1,
        }],
        ..Default::default()
    }
}

#[test]
fn a_stuck_librarian_restocks_only_at_its_own_workstation_within_reach() {
    // (block at the remembered site, its x, day time, restocked)
    for (block, x, day_time, restocked) in [
        ("minecraft:lectern", 3, 3_000, true),
        ("minecraft:lectern", 3, 14_000, false),
        ("minecraft:air", 3, 3_000, false),
        ("minecraft:bell", 3, 3_000, false),
        ("minecraft:composter", 3, 3_000, false),
        // The villager stands at x = 0.5: a lectern in the wall beside it is in reach, one 2 blocks off is not.
        ("minecraft:lectern", 1, 3_000, true),
        ("minecraft:lectern", 2, 3_000, false),
        ("minecraft:lectern", -4, 3_000, false),
    ] {
        with_built_test_world(
            &[],
            |world, registry| {
                build(world, registry, true, None, None);
                if block != "minecraft:air" {
                    world
                        .set(x, 64, 0, registry.default_state(block).unwrap())
                        .unwrap();
                }
            },
            |w, types| {
                let mut w = w.clone();
                // Villager 1's turn to restock: (game_time + 1) is a multiple of 300.
                w.game_time = 3_299;
                w.day_time = day_time;
                let mut m = villager(types, 1);
                m.extra.variant2 = 9; // librarian
                m.extra.home = Some([x, 64, 0]);
                m.extra.trade = Some(Box::new(used_librarian_offer()));
                assert!(step(&mut m, &w, 100), "stuck, so the brain is skipped");
                let uses = m.extra.trade.as_ref().unwrap().offers[0].uses;
                assert_eq!(
                    uses == 0,
                    restocked,
                    "{block} at x {x}, day time {day_time}"
                );
            },
        );
    }
}

#[test]
fn a_released_villager_gets_its_brain_back_at_its_next_check() {
    with_built_test_world(
        &[],
        |world, registry| build(world, registry, true, None, None),
        |w, types| {
            let mut w = w.clone();
            let mut m = villager(types, 1);
            w.game_time = 50;
            assert!(step(&mut m, &w, 100), "walled in");
            // It leaves the cell; the answer stands until the villager's turn (game time + 1 a multiple of 100).
            m.position = Vec3::new(3.5, 64.0, 3.5);
            w.game_time = 60;
            assert!(step(&mut m, &w, 100), "no recheck before its turn");
            w.game_time = 99;
            assert!(!step(&mut m, &w, 100), "free at its recheck");
        },
    );
}

/// A walled pocket on the stone floor: `cells` (x, z) are open, every block around them is stone
/// two high, and `furnish` puts blocks (a bed, a workstation) into the pocket.
fn pocket_is_stuck(cells: &[(i32, i32)], furnish: &[(i32, i32, &str)]) -> Option<bool> {
    let mut result = None;
    with_built_test_world(
        &[],
        |world, registry| {
            build(world, registry, false, None, None);
            let stone = registry.default_state("minecraft:stone").unwrap();
            for x in -3..=4 {
                for z in -3..=3 {
                    if !cells.contains(&(x, z)) {
                        for y in [64, 65] {
                            world.set(x, y, z, stone).unwrap();
                        }
                    }
                }
            }
            for &(x, z, block) in furnish {
                world
                    .set(x, 64, z, registry.default_state(block).unwrap())
                    .unwrap();
            }
        },
        |w, types| result = Some(is_stuck(&villager(types, 1), w)),
    )
    .then_some(())?;
    result
}

#[test]
fn a_villager_with_a_bed_or_a_second_cell_is_still_stuck_but_a_room_is_not() {
    let Some(two) = pocket_is_stuck(&[(0, 0), (1, 0)], &[]) else {
        return;
    };
    assert!(two, "two cells");
    // A bed next to it (a half block it can step onto).
    assert_eq!(
        pocket_is_stuck(&[(0, 0), (1, 0), (2, 0)], &[(1, 0, "minecraft:red_bed")]),
        Some(true)
    );
    // 3×3 cells: it can wander, so it keeps its brain.
    let room: Vec<_> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| (x, z)))
        .collect();
    assert_eq!(pocket_is_stuck(&room, &[]), Some(false));
}
