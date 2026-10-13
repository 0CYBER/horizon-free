//! Named timings within a frame, for finding what makes a frame slow:
//! `HORIZON_SPIKES=MS` prints the spans of every frame whose CPU time
//! exceeds MS milliseconds. Costs one clock read per span when enabled and
//! a relaxed load otherwise.
use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

static ENABLED: AtomicBool = AtomicBool::new(false);
thread_local! {
    static SPANS: RefCell<Vec<(&'static str, f32)>> = const { RefCell::new(Vec::new()) };
}

pub fn enable(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}
/// Run `f`, recording its duration under `name` when spans are enabled.
#[inline]
pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    if !ENABLED.load(Ordering::Relaxed) {
        return f();
    }
    let started = Instant::now();
    let out = f();
    let ms = started.elapsed().as_secs_f32() * 1000.0;
    SPANS.with(|s| s.borrow_mut().push((name, ms)));
    out
}
/// The frame's spans, summed by name, slowest first; clears them.
pub fn take() -> Vec<(&'static str, f32)> {
    let spans = SPANS.with(|s| std::mem::take(&mut *s.borrow_mut()));
    let mut sums: Vec<(&'static str, f32)> = Vec::new();
    for (name, ms) in spans {
        match sums.iter_mut().find(|(n, _)| *n == name) {
            Some(entry) => entry.1 += ms,
            None => sums.push((name, ms)),
        }
    }
    sums.sort_by(|a, b| b.1.total_cmp(&a.1));
    sums
}
