//! Opt-in test accounting for successful host/guest shared-memory copies.
use std::cell::Cell;
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub to_guest_bytes: usize,
    pub from_guest_bytes: usize,
}
thread_local! { static COUNTS: Cell<Option<Counts>> = const { Cell::new(None) }; }
pub(crate) fn record(to_guest: bool, bytes: usize) {
    COUNTS.with(|cell| {
        if let Some(mut c) = cell.get() {
            if to_guest {
                c.to_guest_bytes += bytes;
            } else {
                c.from_guest_bytes += bytes;
            }
            cell.set(Some(c));
        }
    });
}
/// Measures actual runtime copies, not guest-internal serialization or allocator moves.
pub fn measure<R>(run: impl FnOnce() -> R) -> (R, Counts) {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            COUNTS.with(|c| c.set(None));
        }
    }
    COUNTS.with(|c| {
        assert!(c.get().is_none());
        c.set(Some(Counts::default()));
    });
    let _guard = Guard;
    let result = run();
    (result, COUNTS.with(|c| c.get().unwrap()))
}
/// Runs `run` with every non-load guest call in this process given the load budget (30 000
/// epoch ticks, about 30 s) instead of a hook's 100, on whichever thread the game makes it. For
/// tests that each run alone in their process and measure work, not time: a debug build on a
/// loaded runner can take past 100 ms over a megabyte-sized call. A guest that never returns
/// still stops at the load budget.
pub fn with_load_budget<R>(run: impl FnOnce() -> R) -> R {
    use std::sync::atomic::Ordering;
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            crate::runtime::CALL_BUDGET.store(0, Ordering::Relaxed);
        }
    }
    crate::runtime::CALL_BUDGET.store(30_000, Ordering::Relaxed);
    let _guard = Guard;
    run()
}
