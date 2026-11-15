//! Expected outcomes below are literal values from #784's accepted matrix.
//! This harness substitutes resource loading and network publication only; it
//! runs the production coordinator, versioned storage, journal merge and undo.
use super::*;
use storage::ActiveAction;

// Fixture adapter keeps resource-free planners focused on their MenuState.
fn run(
    host: &mut Harness,
    menu: &Menu,
    decide: impl FnOnce(&MenuState) -> Plan,
    fire: impl FnOnce(&mut Harness) -> Reply,
) -> Result<Outcome, Admission> {
    super::run(host, menu, |_, state| decide(state), fire)
}

// Observable root-publication labels, independent of the plan representation.
#[derive(Debug, PartialEq, Eq)]
enum Published {
    Packet,
    Drop,
    Cost(u32),
}

// A genuinely deferred packet effect; explicit block notifications survive abort.
fn packet() -> Effect {
    Effect::BreakProgress(1, [0, 64, 0], 1)
}
fn dropped() -> Effect {
    Effect::Drop([0, 64, 0], ItemStack::new("minecraft:dirt", 2))
}
fn cost(amount: u32) -> Effect {
    Effect::Effect(super::super::DeferredEffect::CustomStat {
        player: 1,
        key: "minecraft:interact_with_anvil",
        amount: amount as i32,
    })
}
fn observed(effect: Effect) -> Published {
    match effect {
        Effect::BreakProgress(1, [0, 64, 0], 1) => Published::Packet,
        Effect::Drop([0, 64, 0], stack) => {
            assert_eq!(stack, ItemStack::new("minecraft:dirt", 2));
            Published::Drop
        }
        Effect::Effect(super::super::DeferredEffect::CustomStat { amount, .. }) => {
            Published::Cost(amount as u32)
        }
        _ => panic!("unexpected root effect"),
    }
}

struct Frame {
    id: u64,
    action: Option<ActiveAction>,
    journal: Journal,
}

struct Harness {
    identity: Identity,
    actor: Residence,
    target: Residence,
    frames: Vec<Frame>,
    before_fire: Option<fn(&mut Self)>,
    decisions: usize,
    dispatches: usize,
    applies: usize,
    published: Vec<Published>,
}

impl Harness {
    fn new() -> Self {
        let world = WorldEpoch::new(17);
        let mut host = Self {
            identity: Identity {
                side: Side::Server,
                world,
                actor: EntityRef::new(world, 1),
                target: EntityRef::new(world, 2),
            },
            actor: Residence::Found,
            target: Residence::Found,
            frames: Vec::new(),
            before_fire: None,
            decisions: 0,
            dispatches: 0,
            applies: 0,
            published: Vec::new(),
        };
        host.open(None);
        host
    }

    fn open(&mut self, menu: Option<MenuId>) -> u64 {
        let id = super::super::frame_token();
        self.frames.push(Frame {
            id,
            action: menu.map(ActiveAction::new),
            journal: Journal::default(),
        });
        id
    }

    fn close(&mut self, commit: bool) {
        let frame = self.frames.pop().unwrap();
        if commit {
            if let Some(parent) = self.frames.last_mut() {
                parent.journal.merge(frame.journal);
            } else {
                self.published
                    .extend(frame.journal.root_effects().into_iter().map(observed));
            }
        } else {
            frame.journal.rollback();
        }
    }

    fn place(&mut self, menu: &Menu, fire: impl FnOnce(&mut Self) -> Reply) -> Outcome {
        run(self, menu, placement, fire).unwrap()
    }

    fn no_success(&self) {
        assert_eq!(self.applies, 0);
        assert_eq!(self.published, []);
    }
}

