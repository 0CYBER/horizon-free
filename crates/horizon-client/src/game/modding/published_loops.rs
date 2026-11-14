//! Published loops (`[[native.loop]]`): hot loops the game owns, with a slot a native mod's helper
//! fills. Like every `[[native.*]]` table this is game code by another route, pinned to the exact
//! build: a loop's slot trait and cursor live beside the loop (`block_entity_ticks.rs` for
//! `block_entities.tick`) and lend the slot the loop's own state, and nothing here is `mod_api`.
//!
//! A loop is written once as a function generic over its slot; its entry, marked
//! `#[horizon_loop]`, calls it with `()`, which compiles to the game's own loop. A mod build compiles
//! one copy of the entry that runs the loop with the installed slots, nested by `(priority, mod
//! id)` (`(A, (B, ()))`: the first answer that is not [`LoopFlow::Continue`] wins), and installs it
//! through the entry's replacement cell. Their state is one value per `Game` (this file), so a
//! single-player process's client and server games never share one.
#![allow(
    dead_code,
    reason = "a mod build's copy of a loop entry uses what the game's own loop does not"
)]
use super::Game;
use horizon_client::Result;
use std::any::Any;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

/// What a slot answers for the entry the loop is about to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoopFlow {
    /// Run the entry, and ask the next mod's slot first.
    Continue,
    /// Leave this entry out of this run of the loop.
    Skip,
    /// End this run of the loop here; what the game does after the loop still runs.
    Stop,
}

/// One mod's row in a published loop, for the crash report: a mod build names each slot with one.
pub(crate) trait LoopMod {
    /// The call a panic inside this mod's slot is reported as.
    const CALL: mod_api::content::crash::ModCall;
}

/// A mod's slot `T`, each of whose calls is marked as inside the mod `N` names: made, called and
/// dropped only through [`LoopSlots`] and [`Self::call`].
pub(crate) struct Attributed<N, T> {
    slot: T,
    mod_row: PhantomData<fn() -> N>,
}

impl<N: LoopMod, T> Attributed<N, T> {
    /// Runs `call` on the slot, marked as inside the mod.
    #[inline(always)]
    pub(crate) fn call<R>(&mut self, call: impl FnOnce(&mut T) -> R) -> R {
        let _inside = mod_api::content::crash::enter_native_mod(N::CALL);
        call(&mut self.slot)
    }
}

/// A panic's payload, as `catch_unwind` returns it.
type Panic = Box<dyn Any + Send>;

/// The slots of one run of a loop, as a mod build nests them: `(A, (B, ()))` of [`Attributed`]
/// slots. Each mod's slot is made and dropped inside that mod's guard, so a panic in its `Default`
/// or its `Drop` names the mod and the loop (`hook loop <name>`) like a panic in its `before`.
pub(crate) trait LoopSlots: Sized + 'static {
    /// Every mod's slot, made with its `Default`. A panic in one mod's drops the slots made before
    /// it (with [`Self::release`]) and goes on unwinding.
    fn make() -> Self;
    /// Drops every mod's slot, each in its own catch: a panic in one mod's `Drop` is reported by
    /// the panic hook as that mod's when it happens, the other mods' slots are still dropped, and
    /// the process never aborts on a second panic. The first such panic is left in `first` when
    /// it holds none yet.
    fn release(self, first: &mut Option<Panic>);
}

impl LoopSlots for () {
    fn make() -> Self {}
    fn release(self, _: &mut Option<Panic>) {}
}

impl<A: LoopSlots, B: LoopSlots> LoopSlots for (A, B) {
    fn make() -> Self {
        let made = A::make();
        match catch_unwind(B::make) {
            Ok(rest) => (made, rest),
            Err(panic) => {
                made.release(&mut None);
                resume_unwind(panic)
            }
        }
    }
    fn release(self, first: &mut Option<Panic>) {
        self.0.release(first);
        self.1.release(first);
    }
}

