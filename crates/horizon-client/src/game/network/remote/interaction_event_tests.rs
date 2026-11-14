//! Behavioural proofs of the broadcast events around scoped interaction Parts.
#[path = "interaction_event_wasm_tests.rs"]
mod interaction_event_wasm_tests;
#[path = "item_fallthrough_tests.rs"]
mod item_fallthrough_tests;
#[path = "item_used_on_block_tests.rs"]
mod item_used_on_block_tests;
#[path = "predicted_use_tests.rs"]
mod predicted_use_tests;
#[path = "use_item_hand_tests.rs"]
mod use_item_hand_tests;
#[path = "use_item_result_tests.rs"]
mod use_item_result_tests;
use super::super::{
    Game, Hit, execution_role::ExecutionRole, lan::LanSettings,
    native_entity_refs::tests::with_game,
};
use horizon_client::{
    chunk::Chunk,
    first_person::Hand,
    items::ItemStack,
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
};
use horizon_core::geometry::Vec3;
use horizon_core::nbt::Tag;
use horizon_protocol::{connection::Pipe, packets::configuration::ClientInformation};
use mod_api::InteractionResult as Answer;
use mod_api::{
    self, BlockPart, ENTITY_ITEMS, Flow, Hooks, HostSide, HostTransaction, ItemPart, ItemResource,
    LivingEntityUseItemFinishEvent, LogicalSide, Registrar, RightClickBlockEvent,
    RightClickItemEvent, ServerHost, TriState, UseItemEvent, UseItemOnBlockEvent, UseItemOnEvent,
    UseOnEvent, UsePhase,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI32, AtomicUsize},
};

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Phase(UsePhase, LogicalSide),
    BlockPart,
    UseOnPart,
    FirstPart,
    Air(LogicalSide, Hand),
    ItemPart,
}

#[derive(Default)]
struct Controls {
    steps: Vec<Step>,
    /// Whether the onItemUseFirst moment is recorded in `steps`.
    record_first: bool,
    first_answer: Option<Answer>,
    first_part_answer: Option<Answer>,
    block_answer: Option<Answer>,
    item_on_answer: Option<Answer>,
    air_answer: Option<Answer>,
    air_main_only: bool,
    consume_item: bool,
    /// The block phase takes one item from the hand before its answer.
    take_in_block: bool,
    expected_hand: Option<Hand>,
    part_answer: Option<Answer>,
    part_pass: bool,
    /// Every hand `use_item_on_block` fired for, in order, instead of asserting one.
    hands: Option<Vec<Hand>>,
    use_block: TriState,
    use_item: TriState,
    /// Each `living_entity_use_item_finish`: side, hand and the item and count of the result it
    /// arrived with.
    finishes: Vec<(LogicalSide, Hand, Option<(String, u32)>)>,
    /// What a finish listener does to the result when set: `None` clears it, `Some(n)` sets its count.
    finish_replacement: Option<Option<u32>>,
    /// Cancel `right_click_block` with this answer.
    click_answer: Option<Answer>,
    /// How many `right_click_block` events fired.
    clicks: usize,
    click_hands: Vec<Hand>,
    use_item_packets: usize,
    /// The click is the host's own, not the fixture guest's: skip the guest identity asserts.
    host_click: bool,
    /// Each `use_item` event: side, hand, `player` and `player_entity_id`.
    item_users: Vec<(LogicalSide, Hand, u128, i32)>,
    /// How many `set_creative_mode_slot` packets the client sent.
    creative_slot_packets: usize,
    expect_empty_item_phase: bool,
}

#[derive(Clone, Default)]
struct Observer(Arc<Mutex<Controls>>);

fn side(host: &HostSide<'_>) -> LogicalSide {
    match host {
        HostSide::Server(_) => LogicalSide::Server,
        HostSide::Client(_) => LogicalSide::Client,
    }
}

