//! Base exception: legacy base-catalog fixtures; new hooks use their own area.
#[test]
fn registration_context_reaches_collectors_in_order_without_a_game_host() {
    struct Registration(&'static str);
    #[mod_api::__subscriptions]
    impl Hooks for Registration {
        fn component_types(
            &self,
            context: &RegistrationContext,
            event: &mut ComponentTypesEvent<'_>,
        ) {
            assert_eq!(context.0, "bootstrap");
            event.types.push(self.0.into());
        }
    }
    install(
        vec![
            ModEntry::new("first", Box::new(Registration("first:component"))),
            ModEntry::new("second", Box::new(Registration("second:component"))),
        ]
        .into(),
    );
    let mut types = Vec::new();
    dispatch_component_types(&RegistrationContext("bootstrap"), &mut types);
    assert_eq!(types, ["first:component", "second:component"]);
}

#[test]
fn only_overrides_are_dispatched_from_the_first_call_across_words() {
    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {}
    struct Listener(Arc<std::sync::atomic::AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn loot_table_load(&self, _: &dyn DataLoadContext, _: &mut LootTableLoadEvent<'_>) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods: Vec<ModEntry> = (0..70)
        .map(|i| {
            if i == 66 {
                ModEntry::new("listener", Box::new(Listener(calls.clone())))
            } else {
                ModEntry::new("quiet", Box::new(Quiet))
            }
        })
        .collect();
    install(mods.into());
    ENTERED.with(|entered| entered.borrow_mut().clear());
    let mut table = serde_json::json!([]);
    dispatch_loot_table_load(&Load("defaults"), "test", &mut table);
    dispatch_loot_table_load(&Load("defaults"), "test", &mut table);
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    ENTERED.with(|entered| {
        let entered = entered.borrow();
        assert_eq!(
            entered.as_slice(),
            [(66, "loot_table_load"), (66, "loot_table_load")]
        );
    });
}

#[test]
fn nested_default_does_not_affect_installed_subscribers() {
    struct Other;
    #[mod_api::__subscriptions]
    impl Hooks for Other {}
    struct Active;
    #[mod_api::__subscriptions]
    impl Hooks for Active {
        fn loot_table_load(&self, _: &dyn DataLoadContext, event: &mut LootTableLoadEvent<'_>) {
            dispatch_modify_default_components(&Load("defaults"), "test", &mut Map::new());
            Other.loot_table_load(&Load("defaults"), event);
        }
    }
    install(vec![ModEntry::new("active", Box::new(Active))].into());
    ENTERED.with(|entered| entered.borrow_mut().clear());
    let mut table = serde_json::json!([]);
    dispatch_loot_table_load(&Load("defaults"), "test", &mut table);
    dispatch_loot_table_load(&Load("defaults"), "test", &mut table);
    ENTERED.with(|entered| {
        assert_eq!(
            entered
                .borrow()
                .iter()
                .filter(|(_, hook)| *hook == "loot_table_load")
                .count(),
            2
        );
    });
}

#[test]
fn empty_dispatch_checks_before_event_construction() {
    let generated = include_str!(concat!(env!("OUT_DIR"), "/mod_api_generated.rs"));
    let function = generated
        .split("pub fn dispatch_loot_table_load(")
        .nth(1)
        .unwrap();
    let guard = function.find("super::listened").unwrap();
    let event = function.find("let mut event").unwrap();
    assert!(guard < event);
}

#[test]
fn command_dispatch_forwards_typed_input_and_source() {
    struct Observer(Arc<Mutex<Vec<i32>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn run_mod_command(
            &self,
            _: &mut dyn ServerHost,
            event: &mut RunModCommandEvent<'_>,
        ) -> Flow {
            self.0
                .lock()
                .unwrap()
                .push(event.command.amount * event.source.sign);
            Flow::Cancel
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    install(vec![ModEntry::new("commands", Box::new(Observer(calls.clone())))].into());
    let command = CommandInvocation { amount: 37 };
    for sign in [1, -1] {
        let mut ran = false;
        dispatch_run_mod_command(&mut TestHost, &CommandSource { sign }, &command, &mut ran);
        assert!(ran);
    }
    assert_eq!(*calls.lock().unwrap(), vec![37, -37]);
}

#[test]
fn advancement_progress_reports_grant_and_revoke_identity() {
    struct Observer(Arc<Mutex<Vec<ProgressType>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn player_advancement_progress(
            &self,
            _: &mut dyn ServerHost,
            event: &mut PlayerAdvancementProgressEvent<'_>,
        ) {
            assert_eq!(event.player, 123);
            assert_eq!(event.player_entity_id, 42);
            assert_eq!(event.name, "Alex");
            assert_eq!(event.advancement, "example:advancement");
            assert_eq!(event.criterion, "criterion");
            assert_eq!(
                event.progress.done,
                event.progress_type == ProgressType::Grant
            );
            self.0.lock().unwrap().push(event.progress_type);
        }
    }
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mods = [ModEntry::new(
        "observer",
        Box::new(Observer(observed.clone())),
    )];
    let mods: Arc<[ModEntry]> = mods.into();
    install(mods.clone());
    for progress_type in [ProgressType::Grant, ProgressType::Revoke] {
        dispatch_player_advancement_progress(
            &mut TestHost,
            calls::PlayerAdvancementProgressCall {
                player: 123,
                player_entity_id: 42,
                name: "Alex",
                advancement: "example:advancement",
                criterion: "criterion",
                progress_type,
                progress: &AdvancementProgress {
                    done: progress_type == ProgressType::Grant,
                },
            },
        );
    }
    assert_eq!(
        *observed.lock().unwrap(),
        vec![ProgressType::Grant, ProgressType::Revoke]
    );
}
#[test]
fn menu_button_mutates_entity_and_cursor_once() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        block_entities: vec![part(1, &calls)],
        ..Parts::default()
    });
    let context = generated::mod_api::item::menu::MenuButtonContext {
        player: 123,
        player_entity_id: 42,
        pos: [1, 2, 3],
        id: 9,
    };
    let mut entity = ModBlockEntity::new("machine");
    let mut carried = None;
    dispatch_menu_button(&mut TestHost, &context, &mut entity, &mut carried);
    assert_eq!(entity.id(), "machine");
    assert_eq!(entity.data, [9]);
    assert_eq!(carried.unwrap().item.as_ref(), "bucket");
}

#[test]
fn general_entity_tick_tells_each_part_of_the_row_in_order() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        entities: vec![part(1, &calls), part(2, &calls)],
        ..Parts::default()
    });
    dispatch_general_entity_tick(&mut TestHost, "mymod:robot", generated::mod_api::EntityRef);
    assert_eq!(*calls.lock().unwrap(), [1, 2]);
}

