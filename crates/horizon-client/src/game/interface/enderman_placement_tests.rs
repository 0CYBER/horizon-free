//! Exercise the actual registered leave-block goal and Game event handler.
use super::{Audio, Game, Options, Progress, Settings, load_resources};
use horizon_client::{
    chunk::Chunk,
    mobs::{Mob, MobEvent, goals::Cx},
};
use horizon_core::{geometry::Vec3, position::ChunkPos};
use mod_api::RandomSource;
use std::sync::Arc;

/// The production tick lends the level's RNG and attachments to goals while
/// they read its blocks. Keep that ownership boundary in the registered-goal test.
fn tick_registered_goals(game: &mut Game<'_>, mob: &mut Mob) -> Vec<MobEvent> {
    let mut goals = std::mem::take(&mut mob.ai.goals);
    let snapshot = [super::creatures::seen(mob, None, game.items())];
    let mut events = Vec::new();
    let mut lent = horizon_client::health::LentLevel::take(&mut game.world);
    {
        let world = game.mob_world_base(&snapshot);
        let mut level = horizon_client::health::DamageLevel::lent(
            &game.world,
            game.pack.assets.registry,
            &mut lent,
            Some(game.items()),
            Some(&game.pack.block_registries),
            &game.damage_hooks,
        );
        goals.tick(
            mob,
            &mut Cx {
                w: &world,
                level: &mut level,
                events: &mut events,
            },
        );
    }
    lent.give_back(&mut game.world);
    mob.ai.goals = goals;
    events
}

#[test]
#[ignore = "daily"]
fn leave_block_goal_uses_real_neighbor_shapes_and_keeps_air_placement() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        super::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = super::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    let registry = game.pack.assets.registry;
    let air = registry.default_state("minecraft:air").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    let kind = game
        .res
        .mob_types()
        .get("minecraft:enderman")
        .unwrap()
        .clone();
    for (name, expected_name) in [("poppy", "air"), ("note_block", "note_block")] {
        for x in 0..8 {
            for z in 0..8 {
                for y in 3..7 {
                    game.world
                        .set(x, y, z, if y == 3 { stone } else { air })
                        .unwrap();
                }
            }
        }
        game.mobs.clear();
        let carried = registry
            .default_state(&format!("minecraft:{name}"))
            .unwrap();
        let mut mob = Mob::new(325, kind.clone(), Vec3::new(3.5, 4.0, 3.5), 0.0);
        mob.random.set_seed(325);
        mob.extra.carried = Some(carried);
        game.mobs.push(mob);
        for _ in 0..48_000 {
            let mut mob = game.mobs.pop().unwrap();
            let events = tick_registered_goals(&mut game, &mut mob);
            game.mobs.push(mob);
            game.handle_mob_events(events).unwrap();
            if game.mobs[0].extra.carried.is_none() {
                break;
            }
        }
        assert_eq!(
            game.mobs[0].extra.carried, None,
            "eligible {name} must empty its hand"
        );
        let mut states = Vec::new();
        for x in 0..8 {
            for z in 0..8 {
                for y in 4..7 {
                    let state = game.world.block(x, y, z);
                    if state != air {
                        states.push(registry.state(state).unwrap());
                    }
                }
            }
        }
        if expected_name == "air" {
            assert!(
                states.is_empty(),
                "poppy normalizes to AIR before placement, not a dropped unsupported flower"
            );
        } else {
            assert_eq!(states.len(), 1);
            assert_eq!(states[0].name, "minecraft:note_block");
            assert_eq!(
                states[0].properties.get("instrument").map(String::as_str),
                Some("basedrum")
            );
        }
    }
    // A waterlogged candidate schedules the real level's fluid tick even
    // when a preceding world write makes the placement ineligible.
    let fence = registry.default_state("minecraft:oak_fence").unwrap();
    let mut descriptor = registry.state(fence).unwrap().clone();
    descriptor
        .properties
        .insert("waterlogged".into(), "true".into());
    let fence = registry.resolve(&descriptor).unwrap();
    let mut mob = Mob::new(326, kind, Vec3::new(3.5, 4.0, 3.5), 0.0);
    mob.random.set_seed(325);
    mob.extra.carried = Some(fence);
    game.mobs.clear();
    game.mobs.push(mob);
    let mut attempt = None;
    for _ in 0..48_000 {
        let mut mob = game.mobs.pop().unwrap();
        let events = tick_registered_goals(&mut game, &mut mob);
        game.mobs.push(mob);
        if let Some(event) = events
            .into_iter()
            .find(|e| matches!(e, MobEvent::EndermanPlaceBlock { .. }))
        {
            attempt = Some(event);
            break;
        }
    }
    let attempt = attempt.expect("the registered leave goal emits an attempt");
    let MobEvent::EndermanPlaceBlock { pos, mob, .. } = &attempt else {
        unreachable!()
    };
    assert_eq!(*mob, 326);
    game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
    let before = game.block_ticks.fluids.count();
    // A stale carried-state identity cannot mutate the world or clear a new hand.
    game.mobs[0].extra.carried = Some(stone);
    game.handle_mob_events(vec![attempt.clone()]).unwrap();
    assert_eq!(game.block_ticks.fluids.count(), before);
    assert_eq!(game.mobs[0].extra.carried, Some(stone));
    game.mobs[0].extra.carried = Some(fence);
    game.handle_mob_events(vec![attempt.clone()]).unwrap();
    assert!(game.block_ticks.fluids.count() > before);
    assert_eq!(game.world.block(pos[0], pos[1], pos[2]), stone);
    assert_eq!(game.mobs[0].extra.carried, Some(fence));
}

