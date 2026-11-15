use super::*;
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct MovementPanic {
    id: u32,
    remaining: AtomicUsize,
    calls: AtomicUsize,
}
struct Hooks(Arc<MovementPanic>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Hooks {
    fn living_speed(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::LivingSpeedEvent<'_>,
    ) {
        if event.entity == self.0.id
            && self
                .0
                .remaining
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
        {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(89_u32);
        }
    }
}
fn prepare(game: &mut Game<'_>) {
    game.world_root = PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
        .join(format!("retained-{}", std::process::id()));
    game.options.dedicated = Some(Vec::new());
    game.load_server_loot();
    game.world.health_rules.difficulty = horizon_world::food::Difficulty::Normal;
    game.world.game_time = 1000;
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for x in 0..16 {
        for z in 0..16 {
            game.world.set(x, 79, z, stone).unwrap();
        }
    }
}

#[test]
#[ignore = "daily"]
fn same_mob_guest_attack_survives_movement_panic() {
    crate::entry::install_client_hooks();
    let probe = Arc::new(MovementPanic {
        id: 71,
        remaining: AtomicUsize::new(1),
        calls: AtomicUsize::new(0),
    });
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "same-mob",
            Box::new(Hooks(probe.clone())),
        )]
        .into(),
        || {
            crate::game::native_guest_turn_tests::with_guests(|game| {
                prepare(game);
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(12.0, 80.0, 12.0)).unwrap());
                let host = game.lan.as_mut().unwrap();
                for i in 0..host.hub.guests.len() {
                    host.hub.guests.update(i, |guest| {
                        guest.loaded = true;
                        guest.load_timeout = 0;
                        guest.position = if guest.conn == 12 {
                            [1.5, 80.0, 0.5]
                        } else {
                            [12.0, 80.0, 12.0]
                        };
                    });
                }
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut mob = Mob::new(71, kind, Vec3::new(1.0, 80.0, 0.5), 0.0);
                mob.on_ground = true;
                mob.ai.speed = 0.2;
                mob.ai.target = Some(horizon_client::mobs::Who::Player);
                game.mobs.push(mob);
                // Who::Player names the selected player; retain its guest binding
                // instead of targeting the faraway host for this attack.
                game.lan.as_mut().unwrap().mob_players.insert(71, 12);
                let host_before = game.player.as_ref().unwrap().health.health();
                let guest_before = game.lan.as_ref().unwrap().hub.guests[0].health;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.tick_creature_entities()
                }));
                assert_eq!(result.unwrap_err().downcast_ref::<u32>(), Some(&89));
                assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
                let pending = &game.creatures.pending[game.dim.id()];
                assert!(
                    pending.batches.iter().any(|batch| {
                        batch.normal.root.events.iter().any(|e| {
                            matches!(
                                e,
                                MobEvent::AttackPlayer(..)
                                    | MobEvent::AttackPlayerWithEffect { .. }
                            )
                        }) || !batch.guests.is_empty()
                    }),
                    "same mob must produce the real AI attack before movement panics"
                );
                game.mobs[0].no_ai = true;
                game.tick_creature_entities().unwrap();
                assert!(
                    game.lan.as_ref().unwrap().hub.guests[0].health < guest_before,
                    "same-mob retained attack must damage the guest"
                );
                assert_eq!(
                    game.player.as_ref().unwrap().health.health(),
                    host_before,
                    "guest attack must never damage the host"
                );
                let after = game.lan.as_ref().unwrap().hub.guests[0].health;
                game.as_guest(12, |game| {
                    game.player.as_mut().unwrap().health.invulnerable_time = 0
                })
                .unwrap();
                game.tick_creature_entities().unwrap();
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guests[0].health,
                    after,
                    "entered guest attack must not replay"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn repeated_producer_panic_has_bounded_batches() {
    crate::entry::install_client_hooks();
    let probe = Arc::new(MovementPanic {
        id: 9000,
        remaining: AtomicUsize::new(MAX_PENDING_MOB_PASSES + 1),
        calls: AtomicUsize::new(0),
    });
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "backlog",
            Box::new(Hooks(probe.clone())),
        )]
        .into(),
        || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                prepare(game);
                let chicken = game
                    .res
                    .mob_types()
                    .get("minecraft:chicken")
                    .unwrap()
                    .clone();
                for id in 1..=1024 {
                    let mut mob = Mob::new(
                        id,
                        chicken.clone(),
                        Vec3::new(
                            f64::from(id % 32) * 0.4 + 0.1,
                            80.0,
                            f64::from((id - 1) / 32) * 0.4 + 0.1,
                        ),
                        0.0,
                    );
                    mob.on_ground = true;
                    game.mobs.push(mob);
                }
                let zombie = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut mob = Mob::new(9000, zombie, Vec3::new(1.0, 80.0, 0.5), 0.0);
                mob.on_ground = true;
                mob.ai.speed = 0.2;
                game.mobs.push(mob);
                for _ in 0..MAX_PENDING_MOB_PASSES {
                    for mob in &mut game.mobs {
                        if mob.id != 9000 {
                            mob.extra.egg_time = 1;
                        } else {
                            mob.position = Vec3::new(1.0, 80.0, 0.5);
                            mob.on_ground = true;
                            mob.velocity = Vec3::ZERO;
                        }
                    }
                    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        game.tick_creature_entities()
                    }))
                    .unwrap_err();
                    assert_eq!(panic.downcast_ref::<u32>(), Some(&89));
                }
                let pending = &game.creatures.pending[game.dim.id()];
                assert_eq!(pending.batches.len(), MAX_PENDING_MOB_PASSES);
                let produced_eggs = pending
                    .batches
                    .iter()
                    .flat_map(|b| &b.normal.root.events)
                    .filter(|event| matches!(event, MobEvent::LayEgg { .. }))
                    .count() as u32;
                let events: usize = pending
                    .batches
                    .iter()
                    .map(|b| b.normal.root.events.len())
                    .sum();
                let storage: usize = pending
                    .batches
                    .iter()
                    .map(|b| b.normal.root.events.capacity() * std::mem::size_of::<MobEvent>())
                    .sum();
                eprintln!(
                    "BACKLOG_AT_CAP:chickens=1024:batches={}:events={events}:egg_obligations={produced_eggs}:shallow_event_capacity_bytes={storage}",
                    pending.batches.len()
                );
                assert!(
                    produced_eggs > 1024,
                    "stress must retain multiple real producer passes"
                );
                assert_eq!(probe.calls.load(Ordering::SeqCst), MAX_PENDING_MOB_PASSES);
                assert!(
                    game.dropped.is_empty(),
                    "consumer must not have entered before refusal"
                );
                // Exactly at the cap, refuse production even though another panic is armed.
                // Drain all previously accepted obligations, never clear or replace them.
                game.tick_creature_entities().unwrap();
                assert_eq!(
                    probe.calls.load(Ordering::SeqCst),
                    MAX_PENDING_MOB_PASSES,
                    "full backlog must refuse the next producer pass"
                );
                assert!(game.creatures.pending[game.dim.id()].batches.is_empty());
                let eggs = |game: &Game<'_>| -> u32 {
                    game.dropped
                        .iter()
                        .filter(|d| {
                            matches!(
                                d.stack.item.as_ref(),
                                "minecraft:egg" | "minecraft:brown_egg" | "minecraft:blue_egg"
                            )
                        })
                        .map(|d| d.stack.count)
                        .sum()
                };
                assert_eq!(
                    eggs(game),
                    produced_eggs,
                    "refusal must deliver every retained egg without loss"
                );
                probe.remaining.store(0, Ordering::SeqCst);
                for mob in &mut game.mobs {
                    mob.no_ai = true;
                }
                let fresh = game
                    .mobs
                    .iter_mut()
                    .find(|m| m.species() == Species::Chicken && m.alive())
                    .unwrap();
                // Earlier failed passes retain physical progress; put this one
                // healthy producer back in the loaded farm, not outside its chunks.
                fresh.position = Vec3::new(14.0, 80.0, 14.0);
                fresh.on_ground = true;
                fresh.velocity = Vec3::ZERO;
                fresh.no_ai = false;
                fresh.extra.egg_time = 1;
                game.tick_creature_entities().unwrap();
                assert_eq!(
                    eggs(game),
                    produced_eggs + 1,
                    "production must resume normally after backlog recovery"
                );
                game.tick_creature_entities().unwrap();
                assert_eq!(
                    eggs(game),
                    produced_eggs + 1,
                    "recovered work must not replay"
                );
            });
        },
    );
}