impl ActionHost for Harness {
    fn identity(&self) -> Identity {
        self.identity
    }
    fn residence(&self, holder: EntityRef) -> Residence {
        if holder == self.identity.actor {
            self.actor
        } else {
            self.target
        }
    }
    fn has_transaction(&self) -> bool {
        !self.frames.is_empty()
    }
    fn menu_busy(&self, menu: MenuId) -> bool {
        self.frames
            .iter()
            .any(|f| f.action.as_ref().is_some_and(|a| a.menu == menu))
    }
    fn open_action(&mut self, menu: MenuId) -> u64 {
        self.open(Some(menu))
    }
    fn action_is_current(&self, savepoint: u64, menu: MenuId) -> bool {
        self.frames
            .last()
            .is_some_and(|f| f.id == savepoint && f.action.as_ref().is_some_and(|a| a.menu == menu))
    }
    fn close_action(&mut self, savepoint: u64, commit: bool) {
        let at = self.frames.iter().position(|f| f.id == savepoint).unwrap();
        while self.frames.len() > at + 1 {
            self.close(false);
        }
        self.close(commit);
    }
    fn journal(&mut self) -> &mut Journal {
        &mut self.frames.last_mut().unwrap().journal
    }
    fn defer(&mut self, effect: Effect) {
        self.journal().defer(effect);
    }
    fn decided(&mut self) {
        self.decisions += 1;
        if let Some(change) = self.before_fire.take() {
            change(self);
        }
    }
    fn dispatched(&mut self) {
        self.dispatches += 1;
    }
    fn applied(&mut self) {
        self.applies += 1;
    }
}

fn stack(item: &str, count: u32) -> Option<ItemStack> {
    Some(ItemStack::new(item, count))
}

pub(super) fn state() -> MenuState {
    MenuState {
        instance: 7,
        container: 23,
        cursor: stack("minecraft:stone", 3),
        slots: vec![None, None, None],
        capacities: vec![64, 64, 64],
        recipe: None,
        registry: 1,
        rules: 1,
        permitted: true,
        cost: 0,
        drag: vec![],
        backing: 1,
        rng: 101,
    }
}

fn fixture() -> (Harness, Menu) {
    (Harness::new(), Menu::new(state()))
}

pub(super) fn placement(view: &MenuState) -> Plan {
    Plan::new(
        ActionKind::Placement,
        vec![
            Step::Write(Mutation::Cursor(None)),
            Step::Write(Mutation::Slot(0, view.cursor.clone())),
            Step::Effect(packet()),
        ],
    )
}

fn quick_move(_: &MenuState) -> Plan {
    Plan::new(
        ActionKind::QuickMove,
        vec![
            Step::Write(Mutation::Slot(0, None)),
            Step::Write(Mutation::Slot(1, stack("minecraft:stone", 4))),
            Step::Write(Mutation::Slot(2, stack("minecraft:stone", 6))),
            Step::Effect(packet()),
        ],
    )
}

fn quick_fixture() -> (Harness, Menu) {
    let mut initial = state();
    initial.cursor = None;
    initial.slots = vec![stack("minecraft:stone", 10), None, None];
    initial.capacities = vec![64, 4, 6];
    (Harness::new(), Menu::new(initial))
}

#[test]
fn accept_placement_cursor_zero_slot_stone_three_one_apply() {
    let (mut host, menu) = fixture();
    assert_eq!(host.place(&menu, |_| Reply::Allow), Outcome::Applied);
    menu.read(|s| {
        assert_eq!(s.cursor, None);
        assert_eq!(s.slots, [stack("minecraft:stone", 3), None, None]);
    });
    assert_eq!((host.decisions, host.dispatches, host.applies), (1, 1, 1));
    assert_eq!(host.published, []);
    host.close(true);
    assert_eq!(host.published, [Published::Packet]);
}

#[test]
fn cancel_after_unrelated_listener_write_restores_all_zero_apply() {
    let (mut host, menu) = fixture();
    let result = host.place(&menu, |host| {
        menu.edit(host, |s| s.slots[2] = stack("minecraft:dirt", 2));
        host.defer(dropped());
        host.defer(cost(2));
        Reply::Cancel
    });
    assert_eq!(result, Outcome::Canceled);
    assert_eq!(menu.read(Clone::clone), state());
    host.no_success();
    host.close(true);
    assert_eq!(host.published, []);
}

#[test]
fn listener_changes_cursor_to_four_stale_restored_zero_recompute() {
    let (mut host, menu) = fixture();
    let mut computes = 0;
    let result = run(
        &mut host,
        &menu,
        |s| {
            computes += 1;
            placement(s)
        },
        |host| {
            menu.edit(host, |s| s.cursor = stack("minecraft:stone", 4));
            Reply::Allow
        },
    );
    assert_eq!(result, Ok(Outcome::Stale(StaleReason::Mutation)));
    assert_eq!(computes, 1);
    assert_eq!(host.dispatches, 1);
    assert_eq!(menu.read(Clone::clone), state());
    host.no_success();
}

