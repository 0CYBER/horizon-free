//! Run alone with private prepared resources: installs process-wide trait mods.
use super::*;
use horizon_client::{
    chunk::Chunk,
    merchant::{ItemCost, MerchantOffer, Trader},
    mobs::Mob,
};
use mod_api::{
    self, EntityInteractEvent, Hooks, HostSide, InteractionResult, ItemInteractMobEvent, ItemPart,
    Registrar, RegistrationContext,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

#[derive(Clone, Default)]
struct Probe {
    calls: Arc<Mutex<Vec<(i32, String, String)>>>,
    answer: Arc<AtomicU8>,
    hands: Arc<Mutex<Vec<mod_api::Hand>>>,
    claim_entity: Arc<AtomicBool>,
    discard: Arc<AtomicBool>,
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for item in ["minecraft:stick", "minecraft:paper"] {
            let key = r.item_key(item);
            r.item_part(key, self.clone());
        }
    }
    fn entity_interact(
        &self,
        _: HostSide<'_>,
        event: &mut EntityInteractEvent<'_>,
    ) -> Option<InteractionResult> {
        self.hands.lock().unwrap().push(event.hand);
        if self.claim_entity.load(Ordering::Relaxed) {
            Some(InteractionResult::SuccessServer)
        } else {
            None
        }
    }
}
#[mod_api::horizon_part]
impl ItemPart for Probe {
    fn item_interact_mob(
        &self,
        host: HostSide<'_>,
        event: &mut ItemInteractMobEvent<'_>,
    ) -> Option<InteractionResult> {
        let HostSide::Server(host) = host else {
            return None;
        };
        let stack = event.stack;
        assert!(stack.count() > 0);
        self.calls.lock().unwrap().push((
            event.entity,
            event.kind.into(),
            mod_api::__private::item_key_id(stack.item())
                .unwrap()
                .to_string(),
        ));
        let answer = self.answer.load(Ordering::Relaxed);
        if answer != 0 && self.discard.load(Ordering::Relaxed) {
            let entity = host.entity(event.entity as u32).unwrap();
            assert!(entity.remove(host, mod_api::RemovalReason::Discarded));
        }
        match answer {
            0 => None,
            1 => Some(InteractionResult::Consume),
            _ => Some(InteractionResult::SuccessServer),
        }
    }
}
fn setup(game: &mut Game<'_>, probe: &Probe, kind: &str, id: u32) {
    game.mobs.clear();
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    game.mobs
        .push(Mob::new(id, kind, Vec3::new(0.5, 64.0, 2.5), 0.0));
    game.station = None;
    game.screen = Screen::Playing;
    game.inventory[0] = Some(ItemStack::new("minecraft:stick", 2));
    game.offhand = Some(ItemStack::new("minecraft:paper", 3));
    game.swing = Default::default();
    game.swings_started = 0;
    game.tick_hit.set(None);
    game.tick_mob = None;
    game.riding = None;
    probe.calls.lock().unwrap().clear();
    probe.answer.store(1, Ordering::Relaxed);
    probe.claim_entity.store(false, Ordering::Relaxed);
    probe.discard.store(false, Ordering::Relaxed);
}
fn target(id: i32) -> Option<InteractTarget> {
    Some(InteractTarget {
        id,
        location: [0.0, 1.0, 0.0],
    })
}

#[test]
#[ignore = "private prepared resources; process-wide mods, run this test alone"]
fn actual_item_interact_mob_fallback_priority_and_filters() {
    crate::game::in_own_process(
        "game::item_interact_mob_tests::actual_item_interact_mob_fallback_priority_and_filters",
        actual_item_interact_mob_fallback_priority_and_filters_isolated,
    );
}