struct InterruptedConsumer {
    producer: Arc<MovementPanic>,
    items: Arc<AtomicUsize>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for InterruptedConsumer {
    fn living_speed(
        &self,
        context: &dyn mod_api::LoopContext,
        event: &mut mod_api::LivingSpeedEvent<'_>,
    ) {
        mod_api::Hooks::living_speed(&Hooks(self.producer.clone()), context, event);
    }
    fn item_spawn(&self, _: &mut dyn mod_api::ServerHost, _: &mut mod_api::ItemSpawnEvent<'_>) {
        let entered = self.items.fetch_add(1, Ordering::SeqCst) + 1;
        if entered == 2 || entered == 3 {
            std::panic::panic_any(90_u32);
        }
    }
}

#[test]
#[ignore = "daily"]
fn interrupted_refusal_stays_latched_until_empty() {
    crate::entry::install_client_hooks();
    let producer = Arc::new(MovementPanic {
        id: 9000,
        remaining: AtomicUsize::new(1),
        calls: AtomicUsize::new(0),
    });
    let items = Arc::new(AtomicUsize::new(0));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "interrupted-consumer",
            Box::new(InterruptedConsumer {
                producer: producer.clone(),
                items: items.clone(),
            }),
        )]
        .into(),
        || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                prepare(game);
                for (id, name) in [(71, "minecraft:chicken"), (9000, "minecraft:zombie")] {
                    let kind = game.res.mob_types().get(name).unwrap().clone();
                    let mut mob = Mob::new(id, kind, Vec3::new(1.0, 80.0, 0.5), 0.0);
                    mob.on_ground = true;
                    mob.ai.speed = 0.2;
                    game.mobs.push(mob);
                }
                let pending = game.creatures.pending.entry(game.dim.id()).or_default();
                pending.retained_passes = MAX_PENDING_MOB_PASSES;
                pending.pushes.push(71, Vec3::new(0.25, 0.0, 0.0));
                // These are already accepted consumer batches. The first completes
                // before the second's real native item callback interrupts the drain.
                for index in 0..MAX_PENDING_MOB_PASSES {
                    let events = (0..if index == 1 { 3 } else { 1 })
                        .map(|_| MobEvent::LayEgg {
                            mob: 71,
                            at: [1.0, 80.0, 0.5],
                            pitch: 1.0,
                        })
                        .collect();
                    pending.batches.push_back(MobOutputBatch {
                        normal: NormalMobActions::new(events),
                        guests: Vec::new(),
                    });
                }
                for expected_items in [2, 3] {
                    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        game.tick_creature_entities()
                    }))
                    .unwrap_err();
                    assert_eq!(
                        panic.downcast_ref::<u32>(),
                        Some(&90),
                        "consumer interruption, not fresh producer panic, must escape"
                    );
                    assert_eq!(items.load(Ordering::SeqCst), expected_items);
                    assert!(!game.creatures.pending[game.dim.id()].pushes.is_empty());
                    assert_eq!(game.mobs[0].velocity.x, 0.0);
                    assert_eq!(
                        producer.calls.load(Ordering::SeqCst),
                        0,
                        "production must stay refused while any older batch remains"
                    );
                    assert_eq!(
                        game.creatures.pending[game.dim.id()].batches.len(),
                        MAX_PENDING_MOB_PASSES - 1
                    );
                }
                game.tick_creature_entities().unwrap();
                assert_eq!(producer.calls.load(Ordering::SeqCst), 0);
                assert!(game.creatures.pending[game.dim.id()].batches.is_empty());
                assert!(game.creatures.pending[game.dim.id()].pushes.is_empty());
                assert_eq!(game.creatures.pending[game.dim.id()].retained_passes, 0);
                assert_eq!(
                    game.mobs[0].velocity.x, 0.25,
                    "push survives both consumer interruptions"
                );
                assert_eq!(
                    items.load(Ordering::SeqCst),
                    MAX_PENDING_MOB_PASSES + 2,
                    "entered actions must not replay and unentered tails must not disappear"
                );
                let egg_count = |game: &Game<'_>| -> u32 {
                    game.dropped
                        .iter()
                        .filter(|d| {
                            matches!(
                                d.stack.item.as_ref(),
                                "minecraft:egg" | "minecraft:brown_egg" | "minecraft:blue_egg"
                            )
                        })
                        .map(|d| d.stack.count)
                        .sum()
                };
                assert_eq!(egg_count(game), (MAX_PENDING_MOB_PASSES + 2) as u32);
                producer.remaining.store(0, Ordering::SeqCst);
                game.mobs[0].extra.egg_time = 1;
                game.tick_creature_entities().unwrap();
                assert_eq!(
                    egg_count(game),
                    (MAX_PENDING_MOB_PASSES + 3) as u32,
                    "normal production resumes only after the refused backlog is empty"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn push_only_failed_passes_are_refused_and_delivered_without_loss() {
    crate::entry::install_client_hooks();
    let probe = Arc::new(MovementPanic {
        id: 9000,
        remaining: AtomicUsize::new(40),
        calls: AtomicUsize::new(0),
    });
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "push_only",
            Box::new(Hooks(probe.clone())),
        )]
        .into(),
        || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                prepare(game);
                for (id, name, x) in [
                    (70, "minecraft:pig", 1.2),
                    (9000, "minecraft:zombie", 10.0),
                    (72, "minecraft:pig", 1.0),
                ] {
                    let kind = game.res.mob_types().get(name).unwrap().clone();
                    let mut mob = Mob::new(id, kind, Vec3::new(x, 80.0, 0.5), 0.0);
                    mob.on_ground = true;
                    mob.ai.speed = 0.2;
                    mob.no_ai = id != 9000;
                    game.mobs.push(mob);
                }
                let mut delivered = 0.0;
                // The pig before the panicking zombie pushes the pig after it.
                // Its vanilla impulse is -sqrt(0.2) * 0.05f32 on X, once per pass.
                let impulse = -0.022360680108198;
                for attempt in 1..=40 {
                    for (mob, x) in game.mobs.iter_mut().zip([1.2, 10.0, 1.0]) {
                        mob.position = Vec3::new(x, 80.0, 0.5);
                        mob.velocity = Vec3::ZERO;
                        mob.on_ground = true;
                        mob.ambient_timer = -10000;
                    }
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        game.tick_creature_entities()
                    }));
                    if attempt % 17 == 0 {
                        outcome
                            .expect("the seventeenth pass must refuse fresh production")
                            .unwrap();
                        assert!((game.mobs[2].velocity.x - 16.0 * impulse).abs() < 1e-9);
                        delivered += game.mobs[2].velocity.x;
                    } else {
                        assert_eq!(outcome.unwrap_err().downcast_ref::<u32>(), Some(&89));
                    }
                    assert!(game.creatures.pending[game.dim.id()].batches.is_empty());
                }
                assert_eq!(probe.calls.load(Ordering::SeqCst), 38);
                game.mobs[2].velocity = Vec3::ZERO;
                game.deliver_pending_mob_actions(game.dim.id()).unwrap();
                delivered += game.mobs[2].velocity.x;
                assert!(
                    (delivered - 38.0 * impulse).abs() < 1e-9,
                    "every accepted push is delivered"
                );
                game.mobs[2].velocity = Vec3::ZERO;
                game.deliver_pending_mob_actions(game.dim.id()).unwrap();
                assert_eq!(game.mobs[2].velocity, Vec3::ZERO, "no replay after drain");
                // A world reset retires the old backlog and its admission state together.
                for (mob, x) in game.mobs.iter_mut().zip([1.2, 10.0, 1.0]) {
                    mob.position = Vec3::new(x, 80.0, 0.5);
                    mob.velocity = Vec3::ZERO;
                    mob.ambient_timer = -10000;
                }
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.tick_creature_entities()
                }))
                .unwrap_err();
                assert_eq!(panic.downcast_ref::<u32>(), Some(&89));
                assert_eq!(game.creatures.pending[game.dim.id()].retained_passes, 1);
                assert!(!game.creatures.pending[game.dim.id()].pushes.is_empty());
                game.clear_pending_mob_actions();
                assert!(game.creatures.pending.is_empty());
                game.deliver_pending_mob_actions(game.dim.id()).unwrap();
                assert_eq!(
                    game.mobs[2].velocity,
                    Vec3::ZERO,
                    "reset must not replay old pushes"
                );
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.tick_creature_entities()
                }))
                .unwrap_err();
                assert_eq!(
                    panic.downcast_ref::<u32>(),
                    Some(&89),
                    "fresh production remains eligible after reset"
                );
                assert_eq!(game.creatures.pending[game.dim.id()].retained_passes, 1);
                game.clear_pending_mob_actions();
            });
        },
    );
}