#[mod_api::__subscriptions]
impl Hooks for Observer {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for block in ["minecraft:lever", "minecraft:stone", "minecraft:campfire"] {
            let block_key = r.block_key(block);
            r.block_part(block_key, self.clone());
        }
        for item in ["minecraft:stick", "minecraft:shield"] {
            let item = r.item_key(item);
            r.item_part(item, self.clone());
        }
    }
    fn client_packet_send(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::ClientPacketSendEvent<'_>,
    ) -> Flow {
        match event.packet.id.path() {
            "use_item" => self.0.lock().unwrap().use_item_packets += 1,
            "set_creative_mode_slot" => self.0.lock().unwrap().creative_slot_packets += 1,
            _ => {}
        }
        Flow::Continue
    }
    fn right_click_block(&self, _: HostSide<'_>, event: &mut RightClickBlockEvent<'_>) -> Flow {
        let mut state = self.0.lock().unwrap();
        state.clicks += 1;
        state.click_hands.push(event.click.hand);
        if let Some(answer) = state.click_answer {
            *event.cancellation_result = answer;
            return Flow::Cancel;
        }
        *event.use_block = state.use_block;
        *event.use_item = state.use_item;
        Flow::Continue
    }
    fn living_entity_use_item_finish(
        &self,
        host: HostSide<'_>,
        event: &mut LivingEntityUseItemFinishEvent<'_>,
    ) {
        let mut state = self.0.lock().unwrap();
        state.finishes.push((
            side(&host),
            event.hand,
            event.result.as_ref().map(|r| {
                let item = mod_api::__private::item_key_id(r.item()).unwrap_or("?");
                (item.to_owned(), r.count())
            }),
        ));
        match state.finish_replacement {
            Some(None) => *event.result = None,
            Some(Some(count)) => {
                if let Some(result) = event.result.as_mut() {
                    result.set_count(count).unwrap();
                }
            }
            None => {}
        }
    }
    fn use_item_on_block(&self, host: HostSide<'_>, event: &mut UseItemOnBlockEvent<'_>) -> Flow {
        assert_eq!(side(&host), event.click.side);
        assert_eq!(event.click.pos, [0, 64, 0]);
        assert_eq!(event.click.hit.face, [0, 1, 0]);
        if event.click.side == LogicalSide::Server && !self.0.lock().unwrap().host_click {
            assert_eq!(event.click.player, 12);
            assert_eq!(event.click.player_entity_id, 12);
        }
        assert_eq!(*event.cancellation_result, Answer::TryWithEmptyHand);
        let mut state = self.0.lock().unwrap();
        if state.expect_empty_item_phase && event.phase == UsePhase::ItemAfterBlock {
            assert!(event.click.stack.is_none());
        }
        if let Some(hands) = &mut state.hands {
            hands.push(event.click.hand);
        } else {
            assert_eq!(event.click.hand, state.expected_hand.unwrap_or(Hand::Main));
        }
        if event.phase != UsePhase::ItemBeforeBlock || state.record_first {
            state.steps.push(Step::Phase(event.phase, event.click.side));
        }
        let answer = match event.phase {
            UsePhase::Block => state.block_answer,
            UsePhase::ItemAfterBlock => state.item_on_answer,
            UsePhase::ItemBeforeBlock => state.first_answer,
            _ => unreachable!("UsePhase: unhandled variant"),
        };
        if let Some(answer) = answer {
            if ((state.consume_item && event.phase == UsePhase::ItemAfterBlock)
                || (state.take_in_block && event.phase == UsePhase::Block))
                && let HostSide::Server(host) = host
            {
                let player = host.player(event.click.player).unwrap();
                let items = player.entity().capability(host, &ENTITY_ITEMS, ()).unwrap();
                let slot = player.selected_slot(host).unwrap();
                let resource = ItemResource::of(event.click.stack.unwrap());
                host.transaction(|host, tx| {
                    assert_eq!(items.extract(host, tx, slot, &resource, 1), 1);
                    Ok::<_, ()>(())
                })
                .unwrap();
            }
            *event.cancellation_result = answer;
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn right_click_item(&self, host: HostSide<'_>, event: &mut RightClickItemEvent<'_>) -> Flow {
        assert_eq!(side(&host), event.click.side);
        assert_eq!(event.click.pos, [0, 64, 3]);
        if event.click.side == LogicalSide::Server && !self.0.lock().unwrap().host_click {
            assert_eq!(event.click.player, 12);
            assert_eq!(event.click.player_entity_id, 12);
        }
        assert_eq!(*event.cancellation_result, Answer::Pass);
        let mut state = self.0.lock().unwrap();
        state
            .steps
            .push(Step::Air(event.click.side, event.click.hand));
        if let Some(answer) = state.air_answer
            && (!state.air_main_only || event.click.hand == Hand::Main)
        {
            *event.cancellation_result = answer;
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}
#[mod_api::horizon_part]
impl BlockPart for Observer {
    fn use_item_on(&self, _: &mut dyn ServerHost, _: &mut UseItemOnEvent<'_>) -> Option<Answer> {
        let mut state = self.0.lock().unwrap();
        state.steps.push(Step::BlockPart);
        None
    }
}
#[mod_api::horizon_part]
impl ItemPart for Observer {
    fn on_item_use_first(
        &self,
        _: &mut dyn ServerHost,
        _: &mut mod_api::OnItemUseFirstEvent<'_>,
    ) -> Option<Answer> {
        let mut state = self.0.lock().unwrap();
        if state.record_first {
            state.steps.push(Step::FirstPart);
        }
        state.first_part_answer
    }
    fn use_on(&self, host: HostSide<'_>, _: &mut UseOnEvent<'_>) -> Option<Answer> {
        let HostSide::Server(_) = host else {
            return None;
        };
        self.0.lock().unwrap().steps.push(Step::UseOnPart);
        Some(Answer::SuccessServer)
    }
    fn use_item(&self, host: HostSide<'_>, event: &mut UseItemEvent<'_>) -> Option<Answer> {
        if let Some(hand) = self.0.lock().unwrap().expected_hand {
            assert_eq!(event.hand, hand);
        }
        let mut state = self.0.lock().unwrap();
        state.item_users.push((
            side(&host),
            event.hand,
            event.player,
            event.player_entity_id,
        ));
        state.steps.push(Step::ItemPart);
        if state.part_pass {
            None
        } else {
            Some(state.part_answer.unwrap_or(Answer::SuccessServer))
        }
    }
}

// The same real Game runs either its server path or its normal local-client
// connection/prediction path, as the existing remote menu fixture does.
fn with_interaction_game(client: bool, test: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    with_interaction_game_at(client, root, test);
}

fn with_interaction_game_at(
    client: bool,
    root: std::path::PathBuf,
    test: impl FnOnce(&mut Game<'_>),
) {
    super::super::native_entity_refs::tests::with_game_at(root, |game| {
        let _peer = if client {
            game.options.execution_role = ExecutionRole::Client;
            let (client, server) = Pipe::pair();
            game.connect_local(client, "InteractionTest").unwrap();
            Some(server)
        } else {
            game.options.dedicated = Some(Vec::new());
            None
        };
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.5)).unwrap());
        game.set_mode_id(1);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        if client {
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
            test(game);
        } else {
            let _receiver = setup_interaction_guest(game);
            game.as_guest(12, |game| {
                game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
                test(game);
            })
            .unwrap();
        }
    });
}

fn hit() -> Hit {
    Hit {
        pos: [0, 64, 0],
        face: [0, 1, 0],
        point: [0.5, 65.0, 0.5],
        inside: false,
    }
}

fn use_block(game: &mut Game<'_>, client: bool) -> bool {
    if client {
        game.remote_use_item_on(Hand::Main)
    } else {
        game.guest_use = Some(super::super::GuestUse::OnBlock(Hand::Main));
        game.use_hand(Some(&hit())).unwrap()
    }
}

#[test]
#[ignore = "requires prepared game resources"]
fn right_click_block_forced_item_use_reaches_empty_hand_on_both_sides() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "empty_hand_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        for client in [false, true] {
            with_interaction_game(client, |game| {
                game.inventory[game.selected] = None;
                game.forced_hit = Some(hit());
                let side = if client {
                    LogicalSide::Client
                } else {
                    LogicalSide::Server
                };
                // RightClickBlock's TRUE enters useOn even for an empty stack;
                // Default and FALSE do not. A phase listener supplies the use.
                for (use_item, used) in [
                    (TriState::True, true),
                    (TriState::Default, false),
                    (TriState::False, false),
                ] {
                    *observer.0.lock().unwrap() = Controls {
                        use_block: TriState::False,
                        use_item,
                        item_on_answer: Some(Answer::Consume),
                        expect_empty_item_phase: true,
                        ..Controls::default()
                    };
                    assert_eq!(use_block(game, client), used);
                    let expected = if used {
                        vec![Step::Phase(UsePhase::ItemAfterBlock, side)]
                    } else {
                        vec![]
                    };
                    assert_eq!(observer.0.lock().unwrap().steps, expected);
                    assert!(game.inventory[game.selected].is_none());
                }
                // Forcing an empty use without a phase answer still passes:
                // there is no stack-specific Part or vanilla item to invoke.
                *observer.0.lock().unwrap() = Controls {
                    use_block: TriState::False,
                    use_item: TriState::True,
                    expect_empty_item_phase: true,
                    ..Controls::default()
                };
                assert!(!use_block(game, client));
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Phase(UsePhase::ItemAfterBlock, side)]
                );
                // TRUE bypasses the stack/cooldown gate, not adventure's
                // placement predicate (which an empty hand cannot satisfy).
                game.set_mode_id(2);
                *observer.0.lock().unwrap() = Controls {
                    use_block: TriState::False,
                    use_item: TriState::True,
                    item_on_answer: Some(Answer::Consume),
                    ..Controls::default()
                };
                assert!(!use_block(game, client));
                assert!(observer.0.lock().unwrap().steps.is_empty());
            });
        }
    });
}

