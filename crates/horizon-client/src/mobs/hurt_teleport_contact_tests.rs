use super::*;
use crate::mobs::world::with_built_test_world;
use horizon_world::registry::BlockRegistry;
use std::sync::Arc;

/// A stone floor over the loaded 2x2 chunks (so a teleport finds ground),
/// `under` below the mob's spot and `at` in it.
fn build(
    under: &'static str,
    at: &'static str,
) -> impl FnOnce(&mut crate::world::World, &BlockRegistry) {
    move |world, registry| {
        let stone = registry.default_state("minecraft:stone").unwrap();
        for cx in -1..1 {
            for cz in -1..1 {
                world
                    .chunks
                    .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
            }
        }
        for x in -16..16 {
            for z in -16..16 {
                world.set(x, 63, z, stone).unwrap();
            }
        }
        world
            .set(0, 63, 0, registry.default_state(under).unwrap())
            .unwrap();
        world
            .set(0, 64, 0, registry.default_state(at).unwrap())
            .unwrap();
    }
}

fn enderman(types: &crate::mobs::MobTypes, seed: i64) -> Mob {
    let kind = Arc::new(types.types["minecraft:enderman"].clone());
    let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    register(&mut m);
    m.random =
        horizon_core::random::EntityRandom::own(horizon_core::random::LegacyRandom::new(seed));
    m.on_ground = true;
    m.stuck_multiplier = Vec3::ZERO;
    m
}

/// `Entity.applyEffectsFromBlocks` fixes the tick's movements before
/// `stepOn`: when the magma's hurt teleports an enderman, the scan still
/// walks the path it had, and the cobweb it stood in still holds it.
#[test]
#[ignore = "daily"]
fn a_magma_teleport_leaves_the_block_scan_on_the_old_path() {
    assert!(
        with_built_test_world(
            &[],
            build("minecraft:magma_block", "minecraft:cobweb"),
            |w, types| {
                let mut teleported = 0;
                for seed in 0..40 {
                    let mut m = enderman(types, seed);
                    let mut test_level = crate::health::TestLevel::new(42);
                    let mut random = test_level.level();
                    let mut events = Vec::new();
                    effects_from_blocks(&mut m, w, &mut random, &mut events);
                    if m.position != Vec3::new(0.5, 64.0, 0.5) {
                        teleported += 1;
                        assert_eq!(
                            m.stuck_multiplier,
                            Vec3::new(0.25, f64::from(0.05_f32), 0.25),
                            "seed {seed}: the cobweb on the old path"
                        );
                    }
                }
                assert!(teleported > 0, "some seed teleports");
            },
        ),
        "vanilla data is missing"
    );
}

/// A hurt in `baseTick` that teleports an enderman (`snapTo` resets the
/// old position) leaves aiStep's fire contact at the landing spot: no
/// fire there, so the fire timer only counts down (no re-ignite to 160).
#[test]
#[ignore = "daily"]
fn a_burning_teleport_does_not_take_the_old_spots_fire_along() {
    assert!(
        with_built_test_world(
            &[],
            build("minecraft:stone", "minecraft:fire"),
            |w, types| {
                let mut teleported = 0;
                for seed in 0..40 {
                    let mut m = enderman(types, seed);
                    m.fire_ticks = 20;
                    // Whether the burning hurt's own teleport lands (one in ten
                    // draws none, and a try can find no ground: the old spot's
                    // fire then ignites it before the second hurt teleports it).
                    let mut first = m.clone();
                    let mut events = Vec::new();
                    hurt(
                        &mut first,
                        w,
                        &mut events,
                        1.0,
                        Damage::of(DamageKind::OnFire),
                    );
                    base_tick(
                        &mut m,
                        w,
                        &mut crate::health::TestLevel::new(0).level(),
                        &mut events,
                    );
                    if first.position != Vec3::new(0.5, 64.0, 0.5) {
                        assert_eq!(m.position, first.position, "seed {seed}: the same teleport");
                        teleported += 1;
                        assert_eq!(m.fire_ticks, 19, "seed {seed}: no fire where it landed");
                    }
                }
                assert!(teleported > 0, "some seed teleports");
            },
        ),
        "vanilla data is missing"
    );
}