#[test]
fn general_entity_tick_without_parts_names_no_table() {
    let calls: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts::default());
    dispatch_general_entity_tick(&mut TestHost, "mymod:robot", generated::mod_api::EntityRef);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn block_entity_submit_dispatches_in_order_with_shared_draw() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        block_entities: vec![part(1, &calls), part(2, &calls)],
        ..Parts::default()
    });
    let mut draw = BlockEntityDraw(Vec::new());
    dispatch_block_entity_submit(
        &TestHost,
        [1, 2, 3],
        "example:machine",
        BlockState::new(BlockKey::Test, 0),
        &ModBlockEntity::default(),
        &mut draw,
    );
    assert_eq!(draw.0, [1, 2]);
}

#[test]
fn slot_rules_mutate_and_item_tick_can_cancel() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mods: Vec<ModEntry> = vec![ModEntry::new(
        "mod-2",
        Box::new(Mod {
            id: 1,
            calls: calls.clone(),
        }),
    )];
    let mods: Arc<[ModEntry]> = mods.into();
    install(mods.clone());
    install_parts(Parts {
        menus: vec![part(1, &calls)],
        ..Parts::default()
    });
    let (mut may_place, mut max_stack) = (true, 64);
    dispatch_menu_slot_rules(
        &TestHost,
        "test:menu",
        0,
        &ItemStack::default(),
        &mut may_place,
        &mut max_stack,
    );
    assert_eq!((may_place, max_stack), (false, 1));
    let (mut duration, mut canceled) = (10, false);
    dispatch_living_entity_use_item_tick(
        HostSide::Server(&mut TestHost),
        &ItemStack::default(),
        None,
        Hand::Main,
        &mut duration,
        &mut canceled,
    );
    assert_eq!((duration, canceled), (9, true));
}