#[test]
fn input_a_b_a_stale_by_version_even_when_snapshot_matches() {
    let (mut host, menu) = fixture();
    let result = host.place(&menu, |host| {
        menu.edit(host, |s| s.cursor = stack("minecraft:dirt", 3));
        menu.edit(host, |s| s.cursor = stack("minecraft:stone", 3));
        assert_eq!(menu.read(Clone::clone), state());
        Reply::Allow
    });
    assert_eq!(result, Outcome::Stale(StaleReason::Mutation));
    assert_eq!(menu.read(Clone::clone), state());
    host.no_success();
}

#[test]
fn replaced_menu_generation_seven_to_eight_stale_no_replacement_write() {
    let (mut host, menu) = fixture();
    assert_eq!(
        host.place(&menu, |host| {
            menu.edit(host, |s| s.instance = 8);
            Reply::Allow
        }),
        Outcome::Stale(StaleReason::MenuInstance)
    );
    assert_eq!(menu.read(Clone::clone), state());
    host.no_success();
}

#[test]
fn quick_move_ten_into_four_and_six_applies_once() {
    let (mut host, menu) = quick_fixture();
    assert_eq!(
        run(&mut host, &menu, quick_move, |_| Reply::Allow),
        Ok(Outcome::Applied)
    );
    assert_eq!(
        menu.read(|s| s.slots.clone()),
        [
            None,
            stack("minecraft:stone", 4),
            stack("minecraft:stone", 6)
        ]
    );
    assert_eq!(host.applies, 1);
    host.close(true);
    assert_eq!(host.published, [Published::Packet]);
}

#[test]
fn quick_move_capacity_change_refuses_whole_move_source_ten() {
    for changed in [1, 2] {
        let (mut host, menu) = quick_fixture();
        let result = run(&mut host, &menu, quick_move, |host| {
            menu.edit(host, |s| s.capacities[changed] -= 1);
            Reply::Allow
        });
        assert_eq!(result, Ok(Outcome::Stale(StaleReason::Mutation)));
        assert_eq!(
            menu.read(|s| s.slots.clone()),
            [stack("minecraft:stone", 10), None, None]
        );
        assert_eq!(menu.read(|s| s.capacities.clone()), [64, 4, 6]);
        host.no_success();
        host.close(true);
        assert_eq!(host.published, []);
    }
}

#[test]
fn nested_commit_then_outer_trap_rolls_back_next_placement_succeeds() {
    let (mut host, menu) = fixture();
    assert_eq!(
        host.place(&menu, |host| {
            host.open(None);
            menu.edit(host, |s| s.slots[2] = stack("minecraft:dirt", 2));
            host.defer(dropped());
            host.close(true);
            panic!("recoverable listener trap");
        }),
        Outcome::ListenerFailed
    );
    assert_eq!(menu.read(Clone::clone), state());
    host.no_success();
    assert_eq!(host.frames.len(), 1);
    assert_eq!(host.place(&menu, |_| Reply::Allow), Outcome::Applied);
    assert_eq!(host.applies, 1);
    host.close(true);
    assert_eq!(host.published, [Published::Packet]);
}

#[test]
fn reentrant_same_menu_admission_busy_zero_inner_tokens() {
    let (mut host, menu) = fixture();
    assert_eq!(
        host.place(&menu, |host| {
            assert_eq!(
                run(
                    host,
                    &menu,
                    |_| panic!("inner decide"),
                    |_| panic!("inner fire")
                ),
                Err(Admission::Busy)
            );
            assert_eq!((host.decisions, host.dispatches), (1, 1));
            Reply::Allow
        }),
        Outcome::Applied
    );
    assert_eq!((host.decisions, host.dispatches, host.applies), (1, 1, 1));
}

fn token(host: &mut Harness, menu: &Menu, plan: Plan) -> Decision {
    let savepoint = host.open_action(menu.id);
    let inner = menu.inner.borrow();
    Decision(Proposed {
        identity: host.identity,
        savepoint,
        target: menu.clone(),
        reads: Reads {
            epoch: inner.epoch,
            menu: inner.state.clone(),
        },
        plan: plan.freeze(&inner.state).expect("fixture plan"),
    })
}

