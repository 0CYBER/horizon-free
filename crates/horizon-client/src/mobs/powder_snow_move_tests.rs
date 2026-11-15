use super::{Mob, MobTypes};
use crate::mobs::world::with_built_test_world;
use horizon_core::geometry::Vec3;
use std::sync::Arc;

fn fixture(block: &str, f: impl FnOnce(&super::MobWorld<'_>, &MobTypes)) {
    assert!(
        with_built_test_world(
            &[],
            |world, registry| {
                let stone = registry.default_state("minecraft:stone").unwrap();
                for cx in -1..1 {
                    for cz in -1..1 {
                        world
                            .chunks
                            .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                    }
                }
                for x in -4..4 {
                    for z in -4..4 {
                        world.set(x, 63, z, stone).unwrap();
                    }
                }
                world
                    .set(0, 64, 0, registry.default_state(block).unwrap())
                    .unwrap();
            },
            |w, types| f(w, types)
        ),
        "vanilla test resources required"
    );
}

fn mob(types: &MobTypes, id: &str, y: f64) -> Mob {
    let mut mob = Mob::new(
        1,
        Arc::new(types.types[id].clone()),
        Vec3::new(0.5, y, 0.5),
        0.0,
    );
    super::ai::register(&mut mob);
    mob
}

#[test]
#[ignore = "daily"]
fn powder_snow_move_standing_and_sinking() {
    fixture("minecraft:powder_snow", |w, types| {
        for (id, boots, stands) in [
            ("minecraft:fox", false, true),
            ("minecraft:rabbit", false, true),
            ("minecraft:zombie", true, true),
            ("minecraft:cow", false, false),
            ("minecraft:zombie", false, false),
        ] {
            let mut mob = mob(types, id, 65.0);
            if boots {
                mob.extra.set_slot_stack(
                    horizon_items::equipment::FEET,
                    Some(crate::items::ItemStack::new("minecraft:leather_boots", 1)),
                );
            }
            let mut test_level = crate::health::TestLevel::new(42);
            let mut events = Vec::new();
            for _ in 0..20 {
                mob.travel(w, &mut test_level.level(), Vec3::ZERO, &mut events);
            }
            if stands {
                assert_eq!(mob.position.y, 65.0, "{id}, boots={boots}");
            } else {
                assert!(mob.position.y < 65.0, "{id}: {}", mob.position.y);
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn powder_snow_move_landing() {
    fixture("minecraft:powder_snow", |w, types| {
        let mut mob = mob(types, "minecraft:pig", 70.0);
        let mut test_level = crate::health::TestLevel::new(42);
        let mut events = Vec::new();
        for _ in 0..40 {
            mob.travel(w, &mut test_level.level(), Vec3::ZERO, &mut events);
            if mob.on_ground {
                assert_eq!(mob.position.y, 64.0 + f64::from(0.9f32));
                return;
            }
        }
        panic!("pig never landed");
    });
}

/// Vanilla 26.3 returns SHAPE_STABLE above scaffolding, including while holding it.
#[test]
#[ignore = "daily"]
fn powder_snow_move_main_hand_scaffolding() {
    fixture("minecraft:scaffolding", |w, types| {
        let mut mob = mob(types, "minecraft:zombie", 65.0);
        mob.extra
            .set_main_hand(Some(Arc::from("minecraft:scaffolding")));
        let mut test_level = crate::health::TestLevel::new(42);
        let mut events = Vec::new();
        for _ in 0..20 {
            mob.travel(w, &mut test_level.level(), Vec3::ZERO, &mut events);
        }
        assert_eq!(mob.position.y, 65.0);
        assert!(
            mob.collision_subject()
                .main_hand
                .unwrap()
                .is_item("minecraft:scaffolding")
        );
    });
}