#[test]
#[ignore = "requires prepared game resources"]
fn right_click_block_explicit_allow_overrides_sneak_skip_on_both_sides() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "sneak_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        for client in [false, true] {
            with_interaction_game(client, |game| {
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let lever = game
                    .with_props(
                        registry.default_state("minecraft:lever").unwrap(),
                        &[("face", "floor")],
                    )
                    .unwrap();
                game.world.set(0, 63, 0, stone).unwrap();
                game.forced_hit = Some(hit());
                game.keys.insert(winit::keyboard::KeyCode::ShiftLeft);
                if let Some(remote) = game.remote.as_mut() {
                    remote.last_input |= super::SHIFT_INPUT;
                }
                // Either hand being full activates the vanilla secondary-use
                // skip. TRUE overrides it; Default and FALSE skip the lever.
                for item_in_main in [true, false] {
                    game.inventory[game.selected] =
                        item_in_main.then(|| ItemStack::new("minecraft:stick", 1));
                    game.offhand = (!item_in_main).then(|| ItemStack::new("minecraft:stick", 1));
                    for (switch, used) in [
                        (TriState::True, true),
                        (TriState::Default, false),
                        (TriState::False, false),
                    ] {
                        game.world.set(0, 64, 0, lever).unwrap();
                        *observer.0.lock().unwrap() = Controls {
                            use_block: switch,
                            use_item: TriState::False,
                            ..Controls::default()
                        };
                        assert_eq!(use_block(game, client), used);
                        assert_eq!(game.world.block(0, 64, 0) != lever, used);
                        let side = if client {
                            LogicalSide::Client
                        } else {
                            LogicalSide::Server
                        };
                        let mut expected = if used {
                            vec![Step::Phase(UsePhase::Block, side)]
                        } else {
                            vec![]
                        };
                        if used && !client {
                            expected.push(Step::BlockPart);
                        }
                        assert_eq!(observer.0.lock().unwrap().steps, expected);
                    }
                }
            });
        }
    });
}

#[test]
#[ignore = "requires prepared game resources"]
fn phase_events_replace_the_behaviour_before_its_part_on_both_sides() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "phase_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        for client in [false, true] {
            with_interaction_game(client, |game| {
                let side = if client {
                    LogicalSide::Client
                } else {
                    LogicalSide::Server
                };
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let lever = game
                    .with_props(
                        registry.default_state("minecraft:lever").unwrap(),
                        &[("face", "floor")],
                    )
                    .unwrap();
                game.world.set(0, 63, 0, stone).unwrap();
                game.world.set(0, 64, 0, lever).unwrap();
                game.forced_hit = Some(hit());
                *observer.0.lock().unwrap() = Controls::default();
                assert!(use_block(game, client));
                assert_ne!(game.world.block(0, 64, 0), lever);
                let mut expected = vec![Step::Phase(UsePhase::Block, side)];
                if !client {
                    expected.push(Step::BlockPart);
                }
                assert_eq!(observer.0.lock().unwrap().steps, expected);

                for answer in [Answer::Success, Answer::SuccessServer, Answer::Consume] {
                    game.world.set(0, 64, 0, lever).unwrap();
                    *observer.0.lock().unwrap() = Controls {
                        block_answer: Some(answer),
                        ..Controls::default()
                    };
                    let swings = game.swings_started;
                    assert!(use_block(game, client));
                    assert_eq!(game.world.block(0, 64, 0), lever);
                    assert_eq!(
                        observer.0.lock().unwrap().steps,
                        [Step::Phase(UsePhase::Block, side)]
                    );
                    let swings = game.swings_started - swings;
                    assert_eq!(
                        swings,
                        u32::from(
                            answer == Answer::Success
                                || (!client && answer == Answer::SuccessServer)
                        )
                    );
                }

                // TRY skips the item method/Part, but still delegates the
                // block's empty method on the main hand, on both sides.
                *observer.0.lock().unwrap() = Controls {
                    block_answer: Some(Answer::TryWithEmptyHand),
                    ..Controls::default()
                };
                assert!(use_block(game, client));
                assert_ne!(game.world.block(0, 64, 0), lever);
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Phase(UsePhase::Block, side)]
                );

                for answer in [Answer::Pass, Answer::Fail] {
                    game.world.set(0, 64, 0, lever).unwrap();
                    *observer.0.lock().unwrap() = Controls {
                        block_answer: Some(answer),
                        item_on_answer: Some(Answer::Consume),
                        ..Controls::default()
                    };
                    assert!(use_block(game, client));
                    assert_eq!(game.world.block(0, 64, 0), lever);
                    assert_eq!(
                        observer.0.lock().unwrap().steps,
                        [
                            Step::Phase(UsePhase::Block, side),
                            Step::Phase(UsePhase::ItemAfterBlock, side),
                        ]
                    );
                }

                // A passing block gets the item phase, before the item Part.
                game.world.set(0, 64, 0, stone).unwrap();
                *observer.0.lock().unwrap() = Controls::default();
                let _ = use_block(game, client);
                let mut expected = vec![Step::Phase(UsePhase::Block, side)];
                if !client {
                    expected.push(Step::BlockPart);
                }
                expected.push(Step::Phase(UsePhase::ItemAfterBlock, side));
                if !client {
                    expected.push(Step::UseOnPart);
                }
                assert_eq!(observer.0.lock().unwrap().steps, expected);

                for answer in [
                    Answer::Success,
                    Answer::SuccessServer,
                    Answer::Consume,
                    Answer::Pass,
                    Answer::Fail,
                    Answer::TryWithEmptyHand,
                ] {
                    *observer.0.lock().unwrap() = Controls {
                        item_on_answer: Some(answer),
                        ..Controls::default()
                    };
                    let swings = game.swings_started;
                    assert_eq!(
                        use_block(game, client),
                        answer.consumes_action() || answer == Answer::Fail
                    );
                    let mut expected = vec![Step::Phase(UsePhase::Block, side)];
                    if !client {
                        expected.push(Step::BlockPart);
                    }
                    expected.push(Step::Phase(UsePhase::ItemAfterBlock, side));
                    assert_eq!(observer.0.lock().unwrap().steps, expected);
                    assert_eq!(
                        game.swings_started - swings,
                        u32::from(
                            answer == Answer::Success
                                || (!client && answer == Answer::SuccessServer)
                        )
                    );
                }

                // The phase event is later than RightClickBlock's choices.
                *observer.0.lock().unwrap() = Controls {
                    use_block: TriState::False,
                    use_item: TriState::False,
                    ..Controls::default()
                };
                assert!(!use_block(game, client));
                assert!(observer.0.lock().unwrap().steps.is_empty());

                // ItemStack.useOn passes before Item.useOn in adventure mode.
                game.set_mode_id(2);
                *observer.0.lock().unwrap() = Controls {
                    use_block: TriState::False,
                    ..Controls::default()
                };
                assert!(!use_block(game, client));
                assert!(observer.0.lock().unwrap().steps.is_empty());
                game.inventory[game.selected]
                    .as_mut()
                    .unwrap()
                    .data_mut()
                    .set_other(
                        "minecraft:can_place_on",
                        Some(Tag::Compound(
                            [("blocks".into(), Tag::String("minecraft:stone".into()))].into(),
                        )),
                    );
                *observer.0.lock().unwrap() = Controls {
                    use_block: TriState::False,
                    item_on_answer: Some(Answer::Success),
                    ..Controls::default()
                };
                assert!(use_block(game, client));
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Phase(UsePhase::ItemAfterBlock, side)]
                );
                game.set_mode_id(1);

                // In 26.3 dousing is CampfireBlock.useItemOn, not Item.useOn:
                // a canceled BLOCK cannot predict it or reach ITEM_AFTER_BLOCK.
                let campfire = registry.default_state("minecraft:campfire").unwrap();
                game.world.set(0, 64, 0, campfire).unwrap();
                game.inventory[game.selected] = Some(ItemStack::new("minecraft:iron_shovel", 1));
                *observer.0.lock().unwrap() = Controls {
                    block_answer: Some(Answer::Success),
                    ..Controls::default()
                };
                assert!(use_block(game, client));
                assert_eq!(game.world.block(0, 64, 0), campfire);
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Phase(UsePhase::Block, side)]
                );
                *observer.0.lock().unwrap() = Controls::default();
                assert!(use_block(game, client));
                assert_ne!(game.world.block(0, 64, 0), campfire);
                let mut expected = vec![Step::Phase(UsePhase::Block, side)];
                if !client {
                    expected.push(Step::BlockPart);
                }
                assert_eq!(observer.0.lock().unwrap().steps, expected);

                game.offhand = Some(ItemStack::new("minecraft:stick", 1));
                *observer.0.lock().unwrap() = Controls {
                    block_answer: Some(Answer::Success),
                    expected_hand: Some(Hand::Off),
                    ..Controls::default()
                };
                let used = if client {
                    game.remote_use_item_on(Hand::Off)
                } else {
                    game.guest_use = Some(super::super::GuestUse::OnBlock(Hand::Off));
                    game.with_hand(Hand::Off, |g| g.use_hand(Some(&hit())).unwrap())
                };
                assert!(used);
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Phase(UsePhase::Block, side)]
                );
            });
        }
    });
}

