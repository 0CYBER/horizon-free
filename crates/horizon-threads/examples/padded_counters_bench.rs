//! What the pool's scheduler counters cost sharing one cache line against a line each, on the
//! notify path a frame or a tick thread takes to hand work to the pool: `notify` bumps `epoch`
//! while seven busy workers bump `far` and `released` and read `threads`, which is the false
//! sharing `jobs::Inner` pads away. The two layouts are twins — the same atomics, the same
//! operations, one generic body — so the only difference measured is the layout.
//! `cargo run --profile fast -p horizon-threads --example padded_counters_bench`
//!
//! The gate the padding has to clear (issue #18234): more than 5 % faster per notify. Each
//! variant's median and MAD are printed, so a run on a loaded machine shows itself as a wide
//! spread rather than as a verdict; the assertion is only on the direction, never on the margin.
use std::hint::black_box;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed, Ordering::SeqCst};
use std::time::{Duration, Instant};

/// One counter on a cache line of its own: what `jobs::Inner` now does per counter.
#[repr(align(64))]
struct Padded<T>(T);

impl<T> Padded<T> {
    const fn new(value: T) -> Self {
        Self(value)
    }
}

impl<T> Deref for Padded<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

const WORKERS: usize = 8;
/// The frame or tick thread's notifies: one per job it queues.
const NOTIFIES: u64 = 1_000_000;
const RUNS: usize = 15;

/// The pool's counters, as `jobs::Inner` carries them.
trait Counters: Sync {
    fn epoch(&self) -> &AtomicU64;
    fn sleepers(&self) -> &AtomicUsize;
    fn far(&self) -> &AtomicUsize;
    fn released(&self) -> &AtomicUsize;
    fn threads(&self) -> &AtomicUsize;
}

/// A line each: what the pool has now.
struct Spaced {
    epoch: Padded<AtomicU64>,
    sleepers: Padded<AtomicUsize>,
    far: Padded<AtomicUsize>,
    released: Padded<AtomicUsize>,
    threads: Padded<AtomicUsize>,
}

impl Spaced {
    fn new() -> Self {
        Self {
            epoch: Padded::new(AtomicU64::new(0)),
            sleepers: Padded::new(AtomicUsize::new(0)),
            far: Padded::new(AtomicUsize::new(0)),
            released: Padded::new(AtomicUsize::new(0)),
            threads: Padded::new(AtomicUsize::new(WORKERS)),
        }
    }
}

/// All of them in one line: what the pool had before (aligned so that the five fit one line,
/// the worst case of the layout being replaced).
#[repr(C, align(64))]
struct Packed {
    epoch: AtomicU64,
    sleepers: AtomicUsize,
    far: AtomicUsize,
    released: AtomicUsize,
    threads: AtomicUsize,
}

impl Packed {
    fn new() -> Self {
        Self {
            epoch: AtomicU64::new(0),
            sleepers: AtomicUsize::new(0),
            far: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
            threads: AtomicUsize::new(WORKERS),
        }
    }
}

macro_rules! counters {
    ($($ty:ident),*) => {$(
        impl Counters for $ty {
            fn epoch(&self) -> &AtomicU64 {
                &self.epoch
            }
            fn sleepers(&self) -> &AtomicUsize {
                &self.sleepers
            }
            fn far(&self) -> &AtomicUsize {
                &self.far
            }
            fn released(&self) -> &AtomicUsize {
                &self.released
            }
            fn threads(&self) -> &AtomicUsize {
                &self.threads
            }
        }
    )*};
}
counters!(Spaced, Packed);

/// One job of a busy worker: `work`'s epoch, `choose`'s threads and far, a far place every
/// eighth job (the compare-exchange that takes it and the fetch that ends it), a release of
/// buffers every sixty-fourth. A busy worker never goes to sleep, so it never writes
/// `sleepers`: that counter is the notifier's to read.
fn job<C: Counters>(counters: &C, round: u64) {
    let _epoch = counters.epoch().load(SeqCst);
    let threads = counters.threads().load(Relaxed);
    if round.is_multiple_of(8) {
        let mut far = counters.far().load(SeqCst);
        while far < threads.saturating_sub(1).max(1) {
            match counters
                .far()
                .compare_exchange_weak(far, far + 1, SeqCst, SeqCst)
            {
                Ok(_) => break,
                Err(now) => far = now,
            }
        }
        counters.far().fetch_sub(1, SeqCst);
    }
    if round.is_multiple_of(64) {
        counters.released().fetch_add(1, Relaxed);
        counters.released().fetch_sub(1, Relaxed);
    }
}

