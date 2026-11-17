use super::*;
use mod_api::ServerHost;

use horizon_client::mobs::{
    BreedExperience,
    finalize::{Attr, set_base},
    goals::{Breed, Cx, Goal},
};
use horizon_core::random::{LegacyRandom, RandomSource};

/// Isolate the goal's stream from earlier tick work, but leave the actual
/// Breed, subsequent aiStep and queued Game event handling in the route.
#[derive(Clone, Debug)]
struct SeededBreed {
    goal: Breed,
    seed: i64,
}
impl Goal for SeededBreed {
    fn box_clone(&self) -> Box<dyn Goal> {
        Box::new(self.clone())
    }
    fn flags(&self) -> u8 {
        self.goal.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.goal.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.goal.can_continue(m, cx)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.random = horizon_core::random::EntityRandom::own(LegacyRandom::new(self.seed));
        self.goal.tick(m, cx);
    }
}
struct Bred {
    child: Mob,
    next_random: i32,
    experience: BreedExperience,
}
fn tick_breeding(
    game: &mut Game<'_>,
    parent_kind: &str,
    partner_kind: &str,
    seed: i64,
    mob_drops: bool,
) -> Bred {
    game.mobs.clear();
    game.ui.rules.set(
        "mob_drops",
        horizon_client::gamerules::Value::Bool(mob_drops),
    );
    let base = game.next_entity;
    game.next_entity += 2;
    let mut parent = Mob::new(
        base + 1,
        game.res.mob_types().get(parent_kind).unwrap().clone(),
        Vec3::new(0.0, 64.0, 0.0),
        0.0,
    );
    let mut partner = Mob::new(
        base + 2,
        game.res.mob_types().get(partner_kind).unwrap().clone(),
        Vec3::new(1.0, 64.0, 0.0),
        0.0,
    );
    parent.persistent = true;
    parent.extra.in_love = 600;
    partner.extra.in_love = 600;
    parent.extra.tame = true;
    partner.extra.tame = true;
    parent.extra.variant = 1;
    partner.extra.variant = 2;
    parent.extra.variant2 = 1;
    partner.extra.variant2 = if horizon_client::mobs::llama::is_llama(partner.species()) {
        4
    } else {
        2
    };
    set_base(&mut parent, Attr::MaxHealth, 20.0);
    set_base(&mut partner, Attr::MaxHealth, 28.0);
    set_base(&mut parent, Attr::MovementSpeed, 0.2);
    set_base(&mut partner, Attr::MovementSpeed, 0.3);
    parent.health = 20.0;
    partner.health = 28.0;
    parent.extra.vector[0] = 0.5;
    partner.extra.vector[0] = 0.9;
    // The camel's client animation delay must not add an unrelated draw.
    parent.extra.ints[0] = 1000;
    parent.ai.targets.clear();
    parent.ai.goals.clear();
    parent.ai.goals.add(
        2,
        SeededBreed {
            goal: Breed::new(1.0),
            seed,
        },
    );
    let others = [partner.seen(None, game.items())];
    let mut events = Vec::new();
    let mut test_level = horizon_client::health::TestLevel::new(321);
    let mut experience = None;
    {
        let world = MobWorld {
            others: &others,
            mob_drops,
            ..game.mob_world_base(&[])
        };
        assert!(!world.asks_baby_spawn);
        for _ in 0..200 {
            // No floor in this fixture: keep the pair in breeding range.
            parent.position = Vec3::new(0.0, 64.0, 0.0);
            parent.tick(&world, &mut test_level.level(), &mut events);
            experience = events.iter().find_map(|event| match event {
                MobEvent::Breed { experience, .. } => Some(*experience),
                _ => None,
            });
            if experience.is_some() {
                break;
            }
            events.clear();
        }
    }
    let experience = experience.expect("the goal must breed");
    let tick_random = parent.random.state();
    game.mobs.extend([parent, partner]);
    game.handle_mob_events(events).unwrap();
    let parent = game.mobs.iter_mut().find(|m| m.id == base + 1).unwrap();
    assert_eq!(
        parent.random.state(),
        tick_random,
        "Game must not redraw inheritance"
    );
    let next_random = parent.random.next_int();
    let child = game.mobs.iter().find(|m| m.extra.age < 0).unwrap().clone();
    Bred {
        child,
        next_random,
        experience,
    }
}