#[test]
fn item_finish_can_clear_vanilla_result() {
    let mods: Vec<ModEntry> = vec![ModEntry::new(
        "mod-3",
        Box::new(Mod {
            id: 1,
            calls: Arc::new(Mutex::new(Vec::new())),
        }),
    )];
    let mods: Arc<[ModEntry]> = mods.into();
    install(mods.clone());
    // Both sides reach the listener, which clears the result.
    for client in [false, true] {
        let mut host = TestHost;
        let side = if client {
            HostSide::Client(&mut host)
        } else {
            HostSide::Server(&mut host)
        };
        let mut result = Some(ItemStack::default());
        dispatch_living_entity_use_item_finish(
            side,
            &ItemStack::default(),
            None,
            Hand::Main,
            0,
            &mut result,
        );
        assert!(result.is_none(), "client={client}");
    }
}

#[test]
fn fragment_cancel_stops_later_mods() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mods: Vec<ModEntry> = vec![
        ModEntry::new(
            "mod-4",
            Box::new(Mod {
                id: 1,
                calls: calls.clone(),
            }),
        ),
        ModEntry::new(
            "mod-5",
            Box::new(Mod {
                id: 2,
                calls: calls.clone(),
            }),
        ),
    ];
    let mods: Arc<[ModEntry]> = mods.into();
    install(mods.clone());
    let mut canceled = false;
    dispatch_mouse_button_pre(&mut TestHost, 0, true, 0, &mut canceled);
    assert!(canceled);
    assert_eq!(*calls.lock().unwrap(), [1]);
    // Each call is entered under the hook's name; the cancelled one's is the last.
    let entered = ENTERED.with(|entered| entered.take());
    assert_eq!(entered, [(0, "mouse_button_pre")]);
}

#[test]
fn placement_replacement_can_refuse_and_stops_later_mods() {
    struct Level;
    impl LevelHost for Level {}
    struct Refuse(Arc<Mutex<Vec<u32>>>, u32);
    impl BlockPart for Refuse {
        fn block_subscribed(&self) -> &'static [Option<HookId>] {
            &[]
        }
        fn state_for_placement(
            &self,
            _: &dyn LevelHost,
            _: &mut StateForPlacementEvent<'_>,
        ) -> Option<Option<BlockState>> {
            self.0.lock().unwrap().push(self.1);
            Some(None)
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        blocks: vec![(
            BlockKey::Test,
            vec![
                leak(Refuse(calls.clone(), 1)),
                leak(Refuse(calls.clone(), 2)),
            ],
        )],
        ..Parts::default()
    });
    let table = parts(HookId::StateForPlacement, "state_for_placement");
    assert_eq!(
        table
            .block_key(content::registry_keys::block_key("test:block"))
            .len(),
        2
    );
    assert!(
        table
            .block_key(content::registry_keys::block_key("missing:block"))
            .is_empty()
    );
    let mut answer = None;
    for state in [
        BlockState::new(content::registry_keys::block_key("test:other").unwrap(), 0),
        BlockState::new(BlockKey::Test, u16::MAX),
    ] {
        dispatch_state_for_placement(
            &Level,
            LevelDimension,
            state,
            &PlacementContext,
            generated::mod_api::FluidState {
                fluid: generated::mod_api::FluidKey,
                amount: 0,
                falling: false,
                source: false,
            },
            &mut answer,
        );
        assert_eq!(answer, None);
        assert!(calls.lock().unwrap().is_empty());
    }
    dispatch_state_for_placement(
        &Level,
        LevelDimension,
        BlockState::new(BlockKey::Test, 0),
        &PlacementContext,
        generated::mod_api::FluidState {
            fluid: generated::mod_api::FluidKey,
            amount: 0,
            falling: false,
            source: false,
        },
        &mut answer,
    );
    assert_eq!(answer, Some(None));
    assert_eq!(*calls.lock().unwrap(), vec![1]);
    dispatch_state_for_placement(
        &Level,
        LevelDimension,
        BlockState::new(BlockKey::Test, 0),
        &PlacementContext,
        generated::mod_api::FluidState {
            fluid: generated::mod_api::FluidKey,
            amount: 0,
            falling: false,
            source: false,
        },
        &mut answer,
    );
    assert_eq!(*calls.lock().unwrap(), vec![1]);
}

