//! The same independently specified matrix crosses native and real wasm dispatch.
use super::*;
#[path = "../../../../../horizon-mod-wasm/tests/fixture/src/interaction.rs"]
mod fixture;

struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        fixture::register(r);
    }
    fn right_click_block(&self, host: HostSide<'_>, event: &mut RightClickBlockEvent<'_>) -> Flow {
        fixture::right_click_block(host, event)
    }
    fn use_item_on_block(&self, host: HostSide<'_>, event: &mut UseItemOnBlockEvent<'_>) -> Flow {
        fixture::use_item_on_block(host, event)
    }
}

#[derive(Debug)]
struct Case {
    controls: [u8; 6],
    held: bool,
    other_held: bool,
    sneak: bool,
    phases: Vec<u8>,
    used: bool,
    // Server and client swing counts, written without result helper methods.
    swings: [u32; 2],
    item_award: bool,
    default_award: bool,
    lever: bool,
}

fn cases(hand: Hand) -> Vec<Case> {
    let mut cases = Vec::new();
    // RightClickBlock: TRUE overrides secondary use; Default skips the block
    // only when sneaking with either hand full; FALSE always skips it.
    // The item phase requires a stack unless useItem is explicitly TRUE.
    for held in [false, true] {
        for other_held in [false, true] {
            for sneak in [false, true] {
                for (block_switch, block) in
                    [(0, !(sneak && (held || other_held))), (1, true), (2, false)]
                {
                    for (item_switch, before, after) in
                        [(0, held, held), (1, held, true), (2, false, false)]
                    {
                        let mut phases = vec![0];
                        if before {
                            phases.push(1);
                        }
                        if block {
                            phases.push(2);
                        }
                        if !block && after {
                            phases.push(3);
                        }
                        cases.push(Case {
                            controls: [block_switch, item_switch, 0, 5, 5, 0],
                            held,
                            other_held,
                            sneak,
                            phases,
                            used: block || after,
                            swings: [0, 0],
                            // ITEM_USED_ON_BLOCK sees the stack copied before the
                            // block phase, whichever phase consumes: air when the
                            // hand is empty, including a forced empty item phase.
                            item_award: block || after,
                            default_award: false,
                            lever: false,
                        });
                    }
                }
            }
        }
    }
    // Canceling RightClickBlock ends its turn even for PASS/TRY; no
    // block or item phase and no advancement is produced.
    for (answer, used, swings) in [
        (1, false, [0, 0]),
        (2, true, [0, 0]),
        (3, true, [1, 1]),
        (4, true, [1, 0]),
        (5, true, [0, 0]),
        (6, false, [0, 0]),
    ] {
        cases.push(Case {
            controls: [0, 0, 0, 0, 0, answer],
            held: true,
            other_held: false,
            sneak: false,
            phases: vec![0],
            used,
            swings,
            item_award: false,
            default_award: false,
            lever: false,
        });
    }
    // Only PASS before the block continues. Every other answer ends the hand:
    // FAIL and the three consuming answers use the click, TRY does not. A
    // before-block answer awards no criterion.
    for (answer, continues, used, swings) in [
        (1, true, true, [0, 0]),
        (2, false, true, [0, 0]),
        (3, false, true, [1, 1]),
        (4, false, true, [1, 0]),
        (5, false, true, [0, 0]),
        (6, false, false, [0, 0]),
    ] {
        cases.push(Case {
            controls: [1, 0, answer, 1, 5, 0],
            held: true,
            other_held: false,
            sneak: false,
            phases: if continues {
                vec![0, 1, 2, 3]
            } else {
                vec![0, 1]
            },
            used,
            swings,
            item_award: continues,
            default_award: false,
            lever: false,
        });
    }
    // A canceled block PASS/FAIL delegates to the item phase; TRY also asks
    // the block's empty method, which stone passes. Consuming answers stop.
    for (answer, continues, swings) in [
        (1, true, [0, 0]),
        (2, true, [0, 0]),
        (3, false, [1, 1]),
        (4, false, [1, 0]),
        (5, false, [0, 0]),
        (6, true, [0, 0]),
    ] {
        cases.push(Case {
            controls: [1, 0, 0, answer, 5, 0],
            held: true,
            other_held: false,
            sneak: false,
            phases: if continues {
                vec![0, 1, 2, 3]
            } else {
                vec![0, 1, 2]
            },
            used: true,
            swings,
            item_award: true,
            default_award: false,
            lever: false,
        });
    }
    // After-block PASS/TRY stop this item method without consuming the click;
    // FAIL vetoes it. Only consuming results award ITEM_USED_ON_BLOCK.
    for (answer, used, swings, award) in [
        (1, false, [0, 0], false),
        (2, true, [0, 0], false),
        (3, true, [1, 1], true),
        (4, true, [1, 0], true),
        (5, true, [0, 0], true),
        (6, false, [0, 0], false),
    ] {
        cases.push(Case {
            controls: [1, 0, 0, 1, answer, 0],
            held: true,
            other_held: false,
            sneak: false,
            phases: vec![0, 1, 2, 3],
            used,
            swings,
            item_award: award,
            default_award: false,
            lever: false,
        });
    }
    // Natural lever use and canceled TRY reach useWithoutItem only on MAIN.
    // Explicit PASS/FAIL in BLOCK skip that method, so the item answers.
    for (answer, empty_method) in [(0, true), (6, true), (1, false), (2, false)] {
        let pulls = empty_method && hand == Hand::Main;
        cases.push(Case {
            controls: [1, 0, 1, answer, 5, 0],
            held: true,
            other_held: false,
            sneak: true,
            phases: if pulls {
                vec![0, 1, 2]
            } else {
                vec![0, 1, 2, 3]
            },
            used: true,
            swings: if pulls { [1, 1] } else { [0, 0] },
            item_award: !pulls,
            default_award: pulls,
            lever: true,
        });
    }
    cases
}