struct RepeatedConsumer {
    fail: std::sync::atomic::AtomicBool,
    entered: AtomicUsize,
}
struct RepeatedConsumerHooks(Arc<RepeatedConsumer>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for RepeatedConsumerHooks {
    fn item_spawn(&self, _: &mut dyn mod_api::ServerHost, _: &mut mod_api::ItemSpawnEvent<'_>) {
        self.0.entered.fetch_add(1, Ordering::SeqCst);
        if self.0.fail.load(Ordering::SeqCst) {
            std::panic::panic_any(91_u32);
        }
    }
}

#[test]
#[ignore = "daily"]
fn successful_mixed_producers_are_bounded_behind_a_repeatedly_failing_consumer() {
    crate::entry::install_client_hooks();
    let probe = Arc::new(RepeatedConsumer {
        fail: std::sync::atomic::AtomicBool::new(true),
        entered: AtomicUsize::new(0),
    });
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "repeated_consumer",
            Box::new(RepeatedConsumerHooks(probe.clone())),
        )]
        .into(),
        || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                prepare(game);
                for id in 0..16 {
                    let kind = game
                        .res
                        .mob_types()
                        .get("minecraft:chicken")
                        .unwrap()
                        .clone();
                    let mut mob = Mob::new(
                        100 + id,
                        kind,
                        Vec3::new(2.0 + (id % 8) as f64, 80.0, 3.0 + (id / 8) as f64),
                        0.0,
                    );
                    mob.no_ai = true;
                    game.mobs.push(mob);
                }
                for (id, x) in [(70, 1.2), (72, 1.0)] {
                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    let mut mob = Mob::new(id, kind, Vec3::new(x, 80.0, 0.5), 0.0);
                    mob.no_ai = true;
                    game.mobs.push(mob);
                }
                let mut ages = Vec::new();
                let mut peak_batches = 0;
                let mut accepted_eggs = 0;
                let mut push_passes = 0;
                for attempt in 1..=40 {
                    for (index, mob) in game.mobs.iter_mut().enumerate() {
                        mob.position = if index < 16 {
                            Vec3::new(2.0 + (index % 8) as f64, 80.0, 3.0 + (index / 8) as f64)
                        } else {
                            Vec3::new(if index == 16 { 1.2 } else { 1.0 }, 80.0, 0.5)
                        };
                        mob.velocity = Vec3::ZERO;
                        mob.on_ground = true;
                        mob.ambient_timer = -10000;
                        if index < 16 {
                            mob.extra.egg_time = if attempt % 2 == 1 { 1 } else { 100000 };
                        }
                    }
                    let age = game.mobs[0].age;
                    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        game.tick_creature_entities()
                    }))
                    .unwrap_err();
                    assert_eq!(
                        panic.downcast_ref::<u32>(),
                        Some(&91),
                        "only the consumer interrupts; admitted producers succeed"
                    );
                    let produced = game.mobs[0].age != age;
                    if produced {
                        if attempt % 2 == 1 {
                            accepted_eggs += 16;
                        }
                        assert!(
                            game.mobs[16].velocity.x != 0.0 || game.mobs[17].velocity.x != 0.0,
                            "real successful producers also push"
                        );
                        push_passes += 1;
                    } else {
                        assert_eq!(
                            game.mobs[16].velocity,
                            Vec3::ZERO,
                            "refusal must not replay an accepted push"
                        );
                        assert_eq!(game.mobs[17].velocity, Vec3::ZERO);
                    }
                    ages.push(game.mobs[0].age);
                    peak_batches =
                        peak_batches.max(game.creatures.pending[game.dim.id()].batches.len());
                }
                eprintln!(
                    "REPEATED_CONSUMER:peak_batches={peak_batches}:producer_ages={ages:?}:accepted_eggs={accepted_eggs}:push_passes={push_passes}"
                );
                assert!(
                    peak_batches <= 16,
                    "successful producers grew retained batches beyond the cap: {peak_batches}"
                );
                assert_eq!(ages[15], 16);
                assert!(
                    ages[16..].iter().all(|&age| age == 16),
                    "refusal starts on call17 and stays latched"
                );
                assert_eq!(push_passes, 16);
                assert!(
                    game.creatures.pending[game.dim.id()].pushes.is_empty(),
                    "successful producer sweeps consumed their pushes"
                );
                probe.fail.store(false, Ordering::SeqCst);
                let velocities = [game.mobs[16].velocity, game.mobs[17].velocity];
                game.deliver_pending_mob_actions(game.dim.id()).unwrap();
                assert_eq!(probe.entered.load(Ordering::SeqCst), accepted_eggs);
                let eggs: u32 = game
                    .dropped
                    .iter()
                    .filter(|d| {
                        matches!(
                            d.stack.item.as_ref(),
                            "minecraft:egg" | "minecraft:brown_egg" | "minecraft:blue_egg"
                        )
                    })
                    .map(|d| d.stack.count)
                    .sum();
                assert_eq!(
                    eggs, accepted_eggs as u32,
                    "every accepted event reaches its real item exactly once"
                );
                assert_eq!([game.mobs[16].velocity, game.mobs[17].velocity], velocities);
                let pending = &game.creatures.pending[game.dim.id()];
                assert_eq!(pending.retained_passes, 0);
                assert!(!pending.refusal_logged);
                game.deliver_pending_mob_actions(game.dim.id()).unwrap();
                assert_eq!(
                    probe.entered.load(Ordering::SeqCst),
                    accepted_eggs,
                    "no event replay"
                );
                for mob in game.mobs.iter_mut().take(16) {
                    mob.extra.egg_time = 1;
                }
                game.tick_creature_entities().unwrap();
                assert_eq!(
                    game.mobs[0].age, 17,
                    "fresh producer admitted after complete drain"
                );
                assert_eq!(probe.entered.load(Ordering::SeqCst), accepted_eggs + 16);
            });
        },
    );
}