#[test]
#[ignore = "requires prepared game resources"]
fn canceled_item_results_keep_the_stack_wrappers_side_effects() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "wrapper_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        let cooling_stack = |item: &str| {
            let mut stack = ItemStack::new(item, 1);
            stack.data_mut().set_other(
                "minecraft:use_cooldown",
                Some(Tag::Compound(
                    [
                        ("seconds".into(), Tag::Float(1.0_f32.to_bits())),
                        (
                            "cooldown_group".into(),
                            Tag::String("test:item_events".into()),
                        ),
                    ]
                    .into(),
                )),
            );
            stack
        };
        for client in [false, true] {
            with_interaction_game(client, |game| {
                game.set_mode_id(0);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                game.forced_hit = Some(hit());
                for answer in [
                    Answer::Success,
                    Answer::SuccessServer,
                    Answer::Consume,
                    Answer::Pass,
                    Answer::Fail,
                    Answer::TryWithEmptyHand,
                ] {
                    game.cooldowns.remove("test:item_events");
                    game.inventory[game.selected] = Some(cooling_stack("minecraft:milk_bucket"));
                    *observer.0.lock().unwrap() = Controls {
                        use_block: TriState::False,
                        item_on_answer: Some(answer),
                        consume_item: answer.consumes_action(),
                        ..Controls::default()
                    };
                    let used = game.stats.get("used", "minecraft:milk_bucket");
                    let _ = use_block(game, client);
                    assert_eq!(
                        game.stats.get("used", "minecraft:milk_bucket"),
                        used + if !client && answer.consumes_action() {
                            1
                        } else {
                            0
                        }
                    );
                    assert_eq!(
                        game.cooldowns.is_on_cooldown("test:item_events"),
                        answer.consumes_action()
                    );
                    assert_eq!(
                        game.held().unwrap().item.as_ref(),
                        if !client && answer.consumes_action() {
                            "minecraft:bucket"
                        } else {
                            "minecraft:milk_bucket"
                        }
                    );
                }
                game.forced_hit = None;
                game.guest_use = None;
                for item in ["minecraft:stick", "minecraft:shield"] {
                    for answer in [Answer::Success, Answer::SuccessServer, Answer::Consume] {
                        game.cooldowns.remove("test:item_events");
                        game.inventory[game.selected] = Some(cooling_stack(item));
                        *observer.0.lock().unwrap() = Controls {
                            air_answer: Some(answer),
                            ..Controls::default()
                        };
                        let used = game.stats.get("used", item);
                        if client {
                            game.remote_use_item(Hand::Main);
                        } else {
                            game.use_hand(None).unwrap();
                        }
                        // ItemStack.use applies components only to instant use,
                        // and unlike useOn does not award the used statistic.
                        assert_eq!(game.stats.get("used", item), used);
                        assert_eq!(
                            game.cooldowns.is_on_cooldown("test:item_events"),
                            item == "minecraft:stick"
                        );
                        assert!(game.using.is_none());
                    }
                }
            });
        }
    });
}

/// A spectator's click fires `right_click_block` on the client like anyone's, and a cancel
/// answers in place of the server's menu turn: SUCCESS swings and ends the click, FAIL ends it,
/// PASS goes on to the other hand.
#[test]
#[ignore = "requires prepared game resources"]
fn a_spectator_fires_right_click_block_on_the_client_and_its_cancel_answers() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "spectator_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            game.set_mode_id(3);
            assert!(game.spectator());
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.forced_hit = Some(hit());
            // Nobody answers: the event fired, and the spectator's click ends there, no swing.
            *observer.0.lock().unwrap() = Controls::default();
            let swings = game.swings_started;
            assert!(game.remote_use_item_on(Hand::Main));
            assert_eq!(observer.0.lock().unwrap().clicks, 1);
            assert_eq!(game.swings_started, swings);
            for (answer, ends, swings_by) in [
                (Answer::Success, true, 1),
                (Answer::Consume, true, 0),
                (Answer::Fail, true, 0),
                (Answer::Pass, false, 0),
            ] {
                *observer.0.lock().unwrap() = Controls {
                    click_answer: Some(answer),
                    ..Controls::default()
                };
                let swings = game.swings_started;
                assert_eq!(game.remote_use_item_on(Hand::Main), ends, "{answer:?}");
                assert_eq!(game.swings_started - swings, swings_by, "{answer:?}");
                assert_eq!(observer.0.lock().unwrap().clicks, 1, "{answer:?}");
            }
        });
    });
}

