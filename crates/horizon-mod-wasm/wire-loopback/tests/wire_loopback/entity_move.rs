//! `entity_move` crosses to the guest and back with its decision intact
//! (#2414): the same hook the native listener sees, through the generator's
//! own wire proxies — no second route.
use super::*;

thread_local! {
    static HEARD: std::cell::RefCell<Option<(u32, String, mod_api::MoveCause, [f64; 3])>> =
        const { std::cell::RefCell::new(None) };
}

/// What the guest's generated `entity_move` listener heard on
/// its last call: entity, kind, cause, and the delta as it arrived.
pub(super) fn take_heard() -> Option<(u32, String, mod_api::MoveCause, [f64; 3])> {
    HEARD.with_borrow_mut(std::mem::take)
}

/// The guest side of the loopback answers `entity_move` with the flow the test
/// set in `ANSWER`, replacing the delta when it says so.
pub(super) enum GuestAnswer {
    Continue,
    Cancel,
    Replace([f64; 3]),
}
thread_local! {
    static ANSWER: std::cell::RefCell<Option<GuestAnswer>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn set_answer(answer: GuestAnswer) {
    ANSWER.with_borrow_mut(|slot| *slot = Some(answer));
}

pub(super) fn guest_listener(
    _: &dyn LoopContext,
    event: &mut mod_api::EntityMoveEvent<'_>,
) -> mod_api::Flow {
    HEARD.with_borrow_mut(|heard| {
        *heard = Some((
            event.entity,
            event.kind.to_owned(),
            event.cause,
            *event.delta,
        ))
    });
    match ANSWER.with_borrow_mut(std::mem::take) {
        None => mod_api::Flow::Continue,
        Some(GuestAnswer::Continue) => mod_api::Flow::Continue,
        Some(GuestAnswer::Cancel) => mod_api::Flow::Cancel,
        Some(GuestAnswer::Replace(delta)) => {
            *event.delta = delta;
            mod_api::Flow::Continue
        }
    }
}

/// One guest dispatch, by the argument shape the generator's own proxy uses
/// (`WasmHandler::entity_move` in `wire_host_generated.rs`): the fields cross
/// one by one, and the answer returns the flow with the written-back fields.
fn call_guest(
    module: &WasmMod,
    event: &mut mod_api::EntityMoveEvent<'_>,
) -> (mod_api::Flow, [f64; 3]) {
    let id = boundary::export_id("Hooks", "entity_move").unwrap();
    let (flow, delta) = module
        .call(
            CallClass::Hook,
            boundary::Frame::default(),
            id,
            None,
            |w| {
                w.put(&event.entity);
                w.put(event.kind);
                w.put(&event.cause);
                w.put(&*event.delta);
                w.put(&event.cancelled);
            },
            |r| {
                let flow: mod_api::Flow = r.take()?;
                let _entity: u32 = r.take()?;
                let _cause: mod_api::MoveCause = r.take()?;
                let _cancelled: bool = r.take()?;
                let delta: [f64; 3] = r.take()?;
                Ok((flow, delta))
            },
        )
        .unwrap();
    (flow, delta)
}

#[test]
fn wire_loopback_entity_move_carries_the_guests_decision() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let context = Context;
    let kind = "minecraft:zombie";
    for (answer, expected_flow, expected_delta) in [
        (
            GuestAnswer::Continue,
            mod_api::Flow::Continue,
            [1.0, 0.0, 0.0],
        ),
        (
            GuestAnswer::Replace([0.0, 0.0, 2.0]),
            mod_api::Flow::Continue,
            [0.0, 0.0, 2.0],
        ),
        (GuestAnswer::Cancel, mod_api::Flow::Cancel, [1.0, 0.0, 0.0]),
    ] {
        set_answer(answer);
        let mut event = mod_api::EntityMoveEvent {
            entity: 7,
            kind,
            cause: mod_api::MoveCause::Piston,
            delta: &mut [1.0, 0.0, 0.0],
            cancelled: false,
        };
        let (flow, delta) = call_guest(&module, &mut event);
        assert_eq!(flow, expected_flow);
        assert_eq!(delta, expected_delta);
        assert_eq!(
            take_heard(),
            Some((
                7,
                kind.to_owned(),
                mod_api::MoveCause::Piston,
                [1.0, 0.0, 0.0]
            )),
            "the guest heard the entity, kind, cause and requested delta"
        );
        let _ = &context;
    }
}

