//! The variants `finalizeSpawn` rolls for rabbits (`Rabbit.getRandomRabbitVariant`),
//! parrots (`Util.getRandom(Parrot.Variant.values())`) and axolotls
//! (`Axolotl.finalizeSpawn` with `AxolotlGroupData`), alone (`/summon`, a spawn
//! egg: group data null) and in a natural pack, whose members share the group
//! data the first one made.
use super::{BiomeFacts, LocalDifficulty, SpawnGroup, finalize, finalize_on_own_random_in};
use crate::mobs::{Anim, Brain, Mob, MobType, Part, Species};
use horizon_core::{
    geometry::Vec3,
    random::{EntityRandom, LegacyRandom, RandomSource},
};
use std::sync::Arc;

/// A type of `species`, without the prepared resources a test machine may not have.
fn mob(species: Species, id: &str, seed: u32) -> Mob {
    let kind = Arc::new(MobType {
        id: id.into(),
        species,
        width: 0.4,
        height: 0.5,
        eye_height: 0.4,
        category: "creature".into(),
        max_health: 10.0,
        speed: 0.3,
        flying_speed: 0.4,
        attack_damage: 0.0,
        attack_knockback: 0.0,
        follow_range: 16.0,
        step_height: 0.6,
        armor: 0.0,
        armor_toughness: 0.0,
        knockback_resistance: 0.0,
        jump_strength: 0.42,
        attributes: Default::default(),
        item_stack_limits: Default::default(),
        gravity: 0.08,
        safe_fall_distance: 3.0,
        fall_damage_multiplier: 1.0,
        water_efficiency: 0.0,
        water_slowdown: 0.8,
        air_speed: 0.02,
        baby_scale: 0.5,
        head_rot_speed: 10.0,
        max_head_x_rot: 40.0,
        max_head_y_rot: 75.0,
        ambient_interval: 80,
        flags: Default::default(),
        model: Arc::new(Part {
            name: "root".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: Vec::new(),
            children: Vec::new(),
        }),
        layers: Default::default(),
        texture: String::new(),
        brain: Brain::Passive,
        anim: Anim::Humanoid { zombie_arms: false },
        sounds: ["entity.cow.hurt", "entity.cow.hurt", "entity.cow.death"],
        step_sound: None,
        food: Default::default(),
        tempt: Default::default(),
    });
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
    let white = BiomeFacts {
        white_rabbit: true,
        ..Default::default()
    };
    let gold = BiomeFacts {
        gold_rabbit: true,
        ..Default::default()
    };
    for seed in 0..64u32 {
        let level = LegacyRandom::new(i64::from(seed) * 7_919 + 1);
        let finalized = |species, id, facts: &BiomeFacts| {
            let mut m = mob(species, id, seed);
            finalize(&mut m, facts, &normal(), &mut level.clone());
            m
        };
        // Rabbit.getRandomRabbitVariant: brown 0, salt 5, black 2 by nextInt(100);
        // white 1 / white splotched 3 under spawns_white_rabbits; always gold 4
        // under spawns_gold_rabbits.
        let roll = level.clone().next_int_bound(100).unwrap();
        let plain = if roll < 50 {
            0
        } else if roll < 90 {
            5
        } else {
            2
        };
        let rabbit = finalized(Species::Rabbit, "minecraft:rabbit", &BiomeFacts::default());
        assert_eq!(rabbit.extra.variant, plain, "seed {seed}, roll {roll}");
        assert!(!rabbit.is_baby());
        let snowy = finalized(Species::Rabbit, "minecraft:rabbit", &white);
        assert_eq!(snowy.extra.variant, if roll < 80 { 1 } else { 3 });
        let desert = finalized(Species::Rabbit, "minecraft:rabbit", &gold);
        assert_eq!(desert.extra.variant, 4);

        let parrot = finalized(Species::Parrot, "minecraft:parrot", &BiomeFacts::default());
        assert_eq!(
            i32::from(parrot.extra.variant),
            level.clone().next_int_bound(5).unwrap()
        );

        let mut r = level.clone();
        let types = [r.next_int_bound(4).unwrap(), r.next_int_bound(4).unwrap()];
        let picked = types[r.next_int_bound(2).unwrap() as usize];
        let axolotl = finalized(
            Species::Axolotl,
            "minecraft:axolotl",
            &BiomeFacts::default(),
        );
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
    for seed in 0..32u32 {
        let mut group = None;
        let mut variants = Vec::new();
        for i in 0..4 {
            let mut m = mob(Species::Rabbit, "minecraft:rabbit", seed * 4 + i);
            finalize_on_own_random_in(&mut m, &BiomeFacts::default(), &normal(), &mut group);
            assert_eq!(m.is_baby(), i > 0, "rabbit {i} of pack {seed}");
            variants.push(m.extra.variant);
        }
        assert!(variants.iter().all(|&v| v == variants[0]), "{variants:?}");
        assert_eq!(
            group,
            Some(SpawnGroup::Rabbit {
                variant: variants[0],
                size: 4
            })
        );

        let mut group = None;
        for i in 0..4 {
            let mut m = mob(Species::Axolotl, "minecraft:axolotl", seed * 4 + i);
            finalize_on_own_random_in(&mut m, &BiomeFacts::default(), &normal(), &mut group);
            let Some(SpawnGroup::Axolotl { types, size }) = group else {
                panic!("no axolotl group: {group:?}");
            };
            assert_eq!(size, i + 1);
            assert!(types.contains(&m.extra.variant) && types.iter().all(|&t| t < 4));
            assert_eq!(m.is_baby(), i >= 2, "axolotl {i} of pack {seed}");
        }
    }
}