/// A cancelled block PASS tries both hands, but neither may send or predict item use.
#[test]
#[ignore = "requires prepared game resources"]
fn a_spectators_passing_block_click_skips_item_use_for_both_hands() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "spectator_pass_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            game.set_mode_id(3);
            assert!(game.spectator());
            game.offhand = Some(ItemStack::new("minecraft:shield", 1));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.forced_hit = Some(hit());
            *observer.0.lock().unwrap() = Controls {
                click_answer: Some(Answer::Pass),
                // Let an erroneous item event reach the Part, which would consume the click.
                part_answer: Some(Answer::Success),
                ..Controls::default()
            };
            let swings = game.swings_started;
            // The real target turn precedes remote_use_hands' item/off-hand sequence.
            assert!(!game.remote_use_item_on(Hand::Main));
            game.remote_use_hands(mod_api::InteractionAttempt::Press);
            let state = observer.0.lock().unwrap();
            assert_eq!(state.clicks, 2);
            assert_eq!(state.click_hands, [Hand::Main, Hand::Off]);
            assert_eq!(state.use_item_packets, 0);
            assert_eq!(
                state
                    .steps
                    .iter()
                    .filter(|s| matches!(s, Step::Air(_, Hand::Main)))
                    .count(),
                0
            );
            assert_eq!(
                state
                    .steps
                    .iter()
                    .filter(|s| matches!(s, Step::Air(_, Hand::Off)))
                    .count(),
                0
            );
            assert_eq!(
                state
                    .steps
                    .iter()
                    .filter(|s| matches!(s, Step::ItemPart))
                    .count(),
                0
            );
            assert_eq!(game.swings_started - swings, 0);
            assert!(game.using.is_none());
        });
    });
}

#[test]
#[ignore = "requires prepared game resources"]
fn air_event_skips_the_item_and_consuming_results_end_the_click() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "air_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        for client in [false, true] {
            with_interaction_game(client, |game| {
                let side = if client {
                    LogicalSide::Client
                } else {
                    LogicalSide::Server
                };
                if client {
                    game.offhand = Some(ItemStack::new("minecraft:shield", 1));
                }
                for (answer, consumes, client_swings, server_swings) in [
                    (Answer::Success, true, 1, 0),
                    (Answer::SuccessServer, true, 0, 1),
                    (Answer::Consume, true, 0, 0),
                    (Answer::Pass, false, 0, 0),
                    (Answer::Fail, false, 0, 0),
                    (Answer::TryWithEmptyHand, false, 0, 0),
                ] {
                    *observer.0.lock().unwrap() = Controls {
                        air_answer: Some(answer),
                        ..Controls::default()
                    };
                    game.swing = Default::default();
                    let swings = game.swings_started;
                    let mut expected = vec![Step::Air(side, Hand::Main)];
                    if client {
                        game.remote_use(mod_api::InteractionAttempt::Press);
                        if !consumes {
                            expected.push(Step::Air(side, Hand::Off));
                        }
                        assert!(game.using.is_none(), "{answer:?}");
                    } else {
                        assert_eq!(game.use_hand(None).unwrap(), consumes, "{answer:?}");
                    }
                    assert_eq!(
                        game.swings_started - swings,
                        if client { client_swings } else { server_swings },
                        "{side:?} {answer:?}"
                    );
                    assert_eq!(
                        observer.0.lock().unwrap().steps,
                        expected,
                        "{side:?} {answer:?}"
                    );
                }
                if client {
                    *observer.0.lock().unwrap() = Controls {
                        air_answer: Some(Answer::Pass),
                        air_main_only: true,
                        part_pass: true,
                        ..Controls::default()
                    };
                    game.remote_use(mod_api::InteractionAttempt::Press);
                    assert!(
                        game.using.is_some(),
                        "passing main hand must allow the shield to rise"
                    );
                    assert_eq!(
                        observer.0.lock().unwrap().steps,
                        [
                            Step::Air(side, Hand::Main),
                            Step::Air(side, Hand::Off),
                            Step::ItemPart
                        ]
                    );
                }
                if !client {
                    *observer.0.lock().unwrap() = Controls::default();
                    assert!(game.use_hand(None).unwrap());
                    assert_eq!(
                        observer.0.lock().unwrap().steps,
                        [Step::Air(side, Hand::Main), Step::ItemPart]
                    );
                }
                *observer.0.lock().unwrap() = Controls::default();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                game.forced_hit = Some(hit());
                // A passing target permits the same hand's Item.use hook.
                assert!(
                    game.right_click_item(Hand::Main, &ItemStack::new("minecraft:stick", 1))
                        .is_none()
                );
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Air(side, Hand::Main)]
                );
                observer.0.lock().unwrap().steps.clear();
                game.forced_hit = None;
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                game.mobs.push(horizon_client::mobs::Mob::new(
                    42,
                    kind,
                    Vec3::new(0.5, 64.0, 5.5),
                    0.0,
                ));
                assert!(game.entity_targeted());
                assert!(
                    game.right_click_item(Hand::Main, &ItemStack::new("minecraft:stick", 1))
                        .is_none()
                );
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Air(side, Hand::Main)]
                );
            });
        }
    });
}

#[test]
#[ignore = "requires prepared game resources"]
fn before_block_cancellation_pass_continues_but_fail_and_consuming_results_stop() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "before_block_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        for client in [false, true] {
            with_interaction_game(client, |game| {
                let side = if client {
                    LogicalSide::Client
                } else {
                    LogicalSide::Server
                };
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let lever = game
                    .with_props(
                        registry.default_state("minecraft:lever").unwrap(),
                        &[("face", "floor")],
                    )
                    .unwrap();
                game.world.set(0, 63, 0, stone).unwrap();
                game.forced_hit = Some(hit());
                // Reference rules: PASS reaches the block; FAIL and each
                // consuming result stop before it. Expected values are literal,
                // independent of InteractionResult::consumes_action.
                for (answer, reaches_block, swings) in [
                    (Answer::Pass, true, 1),
                    (Answer::Fail, false, 0),
                    (Answer::TryWithEmptyHand, false, 0),
                    (Answer::Success, false, 1),
                    (Answer::SuccessServer, false, u32::from(!client)),
                    (Answer::Consume, false, 0),
                ] {
                    game.world.set(0, 64, 0, lever).unwrap();
                    *observer.0.lock().unwrap() = Controls {
                        record_first: true,
                        first_answer: Some(answer),
                        // Cancellation must skip this Part even for PASS.
                        first_part_answer: Some(Answer::Fail),
                        ..Controls::default()
                    };
                    let before = game.swings_started;
                    assert_eq!(
                        use_block(game, client),
                        answer != Answer::TryWithEmptyHand,
                        "{side:?}: {answer:?}"
                    );
                    assert_eq!(game.swings_started - before, swings);
                    assert_eq!(game.world.block(0, 64, 0) != lever, reaches_block);
                    let mut expected = vec![Step::Phase(UsePhase::ItemBeforeBlock, side)];
                    if reaches_block {
                        expected.push(Step::Phase(UsePhase::Block, side));
                        if !client {
                            expected.push(Step::BlockPart);
                        }
                    }
                    assert_eq!(observer.0.lock().unwrap().steps, expected);
                }
                // A block that passes also reaches the after-block item phase.
                game.world.set(0, 64, 0, stone).unwrap();
                *observer.0.lock().unwrap() = Controls {
                    record_first: true,
                    first_answer: Some(Answer::Pass),
                    first_part_answer: Some(Answer::Fail),
                    item_on_answer: Some(Answer::Consume),
                    ..Controls::default()
                };
                assert!(use_block(game, client));
                assert_eq!(game.world.block(0, 64, 0), stone);
                let mut expected = vec![
                    Step::Phase(UsePhase::ItemBeforeBlock, side),
                    Step::Phase(UsePhase::Block, side),
                ];
                if !client {
                    expected.push(Step::BlockPart);
                }
                expected.push(Step::Phase(UsePhase::ItemAfterBlock, side));
                assert_eq!(observer.0.lock().unwrap().steps, expected);
            });
        }
    });
}

