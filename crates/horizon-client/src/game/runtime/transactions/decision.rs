//! Internal action coordinator shared by storage adapters.
//!
//! The storage here is deliberately encapsulated: every edit advances a complete
//! menu epoch, including equal writes and rollback. Actual game menu owners use
//! the engine adapter; resource-free fixtures use their own typed storage view.
//! Both use this same admission, dispatch and savepoint lifetime.
//!
//! Hook routes are not connected yet; allow dead code at this internal boundary
//! during the stage-3 migration rather than exporting a public token API.
#![allow(dead_code)]

use super::After as Effect;
use horizon_client::items::ItemStack;
use mod_api::{EntityRef, WorldEpoch};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

#[path = "menu_owner.rs"]
mod menu_owner;
#[path = "decision_storage.rs"]
mod storage;
pub(in crate::game) use menu_owner::{MenuOwner, OwnerStamp};
#[path = "decision_engine.rs"]
pub(in crate::game) mod engine;
pub(in crate::game) use storage::ActionHost;
pub(super) use storage::{ActiveAction, Journal};

/// Fail closed on exhaustion: a generation must never wrap back to an old one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Generation(u64);

impl Generation {
    fn advance(&mut self) {
        self.0 = self.0.checked_add(1).expect("action generation exhausted");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::game) struct MenuId(u64);

impl MenuId {
    pub(in crate::game) fn fresh() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(
            NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("menu identities exhausted"),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Side {
    Client,
    Server,
}

/// EntityRef already carries the server-wide lifecycle epoch and id. No second
/// entity generation, borrowed entity pointer, or mod-controlled token bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) struct Identity {
    pub side: Side,
    pub world: WorldEpoch,
    pub actor: EntityRef,
    pub target: EntityRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Residence {
    Found,
    Taken,
    Missing,
}

/// All state consulted by the internal menu fixtures. The complete stack value
/// includes components. Recipe/rule answers are stored values, never callbacks
/// invoked during validation. RNG is a reserved state, not a shared draw.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) struct MenuState {
    pub instance: u64,
    pub container: u64,
    pub cursor: Option<ItemStack>,
    pub slots: Vec<Option<ItemStack>>,
    pub capacities: Vec<u32>,
    pub recipe: Option<String>,
    pub registry: u64,
    pub rules: u64,
    pub permitted: bool,
    pub cost: u32,
    pub drag: Vec<usize>,
    pub backing: u64,
    pub rng: u64,
}

struct MenuInner {
    epoch: Generation,
    state: MenuState,
}

/// Owning indirection is internal; no borrowed menu survives dispatch. The id
/// names storage, while `instance` distinguishes replacement menus in it.
#[derive(Clone)]
pub(in crate::game) struct Menu {
    id: MenuId,
    inner: Rc<RefCell<MenuInner>>,
}

impl Menu {
    pub(in crate::game) fn new(state: MenuState) -> Self {
        Self {
            id: MenuId::fresh(),
            inner: Rc::new(RefCell::new(MenuInner {
                epoch: Generation::default(),
                state,
            })),
        }
    }

    pub(in crate::game) fn read<R>(&self, f: impl FnOnce(&MenuState) -> R) -> R {
        f(&self.inner.borrow().state)
    }