impl<N: LoopMod + 'static, T: Default + 'static> LoopSlots for Attributed<N, T> {
    fn make() -> Self {
        let _inside = mod_api::content::crash::enter_native_mod(N::CALL);
        Self {
            slot: T::default(),
            mod_row: PhantomData,
        }
    }
    fn release(self, first: &mut Option<Panic>) {
        let _inside = mod_api::content::crash::enter_native_mod(N::CALL);
        if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(self.slot))) {
            first.get_or_insert(panic);
        }
    }
}

/// A loop's slots kept between runs, their type erased; dropped only through [`LoopSlots::release`].
trait Kept: Any {
    fn release_kept(self: Box<Self>, first: &mut Option<Panic>);
}

impl<T: LoopSlots> Kept for T {
    fn release_kept(self: Box<Self>, first: &mut Option<Panic>) {
        LoopSlots::release(*self, first);
    }
}

/// One published loop's slot state.
enum Cell {
    /// Before the first run, which makes the slots.
    New,
    /// Between runs.
    Ready(Box<dyn Kept>),
    /// Out for a run of the loop.
    Taken,
    /// A slot panicked (made, called or dropped): the loop runs without its slots.
    Poisoned,
}

/// The cells of the loops this game has run with slots, by loop name, in first-run order.
#[derive(Default)]
pub(super) struct LoopStates {
    cells: Vec<LoopEntry>,
}

struct LoopEntry {
    name: &'static str,
    cell: Cell,
    /// Whether the run without slots was logged, for a nested run or after a panic.
    logged: bool,
}

impl LoopStates {
    fn entry(&mut self, name: &'static str) -> &mut LoopEntry {
        let index = match self.cells.iter().position(|entry| entry.name == name) {
            Some(index) => index,
            None => {
                self.cells.push(LoopEntry {
                    name,
                    cell: Cell::New,
                    logged: false,
                });
                self.cells.len() - 1
            }
        };
        &mut self.cells[index]
    }

    /// The loop's slots for a run: `Some(None)` on the first, whose caller makes them; `None`
    /// while a run has them out or after a panic, logged once. A loop has one slot type in a
    /// build (the mod build's copy of its entry), so kept slots are always a `T`.
    fn take<T: LoopSlots>(&mut self, name: &'static str) -> Option<Option<Box<T>>> {
        let entry = self.entry(name);
        match std::mem::replace(&mut entry.cell, Cell::Taken) {
            Cell::New => Some(None),
            Cell::Ready(state) => Some((state as Box<dyn Any>).downcast().ok()),
            other => {
                if !entry.logged {
                    entry.logged = true;
                    log::warn!(
                        "Published loop {name}: {}; it runs without its mods' slots",
                        match other {
                            Cell::Poisoned => "a mod's slot panicked",
                            _ => "it was entered again while it ran",
                        }
                    );
                }
                entry.cell = other;
                None
            }
        }
    }

    fn put(&mut self, name: &'static str, cell: Cell) {
        self.entry(name).cell = cell;
    }
}

impl Drop for LoopStates {
    /// Releases the kept slots, each mod's inside its guard. A panic in a mod's `Drop` was
    /// reported by the panic hook, naming the mod; once every loop's slots are dropped the first
    /// such panic goes on unwinding, unless this game is dropped while another panic unwinds,
    /// which stays the one reported.
    fn drop(&mut self) {
        let mut first = None;
        for entry in self.cells.drain(..) {
            if let Cell::Ready(state) = entry.cell {
                state.release_kept(&mut first);
            }
        }
        if let Some(panic) = first
            && !std::thread::panicking()
        {
            resume_unwind(panic);
        }
    }
}

/// [`Game::run_published_loop`] on any owner of a [`LoopStates`] (`states` finds it).
fn run_with_slots<G, T: LoopSlots>(
    game: &mut G,
    states: fn(&mut G) -> &mut LoopStates,
    name: &'static str,
    run: impl FnOnce(&mut G, &mut T) -> Result<()>,
    vanilla: impl FnOnce(&mut G) -> Result<()>,
) -> Result<()> {
    let Some(kept) = states(game).take::<T>(name) else {
        return vanilla(game);
    };
    let mut slots = match kept {
        Some(slots) => slots,
        None => match catch_unwind(|| Box::new(T::make())) {
            Ok(slots) => slots,
            Err(panic) => {
                states(game).put(name, Cell::Poisoned);
                resume_unwind(panic)
            }
        },
    };
    match catch_unwind(AssertUnwindSafe(|| run(game, &mut slots))) {
        Ok(result) => {
            states(game).put(name, Cell::Ready(slots));
            result
        }
        Err(panic) => {
            // Poisoned before any mod's `Drop` runs. A panic while dropping was reported as its
            // mod's; this first one goes on unwinding.
            states(game).put(name, Cell::Poisoned);
            LoopSlots::release(*slots, &mut None);
            resume_unwind(panic)
        }
    }
}