#[test]
#[ignore = "daily"]
fn equine_breeding_inheritance_precedes_same_tick_healing_and_grazing() {
    super::super::native_entity_refs::tests::with_game(|game| {
        // java.util.Random(42), Horse/Donkey/Llama.getBreedOffspring,
        // then XP, then AbstractHorse.aiStep's nextInt(900)/nextInt(300)
        // (llamas only heal). The attributes use stocks (20,.5,.2) and (28,.9,.3).
        for (parent, partner) in [
            ("minecraft:horse", "minecraft:horse"),
            ("minecraft:donkey", "minecraft:donkey"),
            ("minecraft:horse", "minecraft:donkey"),
            ("minecraft:donkey", "minecraft:horse"),
            ("minecraft:llama", "minecraft:llama"),
            ("minecraft:trader_llama", "minecraft:llama"),
        ] {
            let result = tick_breeding(game, parent, partner, 42, true);
            let horse = parent == "minecraft:horse" && partner == parent;
            let llama = horizon_client::mobs::llama::is_llama(result.child.species());
            assert_eq!(
                result.child.extra.variant,
                if horse {
                    5
                } else if llama {
                    1
                } else {
                    0
                }
            );
            assert_eq!(
                result.child.extra.variant2,
                if horse {
                    2
                } else if llama {
                    4
                } else {
                    0
                }
            );
            let expected = if horse {
                [24.8238468170166, 0.5886834847817017, 0.3030804447968077]
            } else {
                [24.91461181640625, 0.7668932765979051, 0.22812363785863735]
            };
            let actual = [
                f64::from(result.child.attrs.max_health),
                result.child.extra.vector[0],
                result.child.attrs.speed,
            ];
            for (actual, expected) in actual.into_iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 1e-12,
                    "{parent}/{partner}: {actual} != {expected}"
                );
            }
            assert_eq!(
                result.experience,
                BreedExperience::Drawn(Some(if horse || llama { 2 } else { 1 }))
            );
            assert_eq!(
                result.next_random,
                if horse {
                    -1_074_144_810
                } else if llama {
                    1_888_925_827
                } else {
                    652_655_727
                },
                "{parent}/{partner}"
            );
            if parent != partner && !llama {
                assert_eq!(result.child.species(), Species::Mule);
            }
            if parent == "minecraft:trader_llama" {
                assert!(result.child.persistent);
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn wolf_breeding_selects_either_coat_from_the_parent_stream() {
    super::super::native_entity_refs::tests::with_game(|game| {
        // Computed with an independent implementation of the documented java.util.Random
        // algorithm, not from the game's output.
        // Java LCG: s0 = (seed ^ 0x5DEECE66D) & (2^48 - 1),
        // s' = (25214903917*s + 11) mod 2^48. Draw 1 selects the coat;
        // draw 2 selects one of 7 voices; draw 3 yields 1 + (s >> 17) % 7 XP;
        // draw 4 is signed(s >> 16). Neither bounded draw rejects these seeds.
        for (seed, coat, xp, next) in [(42, 1, 7, 205_897_768), (4096, 2, 3, 1_387_017_903)] {
            let result = tick_breeding(game, "minecraft:wolf", "minecraft:wolf", seed, true);
            assert_eq!(result.child.extra.variant, coat);
            assert_eq!(result.experience, BreedExperience::Drawn(Some(xp)));
            assert_eq!(result.next_random, next);
        }
    });
}

#[test]
#[ignore = "daily"]
fn camel_breeding_leaves_the_parent_stream_unchanged() {
    super::super::native_entity_refs::tests::with_game(|game| {
        let kind = "minecraft:camel";
        let result = tick_breeding(game, kind, kind, 42, false);
        assert_eq!(result.next_random, -1_170_105_035); // Random(42).nextInt(), no inheritance.
        assert_eq!(result.child.attrs.max_health, result.child.kind.max_health);
        assert_eq!(result.child.attrs.speed, result.child.kind.speed);
    });
}

#[test]
#[ignore = "daily"]
fn breeding_announces_the_orb_before_the_lower_id_child() {
    use mod_api::{self, BabyEntitySpawnEvent, EntityJoinLevelEvent, Flow, Hooks};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    struct Observer {
        joins: Arc<Mutex<Vec<(u32, String)>>>,
        cancel: Arc<AtomicBool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn baby_entity_spawn(
            &self,
            _: &mut dyn ServerHost,
            _: &mut BabyEntitySpawnEvent<'_>,
        ) -> Flow {
            if self.cancel.load(Ordering::SeqCst) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn entity_join_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            self.joins
                .lock()
                .unwrap()
                .push((event.entity, event.kind.into()));
            Flow::Continue
        }
    }
    let joins = Arc::new(Mutex::new(Vec::new()));
    let cancel = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "breeding-observer",
        Box::new(Observer {
            joins: joins.clone(),
            cancel: cancel.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            assert!(game.mob_world_base(&[]).asks_baby_spawn);
            let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            let base = game.next_entity;
            game.next_entity += 2;
            game.mobs.push(Mob::new(
                base + 1,
                cow.clone(),
                Vec3::new(0.0, 64.0, 0.0),
                0.0,
            ));
            game.mobs
                .push(Mob::new(base + 2, cow, Vec3::new(1.0, 64.0, 0.0), 0.0));
            game.native_joins_skip();
            let parent = game.mobs.iter_mut().find(|mob| mob.id == base + 1).unwrap();
            parent.random = horizon_core::random::EntityRandom::own(horizon_core::random::LegacyRandom::new(0));
            parent.extra.variant = 1;
            parent.extra.age = 6000;
            let child = game.breed(
                base + 1,
                base + 2,
                None,
                None,
                None,
                horizon_client::mobs::BreedExperience::Deferred,
            );
            assert_eq!(child, Some(base + 3));
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == base + 3)
                    .unwrap()
                    .extra
                    .age,
                -24000
            );
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == base + 1)
                    .unwrap()
                    .extra
                    .age,
                6000
            );
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == base + 2)
                    .unwrap()
                    .extra
                    .age,
                6000
            );
            assert_eq!(
                *joins.lock().unwrap(),
                vec![
                    (base + 4, "minecraft:experience_orb".into()),
                    (base + 3, "minecraft:cow".into())
                ]
            );
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == base + 3)
                    .unwrap()
                    .extra
                    .variant,
                1
            );
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == base + 1)
                    .unwrap()
                    .random
                    .state(),
                234_029_835_660_143
            );
            joins.lock().unwrap().clear();
            let fox = game.res.mob_types().get("minecraft:fox").unwrap().clone();
            let fox_base = game.next_entity;
            game.next_entity += 2;
            game.mobs.push(Mob::new(
                fox_base + 1,
                fox.clone(),
                Vec3::new(0.0, 64.0, 0.0),
                0.0,
            ));
            game.mobs
                .push(Mob::new(fox_base + 2, fox, Vec3::new(1.0, 64.0, 0.0), 0.0));
            game.native_joins_skip();
            game.player = Some(
                horizon_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap(),
            );
            let feeder = horizon_client::lan::playerdata::uuid_ints(game.native_acting_id());
            game.love_cause.insert(fox_base + 1, None);
            let fox_child = game.breed(
                fox_base + 1,
                fox_base + 2,
                None,
                None,
                None,
                horizon_client::mobs::BreedExperience::Deferred,
            );
            assert_eq!(fox_child, Some(fox_base + 3));
            let fox_child = game.mobs.iter().find(|mob| mob.id == fox_base + 3).unwrap();
            assert!(horizon_client::mobs::fox::trusts(fox_child, feeder));
            assert_eq!(
                *joins.lock().unwrap(),
                vec![
                    (fox_base + 3, "minecraft:fox".into()),
                    (fox_base + 4, "minecraft:experience_orb".into())
                ]
            );
            joins.lock().unwrap().clear();
            let next = game.next_entity;
            let parent = game.mobs.iter_mut().find(|mob| mob.id == base + 1).unwrap();
            let mut expected = parent.random.clone();
            let choice = parent.random.next_boolean();
            assert_eq!(choice, expected.next_boolean());
            let random = expected.state();
            cancel.store(true, Ordering::SeqCst);
            let child = game.breed(
                base + 1,
                base + 2,
                None,
                Some(choice),
                None,
                horizon_client::mobs::BreedExperience::Deferred,
            );
            assert_eq!(child, None);
            assert_eq!(game.next_entity, next);
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == base + 1)
                    .unwrap()
                    .random
                    .state(),
                random
            );
            assert!(joins.lock().unwrap().is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn first_chicken_breed_matches_without_mods_and_with_an_empty_mod() {
    use horizon_client::mobs::{BreedExperience, MobEvent, goals::Breed};
    use horizon_core::random::LegacyRandom;
    use mod_api::{self, Hooks};
    struct Empty;
    #[mod_api::__subscriptions]
    impl Hooks for Empty {}
    super::super::native_entity_refs::tests::with_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:chicken")
            .unwrap()
            .clone();
        let mut mate = Mob::new(2, kind.clone(), Vec3::new(0.5, 64.0, 0.0), 0.0);
        mate.extra.in_love = 600;
        let others = [mate.seen(None, game.items())];
        let run = || {
            let world = MobWorld {
                others: &others,
                ..game.mob_world_base(&[])
            };
            assert!(!world.asks_baby_spawn);
            let mut parent = Mob::new(1, kind.clone(), Vec3::new(0.0, 64.0, 0.0), 0.0);
            parent.random = horizon_core::random::EntityRandom::own(LegacyRandom::new(123));
            parent.persistent = true;
            parent.extra.in_love = 600;
            parent.extra.egg_time = 10_000;
            parent.ai.targets.clear();
            parent.ai.goals.clear();
            parent.ai.goals.add(2, Breed::new(1.0));
            let mut events = Vec::new();
            let mut test_level = horizon_client::health::TestLevel::new(321);
            let level = &mut test_level.level();
            for _ in 0..200 {
                // The fixture has no floor; keep the pair within breeding range.
                parent.position = Vec3::new(0.0, 64.0, 0.0);
                parent.tick(&world, level, &mut events);
                if let Some(xp) = events.iter().find_map(|event| match event {
                    MobEvent::Breed {
                        experience: BreedExperience::Drawn(Some(xp)),
                        ..
                    } => Some(*xp),
                    _ => None,
                }) {
                    return (parent.random.state(), xp);
                }
                events.clear();
            }
            panic!("chickens did not breed");
        };
        let no_mods = mod_platform::testing::with_mods(Vec::new().into(), run);
        let empty = vec![mod_api::ModEntry::new("empty", Box::new(Empty))].into();
        let unrelated = mod_platform::testing::with_mods(empty, run);
        assert_eq!(no_mods, unrelated);
    });
}

#[test]
#[ignore = "daily"]
fn empty_mod_never_defers_breeding_xp() {
    use mod_api::{self, Hooks};
    struct Empty;
    #[mod_api::__subscriptions]
    impl Hooks for Empty {}
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("empty", Box::new(Empty))].into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            assert!(!game.mob_world_base(&[]).asks_baby_spawn);
            let mut cancel = false;
            game.native_baby_entity_spawn(1, 2, None, None, "minecraft:chicken", &mut cancel);
            assert!(!cancel);
            assert!(!game.mob_world_base(&[]).asks_baby_spawn);
        });
    });
}

#[test]
fn the_player_who_fed_either_animal_is_credited() {
    let mut love = std::collections::HashMap::from([(1, Some(7)), (2, Some(9))]);
    // The parent's feeder first, and both animals are spent.
    assert_eq!(breeding_credit(&mut love, 1, 2), Some(Some(7)));
    assert!(love.is_empty());
    // Only the partner was fed: still credited.
    let mut love = std::collections::HashMap::from([(2, None)]);
    assert_eq!(breeding_credit(&mut love, 1, 2), Some(None));
    // Neither was fed by a player (a bred pair loaded from a save).
    assert_eq!(breeding_credit(&mut love, 1, 2), None);
}