#[test]
fn incoming_damage_carries_the_attack_data() {
    struct Backstab;
    #[mod_api::__subscriptions]
    impl Hooks for Backstab {
        fn living_incoming_damage(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut LivingIncomingDamageEvent<'_>,
        ) -> Flow {
            assert_eq!(event.source.damage_type, "minecraft:player_attack");
            let (from, at, look) = (
                event.source.position.unwrap(),
                event.victim.position,
                event.victim.view,
            );
            let behind = (0..3)
                .map(|axis| (from[axis] - at[axis]) * look[axis])
                .sum::<f64>();
            if behind < 0.0 && event.weapon.is_some() {
                event.container.0 *= 3.0;
            }
            Flow::Continue
        }
    }
    let mods: Vec<ModEntry> = vec![ModEntry::new("backstab-9", Box::new(Backstab))];
    let mods: Arc<[ModEntry]> = mods.into();
    install(mods.clone());
    let held = ItemStack::default();
    for (source_z, weapon, expected) in [
        (-1.0, Some(&held), 12.0),
        (1.0, Some(&held), 4.0),
        (-1.0, None, 4.0),
    ] {
        let source = HurtSource {
            damage_type: "minecraft:player_attack",
            position: Some([0.0, 0.0, source_z]),
        };
        let victim = Victim {
            position: [0.0; 3],
            view: [0.0, 0.0, 1.0],
        };
        let (mut container, mut canceled) = (DamageContainer(4.0), false);
        dispatch_living_incoming_damage(
            &mut TestLevel,
            calls::LivingIncomingDamageCall {
                entity: 1,
                kind: "minecraft:zombie",
                source: &source,
                victim: &victim,
                weapon,
                container: &mut container,
                canceled: &mut canceled,
            },
        );
        assert_eq!((container.0, canceled), (expected, false));
    }
}