/// Normalization must not bypass the existing entity_place cancel contract:
/// even a carried flower that becomes AIR stays in the enderman's hand.
#[test]
#[ignore = "daily"]
fn canceled_normalized_placements_keep_the_original_carried_block() {
    use horizon_core::position::BlockPos;
    use horizon_world::registry::BlockStateId;
    use mod_api::{EntityPlaceEvent, Flow, Hooks, ModEntry, ServerHost};
    use mod_platform::testing::with_mods;
    use std::sync::Mutex;

    struct Cancel {
        heard: Arc<Mutex<Vec<BlockPos>>>,
        normalized: Arc<Mutex<Vec<BlockStateId>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Cancel {
        fn entity_place(&self, _: &mut dyn ServerHost, event: &mut EntityPlaceEvent) -> Flow {
            self.heard.lock().unwrap().push(event.pos);
            self.normalized.lock().unwrap().push(
                crate::game::api_values::TryIntoEngine::try_into_engine(&event.state, &()).unwrap(),
            );
            Flow::Cancel
        }
    }

    let heard = Arc::new(Mutex::new(Vec::new()));
    let normalized = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "enderman_place_probe",
        Box::new(Cancel {
            heard: heard.clone(),
            normalized: normalized.clone(),
        }),
    )];
    with_mods(mods.into(), || {
        super::native_entity_refs::tests::with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let registry = game.pack.assets.registry;
            let air = registry.default_state("minecraft:air").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(3, 3, 3, stone).unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:enderman")
                .unwrap()
                .clone();
            for (i, name) in ["minecraft:poppy", "minecraft:note_block"]
                .into_iter()
                .enumerate()
            {
                let carried = registry.default_state(name).unwrap();
                let mut mob = Mob::new(325, kind.clone(), Vec3::new(3.5, 4.0, 3.5), 0.0);
                mob.extra.carried = Some(carried);
                game.mobs.clear();
                game.mobs.push(mob);
                game.handle_mob_events(vec![MobEvent::EndermanPlaceBlock {
                    mob: 325,
                    pos: [3, 4, 3],
                    carried,
                    occupants: Vec::new(),
                }])
                .unwrap();
                assert_eq!(game.world.block(3, 4, 3), air);
                assert_eq!(game.mobs[0].extra.carried, Some(carried));
                assert_eq!(heard.lock().unwrap().len(), i + 1, "one veto per attempt");
                let state = normalized.lock().unwrap()[i];
                if name == "minecraft:poppy" {
                    assert_eq!(state, air, "the veto sees the AIR-normalized flower");
                } else {
                    let state = registry.state(state).unwrap();
                    assert_eq!(state.name, "minecraft:note_block");
                    assert_eq!(
                        state.properties.get("instrument").map(String::as_str),
                        Some("basedrum"),
                        "the veto sees the normalized note block"
                    );
                }
            }
        });
    });
    assert_eq!(*heard.lock().unwrap(), [BlockPos::new(3, 4, 3); 2]);
}