fn actual_item_interact_mob_fallback_priority_and_filters_isolated() {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let probe = Probe::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("item", Box::new(probe.clone()))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        mod_api::__private::mark_game_thread();
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        crate::game::native_entity_refs::tests::resolve_parts(&game).unwrap();
        game.world = World::new(game.res.facts().air, 1);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        game.yaw = 0.0;
        game.pitch = 0.0;
        game.selected = 0;
        game.set_mode_id(0);

        // The actual click sequence must distinguish identical stacks, including
        // empty hands. A claimed main-hand interaction ends before the off hand.
        for occupied in [false, true] {
            for claimed in [false, true] {
                setup(&mut game, &probe, "minecraft:zombie", 42);
                game.inventory[0] = occupied.then(|| ItemStack::new("minecraft:stick", 1));
                game.offhand = game.inventory[0].clone();
                probe.answer.store(0, Ordering::Relaxed);
                probe.claim_entity.store(claimed, Ordering::Relaxed);
                probe.hands.lock().unwrap().clear();
                game.use_item_as_player().unwrap();
                let expected = if claimed {
                    vec![mod_api::Hand::Main]
                } else {
                    vec![mod_api::Hand::Main, mod_api::Hand::Off]
                };
                assert_eq!(*probe.hands.lock().unwrap(), expected);
            }
        }

        // A custom item willing to own every mob still cannot preempt trading.
        setup(&mut game, &probe, "minecraft:villager", 42);
        game.mobs[0].extra.trade = Some(Box::new(Trader {
            offers: vec![MerchantOffer {
                base_cost_a: ItemCost {
                    stack: ItemStack::new("minecraft:emerald", 1),
                },
                cost_b: None,
                result: ItemStack::new("minecraft:bread", 1),
                uses: 0,
                max_uses: 12,
                reward_exp: true,
                special_price_diff: 0,
                demand: 0,
                price_multiplier: 0.0,
                xp: 1,
            }],
            ..Default::default()
        }));
        assert!(game.interact_entity(target(42)).unwrap());
        assert!(probe.calls.lock().unwrap().is_empty());
        assert!(!game.mobs[0].removed);
        assert_eq!(
            game.station
                .as_ref()
                .unwrap()
                .merchant
                .as_ref()
                .unwrap()
                .mob,
            Some(42)
        );

        // Existing API 29 owns the earlier phase and retains its usual swing.
        setup(&mut game, &probe, "minecraft:zombie", 42);
        probe.claim_entity.store(true, Ordering::Relaxed);
        assert!(game.interact_entity(target(42)).unwrap());
        assert!(probe.calls.lock().unwrap().is_empty());
        assert_eq!(game.swings_started, 1);
        for (answer, consumed, swings) in [(0, false, 0), (1, true, 0), (2, true, 1)] {
            setup(&mut game, &probe, "minecraft:zombie", 42);
            probe.answer.store(answer, Ordering::Relaxed);
            assert_eq!(game.interact_entity(target(42)).unwrap(), consumed);
            assert_eq!(game.swings_started, swings);
            assert_eq!(
                *probe.calls.lock().unwrap(),
                [(42, "minecraft:zombie".into(), "minecraft:stick".into())]
            );
        }

        // An ordinary mob with no own item action reaches the fallback exactly
        // once. The mod may discard it with no loot and end without an arm swing.
        setup(&mut game, &probe, "minecraft:blaze", 42);
        probe.discard.store(true, Ordering::Relaxed);
        assert!(game.interact_entity(None).unwrap());
        assert!(game.mobs[0].removed);
        assert!(game.dropped.is_empty());
        assert_eq!(game.swings_started, 0);
        assert_eq!(probe.calls.lock().unwrap().len(), 1);
        assert_eq!(game.inventory[0].as_ref().unwrap().count, 2);

        // Panda's early false is still a normal pass into the outer fallback.
        setup(&mut game, &probe, "minecraft:panda", 42);
        assert!(game.interact_entity(target(42)).unwrap());
        assert_eq!(probe.calls.lock().unwrap().len(), 1);
        assert_eq!(game.swings_started, 0);

        setup(&mut game, &probe, "minecraft:zombie", 42);
        assert!(
            game.with_hand(Hand::Off, |g| g.interact_entity(target(42)))
                .unwrap()
        );
        assert_eq!(probe.calls.lock().unwrap()[0].2, "minecraft:paper");
        assert_eq!(game.hand, Hand::Main);
        assert_eq!(game.inventory[0].as_ref().unwrap().count, 2);
        assert_eq!(game.offhand.as_ref().unwrap().count, 3);
        assert_eq!(game.swings_started, 0);

        for invalid in ["dead", "removed", "riding", "wrong_id", "empty", "zero"] {
            setup(&mut game, &probe, "minecraft:zombie", 42);
            match invalid {
                "dead" => game.mobs[0].health = 0.0,
                "removed" => game.mobs[0].removed = true,
                "riding" => game.riding = Some(42),
                "empty" => game.inventory[0] = None,
                "zero" => game.inventory[0].as_mut().unwrap().count = 0,
                _ => {}
            }
            assert!(
                !game
                    .interact_entity(target(if invalid == "wrong_id" { 99 } else { 42 }))
                    .unwrap(),
                "{invalid}"
            );
            assert!(probe.calls.lock().unwrap().is_empty(), "{invalid}");
        }
        setup(&mut game, &probe, "minecraft:zombie", 42);
        game.mobs[0].health = 0.0;
        assert!(!game.interact_entity(None).unwrap());
        assert!(probe.calls.lock().unwrap().is_empty());

        // Local selection is occluded; a validated server packet keeps its exact
        // entity ID, independent of the acting player's camera and local ray.
        setup(&mut game, &probe, "minecraft:blaze", 42);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 65, 1, stone).unwrap();
        assert!(!game.interact_entity(None).unwrap());
        assert!(probe.calls.lock().unwrap().is_empty());
        assert!(game.interact_entity(target(42)).unwrap());
        assert_eq!(probe.calls.lock().unwrap().len(), 1);
        setup(&mut game, &probe, "minecraft:zombie", 42);
        game.set_mode_id(3);
        assert_eq!(game.use_item_as_player().unwrap(), None);
        assert!(probe.calls.lock().unwrap().is_empty());
    });
}
