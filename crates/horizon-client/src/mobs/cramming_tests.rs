//! `LivingEntity.pushEntities`'s cramming damage counts
//! `Level.getPushableEntities` (`EntitySelector.pushableBy`), which asks
//! `isPushable`: `AbstractHorse.isPushable` is false while the horse carries
//! a passenger, so a ridden horse is not in the list and cannot fill the
//! crowd a mob is crushed by, where a horse nobody rides can.
use super::*;
use crate::mobs::Seen;
use crate::mobs::world::with_test_world;
use horizon_core::random::LegacyRandom;

#[test]
#[ignore = "daily"]
fn a_ridden_horse_is_not_counted_for_cramming() {
    // Three counted entities are one more than `maxEntityCramming - 1`.
    let ran = with_test_world(&[], |w, types| {
        let base = w.clone();
        let mob = |id: u32, kind: &str, at: Vec3| {
            let mut m = Mob::new(id, std::sync::Arc::new(types.types[kind].clone()), at, 0.0);
            m.no_ai = true;
            m.no_gravity = true;
            m
        };
        let pig = |id: u32, x: f64| mob(id, "minecraft:pig", Vec3::new(x, 1.0, 0.5));
        let horse = |ridden: bool| {
            let mut horse = mob(50, "minecraft:horse", Vec3::new(0.55, 1.0, 0.5));
            if ridden {
                horse.rider = Some(super::super::equine::Rider::default());
            }
            horse
        };
        // Java `new Random(1234567).nextInt(4)` is 0: the quarter that hurts.
        let mut seed = LegacyRandom::new(1234567);
        assert_eq!(seed.next_int_bound(4).unwrap(), 0);
        for (name, crowd, hurt) in [
            ("two pigs", vec![pig(1, 0.6), pig(2, 0.7)], false),
            (
                "three pigs",
                vec![pig(1, 0.6), pig(2, 0.7), pig(3, 0.8)],
                true,
            ),
            (
                "two pigs and a ridden horse",
                vec![pig(1, 0.6), pig(2, 0.7), horse(true)],
                false,
            ),
            (
                "two pigs and an empty horse",
                vec![pig(1, 0.6), pig(2, 0.7), horse(false)],
                true,
            ),
        ] {
            let seen: Vec<Seen> = crowd
                .iter()
                .map(|o| o.seen(None, &crate::items::Items::empty()))
                .collect();
            let w = MobWorld {
                others: &seen,
                max_entity_cramming: 3,
                ..base.clone()
            };
            let mut m = pig(99, 0.5);
            m.random = horizon_core::random::EntityRandom::own(LegacyRandom::new(1234567));
            let full = m.health;
            let mut events = Vec::new();
            push_entities(
                &mut m,
                &w,
                &mut crate::health::TestLevel::new(0).level(),
                &mut events,
            );
            assert_eq!(
                m.last_damage.map(|(d, _)| d.kind),
                hurt.then_some(DamageKind::Cramming),
                "{name}"
            );
            assert_eq!(
                m.health,
                if hurt { full - 6.0 } else { full },
                "{name}: cramming damage"
            );
        }
    });
    // Without the vanilla data under .local/horizon-client there is no world
    // to push in: the test has nothing to check.
    assert!(ran, "vanilla data is missing");
}