#[test]
#[ignore = "daily"]
fn a_placement_listener_discarding_the_enderman_refuses_the_attempt() {
    use mod_api::{EntityPlaceEvent, Flow, Hooks, ModEntry, ServerHost};
    use mod_platform::testing::with_mods;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Discard(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Discard {
        fn entity_place(&self, host: &mut dyn ServerHost, event: &mut EntityPlaceEvent) -> Flow {
            assert!(
                event
                    .placer
                    .expect("the live enderman resolves")
                    .discard(host)
            );
            self.0.fetch_add(1, Ordering::SeqCst);
            Flow::Continue
        }
    }

    let heard = Arc::new(AtomicUsize::new(0));
    let mods = vec![ModEntry::new(
        "enderman_discard_probe",
        Box::new(Discard(heard.clone())),
    )];
    with_mods(mods.into(), || {
        super::native_entity_refs::tests::with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let registry = game.pack.assets.registry;
            let air = registry.default_state("minecraft:air").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(3, 3, 3, stone).unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:enderman")
                .unwrap()
                .clone();
            let mut mob = Mob::new(325, kind, Vec3::new(3.5, 4.0, 3.5), 0.0);
            mob.extra.carried = Some(stone);
            game.mobs.push(mob);
            let attempt = MobEvent::EndermanPlaceBlock {
                mob: 325,
                pos: [3, 4, 3],
                carried: stone,
                occupants: Vec::new(),
            };
            game.handle_mob_events(vec![attempt.clone()]).unwrap();
            assert_eq!(heard.load(Ordering::SeqCst), 1);
            let mob = game.mobs.iter().find(|m| m.id == 325).unwrap();
            // Discard does not set health to zero: alive() alone is not enough.
            assert!(mob.alive());
            assert!(mob.removed);
            assert_eq!(mob.extra.carried, Some(stone));
            assert_eq!(game.world.block(3, 4, 3), air);
            // An already removed enderman cannot start another placement either.
            game.handle_mob_events(vec![attempt]).unwrap();
            assert_eq!(heard.load(Ordering::SeqCst), 1);
            assert_eq!(game.world.block(3, 4, 3), air);
        });
    });
}

#[test]
#[ignore = "daily"]
fn an_air_normalized_placement_empties_the_hand_without_a_block_change_notice() {
    use mod_api::{EntityChangeBlockEvent, Hooks, ModEntry, ServerHost};
    use mod_platform::testing::with_mods;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Changed(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Changed {
        fn entity_change_block(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityChangeBlockEvent<'_>,
        ) {
            assert_eq!(event.from, "minecraft:air");
            assert_eq!(event.to, "minecraft:note_block");
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let heard = Arc::new(AtomicUsize::new(0));
    let mods = vec![ModEntry::new(
        "enderman_change_probe",
        Box::new(Changed(heard.clone())),
    )];
    with_mods(mods.into(), || {
        super::native_entity_refs::tests::with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let registry = game.pack.assets.registry;
            let air = registry.default_state("minecraft:air").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(3, 3, 3, stone).unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:enderman")
                .unwrap()
                .clone();
            for name in ["minecraft:poppy", "minecraft:note_block"] {
                let carried = registry.default_state(name).unwrap();
                let mut mob = Mob::new(325, kind.clone(), Vec3::new(3.5, 4.0, 3.5), 0.0);
                mob.extra.carried = Some(carried);
                game.mobs.clear();
                game.mobs.push(mob);
                game.handle_mob_events(vec![MobEvent::EndermanPlaceBlock {
                    mob: 325,
                    pos: [3, 4, 3],
                    carried,
                    occupants: Vec::new(),
                }])
                .unwrap();
                assert_eq!(game.mobs[0].extra.carried, None);
                let state = game.world.block(3, 4, 3);
                if name == "minecraft:poppy" {
                    assert_eq!(state, air);
                    assert_eq!(heard.load(Ordering::SeqCst), 0);
                } else {
                    assert_eq!(registry.state(state).unwrap().name, "minecraft:note_block");
                    assert_eq!(heard.load(Ordering::SeqCst), 1);
                }
            }
        });
    });
}
