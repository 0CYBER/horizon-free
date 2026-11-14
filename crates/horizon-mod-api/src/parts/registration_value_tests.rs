#![cfg(test)]
use super::*;
use crate::testing::FakeHost;
use std::sync::Mutex;

#[test]
fn dimension_keys_name_one_dimension_each() {
    use mod_vocab::__private::{dimension_id, dimension_key};
    assert_eq!(
        dimension_key("minecraft:overworld"),
        DimensionKey::OVERWORLD
    );
    assert_eq!(
        dimension_id(DimensionKey::OVERWORLD),
        Some("minecraft:overworld")
    );
    assert_eq!(dimension_key("minecraft:the_nether"), DimensionKey::NETHER);
    assert_eq!(dimension_id(DimensionKey::END), Some("minecraft:the_end"));
    let first = dimension_key("test:first");
    let second = dimension_key("test:second");
    assert_ne!(first, second);
    assert_eq!(dimension_key("test:first"), first);
    assert_eq!(dimension_id(second), Some("test:second"));
    assert_eq!(
        mod_vocab::__private::parts_installation::validate_dimension_key(
            mod_vocab::__private::parts_installation::dimension_key_slot(first)
        ),
        Ok(first)
    );
    assert!(mod_vocab::__private::parts_installation::validate_dimension_key(u32::MAX).is_err());
    assert_eq!(
        dimension_id(mod_vocab::__private::parts_installation::dimension_key(
            u32::MAX
        )),
        None
    );
}

#[test]
fn native_host_returns_typed_recipe_rules() {
    struct Recipes(Arc<[Recipe]>);
    impl mod_api::prelude::RecipesHost for Recipes {
        fn recipes_of_type(&self, kind: &str) -> Arc<[Recipe]> {
            if kind == "minecraft:crafting_transmute" {
                self.0.clone()
            } else {
                Arc::from([])
            }
        }
    }
    impl Host for Recipes {}
    mod_api::impl_host_areas!(Recipes, except(recipes));
    let mut recipe = Recipe::new(
        "test:recipe",
        "minecraft:crafting_transmute",
        Arc::new(serde_json::json!({"source": "test"})),
    );
    recipe.rule = Some(RecipeRule::Transmute {
        input: Ingredient::of(vec!["test:input".into()]),
        material: Ingredient::of(vec!["test:material".into()]),
        min: 2,
        max: 4,
        add_count: true,
        result: RecipeTemplate::new(None, 3, None),
    });
    let host = Recipes(Arc::from([recipe]));
    let view = (&host as &dyn Host).recipes_of_type("minecraft:crafting_transmute");
    let Some(RecipeRule::Transmute {
        input,
        material,
        min,
        max,
        add_count,
        result,
    }) = &view[0].rule
    else {
        panic!("wrong rule")
    };
    assert_eq!(input.order, ["test:input"]);
    assert_eq!(material.order, ["test:material"]);
    assert_eq!((*min, *max, *add_count, result.count), (2, 4, true, 3));
    assert_eq!(view[0].json["source"], "test");
    assert!(
        (&host as &dyn Host)
            .recipes_of_type("test:other")
            .is_empty()
    );
}

#[test]
fn client_menu_click_refuses_a_host_without_its_backend() {
    struct Empty;
    impl Host for Empty {}
    mod_api::impl_host_areas!(Empty);
    impl ClientHost for Empty {}
    mod_api::impl_client_host_areas!(Empty);
    let player = __private::game::player_ref(EntityRef::new(WorldEpoch::new(1), 7), 42);
    let menu = __private::game::menu_ref(player, 0, 1);
    assert!(!menu.click(
        &mut Empty,
        &[MenuClick::Pickup {
            slot: Some(0),
            button: MouseButton::Left,
        }]
    ));
}

#[test]
fn level_family_accepts_empty_doubles() {
    struct Empty;
    impl LevelHost for Empty {}
    impl LevelRandomHost for Empty {}
    impl LevelTicksHost for Empty {}
    impl LevelWriteHost for Empty {}

    let level: &dyn LevelHost = &Empty;
    assert_eq!(level.block_state([0, 64, 0]), None);
    assert_eq!(level.block_state([0, i32::MIN, 0]), None);
    let _: &mut dyn LevelRandomHost = &mut Empty;
    let _: &mut dyn LevelTicksHost = &mut Empty;
    let _: &mut dyn LevelWriteHost = &mut Empty;
}

#[test]
fn an_optional_dependency_runs_only_when_built_with_and_installed() {
    let installed = |id| {
        let mut host = FakeHost::new();
        host.registries.loaded(id);
        host
    };
    // The `testing` feature stands for a dependency's: without it the block isn't compiled,
    // with it the game is asked.
    let ran = std::cell::Cell::new(0);
    for host in [installed("testing"), installed("other")] {
        let host: &dyn Host = &host;
        if_installed!(host, testing, {
            ran.set(ran.get() + 1);
        });
    }
    assert_eq!(ran.get(), usize::from(cfg!(feature = "testing")));
}

struct Quiet;
#[mod_api::__subscriptions]
impl Hooks for Quiet {}

