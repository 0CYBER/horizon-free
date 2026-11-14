//! The `entity_move` fixture: the isolated dispatch a global cancel hook with
//! `isolate` generates (#2414 ruling) — one listener per `catch_unwind`, the
//! requested outputs surviving an unwind, and the poison skipping that one
//! listener on the next dispatch.

/// What the fixture listener heard: each requested delta, in dispatch order.
type Heard = Vec<[f64; 3]>;

struct Mover {
    answer: Answer,
    heard: Arc<Mutex<Heard>>,
}

/// What the fixture listener does with one move.
#[derive(Clone, Copy)]
enum Answer {
    /// Replace the delta, then unwind: its writes must not survive it.
    UnwindAfterReplace,
    /// Hear the move and leave it alone.
    Continue,
}

#[mod_api::__subscriptions]
impl Hooks for Mover {
    fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
        self.heard.lock().unwrap().push(*event.delta);
        match self.answer {
            Answer::UnwindAfterReplace => {
                *event.delta = [9.0, 9.0, 9.0];
                panic!("the fixture's move listener unwinds")
            }
            Answer::Continue => Flow::Continue,
        }
    }
}

fn mover(answer: Answer) -> (Arc<Mutex<Heard>>, ModEntry) {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let entry = ModEntry::new(
        "mover",
        Box::new(Mover {
            answer,
            heard: Arc::clone(&heard),
        }),
    );
    (heard, entry)
}

/// One isolated dispatch of the requested delta: what the game's own `canceled`
/// and delta are after it.
fn dispatch(requested: [f64; 3]) -> ([f64; 3], bool) {
    let mut delta = requested;
    let mut canceled = false;
    mod_api::dispatch_entity_move(
        &Loop("server"),
        7,
        "minecraft:zombie",
        MoveCause::SelfMovement,
        &mut delta,
        &mut canceled,
    );
    (delta, canceled)
}

/// A listener that unwinds after replacing the delta keeps the **requested**
/// delta — its writes die with it — and no later listener runs on that
/// invocation.
#[test]
fn an_unwinding_listener_keeps_the_requested_delta_and_ends_the_walk() {
    let (replacer_heard, replacer) = mover(Answer::UnwindAfterReplace);
    let (later_heard, later) = mover(Answer::Continue);
    install(vec![replacer, later].into());
    let (delta, canceled) = dispatch([1.0, 0.0, 0.0]);
    assert_eq!(
        delta,
        [1.0, 0.0, 0.0],
        "the replacement died with the unwind"
    );
    assert!(!canceled);
    assert_eq!(replacer_heard.lock().unwrap().len(), 1);
    assert!(
        later_heard.lock().unwrap().is_empty(),
        "no listener after the unwind runs on that invocation"
    );
}

/// The poison skips **this hook** of the mod for the session: the next
/// dispatch hears nobody, and neither dispatch panics out of the loop.
#[test]
fn a_poisoned_listener_is_skipped_by_the_hook_alone() {
    let (heard, entry) = mover(Answer::UnwindAfterReplace);
    install(vec![entry].into());
    dispatch([1.0, 0.0, 0.0]);
    let (delta, canceled) = dispatch([2.0, 0.0, 0.0]);
    assert_eq!(delta, [2.0, 0.0, 0.0]);
    assert!(!canceled);
    assert_eq!(
        heard.lock().unwrap().len(),
        1,
        "the poisoned listener is not dispatched again"
    );
}

/// An ordinary listener is untouched by isolation: it hears the move, and its
/// replacement stands.
#[test]
fn an_ordinary_listener_still_replaces_the_delta() {
    let (heard, entry) = mover(Answer::Continue);
    install(vec![entry].into());
    let (delta, canceled) = dispatch([1.0, 0.0, 0.0]);
    assert_eq!(delta, [1.0, 0.0, 0.0]);
    assert!(!canceled);
    assert_eq!(heard.lock().unwrap().len(), 1);
}

mod root {
    mod runtime {
        pub(super) struct ListenerScope;
        impl ListenerScope {
            pub(super) fn new() -> Self {
                Self
            }
            pub(super) fn enter(
                &mut self,
                entry: &'static super::ModEntry,
                hook: &'static str,
            ) -> &'static dyn super::Hooks {
                super::enter_listener(entry, hook).1
            }
        }
        thread_local! {
            /// The listeners the isolated dispatches poisoned, as (hook ordinal, id) pairs — the
            /// stand-in for the real runtime's per-hook sets.
            pub(super) static POISONED: std::cell::RefCell<std::collections::HashSet<(usize, &'static str)>> =
                std::cell::RefCell::new(std::collections::HashSet::new());
        }
        pub(super) fn poison(hook: super::HookId, id: &'static str) {
            POISONED.with(|poisoned| poisoned.borrow_mut().insert((hook as usize, id)));
        }
        pub(super) fn poisoned(hook: super::HookId, id: &str) -> bool {
            POISONED.with(|poisoned| {
                poisoned
                    .borrow()
                    .iter()
                    .any(|(poisoned_hook, poisoned_id)| {
                        *poisoned_hook == hook as usize && *poisoned_id == id
                    })
            })
        }
    }
}