#[test]
#[ignore = "requires prepared game resources"]
fn on_item_use_first_runs_before_the_block_and_ends_the_click() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "first_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(false, |game| {
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let lever = game
                .with_props(
                    registry.default_state("minecraft:lever").unwrap(),
                    &[("face", "floor")],
                )
                .unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 64, 0, lever).unwrap();
            game.forced_hit = Some(hit());
            let server = LogicalSide::Server;

            // Every non-PASS Part answer stops before the block, including TRY.
            for (answer, used, swings) in [
                (Answer::Success, true, 1),
                (Answer::SuccessServer, true, 1),
                (Answer::Consume, true, 0),
                (Answer::Fail, true, 0),
                (Answer::TryWithEmptyHand, false, 0),
            ] {
                *observer.0.lock().unwrap() = Controls {
                    record_first: true,
                    first_part_answer: Some(answer),
                    ..Controls::default()
                };
                let before = game.swings_started;
                assert_eq!(use_block(game, false), used);
                assert_eq!(game.swings_started - before, swings);
                assert_eq!(game.world.block(0, 64, 0), lever);
                let first = Step::Phase(UsePhase::ItemBeforeBlock, server);
                assert_eq!(observer.0.lock().unwrap().steps, [first, Step::FirstPart]);
            }

            // The broadcast event goes first: its cancel stops the Part.
            *observer.0.lock().unwrap() = Controls {
                record_first: true,
                first_answer: Some(Answer::Success),
                first_part_answer: Some(Answer::Success),
                ..Controls::default()
            };
            assert!(use_block(game, false));
            assert_eq!(game.world.block(0, 64, 0), lever);
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Phase(UsePhase::ItemBeforeBlock, server)]
            );

            // A pass goes on: the block's phase and the lever flip.
            *observer.0.lock().unwrap() = Controls {
                record_first: true,
                first_part_answer: Some(Answer::Pass),
                ..Controls::default()
            };
            assert!(use_block(game, false));
            assert_ne!(game.world.block(0, 64, 0), lever);
            assert_eq!(
                observer.0.lock().unwrap().steps[..3],
                [
                    Step::Phase(UsePhase::ItemBeforeBlock, server),
                    Step::FirstPart,
                    Step::Phase(UsePhase::Block, server),
                ]
            );
        });
    });
}

#[test]
#[ignore = "requires prepared game resources"]
fn an_item_used_from_the_off_hand_reaches_the_use_hooks_on_the_server() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "off_hand_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(false, |game| {
            let side = LogicalSide::Server;
            game.offhand = Some(ItemStack::new("minecraft:stick", 1));

            // An empty main hand passes, and the off hand uses its item.
            game.inventory[game.selected] = None;
            *observer.0.lock().unwrap() = Controls::default();
            assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Off));
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Air(side, Hand::Off), Step::ItemPart]
            );

            // A main hand item nothing answers for passes too: both hands' hooks, in order.
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:stone", 1));
            *observer.0.lock().unwrap() = Controls::default();
            assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Off));
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [
                    Step::Air(side, Hand::Main),
                    Step::Air(side, Hand::Off),
                    Step::ItemPart
                ]
            );

            // A guest's use packet names the off hand alone.
            game.inventory[game.selected] = None;
            *observer.0.lock().unwrap() = Controls::default();
            game.guest_use = Some(super::super::GuestUse::Item(Hand::Off));
            assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Off));
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Air(side, Hand::Off), Step::ItemPart]
            );

            // A block click: the off hand's `use_item_on_block` phases and its item Part's `use_on`,
            // for the local player and for a guest's use-on-block packet.
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.forced_hit = Some(hit());
            for guest in [None, Some(super::super::GuestUse::OnBlock(Hand::Off))] {
                game.inventory[game.selected] = None;
                *observer.0.lock().unwrap() = Controls {
                    hands: Some(Vec::new()),
                    ..Controls::default()
                };
                game.guest_use = guest;
                assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Off));
                let state = observer.0.lock().unwrap();
                assert!(
                    state
                        .hands
                        .as_ref()
                        .unwrap()
                        .iter()
                        .all(|h| *h == Hand::Off || guest.is_none())
                );
                assert_eq!(state.hands.as_ref().unwrap().last(), Some(&Hand::Off));
                assert_eq!(state.steps.last(), Some(&Step::UseOnPart));
            }
        });
    });
}

fn setup_interaction_guest(
    game: &mut Game<'_>,
) -> std::sync::mpsc::Receiver<horizon_client::lan::Out> {
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            lod_distance: 0,
            ..NetConfig::lan(0, String::new(), 2)
        },
        HubConfig {
            default_game_mode: 1,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: game.dim.id().to_owned(),
            allow_flight: true,
            seed: 0,
            max_players: 2,
            integrated: true,
            levels: game.level_shapes,
        },
    )
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 12,
            uuid: 12,
            name: "interaction_guest".into(),
            properties: Vec::new(),
            info: ClientInformation::new(8),
            brand: String::new(),
            sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:1".parse().unwrap(),
            expected: false,
        },
        12,
        [0.5, 64.0, 3.5],
        10,
    );
    guest.placed = true;
    guest.game_mode = 1;
    game.lan.as_mut().unwrap().hub.guests.push(guest);
    rx
}