/// One notify, as `Inner::notify` does it: bump the epoch, then look at the sleepers.
fn notify<C: Counters>(counters: &C) {
    counters.epoch().fetch_add(1, SeqCst);
    black_box(counters.sleepers().load(SeqCst));
}

/// Seven busy workers and the one thread that queues their work: how long its notifies take
/// (`notify` runs on this thread, timed), and what the workers got through in the same while.
fn measure<C: Counters>(counters: &C) -> (Duration, u64) {
    let stop = AtomicBool::new(false);
    let stop = &stop;
    let done: Vec<AtomicU64> = (0..WORKERS - 1).map(|_| AtomicU64::new(0)).collect();
    // `spawn` returns before the thread runs, so the timed loop waits here until every worker
    // is on the counters: otherwise the first notifies share the line with fewer than seven
    // busy workers, and the startup time of seven threads lands in the measurement.
    let ready = std::sync::Barrier::new(WORKERS);
    let ready = &ready;
    let notified = std::thread::scope(|scope| {
        for slot in done.iter() {
            scope.spawn(move || {
                ready.wait();
                let mut round = 0;
                while !stop.load(Relaxed) {
                    job(counters, round);
                    round += 1;
                }
                slot.store(round, Relaxed);
            });
        }
        ready.wait();
        let start = Instant::now();
        for _ in 0..NOTIFIES {
            notify(counters);
        }
        let elapsed = start.elapsed();
        stop.store(true, Relaxed);
        elapsed
    });
    (notified, done.iter().map(|d| d.load(Relaxed)).sum())
}

/// The middle run's time and the median distance from it, in seconds.
fn median_mad(mut times: Vec<f64>) -> (f64, f64) {
    times.sort_by(f64::total_cmp);
    let median = times[times.len() / 2];
    let mut deviations: Vec<f64> = times.iter().map(|t| (t - median).abs()).collect();
    deviations.sort_by(f64::total_cmp);
    (median, deviations[deviations.len() / 2])
}

fn main() {
    assert!(
        !black_box(cfg!(debug_assertions)),
        "build with --profile fast"
    );
    let mut spaced = Vec::with_capacity(RUNS);
    let mut packed = Vec::with_capacity(RUNS);
    let mut spaced_jobs = Vec::with_capacity(RUNS);
    let mut packed_jobs = Vec::with_capacity(RUNS);
    // One warm-up round of each, so neither pays for a cold heap or a cold instruction cache.
    let (spaced_counters, packed_counters) = (Spaced::new(), Packed::new());
    measure(&spaced_counters);
    measure(&packed_counters);
    for run in 0..RUNS {
        // Interleaved and alternating which goes first, so a machine that slows down or speeds
        // up between runs hits both the same.
        for at in 0..2 {
            let spaced_now = (at == 0) == run.is_multiple_of(2);
            let (time, jobs) = if spaced_now {
                measure(&spaced_counters)
            } else {
                measure(&packed_counters)
            };
            if spaced_now {
                spaced.push(time.as_secs_f64());
                spaced_jobs.push(jobs as f64);
            } else {
                packed.push(time.as_secs_f64());
                packed_jobs.push(jobs as f64);
            }
        }
    }
    let (spaced_median, spaced_mad) = median_mad(spaced);
    let (packed_median, packed_mad) = median_mad(packed);
    let (spaced_ns, packed_ns) = (
        spaced_median / NOTIFIES as f64 * 1e9,
        packed_median / NOTIFIES as f64 * 1e9,
    );
    let gain = (packed_median / spaced_median - 1.0) * 100.0;
    println!(
        "notify on 7 busy workers, per notify: packed {packed_ns:.1} ns, MAD {:.1} %; padded {spaced_ns:.1} ns, MAD {:.1} %; padded is {gain:.1} % faster (gate 5 %: {})",
        packed_mad / packed_median * 100.0,
        spaced_mad / spaced_median * 100.0,
        if gain > 5.0 { "pass" } else { "fail" }
    );
    println!(
        "the 7 workers meanwhile: packed {:.2} M jobs, padded {:.2} M jobs",
        median_mad(packed_jobs).0 / 1e6,
        median_mad(spaced_jobs).0 / 1e6
    );
    assert!(
        spaced_median < packed_median,
        "padded state is not faster than packed"
    );
}
