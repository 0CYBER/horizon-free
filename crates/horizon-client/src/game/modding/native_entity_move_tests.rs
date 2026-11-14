//! The `entity_move` hook's contract at its one firing point,
//! `move_gate::arm` (#2414, design #2308): what a listener may decide for one
//! physical move, what is refused, and what a panic leaves behind.
use horizon_core::geometry::Vec3;
use horizon_world::entity_move::MoverType;
use horizon_world::move_gate::MoveGate;
use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};
use mod_api::{EntityMoveEvent, Flow, Hooks, LoopContext, MoveCause};
use std::sync::{Arc, Mutex};

/// The server's loop context, the side the hook fires on: no resources, just
/// the side and the origin a tick's own loop context carries.
fn server_context() -> horizon_world::native_load_context::LoopContext {
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
    .loop_context(LoopSide::Server, [])
}

/// What the probe listener answers for one move.
#[derive(Clone, Copy)]
enum Answer {
    Continue,
    Cancel,
    Replace([f64; 3]),
    Malformed([f64; 3]),
    Panic,
}

/// What the probe heard: entity, cause and the requested delta.
type Heard = Vec<(u32, MoveCause, [f64; 3])>;

/// A listener that records the move it heard and answers `Answer`.
struct Probe {
    answer: Answer,
    heard: Arc<Mutex<Heard>>,
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
        self.heard
            .lock()
            .unwrap()
            .push((event.entity, event.cause, *event.delta));
        match self.answer {
            Answer::Continue => Flow::Continue,
            Answer::Cancel => Flow::Cancel,
            Answer::Replace(delta) | Answer::Malformed(delta) => {
                *event.delta = delta;
                Flow::Continue
            }
            Answer::Panic => panic!("probe listener unwinds"),
        }
    }
}

/// One armed move with the probe installed: the gate it produced and what the
/// listener heard. The loop context is the server's, the side the hook fires
/// on.
fn armed_with(answer: Answer) -> (MoveGate, Heard) {
    let mut gate = MoveGate::new();
    let (log, mods) = install(answer);
    mod_platform::testing::with_mods(mods, || {
        horizon_client::move_gate::arm(
            &mut gate,
            &server_context(),
            7,
            "minecraft:zombie",
            MoverType::SelfMovement,
            Vec3::new(1.0, 0.0, 0.0),
        );
    });
    (gate, std::mem::take(&mut *log.lock().unwrap()))
}

fn install(answer: Answer) -> (Arc<Mutex<Heard>>, mod_api::Mods) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe {
            answer,
            heard: Arc::clone(&log),
        }),
    )]
    .into();
    (log, mods)
}

/// A Continue listener leaves the armed movement as it was, and hears the
/// entity, its cause and the requested delta.
#[test]
fn continue_runs_the_requested_move() {
    let (gate, heard) = armed_with(Answer::Continue);
    assert_eq!(heard, vec![(7, MoveCause::SelfMovement, [1.0, 0.0, 0.0])]);
    assert!(!gate.skipped());
    assert_eq!(gate.delta(), Vec3::new(1.0, 0.0, 0.0));
}

/// A cancelling listener skips the invocation: the gate says so, and the
/// movement is not applied.
#[test]
fn cancel_skips_the_invocation() {
    let (gate, _) = armed_with(Answer::Cancel);
    assert!(gate.skipped());
}

/// A replacement delta is armed instead of the requested one, still through
/// every ordinary restriction the kind's physics applies.
#[test]
fn a_replacement_delta_is_armed() {
    let (gate, _) = armed_with(Answer::Replace([0.0, 0.0, 1.0]));
    assert!(!gate.skipped());
    assert_eq!(gate.delta(), Vec3::new(0.0, 0.0, 1.0));
}