// A second apply of the same decision does not compile: `apply` takes it by
// value. One apply writes once and publishes once.
#[test]
fn single_apply_writes_once_publishes_once() {
    let (mut host, menu) = fixture();
    let decision = token(&mut host, &menu, menu.read(placement));
    let epoch = menu.inner.borrow().epoch;
    assert_eq!(apply(&mut host, decision, Reply::Allow), Outcome::Applied);
    // Placement is two writes (cursor, slot), each advancing the epoch once.
    assert_eq!(menu.inner.borrow().epoch.0, epoch.0 + 2);
    assert_eq!(host.applies, 1);
    host.close(true);
    host.close(true);
    assert_eq!(host.published, [Published::Packet]);
}

#[test]
fn every_refused_outcome_consumes_token_without_writes() {
    for reply in [
        Reply::Cancel,
        Reply::Failed,
        Reply::Invalid(InvalidReason::Count),
        Reply::Allow,
    ] {
        let (mut host, menu) = fixture();
        let decision = token(&mut host, &menu, menu.read(placement));
        menu.edit(&mut host, |s| s.cursor = stack("minecraft:stone", 4));
        let epoch = menu.inner.borrow().epoch;
        assert_ne!(apply(&mut host, decision, reply), Outcome::Applied);
        assert_eq!(menu.inner.borrow().epoch, epoch);
        host.close(false);
        assert_eq!(menu.read(Clone::clone), state());
        host.no_success();
    }
}

#[test]
fn nan_infinity_and_overflowing_listener_count_invalid_full_rollback() {
    for value in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        4_294_967_296.0,
        -1.0,
        1.5,
    ] {
        let (mut host, menu) = fixture();
        let outcome = host.place(&menu, |host| {
            menu.edit(host, |s| s.slots[2] = stack("minecraft:dirt", 2));
            host.defer(dropped());
            match listener_count(value, u32::MAX) {
                Ok(_) => panic!("malformed listener count accepted"),
                Err(reason) => Reply::Invalid(reason),
            }
        });
        assert!(matches!(
            outcome,
            Outcome::InvalidInput(InvalidReason::Numeric | InvalidReason::Count)
        ));
        assert_eq!(menu.read(Clone::clone), state());
        host.no_success();
        assert_eq!(host.place(&menu, |_| Reply::Allow), Outcome::Applied);
    }
    let (mut host, menu) = fixture();
    assert_eq!(
        host.place(&menu, |host| {
            menu.edit(host, |s| s.cost = 99);
            Reply::Invalid(add_count(u32::MAX, 1).unwrap_err())
        }),
        Outcome::InvalidInput(InvalidReason::Count)
    );
    assert_eq!(menu.read(Clone::clone), state());
    host.no_success();
}

#[test]
fn outer_abort_after_applied_restores_cursor_slots_cost_rng_and_effects() {
    let (mut host, menu) = fixture();
    let result = run(
        &mut host,
        &menu,
        |s| {
            let mut plan = placement(s);
            plan.steps.extend([
                Step::Write(Mutation::Cost(2)),
                Step::Write(Mutation::Rng(102)),
                Step::Effect(cost(2)),
                Step::Effect(dropped()),
            ]);
            plan
        },
        |_| Reply::Allow,
    );
    assert_eq!(result, Ok(Outcome::Applied));
    assert_eq!(menu.read(|s| (s.cost, s.rng)), (2, 102));
    assert_eq!(host.published, []);
    host.close(false);
    assert_eq!(menu.read(Clone::clone), state());
    assert_eq!(host.published, []);
}

#[test]
fn save_during_action_savepoint_refused() {
    let (mut host, menu) = fixture();
    assert_eq!(
        host.place(&menu, |_| {
            // Production save admission uses this exact process-wide signal.
            assert!(!horizon_client::join_seq::visibility::passes_settled());
            Reply::Allow
        }),
        Outcome::Applied
    );
    assert!(!host.menu_busy(menu.id));
}

#[test]
fn no_subscriber_zero_decision_allocations_same_baseline_effect_order() {
    let (mut host, menu) = fixture();
    let outcome = route(
        false,
        &mut host,
        &menu,
        |host| {
            // This baseline uses no owned plan or read set. Its two direct writes
            // have the same journal coverage as any built-in operation.
            menu.edit(host, |s| {
                s.slots[0] = s.cursor.take();
            });
            host.defer(packet());
            Outcome::Applied
        },
        |_, _| panic!("allocated subscribed plan"),
        |_| panic!("dispatch"),
    );
    assert_eq!(outcome, Ok(Outcome::Applied));
    assert_eq!((host.decisions, host.dispatches), (0, 0));
    assert_eq!(
        menu.read(|s| (s.cursor.clone(), s.slots.clone(), s.rng)),
        (None, vec![stack("minecraft:stone", 3), None, None], 101)
    );
    host.close(true);
    assert_eq!(host.published, [Published::Packet]);
}