impl Game<'_> {
    /// Runs the published loop `name` with the slots `T`: `run` with this game's slots for it
    /// (made on the first run), or `vanilla` (the loop with no slots) while the slots are out for
    /// an enclosing run or after a panic. A panic while making the slots or in `run` marks the
    /// loop poisoned, drops the slots made (each mod's inside its guard) and goes on unwinding to
    /// the crash path, which names the mod.
    pub(crate) fn run_published_loop<T: LoopSlots>(
        &mut self,
        name: &'static str,
        run: impl FnOnce(&mut Self, &mut T) -> Result<()>,
        vanilla: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()> {
        run_with_slots(self, |game| &mut game.loop_states, name, run, vanilla)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::content::crash::{ModCall, current_native_call};
    use std::cell::RefCell;

    thread_local! {
        /// What the test slots did, and the mod call each did it inside.
        static SEEN: RefCell<Vec<(&'static str, Option<ModCall>)>> = const { RefCell::new(Vec::new()) };
    }
    fn note(what: &'static str) {
        SEEN.with(|seen| seen.borrow_mut().push((what, current_native_call())));
    }
    fn seen() -> Vec<(&'static str, Option<ModCall>)> {
        SEEN.with(|seen| seen.take())
    }

    macro_rules! row {
        ($row:ident, $id:literal) => {
            struct $row;
            impl LoopMod for $row {
                const CALL: ModCall = ModCall {
                    id: $id,
                    hook: "loop l",
                    part: None,
                };
            }
        };
    }
    row!(A, "a");
    row!(B, "b");

    /// Counts runs; notes its drop.
    #[derive(Default)]
    struct Count(u32);
    impl Drop for Count {
        fn drop(&mut self) {
            note("count dropped");
        }
    }
    /// Panics in its `Default`.
    struct BadMake;
    impl Default for BadMake {
        fn default() -> Self {
            note("bad make");
            panic!("bad make");
        }
    }
    /// Panics in its `Drop`.
    #[derive(Default)]
    struct BadDrop;
    impl Drop for BadDrop {
        fn drop(&mut self) {
            note("bad drop");
            panic!("bad drop");
        }
    }

    /// A game for `run_with_slots`: its loop states, and how often each loop ran.
    #[derive(Default)]
    struct Owner {
        states: LoopStates,
        slotted: u32,
        vanilla: u32,
    }
    fn states(owner: &mut Owner) -> &mut LoopStates {
        &mut owner.states
    }
    fn run<T: LoopSlots>(owner: &mut Owner, slot: impl FnOnce(&mut T)) -> Result<()> {
        mod_platform::__private::initialize().unwrap();
        run_with_slots(
            owner,
            states,
            "l",
            |owner, slots: &mut T| {
                owner.slotted += 1;
                slot(slots);
                Ok(())
            },
            |owner| {
                owner.vanilla += 1;
                Ok(())
            },
        )
    }
    fn message(panic: Panic) -> String {
        panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .unwrap_or_default()
    }

    #[test]
    fn the_slots_are_kept_between_runs_and_made_once() {
        type Slots = (Attributed<A, Count>, ());
        let mut owner = Owner::default();
        for _ in 0..2 {
            run::<Slots>(&mut owner, |slots| slots.0.call(|count| count.0 += 1)).unwrap();
        }
        run::<Slots>(&mut owner, |slots| {
            assert_eq!(slots.0.call(|count| count.0), 2)
        })
        .unwrap();
        assert_eq!((owner.slotted, owner.vanilla), (3, 0));
        // Dropping the game drops each mod's slot inside that mod.
        drop(owner);
        assert_eq!(seen(), [("count dropped", Some(A::CALL))]);
    }

    /// A panic in a mod's `Default` is that mod's, made inside its guard; the slots made before it
    /// are dropped inside theirs, and the loop is poisoned, not left out.
    #[test]
    fn a_panicking_constructor_poisons_the_loop_and_names_its_mod() {
        type Slots = (Attributed<A, Count>, (Attributed<B, BadMake>, ()));
        let mut owner = Owner::default();
        let panic = catch_unwind(AssertUnwindSafe(|| run::<Slots>(&mut owner, |_| {})));
        assert_eq!(message(panic.unwrap_err()), "bad make");
        assert_eq!(
            seen(),
            [
                ("bad make", Some(B::CALL)),
                ("count dropped", Some(A::CALL))
            ]
        );
        assert!(matches!(owner.states.entry("l").cell, Cell::Poisoned));
        assert_eq!(current_native_call(), None);
        // The loop runs without its slots from now on.
        run::<Slots>(&mut owner, |_| {}).unwrap();
        assert_eq!((owner.slotted, owner.vanilla), (0, 1));
    }

    /// After a slot's panic the loop is poisoned and each mod's slot dropped inside that mod; a
    /// panic in one `Drop` neither stops the others' nor replaces the slot's panic.
    #[test]
    fn a_panicking_destructor_after_a_slot_panic_keeps_the_first_panic() {
        type Slots = (Attributed<A, BadDrop>, (Attributed<B, Count>, ()));
        let mut owner = Owner::default();
        let panic = catch_unwind(AssertUnwindSafe(|| {
            run::<Slots>(&mut owner, |_| panic!("slot"))
        }));
        assert_eq!(message(panic.unwrap_err()), "slot");
        assert_eq!(
            seen(),
            [
                ("bad drop", Some(A::CALL)),
                ("count dropped", Some(B::CALL))
            ]
        );
        assert!(matches!(owner.states.entry("l").cell, Cell::Poisoned));
        assert_eq!(current_native_call(), None);
    }

    /// Dropping a game drops every mod's kept slot; a panic in one mod's `Drop` unwinds once all
    /// are dropped.
    #[test]
    fn a_panicking_destructor_when_the_game_drops_unwinds_after_the_rest() {
        type Slots = (Attributed<A, BadDrop>, (Attributed<B, Count>, ()));
        let mut owner = Owner::default();
        run::<Slots>(&mut owner, |_| {}).unwrap();
        let panic = catch_unwind(AssertUnwindSafe(|| drop(owner)));
        assert_eq!(message(panic.unwrap_err()), "bad drop");
        assert_eq!(
            seen(),
            [
                ("bad drop", Some(A::CALL)),
                ("count dropped", Some(B::CALL))
            ]
        );
    }

    #[test]
    fn a_nested_run_gets_no_slots_and_logs_once() {
        type Slots = (Attributed<A, Count>, ());
        let mut owner = Owner::default();
        run::<Slots>(&mut owner, |_| {}).unwrap();
        let outer = owner.states.take::<Slots>("l").unwrap();
        assert!(owner.states.take::<Slots>("l").is_none());
        assert!(owner.states.entry("l").logged);
        assert!(owner.states.take::<Slots>("l").is_none());
        // The outer run puts them back; the next run has them.
        owner.states.put("l", Cell::Ready(outer.unwrap()));
        run::<Slots>(&mut owner, |slots| {
            assert_eq!(slots.0.call(|count| count.0), 0)
        })
        .unwrap();
        assert_eq!((owner.slotted, owner.vanilla), (2, 0));
        drop(owner);
        seen();
    }

    #[test]
    fn a_poisoned_loop_stays_without_slots() {
        let mut states = LoopStates::default();
        assert!(matches!(states.take::<()>("l"), Some(None)));
        states.put("l", Cell::Poisoned);
        assert!(states.take::<()>("l").is_none());
        assert!(states.take::<()>("l").is_none());
        assert!(matches!(states.entry("l").cell, Cell::Poisoned));
        // Another loop is not touched.
        assert!(states.take::<()>("other").is_some());
    }
}