fn advancement_tree(game: &mut Game<'_>) {
    use horizon_client::advancements as a;
    let mut tree = a::Tree::default();
    for (index, name) in ["item_used_on_block", "default_block_use"]
        .into_iter()
        .enumerate()
    {
        let id = format!("test:{name}");
        let trigger = format!("minecraft:{name}");
        tree.by_id.insert(id.clone(), index);
        tree.by_trigger.insert(trigger.clone(), vec![(index, 0)]);
        tree.list.push(a::Advancement {
            id,
            parent: None,
            display: None,
            criteria: vec![a::Criterion {
                name: "seen".into(),
                trigger,
                conditions: serde_json::json!({}).into(),
            }],
            requirements: vec![vec!["seen".into()]],
            rewards: Default::default(),
        });
    }
    game.adv.tree = Arc::new(tree);
    game.adv.loaded = true;
    game.adv.progress = Default::default();
}

fn matrix(mods: mod_api::Mods) {
    // with_mods registers the mods again for their controllers: install Parts after it, as
    // registration-time declarations (a custom statistic) refuse once the registry is frozen.
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
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
                for hand in [Hand::Main, Hand::Off] {
                    for case in cases(hand) {
                        let state = if case.lever { lever } else { stone };
                        game.world.set(0, 64, 0, state).unwrap();
                        let held = case.held.then(|| ItemStack::new("minecraft:stick", 1));
                        let other = case
                            .other_held
                            .then(|| ItemStack::new("minecraft:stick", 1));
                        (game.inventory[game.selected], game.offhand) = if hand == Hand::Main {
                            (held, other)
                        } else {
                            (other, held)
                        };
                        game.keys.clear();
                        if case.sneak {
                            game.keys.insert(winit::keyboard::KeyCode::ShiftLeft);
                        }
                        if let Some(remote) = game.remote.as_mut() {
                            remote.last_input = if case.sneak {
                                super::super::SHIFT_INPUT
                            } else {
                                0
                            };
                        }
                        game.swing = Default::default();
                        game.use_failed = false;
                        advancement_tree(game);
                        let mut control = mod_api::ModBlockEntity::new(
                            "wasm_fixture:interaction_control".into(),
                            Default::default(),
                        );
                        control.data = case.controls.iter().map(|&b| i32::from(b)).collect();
                        game.native_block_entity_tick(
                            &mut mod_api::BlockEntityTickReply::Changed,
                            [0, 64, 0],
                            &mut control,
                        );
                        assert!(
                            control.data.is_empty(),
                            "the control callback must run and accept {case:?}"
                        );
                        let swings = game.swings_started;
                        let used = if client {
                            game.remote_use_item_on(hand)
                        } else {
                            game.guest_use = Some(super::super::super::GuestUse::OnBlock(hand));
                            game.with_hand(hand, |g| g.use_hand(Some(&hit()))).unwrap()
                        };
                        assert_eq!(used, case.used, "{client:?} {hand:?} {case:?}");
                        assert_eq!(
                            game.swings_started - swings,
                            case.swings[usize::from(client)],
                            "{client:?} {hand:?} {case:?}"
                        );
                        if game.swings_started != swings {
                            assert_eq!(game.swing.attack_hand(), hand);
                        }
                        assert_eq!(
                            game.world.block(0, 64, 0) != state,
                            case.default_award,
                            "{client:?} {hand:?} {case:?}"
                        );
                        for (name, expected) in [
                            ("item_used_on_block", case.item_award),
                            ("default_block_use", case.default_award),
                        ] {
                            assert_eq!(
                                game.adv.progress.has(&format!("test:{name}"), "seen"),
                                !client && expected,
                                "{client:?} {hand:?} {name} {case:?}"
                            );
                        }
                        game.native_block_entity_tick(
                            &mut mod_api::BlockEntityTickReply::Changed,
                            [0, 64, 0],
                            &mut control,
                        );
                        let expected: Vec<i32> = case
                            .phases
                            .iter()
                            .flat_map(|phase| {
                                [
                                    i32::from(*phase),
                                    i32::from(hand == Hand::Off),
                                    i32::from(client),
                                    i32::from(!case.held),
                                ]
                            })
                            .collect();
                        assert_eq!(control.data, expected, "{client:?} {hand:?} {case:?}");
                        assert_eq!(
                            game.with_hand(hand, |g| g
                                .held()
                                .map(|s| (s.item.to_string(), s.count))),
                            case.held.then(|| ("minecraft:stick".into(), 1))
                        );
                    }
                }
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn native_interaction_shared_matrix() {
    matrix(vec![mod_api::ModEntry::new("wasm_fixture", Box::new(Native))].into());
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES, HORIZON_WASM_FIXTURE and HORIZON_WASM_STD; build with horizon-mod-wasm/listener-fixture"]
fn wasm_interaction_shared_matrix() {
    // Enters the guest: libtest's thread is smaller than the guest stack policy requires. The
    // load budget: this checks behaviour, not speed, and a loaded runner can pass 100 ms.
    horizon_guest_stack::run_on_guest_stack(|| {
        horizon_mod_wasm::copy_probe::with_load_budget(
            wasm_interaction_shared_matrix_on_guest_stack,
        )
    });
}

fn wasm_interaction_shared_matrix_on_guest_stack() {
    use horizon_mod_package::manifest::Package;
    use horizon_mod_wasm::{WasmPackage, load};
    let path = std::path::PathBuf::from(std::env::var_os("HORIZON_WASM_FIXTURE").unwrap());
    let manifest = Package::read(&path).unwrap().manifest;
    let info = mod_api::PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
        vec![manifest.id.clone()],
        vec![],
    );
    let loaded = load(
        &std::fs::read(std::env::var_os("HORIZON_WASM_STD").unwrap()).unwrap(),
        vec![WasmPackage {
            manifest,
            module: Package::read_wasm(&path).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    assert_eq!(loaded.mods.len(), 1);
    loaded.initialize(&[info]).unwrap();
    matrix(loaded.mods.into());
}