#[test]
fn taken_holder_busy_at_admission_stale_at_apply_no_retry() {
    for actor in [false, true] {
        let (mut host, menu) = fixture();
        if actor {
            host.actor = Residence::Taken;
        } else {
            host.target = Residence::Taken;
        }
        assert_eq!(
            run(&mut host, &menu, |_| panic!("decide"), |_| panic!("fire")),
            Err(Admission::Busy)
        );
        assert_eq!(
            (host.decisions, host.dispatches, host.frames.len()),
            (0, 0, 1)
        );
        host.actor = Residence::Found;
        host.target = Residence::Found;
        let result = host.place(&menu, |host| {
            if actor {
                host.actor = Residence::Taken;
            } else {
                host.target = Residence::Taken;
            }
            Reply::Allow
        });
        assert_eq!(
            result,
            Outcome::Stale(if actor {
                StaleReason::Actor
            } else {
                StaleReason::Target
            })
        );
        assert_eq!(menu.read(Clone::clone), state());
        host.no_success();
    }
}

#[test]
fn whole_menu_epoch_covers_rules_recipe_permissions_backing_drag_and_rng() {
    let edits: [fn(&mut MenuState); 8] = [
        |s| s.rules += 1,
        |s| s.recipe = Some("test:other".into()),
        |s| s.registry += 1,
        |s| s.permitted = false,
        |s| s.backing += 1,
        |s| s.drag.push(1),
        |s| s.rng += 1,
        |s| s.container += 1,
    ];
    for edit in edits {
        let (mut host, menu) = fixture();
        assert_eq!(
            host.place(&menu, |host| {
                menu.edit(host, edit);
                Reply::Allow
            }),
            Outcome::Stale(StaleReason::Mutation)
        );
        assert_eq!(menu.read(Clone::clone), state());
        host.no_success();
    }
}

#[test]
fn invalid_plan_suffix_has_no_partial_successful_prefix() {
    let (mut host, menu) = quick_fixture();
    let result = run(
        &mut host,
        &menu,
        |_| {
            Plan::new(
                ActionKind::QuickMove,
                vec![
                    Step::Write(Mutation::Slot(0, None)),
                    Step::Effect(packet()),
                    Step::Write(Mutation::Slot(1, stack("minecraft:stone", 4))),
                    Step::Write(Mutation::Slot(2, stack("minecraft:stone", 7))),
                ],
            )
        },
        |_| panic!("invalid plan must not dispatch"),
    );
    assert_eq!(result, Ok(Outcome::InvalidInput(InvalidReason::Count)));
    assert_eq!(
        menu.read(|s| s.slots.clone()),
        [stack("minecraft:stone", 10), None, None]
    );
    assert_eq!(host.dispatches, 0);
    host.no_success();
}

#[test]
fn nested_other_menu_commit_is_owned_by_outer_action() {
    let (mut host, menu) = fixture();
    let other = Menu::new(state());
    assert_eq!(
        host.place(&menu, |host| {
            assert_eq!(host.place(&other, |_| Reply::Allow), Outcome::Applied);
            assert_eq!(other.read(|s| s.cursor.clone()), None);
            Reply::Cancel
        }),
        Outcome::Canceled
    );
    assert_eq!(menu.read(Clone::clone), state());
    assert_eq!(other.read(Clone::clone), state());
    host.close(true);
    assert_eq!(host.published, []);
}

