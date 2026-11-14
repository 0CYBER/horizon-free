//! `Ghast.explosionPower`: the save's `ExplosionPower` byte is the power the
//! ghast hands its `LargeFireball`, whose blast uses it (`LargeFireball`'s
//! constructor and `onHit`), and `addAdditionalSaveData` writes it back.
use super::super::world::{test_player, with_built_test_world};
use super::*;
use crate::mobs::MobWorld;
use crate::projectiles::ProjectileKind;
use horizon_core::random::LegacyRandom;
use horizon_world::nbt::Tag;
use std::sync::Arc;

fn ghast_tag(power: Option<Tag>) -> Tag {
    let mut tag = Tag::compound([
        ("id", Tag::String("minecraft:ghast".into())),
        (
            "Pos",
            Tag::List(vec![
                Tag::Double(8.5_f64.to_bits()),
                Tag::Double(100.0_f64.to_bits()),
                Tag::Double(8.5_f64.to_bits()),
            ]),
        ),
    ]);
    if let (Some(power), Tag::Compound(c)) = (power, &mut tag) {
        c.insert("ExplosionPower".into(), power);
    }
    tag
}

/// The power of the one large fireball `goal` shot over 20 ticks, as
/// `Ghast.GhastShootFireballGoal` counts its charge to 20.
fn shot_power(m: &mut Mob, goal: &mut GhastShoot, world: &MobWorld<'_>) -> i32 {
    let (mut lr, mut events) = (LegacyRandom::new(0), Vec::new());
    for _ in 0..20 {
        let mut cx = Cx {
            w: world,
            lr: &mut lr,
            events: &mut events,
        };
        goal.tick(m, &mut cx);
    }
    events
        .iter()
        .find_map(|e| match e {
            MobEvent::Projectile(p) => match p.kind {
                ProjectileKind::LargeFireball { power } => Some(power),
                _ => None,
            },
            _ => None,
        })
        .expect("the ghast shot a large fireball")
}

/// `Ghast.readAdditionalSaveData` reads `ExplosionPower` with `getByteOr(.., 1)`,
/// the goal gives it to the `LargeFireball` it spawns, and
/// `Ghast.addAdditionalSaveData` saves it back.
#[test]
fn ghast_explosion_power_sizes_the_fireball_it_shoots() {
    let ran = with_built_test_world(
        &[],
        |world, _| {
            for cx in 0..2 {
                for cz in 0..2 {
                    world.chunks.insert(
                        (cx, cz),
                        std::sync::Arc::new(crate::chunk::Chunk::empty(cx, cz)),
                    );
                }
            }
        },
        |w, types| {
            let mut world = w.clone();
            world.player = Some(test_player(Vec3::new(8.5, 100.0, 16.5)));
            let types: std::collections::HashMap<_, _> = types
                .types
                .iter()
                .map(|(id, ty)| (id.clone(), Arc::new(ty.clone())))
                .collect();
            for (saved, expected) in [
                (None, 1),
                (Some(Tag::Byte(6)), 6),
                (Some(Tag::Byte(0)), 0),
                // getByteOr takes any numeric tag, narrowed to a byte.
                (Some(Tag::Long(300)), 44),
            ] {
                let tag = ghast_tag(saved.clone());
                let mut m = crate::entity_nbt::mob_from_nbt(&tag, &types, 7, None)
                    .expect("the ghast loads");
                m.ai.target = Some(Who::Player);
                let mut goal = GhastShoot { charge: 0 };
                assert_eq!(
                    shot_power(&mut m, &mut goal, &world),
                    expected,
                    "ExplosionPower={saved:?}"
                );
                let back = crate::entity_nbt::mob_to_nbt(&m, [0; 4], Some(&tag), None, |_| None);
                assert_eq!(
                    back.get("ExplosionPower"),
                    Some(&Tag::Byte(expected as i8)),
                    "ExplosionPower={saved:?}"
                );
            }
            // A ghast that never loaded a save (a natural spawn) starts from
            // vanilla's field initialiser, not from a zeroed scratch slot.
            let mut fresh = Mob::new(
                8,
                types["minecraft:ghast"].clone(),
                Vec3::new(8.5, 100.0, 8.5),
                0.0,
            );
            fresh.ai.target = Some(Who::Player);
            let mut goal = GhastShoot { charge: 0 };
            assert_eq!(
                shot_power(&mut fresh, &mut goal, &world),
                1,
                "a fresh ghast"
            );
        },
    );
    let _ = ran;
}
