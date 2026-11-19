//! The variants `finalizeSpawn` rolls for rabbits (`Rabbit.getRandomRabbitVariant`),
//! parrots (`Util.getRandom(Parrot.Variant.values())`) and axolotls
//! (`Axolotl.finalizeSpawn` with `AxolotlGroupData`), alone (`/summon`, a spawn
//! egg: group data null) and in a natural pack, whose members share the group
//! data the first one made. A pack of tropical fish also ends at a loner
//! (`TropicalFish.isSchool`, which `Spawned.school` carries to the spawner).
use super::LocalDifficulty;
use super::tests::{run_finalize_in, types};
use crate::mobs::{Mob, finalize::Group};
use horizon_core::{
    geometry::Vec3,
    random::{EntityRandom, LegacyRandom, RandomSource},
};
use std::sync::Arc;

/// A mob of `kind` whose own random is seeded from `seed`.
fn mob(types: &crate::mobs::MobTypes, kind: &str, seed: u32) -> Mob {
    let kind = Arc::new(types.types.get(kind).unwrap().clone());
    let mut m = Mob::new(seed, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    m.random = EntityRandom::own(LegacyRandom::new(i64::from(seed) * 31 + 7));
    m
}

fn normal() -> LocalDifficulty {
    LocalDifficulty::new(2, 0, 0, 1.0)
}

/// A lone spawn draws the variant from the level's random first, before
/// `Mob.finalizeSpawn`'s own draws.
#[test]
fn a_lone_rabbit_parrot_and_axolotl_take_the_level_randoms_first_draws() {
    let Some(types) = types() else { return };
    for seed in 0..64u32 {
        let finalized = |kind: &str| {
            let mut m = mob(&types, kind, seed);
            let level: LegacyRandom = (*m.random).clone();
            run_finalize_in(&mut m, normal(), None);
            (m, level)
        };
        // Rabbit.getRandomRabbitVariant: brown 0, salt 5, black 2 by nextInt(100).
        let (rabbit, level) = finalized("minecraft:rabbit");
        let roll = level.clone().next_int_bound(100).unwrap();
        let plain = if roll < 50 {
            0
        } else if roll < 90 {
            5
        } else {
            2
        };
        assert_eq!(rabbit.extra.variant, plain, "seed {seed}, roll {roll}");
        assert!(!rabbit.is_baby());

        let (parrot, level) = finalized("minecraft:parrot");
        assert_eq!(
            i32::from(parrot.extra.variant),
            level.clone().next_int_bound(5).unwrap()
        );

        let (axolotl, level) = finalized("minecraft:axolotl");
        let mut r = level;
        let pair = [r.next_int_bound(4).unwrap(), r.next_int_bound(4).unwrap()];
        let picked = pair[r.next_int_bound(2).unwrap() as usize];
        assert_eq!(i32::from(axolotl.extra.variant), picked, "seed {seed}");
        assert!(!axolotl.is_baby());
    }
}

/// `NaturalSpawner.spawnCategoryForPosition` hands each member the previous
/// one's group data: a pack of rabbits shares the first one's variant and the
/// rest are babies (`RabbitGroupData`'s chance 1); a pack of axolotls picks
/// from the first one's two variants, and the third on are babies.
#[test]
fn a_pack_shares_its_first_members_group_data() {
    let Some(types) = types() else { return };
    for seed in 0..32u32 {
        let mut group = None;
        let mut variants = Vec::new();
        for i in 0..4 {
            let mut m = mob(&types, "minecraft:rabbit", seed * 4 + i);
            group = run_finalize_in(&mut m, normal(), group);
            assert_eq!(m.is_baby(), i > 0, "rabbit {i} of pack {seed}");
            variants.push(m.extra.variant);
        }
        assert!(variants.iter().all(|&v| v == variants[0]), "{variants:?}");
        assert!(
            matches!(group, Some(Group::Rabbit(_, v)) if v == variants[0]),
            "{group:?}"
        );

        let mut group = None;
        for i in 0..4 {
            let mut m = mob(&types, "minecraft:axolotl", seed * 4 + i);
            group = run_finalize_in(&mut m, normal(), group);
            let Some(Group::Axolotl(_, pair)) = group else {
                panic!("no axolotl group: {group:?}");
            };
            assert!(pair.contains(&m.extra.variant) && pair.iter().all(|&t| t < 4));
            assert_eq!(m.is_baby(), i >= 2, "axolotl {i} of pack {seed}");
        }
    }
}

/// What vanilla's draws settled for one tropical fish: `isSchool`, which the
/// spawner reads back as `isMaxGroupSizeReached = !isSchool`, and the variant
/// it wears.
struct Fish {
    school: bool,
    variant: (u8, u8, u8),
}

/// `Mob.finalizeSpawn` (Mob.java:1081-1089) and `TropicalFish.finalizeSpawn`
/// (TropicalFish.java:238-252) replayed on `random`, which stands for the
/// level's: the follow range bonus (`random.triangle(0.0, 0.11485000000000001)`,
/// two `nextDouble`s) and left-handedness, then the school roll. `shared` is
/// the variant the pack already rolled (`TropicalFishGroupData`): its members
/// take that one and draw nothing further.
fn vanilla_tropical_fish(random: &mut LegacyRandom, shared: Option<(u8, u8, u8)>) -> Fish {
    let _bonus = random.next_double() - random.next_double();
    let _left_handed = random.next_float() < 0.05;
    if let Some(v) = shared {
        return Fish {
            school: true,
            variant: v,
        };
    }
    if f64::from(random.next_float()) < 0.9 {
        // `Util.getRandom(TropicalFish.COMMON_VARIANTS, random)`: one of that
        // table's rows, which the pack's first member keeps for the rest.
        let row = random.next_int_bound(22).unwrap() as usize;
        Fish {
            school: true,
            variant: crate::mobs::aquatic::COMMON_TROPICAL[row],
        }
    } else {
        // A loner wears a random `Pattern` (12 of them) and two random
        // `DyeColor`s (16).
        Fish {
            school: false,
            variant: (
                random.next_int_bound(12).unwrap() as u8,
                random.next_int_bound(16).unwrap() as u8,
                random.next_int_bound(16).unwrap() as u8,
            ),
        }
    }
}

/// `NaturalSpawner.spawnCategoryForPosition` breaks a pack at the first member
/// whose `isMaxGroupSizeReached` answers true (NaturalSpawner.java:212-214);
/// for a tropical fish that is `!isSchool`, so a pack is a loner alone. A
/// member after a school's first one takes its `TropicalFishGroupData` variant
/// and follows it, and the pack goes on.
#[test]
fn a_pack_of_tropical_fish_ends_at_its_first_loner() {
    let Some(types) = types() else { return };
    let (mut loner_packs, mut schools) = (0u32, 0u32);
    for seed in 0..64u32 {
        let mut group = None;
        let mut shared = None;
        let mut length = 0;
        for i in 0..4u32 {
            let mut m = mob(&types, "minecraft:tropical_fish", seed * 4 + i);
            let mut replay: LegacyRandom = (*m.random).clone();
            let expected = vanilla_tropical_fish(&mut replay, shared);
            group = run_finalize_in(&mut m, normal(), group);
            assert_eq!(
                !m.extra.lone, expected.school,
                "seed {seed}, member {i}: isSchool"
            );
            assert_eq!(
                (m.extra.variant, m.extra.variant2, m.extra.variant3),
                expected.variant,
                "seed {seed}, member {i}: variant"
            );
            shared = Some(expected.variant);
            if i > 0 {
                assert_eq!(
                    m.extra.leader,
                    Some(seed * 4),
                    "seed {seed}, member {i}: leader"
                );
            }
            length += 1;
            // The pack ends here: the first loner is the whole pack.
            if !expected.school {
                break;
            }
        }
        if length == 1 {
            loner_packs += 1;
        } else {
            schools += 1;
        }
    }
    assert!(
        loner_packs > 0 && schools > 0,
        "{loner_packs} packs of one loner, {schools} schools"
    );
}