/// A native listener that replaces every delta with `[0.0, 0.0, 2.0]`: the
/// earlier listener of the trap test below.
struct NativeReplacer;

#[mod_api::__subscriptions]
impl mod_api::Hooks for NativeReplacer {
    fn entity_move(
        &self,
        _: &dyn LoopContext,
        event: &mut mod_api::EntityMoveEvent<'_>,
    ) -> mod_api::Flow {
        *event.delta = [0.0, 0.0, 2.0];
        mod_api::Flow::Continue
    }
}

/// A native listener that only counts its calls: the later listener of the
/// trap test, which must not run on the trapped invocation.
struct NativeTally(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for NativeTally {
    fn entity_move(
        &self,
        _: &dyn LoopContext,
        _: &mut mod_api::EntityMoveEvent<'_>,
    ) -> mod_api::Flow {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        mod_api::Flow::Continue
    }
}

/// A wasm listener whose call fails after an earlier listener replaced the
/// delta (review round 1): `APPEND_ATTACK` 3 stands in for a trapped or
/// refused guest call — the reply comes back short, decodes as nothing, and
/// the generated proxy sees `None`. The isolated dispatch gives it the same
/// treatment a native panic gets: the **requested** delta is restored — not
/// the replacer's `[0, 0, 2]` — the trapped listener is switched off for
/// this hook alone, and no later listener runs on that invocation.
#[test]
fn a_trapped_wasm_listener_after_a_replacement_keeps_the_requested_delta() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods = [
        mod_api::ModEntry::new("replacer", Box::new(NativeReplacer)),
        mod_api::ModEntry::new(
            "wire_test",
            Box::new(WasmMod {
                group: 0,
                subscribed: &[Some(mod_api::HookId::EntityMove)],
                listeners: &[],
                corrupt: false,
            }),
        ),
        mod_api::ModEntry::new("tally", Box::new(NativeTally(calls.clone()))),
    ];
    mod_api::testing::with_mods(std::sync::Arc::from(mods), || {
        let dispatch = |delta: &mut [f64; 3], canceled: &mut bool| {
            mod_api::dispatch_entity_move(
                &Context,
                7,
                "minecraft:zombie",
                mod_api::MoveCause::SelfMovement,
                delta,
                canceled,
            );
        };
        let (mut delta, mut canceled) = ([1.0, 0.0, 0.0], false);
        // The guest's reply loses its last byte: as a trapped or refused
        // call, nothing decodes.
        APPEND_ATTACK.set(3);
        dispatch(&mut delta, &mut canceled);
        APPEND_ATTACK.set(0);
        assert_eq!(
            delta,
            [1.0, 0.0, 0.0],
            "the requested delta survived the trap, not the replacer's [0.0, 0.0, 2.0]"
        );
        assert!(!canceled, "a trapped listener cancels nothing");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "no listener after the trapped one ran on that invocation"
        );
        dispatch(&mut delta, &mut canceled);
        assert_eq!(
            delta,
            [0.0, 0.0, 2.0],
            "the trapped listener is off: the replacer's delta stands again"
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "the later listener runs once the trapped one is switched off"
        );
    });
}

pub(super) struct EntityMoveHooks;
impl EntityMoveHooks {
    pub(super) fn entity_move(
        &self,
        context: &dyn LoopContext,
        event: &mut mod_api::EntityMoveEvent<'_>,
    ) -> mod_api::Flow {
        guest_listener(context, event)
    }
}