#[test]
fn server_services_forward_commands_messages_and_player_identity() {
    let mut server = FakeHost::new();
    server.commands.known("list");
    server.messages.blocked("blocked");
    server.join(7, 42);
    server.players.acting = Some(7);
    let host: &mut dyn ServerHost = &mut server;
    assert!(host.commands().run_command("list"));
    assert!(!host.commands().run_command("missing"));
    host.commands().feedback(Text::literal("answer")).unwrap();
    host.messages()
        .broadcast(Text::literal("everyone"))
        .unwrap();
    assert_eq!(host.messages().text_allowed("blocked"), Some(false));
    let player = host.players().acting_player().unwrap();
    assert_eq!(host.players().server_players(), [player]);
    assert_eq!((player.uuid(), player.entity().id()), (7, 42));
    assert_eq!(server.commands.ran, ["list", "missing"]);
    assert_eq!(server.commands.feedback, [Text::literal("answer")]);
    assert_eq!(
        server.messages.broadcast_lines(),
        [Text::literal("everyone")]
    );
}

#[test]
fn render_hand_chains_pose_and_draw_until_cancelled() {
    struct HandMod {
        offset: f32,
        cancel: bool,
    }
    /// Counts the pushes the mods make on the one draw they share.
    struct Draw(u32);
    impl BlockEntityDrawContext for Draw {
        fn push(&mut self) {
            self.0 += 1;
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for HandMod {
        fn render_hand(&self, _host: &dyn ClientHost, event: &mut RenderHandEvent<'_, '_>) -> Flow {
            assert_eq!((event.render.player, event.render.player_entity_id), (7, 9));
            event.render.translation[0] += self.offset;
            event.draw.push();
            if self.cancel {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
    let mods = [
        ModEntry::new(
            "hand-first",
            Box::new(HandMod {
                offset: 1.0,
                cancel: false,
            }),
        ),
        ModEntry::new(
            "hand-cancel",
            Box::new(HandMod {
                offset: 2.0,
                cancel: true,
            }),
        ),
        ModEntry::new(
            "hand-last",
            Box::new(HandMod {
                offset: 4.0,
                cancel: false,
            }),
        ),
    ];
    install(mods.into());
    let host = FakeHost::new();
    let mut render = HandRender {
        player: 7,
        player_entity_id: 9,
        hand: Hand::Main,
        stack: None,
        partial: 0.5,
        interpolated_pitch: 0.0,
        swing: 0.0,
        equip: 0.0,
        packed_light: 0,
        translation: [0.0; 3],
        rotation: [0.0; 3],
        scale: 1.0,
    };
    let mut canceled = false;
    let mut draw = Draw(0);
    dispatch_render_hand(&host, &mut render, &mut draw, &mut canceled);
    assert!(canceled);
    assert_eq!(render.translation, [3.0, 0.0, 0.0]);
    // The mod that cancelled drew too; the one after it was not asked.
    assert_eq!(draw.0, 2);
}

/// Installs one mod, `id`, that attaches `part` to sticks.
fn stick_part<P: ItemPart + Clone>(id: &'static str, part: P) {
    struct StickMod<P>(P);
    #[mod_api::__subscriptions]
    impl<P: ItemPart + Clone> Hooks for StickMod<P> {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let item = r.item_key("minecraft:stick");
            r.item_part(item, self.0.clone());
        }
    }
    let mods: Mods = vec![ModEntry::new(id, Box::new(StickMod(part)))].into();
    install(mods.clone());
    install_parts(&mods, &crate::__private::part_tests::rule_schema(), |_| {
        false
    })
    .unwrap();
}

/// Claims a use on stone, after telling the player what the block is.
#[derive(Clone)]
struct Probe;

#[mod_api::horizon_part]
impl ItemPart for Probe {
    fn use_on(&self, host: HostSide<'_>, event: &mut UseOnEvent<'_>) -> Option<InteractionResult> {
        let HostSide::Server(host) = host else {
            return None;
        };
        let player = host.players().acting_player().expect("a player acts");
        let level = host.level("minecraft:overworld").expect("a loaded level");
        let block = level.block(host, event.pos).expect("a loaded block");
        let name = __private::block_key_id(block).unwrap().to_owned();
        player
            .action_bar(host, Text::literal(name.clone()))
            .unwrap();
        if name == "minecraft:stone" {
            Some(InteractionResult::SuccessServer)
        } else {
            None
        }
    }
}

#[test]
fn conversion_post_reports_both_entities_once() {
    struct Observer(Arc<Mutex<Vec<(u32, String, u32, String)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn living_conversion_post(
            &self,
            _host: &mut dyn ServerHost,
            event: &mut LivingConversionPostEvent<'_>,
        ) {
            self.0.lock().unwrap().push((
                event.entity,
                event.kind.to_owned(),
                event.converted,
                event.converted_kind.to_owned(),
            ));
        }
    }
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mods = [ModEntry::new(
        "conversion-observer",
        Box::new(Observer(observed.clone())),
    )];
    install(mods.into());
    let mut host = FakeHost::new();
    dispatch_living_conversion_post(
        &mut host,
        12,
        "minecraft:pig",
        13,
        "minecraft:zombified_piglin",
    );
    assert_eq!(
        *observed.lock().unwrap(),
        [(
            12,
            "minecraft:pig".into(),
            13,
            "minecraft:zombified_piglin".into()
        )]
    );
}

#[test]
fn game_mode_change_dispatches_the_shared_type_and_honors_cancellation_and_redirects() {
    struct VetoSpectator;
    #[mod_api::__subscriptions]
    impl Hooks for VetoSpectator {
        fn player_game_mode_changed(
            &self,
            _host: &mut dyn ServerHost,
            event: &mut PlayerGameModeChangedEvent<'_>,
        ) -> Flow {
            assert_eq!((event.player, event.name), (7, "Probe"));
            assert_eq!(event.from, horizon_core::game_mode::GameMode::Adventure);
            match *event.to {
                GameMode::Spectator => Flow::Cancel,
                GameMode::Creative => {
                    *event.to = GameMode::Survival;
                    Flow::Continue
                }
                _ => Flow::Continue,
            }
        }
    }
    install(vec![ModEntry::new("mode-veto", Box::new(VetoSpectator))].into());
    let mut host = FakeHost::new();
    for (mut to, expected_cancel, applied) in [
        (GameMode::Spectator, true, GameMode::Spectator),
        (GameMode::Survival, false, GameMode::Survival),
        (GameMode::Creative, false, GameMode::Survival),
    ] {
        let mut cancel = false;
        dispatch_player_game_mode_changed(
            &mut host,
            7,
            "Probe",
            horizon_core::game_mode::GameMode::Adventure,
            &mut to,
            &mut cancel,
        );
        assert_eq!((cancel, to), (expected_cancel, applied));
    }
}

#[test]
fn xp_change_passes_identity_and_allows_signed_amount_or_cancellation() {
    struct XpHook(bool);
    #[mod_api::__subscriptions]
    impl Hooks for XpHook {
        fn player_xp_change(
            &self,
            _host: &mut dyn ServerHost,
            event: &mut PlayerXpChangeEvent<'_>,
        ) -> Flow {
            assert_eq!(
                (event.player, event.player_entity_id, event.name),
                (7, 42, "Probe")
            );
            event.amount = -5;
            if self.0 { Flow::Cancel } else { Flow::Continue }
        }
    }
    let mut host = FakeHost::new();
    for cancelled in [false, true] {
        let mods = [ModEntry::new("xp-hook", Box::new(XpHook(cancelled)))];
        install(mods.into());
        let (mut amount, mut cancel) = (10, false);
        dispatch_player_xp_change(&mut host, 7, 42, "Probe", &mut amount, &mut cancel);
        assert_eq!((amount, cancel), (-5, cancelled));
    }
}

struct MorePhantoms;

#[mod_api::__subscriptions]
impl Hooks for MorePhantoms {
    fn player_spawn_phantoms(
        &self,
        _host: &mut dyn ServerHost,
        event: &mut PlayerSpawnPhantomsEvent<'_>,
    ) -> Option<PhantomSpawnResult> {
        *event.phantoms = event.phantoms.map(|count| count + 2);
        None
    }
}

#[test]
fn phantom_listener_can_increase_the_default_count() {
    let mods = [ModEntry::new("phantoms", Box::new(MorePhantoms))];
    install(mods.into());
    let mut host = FakeHost::new();
    let mut count = Some(3);
    let mut result = None;
    dispatch_player_spawn_phantoms(&mut host, 7, "Probe", [0; 3], &mut count, &mut result);
    assert_eq!(count, Some(5));
    assert!(result.is_none());
}

/// Asks the host to tell a player without anyone having implemented it.
#[derive(Clone)]
struct Chatty;

#[mod_api::horizon_part]
impl ItemPart for Chatty {
    fn use_on(&self, host: HostSide<'_>, _event: &mut UseOnEvent<'_>) -> Option<InteractionResult> {
        let HostSide::Server(host) = host else {
            return None;
        };
        host.commands().run_command("say hello");
        None
    }
}

fn stick() -> ItemStack {
    crate::__private::part_tests::stack("minecraft:stick", 1, 0)
}

#[test]
#[should_panic(expected = "native mod id must not be empty")]
fn an_entry_requires_a_nonempty_id() {
    ModEntry::new("", Box::new(Quiet));
}

#[test]
fn player_turn_mods_receive_identity_and_chain_changes() {
    struct Turn;
    #[mod_api::__subscriptions]
    impl Hooks for Turn {
        fn calculate_player_turn(
            &self,
            _host: &dyn ClientHost,
            event: &mut CalculatePlayerTurnEvent,
        ) {
            assert_eq!((event.player_entity_id, event.player_uuid), (42, 123));
            event.mouse_sensitivity *= 2.0;
            event.cinematic_camera_enabled = !event.cinematic_camera_enabled;
        }
    }
    let host = FakeHost::new();
    let mods: Vec<ModEntry> = vec![
        ModEntry::new("turn-1", Box::new(Turn)),
        ModEntry::new("turn-2", Box::new(Turn)),
    ];
    install(mods.into());
    let mut sensitivity = 0.25;
    let mut cinematic = false;
    dispatch_calculate_player_turn(&host, 42, 123, &mut sensitivity, &mut cinematic);
    assert_eq!(sensitivity, 1.0);
    assert!(!cinematic);
}

#[test]
fn a_part_reads_the_clicked_block_and_tells_the_acting_player() {
    // Another test installs parts with tags: a dispatch must run on a game thread.
    mark_game_thread();
    stick_part("probe", Probe);
    // The level answers blocks as their load-resolved keys.
    mod_vocab::__private::resolve_block_keys(["minecraft:stone", "minecraft:dirt"], |_| {
        let properties = std::collections::BTreeMap::new();
        horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
            .map_err(|error| format!("{error:?}"))
    })
    .unwrap();
    let stack = stick();
    for (block, expected) in [("minecraft:stone", true), ("minecraft:dirt", false)] {
        let (pos, face) = ([1, 2, 3], [0, 1, 0]);
        let mut host = FakeHost::new();
        host.level.set_block(pos, block);
        host.join(7, 41);
        host.players.acting = Some(7);
        let mut result = None;
        let hit = top_hit();
        dispatch_use_on(
            HostSide::Server(&mut host),
            calls::UseOnCall {
                stack: &stack,
                pos,
                face,
                hand: Hand::Main,
                hit,
                player: 7,
                player_entity_id: 41,
                result: &mut result,
            },
        );
        assert_eq!(
            result,
            expected.then_some(InteractionResult::SuccessServer),
            "{block}"
        );
        let player = host.players.get(7).unwrap();
        assert_eq!(player.action_bar, [Text::literal(block)], "{block}");
    }
}

#[test]
fn mining_events_identify_the_player_and_keep_mod_decisions() {
    struct MiningProbe(Arc<Mutex<Vec<u32>>>);
    #[mod_api::__subscriptions]
    impl Hooks for MiningProbe {
        fn player_harvest_check(
            &self,
            _host: crate::HostSide,
            event: &mut PlayerHarvestCheckEvent,
        ) {
            self.0.lock().unwrap().push(event.player);
            event.can_harvest = false;
        }
        fn player_break_speed(
            &self,
            _host: crate::HostSide,
            event: &mut PlayerBreakSpeedEvent,
        ) -> Flow {
            self.0.lock().unwrap().push(event.player);
            event.new_speed = 2.0;
            Flow::Continue
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = [ModEntry::new(
        "mining-probe",
        Box::new(MiningProbe(seen.clone())),
    )];
    install(mods.into());
    let mut host = FakeHost::new();
    let mut harvest = true;
    let mut speed = 1.0;
    let mut canceled = false;
    dispatch_player_harvest_check(HostSide::Server(&mut host), 42, [1, 2, 3], 7, &mut harvest);
    dispatch_player_break_speed(
        HostSide::Server(&mut host),
        42,
        [1, 2, 3],
        7,
        1.0,
        &mut speed,
        &mut canceled,
    );
    assert_eq!(*seen.lock().unwrap(), [42, 42]);
    assert!(!harvest);
    assert_eq!(speed, 2.0);
    assert!(!canceled);
}

#[test]
fn an_inventory_tick_reaches_only_the_parts_of_the_ticked_item() {
    #[derive(Clone)]
    struct Ticked(Arc<Mutex<Vec<(i32, bool)>>>);
    #[mod_api::horizon_part]
    impl ItemPart for Ticked {
        fn inventory_tick(&self, _: &mut dyn ServerHost, event: &mut InventoryTickEvent<'_>) {
            self.0.lock().unwrap().push((event.slot, event.selected));
        }
    }
    struct Both(Ticked, Ticked);
    #[mod_api::__subscriptions]
    impl Hooks for Both {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let (stick, stone) = (r.item_key("minecraft:stick"), r.item_key("minecraft:stone"));
            r.item_part(stick, self.0.clone());
            r.item_part(stone, self.1.clone());
        }
    }
    // Another test installs parts with tags: a dispatch must run on a game thread.
    mark_game_thread();
    let (sticks, stones) = (Ticked(Default::default()), Ticked(Default::default()));
    let mods: Mods = vec![ModEntry::new(
        "tickers",
        Box::new(Both(sticks.clone(), stones.clone())),
    )]
    .into();
    install(mods.clone());
    install_parts(&mods, &crate::__private::part_tests::rule_schema(), |_| {
        false
    })
    .unwrap();
    let mut host = FakeHost::new();
    let stone = crate::__private::part_tests::stack("minecraft:stone", 1, 0);
    let diamond = crate::__private::part_tests::stack("minecraft:diamond", 1, 0);
    dispatch_inventory_tick(&mut host, &stick(), 4, true);
    dispatch_inventory_tick(&mut host, &stone, -106, false);
    dispatch_inventory_tick(&mut host, &stick(), 100, false);
    dispatch_inventory_tick(&mut host, &diamond, 0, false);
    assert_eq!(*sticks.0.lock().unwrap(), [(4, true), (100, false)]);
    assert_eq!(*stones.0.lock().unwrap(), [(-106, false)]);
}

#[test]
fn a_damage_reaches_only_the_parts_of_the_worn_item_and_changes_its_wear() {
    #[derive(Clone)]
    struct Worn(Arc<Mutex<Vec<i32>>>, i32, bool);
    #[mod_api::horizon_part]
    impl ItemPart for Worn {
        fn damage_item(&self, _: &mut dyn ServerHost, event: &mut DamageItemEvent<'_>) -> Flow {
            self.0.lock().unwrap().push(event.amount);
            event.amount *= self.1;
            if self.2 { Flow::Cancel } else { Flow::Continue }
        }
    }
    struct Both(Worn, Worn);
    #[mod_api::__subscriptions]
    impl Hooks for Both {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let (stick, stone) = (r.item_key("minecraft:stick"), r.item_key("minecraft:stone"));
            r.item_part(stick, self.0.clone());
            r.item_part(stone, self.1.clone());
        }
    }
    // Another test installs parts with tags: a dispatch must run on a game thread.
    mark_game_thread();
    // The stick's part triples the wear and the stone's cancels it.
    let sticks = Worn(Default::default(), 3, false);
    let stones = Worn(Default::default(), 1, true);
    let mods: Mods = vec![ModEntry::new(
        "wearers",
        Box::new(Both(sticks.clone(), stones.clone())),
    )]
    .into();
    install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    let mut host = FakeHost::new();
    let stone = crate::parts::tests::stack("minecraft:stone", 1, 0);
    let diamond = crate::parts::tests::stack("minecraft:diamond", 1, 0);
    let mut wear = |stack: &ItemStack, points: i32| {
        let (mut amount, mut canceled) = (points, false);
        dispatch_damage_item(
            &mut host,
            stack,
            None,
            &mut amount,
            &mut false,
            &mut canceled,
        );
        (amount, canceled)
    };
    assert_eq!(wear(&stick(), 4), (12, false));
    assert_eq!(wear(&stone, 7), (7, true));
    assert_eq!(wear(&diamond, 5), (5, false));
    assert_eq!(wear(&stick(), 1), (3, false));
    assert_eq!(*sticks.0.lock().unwrap(), [4, 1]);
    assert_eq!(*stones.0.lock().unwrap(), [7]);
    assert!(crate::__private::item_listens(
        HookId::DamageItem,
        "minecraft:stick"
    ));
    assert!(!crate::__private::item_listens(
        HookId::DamageItem,
        "minecraft:diamond"
    ));
}

#[test]
fn an_item_update_reaches_only_the_parts_of_the_ticked_item_and_cancels_for_its_own() {
    #[derive(Clone)]
    struct Updated(Arc<Mutex<Vec<EntityRef>>>, bool);
    #[mod_api::horizon_part]
    impl ItemPart for Updated {
        fn on_entity_item_update(
            &self,
            _: &mut dyn ServerHost,
            event: &mut OnEntityItemUpdateEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push(event.item.entity());
            if self.1 { Flow::Cancel } else { Flow::Continue }
        }
    }
    struct Both(Updated, Updated);
    #[mod_api::__subscriptions]
    impl Hooks for Both {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let (stick, stone) = (r.item_key("minecraft:stick"), r.item_key("minecraft:stone"));
            r.item_part(stick, self.0.clone());
            r.item_part(stone, self.1.clone());
        }
    }
    // Another test installs parts with tags: a dispatch must run on a game thread.
    mark_game_thread();
    let sticks = Updated(Default::default(), true);
    let stones = Updated(Default::default(), false);
    let mods: Mods = vec![ModEntry::new(
        "updaters",
        Box::new(Both(sticks.clone(), stones.clone())),
    )]
    .into();
    install(mods.clone());
    install_parts(&mods, &crate::__private::part_tests::rule_schema(), |_| {
        false
    })
    .unwrap();
    let mut host = FakeHost::new();
    let stone = crate::__private::part_tests::stack("minecraft:stone", 1, 0);
    let diamond = crate::__private::part_tests::stack("minecraft:diamond", 1, 0);
    let entity = |id| EntityRef::new(WorldEpoch::new(1), id);
    let mut update = |stack: &ItemStack, id| {
        let mut canceled = false;
        dispatch_on_entity_item_update(
            &mut host,
            crate::__private::OnEntityItemUpdateCall {
                stack,
                item: ItemEntityRef::new(entity(id)),
                canceled: &mut canceled,
            },
        );
        canceled
    };
    assert!(update(&stick(), 11));
    assert!(!update(&stone, 12));
    assert!(!update(&diamond, 13));
    assert!(update(&stick(), 14));
    assert_eq!(*sticks.0.lock().unwrap(), [entity(11), entity(14)]);
    assert_eq!(*stones.0.lock().unwrap(), [entity(12)]);
    assert!(crate::__private::item_listens(
        HookId::OnEntityItemUpdate,
        "minecraft:stick"
    ));
    assert!(!crate::__private::item_listens(
        HookId::OnEntityItemUpdate,
        "minecraft:diamond"
    ));
    assert!(!crate::__private::item_listens(
        HookId::InventoryTick,
        "minecraft:diamond"
    ));
}

#[test]
#[should_panic(expected = "Commands::run_command")]
fn an_unexpected_host_call_names_the_method() {
    /// A mod's own double, with only the services the hook reaches.
    struct Silent;
    impl Host for Silent {}
    crate::impl_host_areas!(Silent);
    impl ServerHost for Silent {}
    crate::impl_server_host_areas!(Silent, except(services));
    impl crate::prelude::ServicesServerHost for Silent {
        fn commands(&mut self) -> &mut dyn Commands {
            self
        }
    }
    impl Commands for Silent {}
    // Another test installs parts with tags: a dispatch must run on a game thread.
    mark_game_thread();
    stick_part("chatty", Chatty);
    let mut host = Silent;
    let mut result = None;
    let (pos, face) = ([0; 3], [0, 1, 0]);
    dispatch_use_on(
        HostSide::Server(&mut host),
        calls::UseOnCall {
            stack: &stick(),
            pos,
            face,
            hand: Hand::Main,
            hit: top_hit(),
            player: 7,
            player_entity_id: 41,
            result: &mut result,
        },
    );
}

/// The top of the block at the origin, clicked in its middle.
fn top_hit() -> BlockHit {
    BlockHit::new([0.5, 1.0, 0.5], [0, 1, 0], false)
}

/// Takes a click with the off hand on a block's top half; its sticks check what they hear.
struct OffHandTop;

/// A stick's use, checking the hand, the hit and the player.
#[derive(Clone)]
struct HeardStick;

#[mod_api::horizon_part]
impl ItemPart for HeardStick {
    fn use_on(&self, _: HostSide<'_>, event: &mut UseOnEvent<'_>) -> Option<InteractionResult> {
        assert_eq!((event.player, event.player_entity_id), (7, 41));
        assert_eq!((event.hand, event.hit), (Hand::Off, top_hit()));
        HEARD_STICK.store(true, std::sync::atomic::Ordering::SeqCst);
        None
    }
}

static HEARD_STICK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[mod_api::__subscriptions]
impl Hooks for OffHandTop {
    fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
        let item = r.item_key("minecraft:stick");
        r.item_part(item, HeardStick);
    }

    fn right_click_block(&self, _: crate::HostSide, event: &mut RightClickBlockEvent) -> Flow {
        let click = event.click;
        let top_half = click.hit.point[1] - f64::from(click.pos[1]) >= 0.5;
        if click.hand == Hand::Off
            && top_half
            && click.player == 7
            && click.player_entity_id == 41
            && !click.hit.inside
        {
            *event.cancellation_result = InteractionResult::Fail;
            Flow::Cancel
        } else {
            // A held stick never uses the block; an empty hand always does.
            *event.use_block = if click.stack.is_some() {
                TriState::False
            } else {
                TriState::True
            };
            Flow::Continue
        }
    }
}

#[test]
fn a_block_use_hears_the_hand_the_hit_and_the_player() {
    mark_game_thread();
    let mods: Mods = vec![ModEntry::new("off-hand-top", Box::new(OffHandTop))].into();
    install(mods.clone());
    install_parts(&mods, &crate::__private::part_tests::rule_schema(), |_| {
        false
    })
    .unwrap();
    let mut host = FakeHost::new();
    let mut result = None;
    dispatch_use_on(
        HostSide::Server(&mut host),
        calls::UseOnCall {
            stack: &stick(),
            pos: [0; 3],
            face: [0, 1, 0],
            hand: Hand::Off,
            hit: top_hit(),
            player: 7,
            player_entity_id: 41,
            result: &mut result,
        },
    );
    assert_eq!(result, None);
    assert!(HEARD_STICK.load(std::sync::atomic::Ordering::SeqCst));
    let low = BlockHit::new([0.5, 0.25, 0.0], [0, 0, -1], false);
    let held = stick();
    for (hand, hit, stack, canceled, answer, use_block) in [
        (
            Hand::Off,
            top_hit(),
            None,
            true,
            InteractionResult::Fail,
            TriState::Default,
        ),
        (
            Hand::Main,
            top_hit(),
            Some(&held),
            false,
            InteractionResult::Pass,
            TriState::False,
        ),
        (
            Hand::Off,
            low,
            None,
            false,
            InteractionResult::Pass,
            TriState::True,
        ),
    ] {
        let click = mod_vocab::__private::parts_installation::block_click(
            LogicalSide::Server,
            LevelDimension::Overworld,
            [0; 3],
            hit,
            hand,
            stack,
            (7, 41),
        );
        let (mut block, mut item) = (TriState::Default, TriState::Default);
        let (mut result, mut was_canceled) = (InteractionResult::Pass, false);
        dispatch_right_click_block(
            HostSide::Server(&mut host),
            &click,
            &mut block,
            &mut item,
            &mut result,
            &mut was_canceled,
        );
        assert_eq!(
            (was_canceled, result, block, item),
            (canceled, answer, use_block, TriState::Default),
            "{hand:?} {hit:?}"
        );
    }
}

/// Writes down every game rule change and level save it hears.
struct Recorder(Arc<Mutex<Vec<String>>>);

#[mod_api::__subscriptions]
impl Hooks for Recorder {
    fn game_rule_changed(&self, _host: &mut dyn ServerHost, event: &mut GameRuleChangedEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .push(format!("{} {}", event.rule, event.value));
    }
    fn level_save(&self, _host: &mut dyn ServerHost, event: &mut LevelSaveEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .push(format!("saved {}", event.dimension));
    }
}

#[test]
fn game_rule_change_reports_optional_actor() {
    struct ActorRecorder(Arc<Mutex<Vec<Option<(u128, i32, String)>>>>);
    #[mod_api::__subscriptions]
    impl Hooks for ActorRecorder {
        fn game_rule_changed(&self, _: &mut dyn ServerHost, event: &mut GameRuleChangedEvent<'_>) {
            self.0.lock().unwrap().push(
                event
                    .actor
                    .map(|actor| (actor.uuid, actor.entity_id, actor.name.to_owned())),
            );
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = [ModEntry::new(
        "actor-recorder",
        Box::new(ActorRecorder(seen.clone())),
    )];
    install(mods.into());
    let mut host = FakeHost::new();
    let actor = GameRuleActor {
        uuid: 123,
        entity_id: 42,
        name: "Alex",
    };
    dispatch_game_rule_changed(
        &mut host,
        "minecraft:keep_inventory",
        GameRuleValue::Bool(true),
        Some(&actor),
    );
    dispatch_game_rule_changed(
        &mut host,
        "minecraft:keep_inventory",
        GameRuleValue::Bool(false),
        None,
    );
    assert_eq!(
        *seen.lock().unwrap(),
        [Some((123, 42, "Alex".into())), None]
    );
}

/// API dispatch preserves each accepted rule's id and typed value for every mod.
/// Engine schema validation belongs to the game's rule setter tests.
#[test]
fn a_game_rule_change_reaches_every_mod_with_its_typed_value() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: Vec<ModEntry> = vec![
        ModEntry::new("recorder-10", Box::new(Recorder(seen.clone()))),
        ModEntry::new("recorder-11", Box::new(Recorder(seen.clone()))),
    ];
    install(mods.into());
    let mut host = FakeHost::new();
    for (id, value) in [
        ("minecraft:keep_inventory", GameRuleValue::Bool(true)),
        ("minecraft:random_tick_speed", GameRuleValue::Int(7)),
    ] {
        dispatch_game_rule_changed(&mut host, id, value, None);
    }
    assert_eq!(
        *seen.lock().unwrap(),
        [
            "minecraft:keep_inventory true",
            "minecraft:keep_inventory true",
            "minecraft:random_tick_speed 7",
            "minecraft:random_tick_speed 7",
        ]
    );
}

/// `LevelEvent.Save` as a world save fires it: once for each level, in
/// the order they are saved, to every mod.
#[test]
fn each_saved_level_reaches_every_mod() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: Vec<ModEntry> = vec![
        ModEntry::new("recorder-12", Box::new(Recorder(seen.clone()))),
        ModEntry::new("recorder-13", Box::new(Recorder(seen.clone()))),
    ];
    install(mods.into());
    let mut host = FakeHost::new();
    for dimension in ["minecraft:the_nether", "minecraft:overworld"] {
        dispatch_level_save(&mut host, dimension);
    }
    assert_eq!(
        *seen.lock().unwrap(),
        [
            "saved minecraft:the_nether",
            "saved minecraft:the_nether",
            "saved minecraft:overworld",
            "saved minecraft:overworld",
        ]
    );
}

/// Keeps the cod: reads the catch, sets the rod's damage, and cancels a
/// catch with cod in it.
struct CodKeeper;

#[mod_api::__subscriptions]
impl Hooks for CodKeeper {
    fn item_fished(&self, _host: &mut dyn ServerHost, event: &mut ItemFishedEvent<'_>) -> Flow {
        event.rod_damage = 4;
        let cod = event
            .drops
            .iter()
            .any(|s| s.item() == crate::__private::part_tests::fixture_item_key("minecraft:cod"));
        if cod { Flow::Cancel } else { Flow::Continue }
    }
}

/// Fails the test if a mod after a cancel is asked.
struct Unreached;

#[mod_api::__subscriptions]
impl Hooks for Unreached {
    fn item_fished(&self, _host: &mut dyn ServerHost, _event: &mut ItemFishedEvent<'_>) -> Flow {
        panic!("asked after a cancel");
    }
}

/// `ItemFishedEvent`: a mod reads the catch and sets the rod's damage,
/// which stays whether or not it cancels; a cancel stops later mods.
#[test]
fn a_fishing_catch_keeps_the_mods_rod_damage_and_its_cancel() {
    install_parts(&[], &crate::__private::part_tests::rule_schema(), |_| false).unwrap();
    let mut host = FakeHost::new();
    let mods: Vec<ModEntry> = vec![
        ModEntry::new("cod-keeper", Box::new(CodKeeper)),
        ModEntry::new("unreached", Box::new(Unreached)),
    ];
    install(mods.into());
    let ask = |host: &mut FakeHost, item: &str| {
        let mut drops = vec![crate::__private::part_tests::stack(item, 1, 0)];
        let fished = mod_vocab::__private::parts_installation::item_fished(9, [0.5, 62.0, 0.5]);
        let (mut rod_damage, mut cancel) = (1, false);
        dispatch_item_fished(
            host,
            7,
            3,
            &fished,
            &mut rod_damage,
            &mut cancel,
            &mut drops,
            &mut |_: &Vec<crate::ItemStack>| -> Result<(), crate::StackWriteError> { Ok(()) },
        );
        (rod_damage, cancel)
    };
    assert_eq!(ask(&mut host, "minecraft:cod"), (4, true));
    let mods: Vec<ModEntry> = vec![ModEntry::new("cod-keeper", Box::new(CodKeeper))];
    install(mods.into());
    let mut drops = vec![crate::__private::part_tests::stack(
        "minecraft:salmon",
        1,
        0,
    )];
    let fished = mod_vocab::__private::parts_installation::item_fished(9, [0.5, 62.0, 0.5]);
    let (mut rod_damage, mut cancel) = (2, false);
    dispatch_item_fished(
        &mut host,
        7,
        3,
        &fished,
        &mut rod_damage,
        &mut cancel,
        &mut drops,
        &mut |_: &Vec<crate::ItemStack>| -> Result<(), crate::StackWriteError> { Ok(()) },
    );
    assert_eq!((rod_damage, cancel), (4, false));
}

#[test]
fn an_ability_question_reaches_only_the_parts_of_the_asked_item_and_names_the_ability() {
    /// Grants `granted` and denies everything else `denied` names; keeps what it is not asked about.
    #[derive(Clone)]
    struct Abilities {
        granted: &'static str,
        denied: &'static str,
        asked: Arc<Mutex<Vec<String>>>,
    }
    #[mod_api::horizon_part]
    impl ItemPart for Abilities {
        fn can_perform_action(&self, _: &dyn Host, event: &mut CanPerformActionEvent<'_>) {
            self.asked.lock().unwrap().push(event.ability.to_owned());
            if event.ability == self.granted {
                event.can_perform = true;
            } else if event.ability == self.denied {
                event.can_perform = false;
            }
        }
    }
    struct Both(Abilities, Abilities);
    #[mod_api::__subscriptions]
    impl Hooks for Both {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let (stick, stone) = (r.item_key("minecraft:stick"), r.item_key("minecraft:stone"));
            r.item_part(stick, self.0.clone());
            r.item_part(stone, self.1.clone());
        }
    }
    // Another test installs parts with tags: a dispatch must run on a game thread.
    mark_game_thread();
    let sticks = Abilities {
        granted: "shield_block",
        denied: "shears_carve",
        asked: Default::default(),
    };
    let stones = Abilities {
        granted: "my_mod_chop",
        denied: "my_mod_chop_not",
        asked: Default::default(),
    };
    let mods: Mods = vec![ModEntry::new(
        "abilities",
        Box::new(Both(sticks.clone(), stones.clone())),
    )]
    .into();
    install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    let host = FakeHost::new();
    let stone = crate::parts::tests::stack("minecraft:stone", 1, 0);
    let diamond = crate::parts::tests::stack("minecraft:diamond", 1, 0);
    let ask = |stack: &ItemStack, ability: &str, vanilla: bool| {
        let mut can_perform = vanilla;
        dispatch_can_perform_action(&host, stack, ability, &mut can_perform);
        can_perform
    };
    // The part answers per ability: a grant, a denial and what it leaves alone.
    assert!(ask(&stick(), "shield_block", false));
    assert!(!ask(&stick(), "shears_carve", true));
    assert!(ask(&stick(), "shears_harvest", true));
    assert!(!ask(&stick(), "shears_harvest", false));
    // Another item's part hears only its own item, a part-less item keeps vanilla's answer.
    assert!(ask(&stone, "my_mod_chop", false));
    assert!(!ask(&stone, "shield_block", false));
    assert!(ask(&diamond, "shears_carve", true));
    assert!(!ask(&diamond, "shield_block", false));
    assert_eq!(
        *sticks.asked.lock().unwrap(),
        [
            "shield_block",
            "shears_carve",
            "shears_harvest",
            "shears_harvest"
        ]
    );
    assert_eq!(
        *stones.asked.lock().unwrap(),
        ["my_mod_chop", "shield_block"]
    );
}

/// Whether `stack` carries `minecraft:custom_data` (a "coin").
fn carries_custom_data(stack: &ItemStack) -> bool {
    stack.components().is_some_and(|c| {
        c.as_nbt()
            .contains_key(&horizon_core::nbt::JavaString::from(
                "minecraft:custom_data",
            ))
    })
}

/// `stack` with `minecraft:custom_data={coin:1b}`.
fn coin(item: &str) -> ItemStack {
    use horizon_core::nbt::{Compound, JavaString, Tag};
    let mut stack = crate::__private::part_tests::stack(item, 1, 0);
    let data = Compound::from([(JavaString::from("coin"), Tag::Byte(1))]);
    stack
        .set_components(Compound::from([(
            JavaString::from("minecraft:custom_data"),
            Tag::Compound(data),
        )]))
        .unwrap();
    stack
}

#[test]
fn piglin_answers_reach_only_the_asked_items_parts_and_write_back_both_ways() {
    /// Currency and gold armour exactly when the stack carries custom data; records the wearer.
    #[derive(Clone)]
    struct Coins(Arc<Mutex<Vec<u32>>>);
    #[mod_api::horizon_part]
    impl ItemPart for Coins {
        fn is_piglin_currency(&self, _: &dyn Host, event: &mut IsPiglinCurrencyEvent<'_>) {
            event.currency = carries_custom_data(event.stack);
        }
        fn makes_piglins_neutral(&self, _: &dyn Host, event: &mut MakesPiglinsNeutralEvent<'_>) {
            self.0.lock().unwrap().push(event.wearer.id());
            event.neutral = carries_custom_data(event.stack);
        }
    }
    struct CoinMod(Coins);
    #[mod_api::__subscriptions]
    impl Hooks for CoinMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let emerald = r.item_key("minecraft:emerald");
            r.item_part(emerald, self.0.clone());
        }
    }
    mark_game_thread();
    let wearers = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "coins",
        Box::new(CoinMod(Coins(wearers.clone()))),
    )]
    .into();
    install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    let host = FakeHost::new();
    let wearer = LivingRef::new(EntityRef::new(WorldEpoch::new(1), 77));
    let currency = |stack: &ItemStack, vanilla: bool| {
        let mut currency = vanilla;
        dispatch_is_piglin_currency(&host, stack, &mut currency);
        currency
    };
    let neutral = |stack: &ItemStack, vanilla: bool| {
        let mut neutral = vanilla;
        dispatch_makes_piglins_neutral(&host, stack, wearer, &mut neutral);
        neutral
    };
    let plain = crate::__private::part_tests::stack("minecraft:emerald", 1, 0);
    let gold = crate::__private::part_tests::stack("minecraft:gold_ingot", 1, 0);
    // The part's `true` and `false` both replace the answer it was given.
    assert!(currency(&coin("minecraft:emerald"), false));
    assert!(!currency(&plain, true));
    assert!(neutral(&coin("minecraft:emerald"), false));
    assert!(!neutral(&plain, true));
    assert_eq!(*wearers.lock().unwrap(), [77, 77]);
    // An item with no part keeps vanilla's answer, whatever it carries.
    assert!(currency(&gold, true));
    assert!(!currency(&coin("minecraft:gold_ingot"), false));
    assert!(!neutral(&coin("minecraft:gold_ingot"), false));
    assert_eq!(wearers.lock().unwrap().len(), 2);
}