#[test]
#[ignore = "requires prepared game resources"]
fn a_server_client_fires_use_item_finish_when_the_server_completes_the_use() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "finish_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            let offhand = super::super::shield::OFFHAND_SLOT;
            // (slot, creative, used stack, replacement, what the listener sees, what the slot
            // holds after), the expectations from vanilla's client finishUsingItem.
            let cases: [(
                usize,
                bool,
                (&str, u32),
                Option<Option<u32>>,
                Option<(&str, u32)>,
                Option<(&str, u32)>,
            ); 9] = [
                // A consumable loses one.
                (
                    0,
                    false,
                    ("minecraft:apple", 3),
                    None,
                    Some(("minecraft:apple", 2)),
                    Some(("minecraft:apple", 2)),
                ),
                // Nothing is used up in creative mode.
                (
                    0,
                    true,
                    ("minecraft:apple", 3),
                    None,
                    Some(("minecraft:apple", 3)),
                    Some(("minecraft:apple", 3)),
                ),
                // The last stew leaves its use remainder, the bowl, in the hand.
                (
                    0,
                    false,
                    ("minecraft:mushroom_stew", 1),
                    None,
                    Some(("minecraft:bowl", 1)),
                    Some(("minecraft:bowl", 1)),
                ),
                // A spyglass is not a consumable: it comes back as it was.
                (
                    0,
                    false,
                    ("minecraft:spyglass", 1),
                    None,
                    Some(("minecraft:spyglass", 1)),
                    Some(("minecraft:spyglass", 1)),
                ),
                // The last apple leaves nothing.
                (0, false, ("minecraft:apple", 1), None, None, None),
                // A replacement goes into the hand: survival, cleared, creative.
                (
                    0,
                    false,
                    ("minecraft:apple", 3),
                    Some(Some(7)),
                    Some(("minecraft:apple", 2)),
                    Some(("minecraft:apple", 7)),
                ),
                (
                    0,
                    false,
                    ("minecraft:apple", 3),
                    Some(None),
                    Some(("minecraft:apple", 2)),
                    None,
                ),
                (
                    0,
                    true,
                    ("minecraft:apple", 3),
                    Some(Some(7)),
                    Some(("minecraft:apple", 3)),
                    Some(("minecraft:apple", 7)),
                ),
                // The off hand, with a replacement.
                (
                    offhand,
                    false,
                    ("minecraft:apple", 2),
                    Some(Some(5)),
                    Some(("minecraft:apple", 1)),
                    Some(("minecraft:apple", 5)),
                ),
            ];
            let owned = |s: Option<(&str, u32)>| s.map(|(item, count)| (item.to_owned(), count));
            for (slot, creative, (item, count), replacement, seen, after) in cases {
                game.creative = creative;
                game.inventory[game.selected] = None;
                game.offhand = None;
                let slot = if slot == offhand {
                    offhand
                } else {
                    game.selected
                };
                *game.stack_in_mut(slot) = Some(ItemStack::new(item, count));
                game.using = Some((slot, 5));
                *observer.0.lock().unwrap() = Controls {
                    finish_replacement: replacement,
                    ..Controls::default()
                };
                game.remote_complete_using();
                let hand = if slot == offhand {
                    Hand::Off
                } else {
                    Hand::Main
                };
                assert_eq!(
                    observer.0.lock().unwrap().finishes,
                    [(LogicalSide::Client, hand, owned(seen))],
                    "{item} x{count} creative={creative} {replacement:?}"
                );
                let held = game.stack_in(slot).map(|s| (s.item.to_string(), s.count));
                assert_eq!(
                    held,
                    owned(after),
                    "{item} x{count} creative={creative} {replacement:?}"
                );
            }
            // The stack's use cooldown starts with the finish.
            game.creative = false;
            let mut cooling = ItemStack::new("minecraft:apple", 2);
            cooling.data_mut().set_other(
                "minecraft:use_cooldown",
                Some(Tag::Compound(
                    [
                        ("seconds".into(), Tag::Float(1.0_f32.to_bits())),
                        ("cooldown_group".into(), Tag::String("test:finish".into())),
                    ]
                    .into(),
                )),
            );
            game.cooldowns.remove("test:finish");
            game.inventory[game.selected] = Some(cooling);
            game.using = Some((game.selected, 5));
            *observer.0.lock().unwrap() = Controls::default();
            game.remote_complete_using();
            assert!(game.cooldowns.is_on_cooldown("test:finish"));
            // The server's inventory update replaces a mod's replacement.
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:apple", 3));
            game.using = Some((game.selected, 5));
            *observer.0.lock().unwrap() = Controls {
                finish_replacement: Some(Some(7)),
                ..Controls::default()
            };
            game.remote_complete_using();
            assert_eq!(game.held().map(|s| s.count), Some(7));
            let server = game.to_network(&Some(ItemStack::new("minecraft:apple", 2)));
            game.handle_packet(
                horizon_protocol::packets::play::Clientbound::SetPlayerInventory(
                    horizon_protocol::packets::play::SetPlayerInventory {
                        slot: game.selected as i32,
                        item: server,
                    },
                ),
            )
            .unwrap();
            assert_eq!(game.held().map(|s| s.count), Some(2));
        });
    });
}

/// Finishing a use sends no creative packet in vanilla (`LivingEntity.completeUsingItem`); its
/// inventory-slot edits go out through `CreativeInventoryListener`, which
/// `CreativeModeInventoryScreen` listens with only while it is open. (Hotbar loads and creative
/// drops send their own packets; they are not this path.) So a hand a finish listener replaced
/// in creative mode is not sent as `SetCreativeModeSlot` by either of this client's senders.
#[test]
#[ignore = "requires prepared game resources"]
fn a_creative_hand_replaced_by_a_finish_listener_is_not_sent_as_a_creative_slot() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "finish_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            let offhand = super::super::shield::OFFHAND_SLOT;
            let packets = |observer: &Observer| observer.0.lock().unwrap().creative_slot_packets;
            // Finish the use of `slot`, a listener setting the result's count to `count`.
            let finish = |game: &mut Game<'_>, slot: usize, count: u32| {
                observer.0.lock().unwrap().finish_replacement = Some(Some(count));
                game.using = Some((slot, 5));
                game.remote_complete_using();
            };
            // Both senders: the per-tick scan and the one a creative click ends with.
            let sync = |game: &mut Game<'_>| {
                game.remote_send_state(0);
                game.send_creative_slots();
            };
            game.creative = true;
            game.selected = 2;
            game.inventory[2] = Some(ItemStack::new("minecraft:apple", 3));
            game.offhand = Some(ItemStack::new("minecraft:apple", 2));
            game.remote_synced();
            // The main hand.
            finish(game, 2, 7);
            assert_eq!(game.inventory[2].as_ref().map(|s| s.count), Some(7));
            sync(game);
            assert_eq!(packets(&observer), 0);
            // The off hand.
            finish(game, offhand, 9);
            assert_eq!(game.offhand.as_ref().map(|s| s.count), Some(9));
            sync(game);
            assert_eq!(packets(&observer), 0);
            // The selection moves on before the sync: the slot the use had is still synced.
            finish(game, 2, 4);
            game.selected = 5;
            sync(game);
            assert_eq!(game.inventory[2].as_ref().map(|s| s.count), Some(4));
            assert_eq!(packets(&observer), 0);
            // Two finishes before a sync.
            game.selected = 2;
            finish(game, 2, 6);
            finish(game, 2, 8);
            sync(game);
            assert_eq!(packets(&observer), 0);
            // An edit away and back to the synced value before a sync is no change.
            game.inventory[2] = Some(ItemStack::new("minecraft:apple", 5));
            game.inventory[2] = Some(ItemStack::new("minecraft:apple", 8));
            sync(game);
            assert_eq!(packets(&observer), 0);
            // A creative edit of the hand afterwards is still sent: one packet.
            game.inventory[2] = Some(ItemStack::new("minecraft:apple", 5));
            sync(game);
            assert_eq!(packets(&observer), 1);
        });
    });
}