/// A replacement outside the ±64-block budget, or one that is not a finite
/// number, is refused whole: the movement stands as it was (design #2308:
/// reject, never clamp).
#[test]
fn a_malformed_replacement_is_refused_whole() {
    for delta in [
        [f64::NAN, 0.0, 0.0],
        [0.0, f64::INFINITY, 0.0],
        [65.0, 0.0, 0.0],
    ] {
        let (gate, _) = armed_with(Answer::Malformed(delta));
        assert!(!gate.skipped(), "a malformed move must not skip it");
        assert_eq!(
            gate.delta(),
            Vec3::new(1.0, 0.0, 0.0),
            "the requested movement stands: {delta:?}"
        );
    }
}

/// A listener that unwinds keeps the **requested** delta — its writes die
/// with it — and the poison skips **this hook** of its mod for the session:
/// the next move runs without it, and neither move panics out of the gate
/// (#2414 maintainer ruling). The same mod's other hooks still run, and a
/// panic anywhere else does not poison anyone — the two tests below.
#[test]
fn a_panicking_listener_keeps_the_requested_move_and_is_disabled() {
    let (log, mods) = install(Answer::Panic);
    mod_platform::testing::with_mods(mods, || {
        let mut gate = MoveGate::new();
        horizon_client::move_gate::arm(
            &mut gate,
            &server_context(),
            7,
            "minecraft:zombie",
            MoverType::SelfMovement,
            Vec3::new(1.0, 0.0, 0.0),
        );
        assert!(!gate.skipped());
        assert_eq!(gate.delta(), Vec3::new(1.0, 0.0, 0.0));
        // The poison skips this hook of the mod: this arm hears nobody.
        let mut gate = MoveGate::new();
        horizon_client::move_gate::arm(
            &mut gate,
            &server_context(),
            7,
            "minecraft:zombie",
            MoverType::SelfMovement,
            Vec3::new(2.0, 0.0, 0.0),
        );
        assert_eq!(gate.delta(), Vec3::new(2.0, 0.0, 0.0));
    });
    assert_eq!(
        log.lock().unwrap().len(),
        1,
        "the poisoned listener is not dispatched again"
    );
}

/// The requested delta, not an earlier listener's replacement, is what a
/// later panicking listener leaves behind (#2414 maintainer ruling: «A panic
/// keeps the requested delta (not a replaced one)»).
#[test]
fn a_panic_after_a_replacement_keeps_the_requested_delta() {
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new(
            "replacer",
            Box::new(Probe {
                answer: Answer::Replace([0.0, 0.0, 9.0]),
                heard: Arc::new(Mutex::new(Vec::new())),
            }),
        ),
        mod_api::ModEntry::new(
            "panicker",
            Box::new(Probe {
                answer: Answer::Panic,
                heard: Arc::new(Mutex::new(Vec::new())),
            }),
        ),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut gate = MoveGate::new();
        horizon_client::move_gate::arm(
            &mut gate,
            &server_context(),
            7,
            "minecraft:zombie",
            MoverType::SelfMovement,
            Vec3::new(1.0, 0.0, 0.0),
        );
        assert!(!gate.skipped());
        assert_eq!(
            gate.delta(),
            Vec3::new(1.0, 0.0, 0.0),
            "the requested delta, not the (0, 0, 9) replacement"
        );
    });
}

/// With no listener the movement is armed unchanged: the same decision a move
/// without the hook produces, for the no-listener oracle of design #2308.
#[test]
fn without_listeners_the_move_stands() {
    let mut gate = MoveGate::new();
    mod_platform::testing::with_mods(
        Vec::<mod_api::ModEntry>::new().into_iter().collect(),
        || {
            horizon_client::move_gate::arm(
                &mut gate,
                &server_context(),
                7,
                "minecraft:zombie",
                MoverType::Piston,
                Vec3::new(0.5, 0.0, 0.0),
            );
        },
    );
    assert!(!gate.skipped());
    assert_eq!(gate.delta(), Vec3::new(0.5, 0.0, 0.0));
}

