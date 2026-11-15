//! A camel husk never breeds, an unsaddled camel sits down under its rider,
//! and a camel is tempted by cactus.
use super::*;
use crate::mobs::MobType;
use crate::mobs::ai::{feed_animal, takes_food};
use crate::mobs::world::with_test_world;
use horizon_core::random::LegacyRandom;
use std::sync::Arc;

/// `CamelHusk.canMate` and `CamelHusk.canFallInLove` are both false, and
/// `AnimalMakeLove` only ever pairs a `Camel` with a `Camel`: two husks fed
/// their food never walk off together, and never spawn a child.
#[test]
#[ignore = "daily"]
fn a_camel_husk_never_breeds() {
    let mut mobs = None;
    let ran = with_test_world(&[], |_, types| {
        let husk = Arc::new(types.types["minecraft:camel_husk"].clone());
        let camel = Arc::new(types.types["minecraft:camel"].clone());
        let make = |id: u32, kind: Arc<MobType>, x: f64, in_love: bool| {
            let mut m = Mob::new(id, kind, Vec3::new(x, 64.0, 0.5), 0.0);
            register(&mut m);
            if in_love {
                m.extra.in_love = 600;
            }
            m.on_ground = true;
            m
        };
        mobs = Some((
            make(1, husk.clone(), 0.5, true),
            make(2, husk.clone(), 2.5, true),
            make(3, camel.clone(), 4.5, true),
            make(4, camel, 6.5, true),
            make(5, husk, 0.5, false),
        ));
    });
    assert!(ran, "vanilla data is missing");
    let Some((mut husk, husk_partner, mut camel, camel_partner, mut fed)) = mobs else {
        return;
    };
    let seen = [
        husk_partner.seen(None, &crate::items::Items::empty()),
        camel_partner.seen(None, &crate::items::Items::empty()),
    ];
    let ran = with_test_world(&seen, |w, _| {
        let mut lr = crate::health::TestLevel::new(0);
        let mut events = Vec::new();
        let husk_breeds = {
            let mut cx = Cx {
                w,
                level: &mut lr.level(),
                events: &mut events,
            };
            Breed::new(1.0).can_use(&mut husk, &mut cx)
        };
        assert!(!husk_breeds, "a camel husk has no partner to breed with");
        assert!(events.is_empty());
        // The control: a camel in love still finds the other camel.
        let camel_breeds = {
            let mut cx = Cx {
                w,
                level: &mut lr.level(),
                events: &mut events,
            };
            Breed::new(1.0).can_use(&mut camel, &mut cx)
        };
        assert!(camel_breeds, "a camel in love still breeds");
    });
    assert!(ran, "vanilla data is missing");
    // `Animal.mobInteract` asks `canFallInLove` before it sets love mode, so a
    // husk fed its food never even starts (its `canBeABaby` is false too).
    let mut level = LegacyRandom::new(0);
    assert!(!takes_food(&fed), "a husk never falls in love over food");
    assert!(feed_animal(&mut fed, &mut level, None).is_none());
    assert_eq!(fed.extra.in_love, 0);
}

/// `CamelAi.RandomSitting` kneels only without a *controlling* passenger
/// (`Camel.hasControllingPassenger`, which `AbstractHorse.getControllingPassenger`
/// answers for a player only when the camel is saddled): a player on an
/// unsaddled camel is carried down, a saddled one drives.
#[test]
#[ignore = "daily"]
fn an_unsaddled_camel_sits_down_under_its_rider() {
    let sits = |saddled: bool, rider: bool| {
        let mut camel = None;
        assert!(
            with_test_world(&[], |_, types| {
                let kind = Arc::new(types.types["minecraft:camel"].clone());
                let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                m.extra.set_saddled(saddled);
                if rider {
                    m.rider = Some(crate::mobs::equine::Rider::default());
                }
                camel = Some(m);
            }),
            "vanilla data is missing"
        );
        let mut m = camel?;
        m.age = 1000;
        m.extra.egg_time = 0;
        m.on_ground = true;
        let mut started = false;
        assert!(
            with_test_world(&[], |w, _| {
                let mut lr = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let mut cx = Cx {
                    w,
                    level: &mut lr.level(),
                    events: &mut events,
                };
                started = (0..64).any(|_| RandomSitting.can_use(&mut m, &mut cx));
            }),
            "vanilla data is missing"
        );
        Some(started)
    };
    let Some(alone) = sits(false, false) else {
        return;
    };
    assert!(alone, "a camel with nobody on it sits down");
    assert_eq!(
        sits(false, true),
        Some(true),
        "a player on an unsaddled camel does not stop it kneeling"
    );
    assert_eq!(
        sits(true, true),
        Some(false),
        "a saddled camel under its driver never kneels"
    );
}

/// `Camel.isFood` is `#minecraft:camel_food` (a cactus) and `CamelHusk.isFood`
/// is `#minecraft:camel_husk_food` (a rabbit's foot). `FollowTemptation` reads
/// the same tag, so a cactus tempts a camel.
#[test]
#[ignore = "daily"]
fn a_camel_is_tempted_by_cactus() {
    let mut got = None;
    let ran = with_test_world(&[], |_, types| {
        got = Some((
            types.types["minecraft:camel"].food.to_vec(),
            types.types["minecraft:camel"].tempt.to_vec(),
            types.types["minecraft:camel_husk"].tempt.to_vec(),
        ));
    });
    assert!(ran, "vanilla data is missing");
    let Some((camel_food, camel_tempt, husk_tempt)) = got else {
        return;
    };
    let has = |list: &[String], item: &str| list.iter().any(|i| i == item);
    assert!(
        has(&camel_food, "minecraft:cactus"),
        "a cactus is camel food, got {camel_food:?}"
    );
    assert!(
        has(&camel_tempt, "minecraft:cactus"),
        "a cactus tempts a camel, got {camel_tempt:?}"
    );
    assert!(
        has(&husk_tempt, "minecraft:rabbit_foot"),
        "a rabbit's foot tempts a camel husk, got {husk_tempt:?}"
    );
}