#[path = "interaction_key_tests.rs"]
mod interaction_key_tests;
#[path = "mapping_press_tests.rs"]
mod mapping_press_tests;

#[test]
#[ignore = "requires prepared game resources"]
fn item_use_overlap_matrix_on_the_server() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "overlap",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(false, |game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.forced_hit = Some(hit());
            item_used_on_block_tests::install_criteria(game, &[0], "minecraft:air");
            for hand in [Hand::Main, Hand::Off] {
                for held in [false, true] {
                    for sneak in [false, true] {
                        for gate in [TriState::True, TriState::Default, TriState::False] {
                            for first in [Answer::Pass, Answer::TryWithEmptyHand, Answer::Consume] {
                                for after in
                                    [Answer::Success, Answer::Fail, Answer::Pass, Answer::Consume]
                                {
                                    game.inventory[game.selected] = None;
                                    game.offhand = None;
                                    if held {
                                        let stack = Some(ItemStack::new("minecraft:stick", 1));
                                        if hand == Hand::Main {
                                            game.inventory[game.selected] = stack;
                                        } else {
                                            game.offhand = stack;
                                        }
                                    }
                                    if sneak {
                                        game.keys.insert(winit::keyboard::KeyCode::ShiftLeft);
                                    } else {
                                        game.keys.remove(&winit::keyboard::KeyCode::ShiftLeft);
                                    }
                                    *observer.0.lock().unwrap() = Controls {
                                        expected_hand: Some(hand),
                                        record_first: true,
                                        first_answer: Some(first),
                                        block_answer: Some(Answer::Pass),
                                        item_on_answer: Some(after),
                                        use_item: gate,
                                        expect_empty_item_phase: !held,
                                        ..Controls::default()
                                    };
                                    let stopped =
                                        held && gate != TriState::False && first != Answer::Pass;
                                    let item_phase = !stopped
                                        && gate != TriState::False
                                        && (held || gate == TriState::True);
                                    let expected = if stopped {
                                        first == Answer::Consume
                                    } else {
                                        item_phase
                                            && matches!(
                                                after,
                                                Answer::Success | Answer::Fail | Answer::Consume
                                            )
                                    };
                                    game.guest_use = Some(super::super::GuestUse::OnBlock(hand));
                                    game.adv.progress = Default::default();
                                    let swings = game.swings_started;
                                    let used =
                                        game.with_hand(hand, |g| g.use_hand(Some(&hit())).unwrap());
                                    assert_eq!(
                                        used, expected,
                                        "{hand:?} held={held} sneak={sneak} {gate:?} {first:?} {after:?}"
                                    );
                                    assert_eq!(
                                        game.swings_started - swings,
                                        u32::from(item_phase && after == Answer::Success)
                                    );
                                    assert_eq!(
                                        game.adv.progress.has("test:sticks0", "seen"),
                                        item_phase
                                            && matches!(after, Answer::Success | Answer::Consume)
                                    );
                                    let state = observer.0.lock().unwrap();
                                    assert_eq!(
                                        state.steps.contains(&Step::Phase(
                                            UsePhase::ItemAfterBlock,
                                            LogicalSide::Server
                                        )),
                                        item_phase
                                    );
                                    assert_eq!(
                                        state.steps.contains(&Step::Phase(
                                            UsePhase::Block,
                                            LogicalSide::Server
                                        )),
                                        !stopped && !(sneak && held)
                                    );
                                }
                            }
                        }
                    }
                }
            }
        });
    });
}

/// Item.useOn without item behavior or a BLOCK_TRANSFORMER is PASS, and
/// MultiPlayerGameMode.performUseItemOn returns it without a swing. An unknown
/// item must keep that inert fallback even when RightClickBlock forces use.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn resource_gated_unknown_held_item_forced_use_predicts_pass_without_a_swing() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "unknown_forced_use",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        for client in [true, false] {
            with_interaction_game(client, |game| {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                game.forced_hit = Some(hit());
                // Keep the API key resolvable so RightClickBlock really runs,
                // but remove its gameplay data to exercise the unknown-item
                // fallback. The suffix also catches premature spawn-egg prediction.
                let stack = ItemStack::new("minecraft:pig_spawn_egg", 2);
                assert!(mod_api::__private::item_key(&stack.item).is_some());
                let mut items = game.items().clone();
                assert!(items.items.remove(&stack.item).is_some());
                game.server_items = Some(Arc::new(items));
                assert!(game.items().get(&stack.item).is_none());
                game.inventory[game.selected] = Some(stack.clone());
                game.offhand = None;
                *observer.0.lock().unwrap() = Controls {
                    use_block: TriState::False,
                    use_item: TriState::True,
                    first_answer: Some(Answer::Pass),
                    item_on_answer: Some(Answer::Success),
                    ..Controls::default()
                };
                let swings = game.swings_started;
                // Literal vanilla fallback expectations, independently applied
                // to client prediction and authoritative server use (not one
                // result compared with the other).
                assert!(!use_block(game, client), "client={client}: PASS");
                assert_eq!(game.swings_started - swings, 0, "client={client}");
                assert_eq!(game.held(), Some(&stack));
                assert_eq!(game.world.block(0, 64, 0), stone);
                assert!(game.using.is_none());
                let state = observer.0.lock().unwrap();
                assert_eq!(state.clicks, 1, "the forcing listener must have run");
                assert!(
                    !state
                        .steps
                        .iter()
                        .any(|step| matches!(step, Step::Phase(UsePhase::ItemAfterBlock, _))),
                    "unknown items cannot reach the forced item's success listener"
                );
            });
        }
    });
}

#[test]
#[ignore = "requires prepared game resources"]
fn a_server_client_keeps_a_bundle_use_until_the_server_completes_it() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "finish_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:bundle", 1));
            // The use starts at tick 0, which records the item in use.
            game.using = Some((game.selected, 0));
            // 200 client ticks: the use is still on, waiting for the server.
            for _ in 0..200 {
                game.remote_tick_using();
            }
            assert_eq!(game.using, Some((game.selected, 200)));
            game.remote_tick_using();
            assert_eq!(game.using, Some((game.selected, 201)));
            // Entity event 9 arrives late: the client-side finish fires once.
            game.remote_complete_using();
            assert_eq!(game.using, None);
            assert_eq!(
                observer.0.lock().unwrap().finishes,
                [(
                    LogicalSide::Client,
                    Hand::Main,
                    Some(("minecraft:bundle".to_owned(), 1))
                )]
            );
        });
    });
}