#[test]
fn beacon_dispatch_preserves_optional_effect_keys() {
    struct Beacon(Arc<Mutex<Vec<[Option<EffectKey>; 2]>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Beacon {
        fn player_change_beacon_effect(
            &self,
            _: &mut dyn ServerHost,
            event: &mut PlayerChangeBeaconEffectEvent<'_>,
        ) {
            self.0
                .lock()
                .unwrap()
                .push([event.primary, event.secondary]);
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    install(vec![ModEntry::new("beacon", Box::new(Beacon(calls.clone())))].into());
    dispatch_player_change_beacon_effect(
        &mut TestHost,
        42,
        "player",
        [1, 2, 3],
        Some(EffectKey(7)),
        None,
    );
    dispatch_player_change_beacon_effect(
        &mut TestHost,
        43,
        "other",
        [4, 5, 6],
        None,
        Some(EffectKey(11)),
    );
    assert_eq!(
        *calls.lock().unwrap(),
        [[Some(EffectKey(7)), None], [None, Some(EffectKey(11))]],
    );
}

#[test]
fn dispatch_contracts() {
    struct Weapon;
    #[mod_api::__subscriptions]
    impl Hooks for Weapon {
        fn living_knockback(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut LivingKnockbackEvent<'_>,
        ) -> Flow {
            assert_eq!(
                (event.entity, event.kind, event.attacker),
                (12, "minecraft:armor_stand", 7)
            );
            assert_eq!(event.source.damage_type, "minecraft:player_attack");
            assert!(event.attacker_main.is_none() && event.attacker_off.is_none());
            assert_eq!(event.entry, KnockbackEntry::Full);
            assert_eq!(event.strength, 0.4);
            event.strength *= 2.0;
            event.x = -event.x;
            Flow::Cancel
        }
    }
    let weapon_mods: Vec<ModEntry> = vec![ModEntry::new("weapon-10", Box::new(Weapon))];
    let weapon_mods: Arc<[ModEntry]> = weapon_mods.into();
    install(weapon_mods.clone());
    let (mut strength, mut x, mut z, mut canceled) = (0.4, 1.0, 0.0, false);
    let source = HurtSource {
        damage_type: "minecraft:player_attack",
        position: None,
    };
    dispatch_living_knockback(
        &mut TestLevel,
        calls::LivingKnockbackCall {
            entity: 12,
            kind: "minecraft:armor_stand",
            source: &source,
            attacker: 7,
            attacker_main: None,
            attacker_off: None,
            entry: KnockbackEntry::Full,
            strength: &mut strength,
            x: &mut x,
            z: &mut z,
            canceled: &mut canceled,
        },
    );
    assert_eq!((strength, x, z), (0.8, -1.0, 0.0));
    assert!(canceled);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mods: Vec<ModEntry> = (1..=2)
        .map(|id| {
            ModEntry::new(
                "mod-11",
                Box::new(Mod {
                    id,
                    calls: calls.clone(),
                }),
            )
        })
        .collect();
    let mods: Arc<[ModEntry]> = mods.into();
    install(mods.clone());
    let mut table = serde_json::json!([]);
    dispatch_loot_table_load(&Load("defaults"), "table", &mut table);
    assert_eq!(table, serde_json::json!([1, 2]));
    let mut components = Map::new();
    dispatch_modify_default_components(&Load("defaults"), "item", &mut components);
    assert_eq!(components["order"], serde_json::json!([1, 2]));
    let mut host = TestHost;
    dispatch_level_tick_pre(HostSide::Server(&mut host), LevelRef);
    assert_eq!(*calls.lock().unwrap(), [1, 2]);
    calls.lock().unwrap().clear();
    let mut zoom = None;
    dispatch_zoom(&host, &mut zoom);
    assert_eq!(zoom, Some(Zoom(1)));
    assert_eq!(*calls.lock().unwrap(), [1]);
    calls.lock().unwrap().clear();
    dispatch_zoom(&host, &mut zoom);
    assert!(calls.lock().unwrap().is_empty());
    install_parts(Parts {
        items: vec![part(1, &calls), part(2, &calls)],
        ..Parts::default()
    });
    let mut lines = Vec::new();
    dispatch_tooltip(&host, &ItemStack::default(), &mut lines);
    assert_eq!(lines.iter().map(|line| line.1).collect::<Vec<_>>(), [1, 2]);
    let mut apply = false;
    calls.lock().unwrap().clear();
    dispatch_should_apply_effect_tick_this_tick(&host, EffectKey(7), 19, 2, &mut apply);
    assert!(!apply);
    assert_eq!(*calls.lock().unwrap(), [1, 2]);
    let source = HurtSource {
        damage_type: "damage",
        position: None,
    };
    let victim = Victim {
        position: [0.0; 3],
        view: [0.0; 3],
    };
    let mut container = DamageContainer(1.0);
    let mut canceled = false;
    dispatch_living_incoming_damage(
        &mut TestLevel,
        calls::LivingIncomingDamageCall {
            entity: 0,
            kind: "kind",
            source: &source,
            victim: &victim,
            weapon: None,
            container: &mut container,
            canceled: &mut canceled,
        },
    );
    // Each mod sees the one before: (1 × 2 + 1) × 2 + 2, kept though mod 2 cancels.
    assert_eq!(container.0, 8.0);
    assert!(canceled);
    let chained: Vec<ModEntry> = [1, 3]
        .map(|id| {
            ModEntry::new(
                "mod-12",
                Box::new(Mod {
                    id,
                    calls: calls.clone(),
                }),
            )
        })
        .into();
    let chained: Arc<[ModEntry]> = chained.into();
    install(chained.clone());
    let mut container = DamageContainer(1.0);
    let mut canceled = false;
    dispatch_living_incoming_damage(
        &mut TestLevel,
        calls::LivingIncomingDamageCall {
            entity: 0,
            kind: "kind",
            source: &source,
            victim: &victim,
            weapon: None,
            container: &mut container,
            canceled: &mut canceled,
        },
    );
    assert_eq!(container.0, 9.0);
    assert!(!canceled);
    install(mods.clone());
    calls.lock().unwrap().clear();
    let mut result = None;
    dispatch_use_on(
        HostSide::Server(&mut host),
        calls::UseOnCall {
            stack: &ItemStack::default(),
            pos: [0; 3],
            face: [0; 3],
            hand: Hand::Main,
            hit: BlockHit,
            player: 0,
            player_entity_id: 0,
            result: &mut result,
        },
    );
    assert_eq!(result, Some(InteractionResult::SuccessServer));
    assert_eq!(*calls.lock().unwrap(), [1]);
    calls.lock().unwrap().clear();
    dispatch_use_on(
        HostSide::Server(&mut host),
        calls::UseOnCall {
            stack: &ItemStack::default(),
            pos: [0; 3],
            face: [0; 3],
            hand: Hand::Main,
            hit: BlockHit,
            player: 0,
            player_entity_id: 0,
            result: &mut result,
        },
    );
    assert!(calls.lock().unwrap().is_empty());
}

mod stubs {
    #[derive(Clone, Copy)]
    pub(super) struct FluidState;
    pub(super) struct PlacementContext;
    #[derive(Clone, Copy)]
    pub(super) struct BlockId;
    pub(super) struct Ticks;
    pub(super) struct ClockAdjustment;
    pub(super) struct Compound;
    pub(super) type Dispensed = (Option<super::ItemStack>, i32);
    pub(super) struct LootDrops<'a>(pub &'a ());
    pub(super) struct HandRender<'a>(pub &'a ());
    pub(super) struct Suggestion;
    pub(super) struct BiomeColors;
    pub(super) struct UseAnimation;
    #[derive(Clone, Copy)]
    pub(super) enum InteractionKey {
        Attack,
        Use,
        PickBlock,
    }
    #[derive(Clone, Copy)]
    pub(super) enum InteractionAttempt {
        Press,
        Held,
    }
    pub(super) struct MobRender<'a>(pub &'a ());
    pub(super) struct Shake;
    pub(super) struct Music;
    pub(super) struct MusicLayer;
    pub(super) struct BlockRim;
    pub(super) struct ColoredLight;
    pub(super) struct Minimap;
    pub(super) struct PanelEvent;
    pub(super) struct Screen;
    pub(super) struct ModContainer;
    pub(super) struct MerchantOffer;
    pub(super) struct ModFluidTanks;
    pub(super) struct ModEnergyBuffer;
    pub(super) struct DataMapType;
    pub(super) struct MapDecoration;
    pub(super) struct MapInfo<'a>(pub &'a ());
    pub(super) struct PlayerRender<'a>(pub &'a ());
    pub(super) struct PosedParts;
    pub(super) struct HiddenParts;
    pub(super) struct Command;
    pub(super) struct LifecycleContext;
    pub(super) struct Load(pub &'static str);
    pub(super) struct ServerPlayer;
    pub(super) struct CommandSource {
        pub(super) sign: i32,
    }
    pub(super) struct CommandInvocation {
        pub(super) amount: i32,
    }
    #[derive(Clone, Copy)]
    pub(super) struct ServerTickPhase;
    #[derive(Clone, Copy)]
    pub(super) struct WorldPhase;
}

mod api {
    #[derive(Clone, Copy)]
    #[non_exhaustive]
    pub struct FluidState {
        pub fluid: FluidKey,
        pub amount: u8,
        pub falling: bool,
        pub source: bool,
    }
    #[derive(Clone, Copy)]
    pub struct RaidRef;
    pub struct ClientBossBar;
    #[derive(Clone, Copy)]
    pub enum KeyAction {}
    pub mod fog {
        #[derive(Clone, Copy)]
        pub struct FogType;
        pub struct FogData;
    }
}