#[test]
fn recipe_two_stone_accept_consumes_once_changed_input_stale_restored() {
    for changed in [false, true] {
        let mut initial = state();
        initial.cursor = None;
        initial.slots[0] = stack("minecraft:stone", 2);
        initial.recipe = Some("test:two_stone".into());
        let menu = Menu::new(initial.clone());
        let mut host = Harness::new();
        let result = run(
            &mut host,
            &menu,
            |_| {
                Plan::new(
                    ActionKind::RecipePlacement,
                    vec![
                        Step::Write(Mutation::Slot(0, None)),
                        Step::Write(Mutation::Slot(1, stack("minecraft:stone_button", 1))),
                        Step::Effect(packet()),
                    ],
                )
            },
            |host| {
                if changed {
                    menu.edit(host, |s| s.slots[0] = stack("minecraft:stone", 1));
                }
                Reply::Allow
            },
        );
        if changed {
            assert_eq!(result, Ok(Outcome::Stale(StaleReason::Mutation)));
            assert_eq!(menu.read(Clone::clone), initial);
            host.no_success();
        } else {
            assert_eq!(result, Ok(Outcome::Applied));
            assert_eq!(
                menu.read(|s| s.slots.clone()),
                [None, stack("minecraft:stone_button", 1), None]
            );
            assert_eq!(host.applies, 1);
        }
    }
}

#[test]
fn abandoned_nested_listener_transaction_refuses_and_rolls_back() {
    let (mut host, menu) = fixture();
    assert_eq!(
        host.place(&menu, |host| {
            host.open(None);
            menu.edit(host, |s| s.cost = 3);
            Reply::Allow
        }),
        Outcome::InvalidInput(InvalidReason::ForeignSavepoint)
    );
    assert_eq!(menu.read(Clone::clone), state());
    assert_eq!(host.frames.len(), 1);
    host.no_success();
}

#[test]
fn same_value_write_and_rolled_back_child_still_invalidate_epoch() {
    for abort_child in [false, true] {
        let (mut host, menu) = fixture();
        assert_eq!(
            host.place(&menu, |host| {
                host.open(None);
                menu.edit(host, |s| s.cursor = stack("minecraft:stone", 3));
                host.close(!abort_child);
                Reply::Allow
            }),
            Outcome::Stale(StaleReason::Mutation)
        );
        assert_eq!(menu.read(Clone::clone), state());
        host.no_success();
    }
}

#[test]
fn token_from_another_session_invalid_input_zero_mutations() {
    let (mut first, menu) = fixture();
    let decision = token(&mut first, &menu, menu.read(placement));
    let mut other = Harness::new();
    other.open_action(menu.id);
    let epoch = menu.inner.borrow().epoch;
    assert_eq!(
        apply(&mut other, decision, Reply::Allow),
        Outcome::InvalidInput(InvalidReason::ForeignSavepoint)
    );
    assert_eq!(menu.inner.borrow().epoch, epoch);
    assert_eq!(menu.read(Clone::clone), state());
    other.no_success();
    first.no_success();
    other.close(false);
    first.close(false);
}

#[test]
fn changed_world_or_side_stale_without_retry_or_retarget() {
    for change_world in [false, true] {
        let (mut host, menu) = fixture();
        assert_eq!(
            host.place(&menu, |host| {
                if change_world {
                    host.identity.world = WorldEpoch::new(18);
                } else {
                    host.identity.side = Side::Client;
                }
                Reply::Allow
            }),
            Outcome::Stale(if change_world {
                StaleReason::World
            } else {
                StaleReason::Side
            })
        );
        assert_eq!(menu.read(Clone::clone), state());
        assert_eq!((host.decisions, host.dispatches), (1, 1));
        host.no_success();
    }
}

#[test]
fn malformed_listener_slot_or_layout_invalid_full_rollback() {
    for layout in [false, true] {
        let (mut host, menu) = fixture();
        assert_eq!(
            host.place(&menu, |host| {
                menu.edit(host, |s| {
                    if layout {
                        s.capacities.pop();
                    } else {
                        s.drag.push(99);
                    }
                    s.cost = 5;
                    s.rng = 999;
                });
                host.defer(dropped());
                Reply::Allow
            }),
            Outcome::InvalidInput(if layout {
                InvalidReason::Layout
            } else {
                InvalidReason::Slot
            })
        );
        assert_eq!(menu.read(Clone::clone), state());
        host.no_success();
        assert_eq!(host.place(&menu, |_| Reply::Allow), Outcome::Applied);
    }
}

#[test]
fn full_stack_components_participate_and_component_a_b_a_is_stale() {
    let (mut host, menu) = fixture();
    assert_eq!(
        host.place(&menu, |host| {
            menu.edit(host, |s| s.cursor.as_mut().unwrap().damage = 1);
            menu.edit(host, |s| s.cursor.as_mut().unwrap().damage = 0);
            Reply::Allow
        }),
        Outcome::Stale(StaleReason::Mutation)
    );
    assert_eq!(menu.read(Clone::clone), state());
    host.no_success();
}

