//! The variants `finalizeSpawn` rolls for rabbits (`Rabbit.getRandomRabbitVariant`),
//! parrots (`Util.getRandom(Parrot.Variant.values())`) and axolotls
//! (`Axolotl.finalizeSpawn` with `AxolotlGroupData`), alone (`/summon`, a spawn
//! egg: group data null) and in a natural pack, whose members share the group
//! data the first one made.
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