    /// Journal before granting write access; even an equal or panicking write
    /// advances the epoch. Rollback restores content, never the epoch counter.
    pub(in crate::game) fn edit<H: ActionHost, R>(
        &self,
        host: &mut H,
        f: impl FnOnce(&mut MenuState) -> R,
    ) -> R {
        host.journal().keep(self);
        let mut inner = self.inner.borrow_mut();
        inner.epoch.advance();
        f(&mut inner.state)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum ActionKind {
    Placement,
    QuickMove,
    RecipePlacement,
}

#[derive(Clone, Debug)]
pub(in crate::game) enum Mutation {
    Cursor(Option<ItemStack>),
    Slot(usize, Option<ItemStack>),
    Cost(u32),
    Rng(u64),
}

/// Use the existing deferred-effect vocabulary and root publisher. There is no
/// parallel packet/drop publication path specific to decisions.
pub(in crate::game) enum Step {
    Write(Mutation),
    Effect(Effect),
}

pub(in crate::game) struct Plan {
    kind: ActionKind,
    steps: Vec<Step>,
}

impl Plan {
    pub(in crate::game) fn new(kind: ActionKind, steps: Vec<Step>) -> Self {
        Self { kind, steps }
    }
}

/// Typed whole-menu read set, conservative until all real route writes can
/// supply complete per-slot epochs. Snapshots supplement, never replace, epoch.
pub(in crate::game) struct Reads {
    epoch: Generation,
    menu: MenuState,
}

struct Proposed<T, R, P> {
    identity: Identity,
    savepoint: u64,
    target: T,
    reads: R,
    plan: P,
}

/// Opaque, deliberately non-Clone and non-Copy. Single use is a by-value
/// property: `apply` takes the decision, so every terminal outcome consumes
/// it and a second application does not compile. A decision dropped without
/// `apply` publishes nothing; the action guard closes its savepoint.
pub(in crate::game) struct Decision<T = Menu, R = Reads, P = FrozenPlan>(Proposed<T, R, P>);

/// Storage-specific capture, validation and frozen writes. The coordinator owns
/// admission, identity checks, dispatch, consumption and savepoint lifetime for
/// both real menu owners and the resource-free storage fixtures.
pub(in crate::game) trait ActionTarget<H: ActionHost>: Clone {
    type Reads;
    type View: ?Sized;
    type Plan;
    type Frozen;

    fn id(&self) -> MenuId;
    fn keep(&self, _host: &mut H) {}
    fn capture(&self, host: &H) -> Result<Self::Reads, Outcome>;
    fn view(reads: &Self::Reads) -> &Self::View;
    fn freeze(&self, reads: &Self::Reads, plan: Self::Plan) -> Result<Self::Frozen, InvalidReason>;
    fn empty(plan: &Self::Frozen) -> bool;
    fn validate(&self, host: &H, reads: &Self::Reads) -> Result<(), Outcome>;
    fn apply(&self, host: &mut H, plan: Self::Frozen);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum StaleReason {
    World,
    Side,
    Actor,
    Target,
    MenuInstance,
    Mutation,
    Snapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum InvalidReason {
    ForeignSavepoint,
    ForeignSession,
    Slot,
    Count,
    Numeric,
    Layout,
    Permission,
    Plan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Outcome {
    Applied,
    Canceled,
    Stale(StaleReason),
    InvalidInput(InvalidReason),
    ListenerFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Admission {
    Busy,
    NoTransaction,
    MissingHolder,
}

/// Validated callback output is separate from the token's immutable plan.
/// This stage has no operation-specific output overrides: future routes must
/// define their own typed output contract rather than editing the token.
pub(in crate::game) enum Reply {
    Allow,
    Cancel,
    Invalid(InvalidReason),
    Failed,
}

/// Boundary helpers for untrusted numbers. No float-to-int saturation or
/// overflowing arithmetic may turn malformed output into a valid write.
pub(in crate::game) fn listener_count(value: f64, maximum: u32) -> Result<u32, InvalidReason> {
    if !value.is_finite() {
        return Err(InvalidReason::Numeric);
    }
    if value < 0.0 || value.fract() != 0.0 || value > f64::from(maximum) {
        return Err(InvalidReason::Count);
    }
    Ok(value as u32)
}

pub(in crate::game) fn add_count(a: u32, b: u32) -> Result<u32, InvalidReason> {
    a.checked_add(b).ok_or(InvalidReason::Count)
}

fn validate_state(state: &MenuState) -> Result<(), InvalidReason> {
    if state.slots.len() != state.capacities.len() {
        return Err(InvalidReason::Layout);
    }
    if state.drag.iter().any(|slot| *slot >= state.slots.len()) {
        return Err(InvalidReason::Slot);
    }
    if state.cursor.as_ref().is_some_and(|s| s.count == 0) {
        return Err(InvalidReason::Count);
    }
    for (stack, capacity) in state.slots.iter().zip(&state.capacities) {
        if stack
            .as_ref()
            .is_some_and(|s| s.count == 0 || s.count > *capacity)
        {
            return Err(InvalidReason::Count);
        }
    }
    Ok(())
}

impl<T, R, P> Proposed<T, R, P> {
    fn validate<H: ActionHost>(&self, host: &H) -> Result<(), Outcome>
    where
        T: ActionTarget<H, Reads = R, Frozen = P>,
    {
        if !host.action_is_current(self.savepoint, self.target.id()) {
            return Err(Outcome::InvalidInput(if host.has_transaction() {
                InvalidReason::ForeignSavepoint
            } else {
                InvalidReason::ForeignSession
            }));
        }
        let live = host.identity();
        if live.world != self.identity.world {
            return Err(Outcome::Stale(StaleReason::World));
        }
        if live.side != self.identity.side {
            return Err(Outcome::Stale(StaleReason::Side));
        }
        if live.actor != self.identity.actor
            || host.residence(self.identity.actor) != Residence::Found
        {
            return Err(Outcome::Stale(StaleReason::Actor));
        }
        if live.target != self.identity.target
            || host.residence(self.identity.target) != Residence::Found
        {
            return Err(Outcome::Stale(StaleReason::Target));
        }
        self.target.validate(host, &self.reads)
    }
}

/// Only a structurally validated plan can enter an opaque decision. Listener
/// output cannot mutate it, so apply need only recheck the live preconditions.
pub(in crate::game) struct FrozenPlan {
    kind: ActionKind,
    steps: Vec<Step>,
}

impl Plan {
    /// Validate the complete ordered write plan on a private preview once. A
    /// bad suffix cannot leave a successful quick-move prefix.
    fn freeze(self, reads: &MenuState) -> Result<FrozenPlan, InvalidReason> {
        let mut preview = reads.clone();
        if !preview.permitted {
            return Err(InvalidReason::Permission);
        }
        for step in &self.steps {
            if let Step::Write(write) = step {
                write_to(&mut preview, write)?;
                validate_state(&preview)?;
            }
        }
        Ok(FrozenPlan {
            kind: self.kind,
            steps: self.steps,
        })
    }
}

fn write_to(state: &mut MenuState, write: &Mutation) -> Result<(), InvalidReason> {
    match write {
        Mutation::Cursor(stack) => state.cursor.clone_from(stack),
        Mutation::Slot(slot, stack) => {
            let destination = state.slots.get_mut(*slot).ok_or(InvalidReason::Slot)?;
            destination.clone_from(stack);
        }
        Mutation::Cost(cost) => state.cost = *cost,
        Mutation::Rng(rng) => state.rng = *rng,
    }
    Ok(())
}

/// Takes the decision by value, so cancellation and malformed output consume
/// it too. Validation immediately precedes the first write to the live overlay.
fn apply<H: ActionHost, T: ActionTarget<H>>(
    host: &mut H,
    token: Decision<T, T::Reads, T::Frozen>,
    reply: Reply,
) -> Outcome {
    let Decision(proposed) = token;
    match reply {
        Reply::Cancel => return Outcome::Canceled,
        Reply::Failed => return Outcome::ListenerFailed,
        Reply::Invalid(reason) => return Outcome::InvalidInput(reason),
        Reply::Allow => {}
    }
    if let Err(outcome) = proposed.validate(host) {
        return outcome;
    }
    // No callbacks or reentrant admission between validation and writes.
    proposed.target.apply(host, proposed.plan);
    host.applied();
    Outcome::Applied
}

impl<H: ActionHost> ActionTarget<H> for Menu {
    type Reads = Reads;
    type View = MenuState;
    type Plan = Plan;
    type Frozen = FrozenPlan;

    fn id(&self) -> MenuId {
        self.id
    }

    fn capture(&self, _host: &H) -> Result<Reads, Outcome> {
        let inner = self.inner.borrow();
        Ok(Reads {
            epoch: inner.epoch,
            menu: inner.state.clone(),
        })
    }

    fn freeze(&self, reads: &Reads, plan: Plan) -> Result<FrozenPlan, InvalidReason> {
        plan.freeze(&reads.menu)
    }

    fn view(reads: &Reads) -> &MenuState {
        &reads.menu
    }

    fn empty(plan: &FrozenPlan) -> bool {
        plan.steps.is_empty()
    }

    fn validate(&self, _host: &H, reads: &Reads) -> Result<(), Outcome> {
        let inner = self.inner.borrow();
        validate_state(&inner.state).map_err(Outcome::InvalidInput)?;
        if inner.state.instance != reads.menu.instance {
            return Err(Outcome::Stale(StaleReason::MenuInstance));
        }
        if inner.epoch != reads.epoch {
            return Err(Outcome::Stale(StaleReason::Mutation));
        }
        if inner.state != reads.menu {
            return Err(Outcome::Stale(StaleReason::Snapshot));
        }
        Ok(())
    }

    fn apply(&self, host: &mut H, plan: FrozenPlan) {
        for step in plan.steps {
            match step {
                Step::Write(write) => self.edit(host, |state| {
                    write_to(state, &write).expect("validated internal plan");
                }),
                Step::Effect(effect) => host.defer(effect),
            }
        }
    }
}

/// No mutable borrow crosses decide or fire. The guard aborts on unwind in
/// decide/apply as well as dispatch; native unwind and a recoverable wasm trap
/// both become ListenerFailed. Process abort and failed rollback are not caught.
struct ActionGuard<'a, H: ActionHost> {
    host: &'a mut H,
    savepoint: u64,
    closed: bool,
}

impl<H: ActionHost> Drop for ActionGuard<'_, H> {
    fn drop(&mut self) {
        if !self.closed {
            self.host.close_action(self.savepoint, false);
        }
    }
}

/// Entry for subscribed actions only. Admission precedes savepoint creation,
/// snapshot/plan allocation, and dispatch. The enclosing Host transaction owns
/// every committed child and is the sole publisher of effects.
pub(in crate::game) fn run<H: ActionHost, T: ActionTarget<H>>(
    host: &mut H,
    menu: &T,
    decide: impl FnOnce(&H, &T::View) -> T::Plan,
    fire: impl FnOnce(&mut H) -> Reply,
) -> Result<Outcome, Admission> {
    if !host.has_transaction() {
        return Err(Admission::NoTransaction);
    }
    if host.menu_busy(menu.id()) {
        return Err(Admission::Busy);
    }
    let identity = host.identity();
    for holder in [identity.actor, identity.target] {
        match host.residence(holder) {
            Residence::Found => {}
            Residence::Taken => return Err(Admission::Busy),
            Residence::Missing => return Err(Admission::MissingHolder),
        }
    }
    let savepoint = host.open_action(menu.id());
    let guard = ActionGuard {
        host,
        savepoint,
        closed: false,
    };
    let mut guard = guard;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let reads = match menu.capture(guard.host) {
            Ok(reads) => reads,
            Err(outcome) => return outcome,
        };
        menu.keep(guard.host);
        let plan = decide(guard.host, T::view(&reads));
        let plan = match menu.freeze(&reads, plan) {
            Ok(plan) => plan,
            Err(reason) => return Outcome::InvalidInput(reason),
        };
        let token = Decision(Proposed {
            identity,
            savepoint,
            target: menu.clone(),
            reads,
            plan,
        });
        guard.host.decided();
        // Returning before apply drops the decision unused.
        if T::empty(&token.0.plan) {
            return Outcome::Canceled;
        }
        // First validation happens before dispatch.
        if let Err(outcome) = token.0.validate(guard.host) {
            return outcome;
        }
        guard.host.dispatched();
        let reply = fire(guard.host);
        apply(guard.host, token, reply)
    }))
    .unwrap_or(Outcome::ListenerFailed);
    guard
        .host
        .close_action(savepoint, outcome == Outcome::Applied);
    guard.closed = true;
    Ok(outcome)
}

/// Subscriber lookup is resolved by the caller at load, not by scanning mods.
/// The fast branch does not enter the coordinator, snapshot a menu, or construct
/// a Vec/read set/token. Baseline retains its enclosing transaction guarantees.
pub(in crate::game) fn route<H: ActionHost, T: ActionTarget<H>>(
    subscribed: bool,
    host: &mut H,
    menu: &T,
    baseline: impl FnOnce(&mut H) -> Outcome,
    decide: impl FnOnce(&H, &T::View) -> T::Plan,
    fire: impl FnOnce(&mut H) -> Reply,
) -> Result<Outcome, Admission> {
    if !subscribed {
        return Ok(baseline(host));
    }
    run(host, menu, decide, fire)
}

#[cfg(test)]
#[path = "decision_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "decision_game_tests.rs"]
mod game_tests;