#[test]
fn empty_decision_dispatches_nothing() {
    let (mut host, menu) = fixture();
    assert_eq!(
        run(
            &mut host,
            &menu,
            |_| Plan::new(ActionKind::Placement, vec![]),
            |_| panic!("no-action dispatch")
        ),
        Ok(Outcome::Canceled)
    );
    assert_eq!(host.dispatches, 0);
    assert_eq!(menu.read(Clone::clone), state());
    host.no_success();
}

#[test]
fn decide_unwind_releases_action_and_next_placement_succeeds() {
    let (mut host, menu) = fixture();
    assert_eq!(
        run(
            &mut host,
            &menu,
            |_| panic!("decide failed"),
            |_| panic!("must not fire")
        ),
        Ok(Outcome::ListenerFailed)
    );
    assert_eq!(host.frames.len(), 1);
    assert!(!host.menu_busy(menu.id));
    assert_eq!(host.dispatches, 0);
    assert_eq!(host.place(&menu, |_| Reply::Allow), Outcome::Applied);
}

#[test]
fn live_pre_fire_validation_refuses_before_listener_dispatch() {
    let (mut host, menu) = fixture();
    host.before_fire = Some(|host| host.target = Residence::Missing);
    assert_eq!(
        host.place(&menu, |_| panic!("must validate before fire")),
        Outcome::Stale(StaleReason::Target)
    );
    assert_eq!((host.decisions, host.dispatches), (1, 0));
    assert_eq!(menu.read(Clone::clone), state());
    host.no_success();
    assert_eq!(host.frames.len(), 1);
}

#[test]
fn every_refused_action_publishes_zero_listener_packets_drops_and_costs_at_root() {
    for case in 0..6 {
        let (mut host, menu) = fixture();
        let result = host.place(&menu, |host| {
            host.defer(packet());
            host.defer(dropped());
            host.defer(cost(2));
            match case {
                0 => Reply::Cancel,
                1 => {
                    menu.edit(host, |s| s.cursor = stack("minecraft:stone", 4));
                    Reply::Allow
                }
                2 => {
                    menu.edit(host, |s| s.cursor = stack("minecraft:dirt", 3));
                    menu.edit(host, |s| s.cursor = stack("minecraft:stone", 3));
                    Reply::Allow
                }
                3 => {
                    menu.edit(host, |s| s.instance = 8);
                    Reply::Allow
                }
                4 => Reply::Invalid(InvalidReason::Numeric),
                5 => Reply::Failed,
                _ => unreachable!(),
            }
        });
        assert_eq!(
            result,
            match case {
                0 => Outcome::Canceled,
                1 | 2 => Outcome::Stale(StaleReason::Mutation),
                3 => Outcome::Stale(StaleReason::MenuInstance),
                4 => Outcome::InvalidInput(InvalidReason::Numeric),
                5 => Outcome::ListenerFailed,
                _ => unreachable!(),
            }
        );
        assert_eq!(menu.read(Clone::clone), state());
        host.no_success();
        host.close(true);
        assert_eq!(host.published, []);
    }
}

// Abandoned decisions: the token is dropped without apply, once by the refusal
// before dispatch and once by a listener unwind. Either way the savepoint is
// closed (frame popped, which drops its save-gate lease), the menu is free and
// restored, and nothing is applied or published.
#[test]
fn abandoned_token_closes_savepoint_and_publishes_nothing() {
    for unwind in [false, true] {
        let (mut host, menu) = fixture();
        if !unwind {
            host.before_fire = Some(|host| host.identity.side = Side::Client);
        }
        let outcome = host.place(&menu, |host| {
            menu.edit(host, |s| s.slots[2] = stack("minecraft:dirt", 2));
            host.defer(dropped());
            panic!("listener trap with a live decision");
        });
        if unwind {
            assert_eq!(outcome, Outcome::ListenerFailed);
            assert_eq!(host.dispatches, 1);
        } else {
            assert_eq!(outcome, Outcome::Stale(StaleReason::Side));
            assert_eq!(host.dispatches, 0);
        }
        assert_eq!(host.frames.len(), 1);
        assert!(host.frames[0].action.is_none());
        assert!(!host.menu_busy(menu.id));
        assert_eq!(menu.read(Clone::clone), state());
        host.close(true);
        host.no_success();
    }
}