/// A mod whose `entity_move` listener panicked keeps every other hook: the
/// poison skips this hook alone (#2414 ruling), so its `entity_tick_pre`
/// still hears the same entity on the same session.
#[test]
fn a_poisoned_move_listener_keeps_its_other_hooks() {
    use mod_api::{EntityTickPreEvent, ServerHost};
    struct Both {
        moves: Arc<Mutex<Heard>>,
        ticks: Arc<Mutex<usize>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Both {
        fn entity_move(&self, _: &dyn LoopContext, _: &mut EntityMoveEvent<'_>) -> Flow {
            self.moves.lock().unwrap().clear();
            panic!("the move listener unwinds")
        }
        fn entity_tick_pre(&self, _: &mut dyn ServerHost, _: &mut EntityTickPreEvent<'_>) -> Flow {
            *self.ticks.lock().unwrap() += 1;
            Flow::Continue
        }
    }
    let ticks = Arc::new(Mutex::new(0usize));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "both",
        Box::new(Both {
            moves: Arc::new(Mutex::new(Vec::new())),
            ticks: ticks.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let context = server_context();
        let mut gate = MoveGate::new();
        horizon_client::move_gate::arm(
            &mut gate,
            &context,
            7,
            "minecraft:zombie",
            MoverType::SelfMovement,
            Vec3::new(1.0, 0.0, 0.0),
        );
        assert_eq!(gate.delta(), Vec3::new(1.0, 0.0, 0.0));
        // The other hook of the same mod still runs, on the same session.
        struct World;
        impl mod_api::Host for World {}
        mod_api::impl_host_areas!(World);
        impl ServerHost for World {}
        mod_api::impl_server_host_areas!(World);
        let mut world = World;
        let mut canceled = false;
        mod_api::dispatch_entity_tick_pre(
            &mut world,
            "minecraft:overworld",
            7,
            "minecraft:zombie",
            &mut canceled,
        );
        assert_eq!(*ticks.lock().unwrap(), 1, "the other hook still runs");
    });
}

/// A panic outside `entity_move` — another hook of the same mod, unwound by
/// its own caller, as Parts' panics are (#3685) — poisons nobody: the next
/// move still reaches the mod's listener.
#[test]
fn a_panic_elsewhere_does_not_poison_the_move_listener() {
    use mod_api::{EntityTickPreEvent, ServerHost};
    struct Ticking {
        moves: Arc<Mutex<Heard>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Ticking {
        fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
            self.moves
                .lock()
                .unwrap()
                .push((event.entity, event.cause, *event.delta));
            Flow::Continue
        }
        fn entity_tick_pre(&self, _: &mut dyn ServerHost, _: &mut EntityTickPreEvent<'_>) -> Flow {
            panic!("a panic outside entity_move")
        }
    }
    let moves = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "ticking",
        Box::new(Ticking {
            moves: moves.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let context = server_context();
        // The other hook unwinds; its caller catches it, as the game catches
        // a Part's panic (#3685).
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            struct World;
            impl mod_api::Host for World {}
            mod_api::impl_host_areas!(World);
            impl ServerHost for World {}
            mod_api::impl_server_host_areas!(World);
            let mut world = World;
            let mut canceled = false;
            mod_api::dispatch_entity_tick_pre(
                &mut world,
                "minecraft:overworld",
                7,
                "minecraft:zombie",
                &mut canceled,
            );
        }));
        assert!(caught.is_err(), "the other hook unwound");
        // The move listener is not poisoned by that panic.
        let mut gate = MoveGate::new();
        horizon_client::move_gate::arm(
            &mut gate,
            &context,
            7,
            "minecraft:zombie",
            MoverType::SelfMovement,
            Vec3::new(1.0, 0.0, 0.0),
        );
        assert_eq!(
            moves.lock().unwrap().len(),
            1,
            "the move listener still runs after a panic elsewhere"
        );
    });
}
