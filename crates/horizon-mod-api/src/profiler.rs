//! Host-only Level 1 timings. Totals are inclusive: nested calls also contribute to their parent.
//! No arguments or game data are retained. Start/stop must run outside mod calls; stop drains
//! in-flight guards. Repeated start is a no-op; repeated stop preserves the last report.
#[path = "profiler/firefox.rs"]
mod firefox;
use crate::content::crash::ModCall;
use std::cell::{Cell, UnsafeCell};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering::SeqCst};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const THREADS: usize = 64;
const CALLS: usize = 4096;
// This cannot index any real record, even after capacity exhaustion.
const OVERFLOW: usize = usize::MAX;
pub(crate) static ENABLED: AtomicBool = AtomicBool::new(false);
static EPOCH: AtomicUsize = AtomicUsize::new(0);
// Unlike the admission epoch, this remains valid while stop drains existing guards.
static POOL_EPOCH: AtomicUsize = AtomicUsize::new(0);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static POOL: AtomicPtr<Pool> = AtomicPtr::new(std::ptr::null_mut());
static CONTROL: Mutex<()> = Mutex::new(());
thread_local! { static SLOT: Cell<(usize, usize)> = const { Cell::new((0, 0)) }; }

/// One call, shared by native and wasm timing producers. Names are resolved at load time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub mod_id: &'static str,
    pub hook: &'static str,
    pub part: Option<&'static str>,
    pub nanoseconds: u64,
}
/// Inclusive nanoseconds; bucket 0 contains 0 and 1, bucket n contains [2^n, 2^(n+1)).
#[derive(Debug)]
pub(crate) struct Aggregate {
    pub count: u64,
    pub total: u128,
    pub min: u64,
    pub max: u64,
    pub histogram: [u64; 64],
}
impl Default for Aggregate {
    fn default() -> Self {
        Self {
            count: 0,
            total: 0,
            min: 0,
            max: 0,
            histogram: [0; 64],
        }
    }
}
type Key = (&'static str, &'static str, Option<&'static str>);
#[derive(Default, Debug)]
pub(crate) struct Report {
    pub calls: BTreeMap<Key, Aggregate>,
    /// Capacity exhaustion is explicit, never silently presented as a complete profile.
    pub dropped: usize,
}
impl Report {
    fn completed(&mut self, stored: StoredRecord) {
        if stored.completed {
            self.record(stored.record);
        } else {
            self.dropped += 1;
        }
    }
    fn record(&mut self, record: Record) {
        let a = self
            .calls
            .entry((record.mod_id, record.hook, record.part))
            .or_default();
        a.min = if a.count == 0 {
            record.nanoseconds
        } else {
            a.min.min(record.nanoseconds)
        };
        a.count += 1;
        a.total += u128::from(record.nanoseconds);
        a.max = a.max.max(record.nanoseconds);
        a.histogram[(63 - record.nanoseconds.max(1).leading_zeros()) as usize] += 1;
    }
    /// Mods sorted by total time, then hooks/parts by total within each mod.
    pub fn summary(&self) -> String {
        use std::fmt::Write;
        let mut mods = BTreeMap::<_, u128>::new();
        for ((id, _, _), a) in &self.calls {
            *mods.entry(*id).or_default() += a.total;
        }
        let mut mods: Vec<_> = mods.into_iter().collect();
        mods.sort_by_key(|(id, total)| (std::cmp::Reverse(*total), *id));
        let mut text = format!(
            "Inclusive native/wasm call timings (ns); dropped: {}\n",
            self.dropped
        );
        for (id, total) in mods {
            writeln!(text, "{id}: {total}").unwrap();
            let mut calls: Vec<_> = self
                .calls
                .iter()
                .filter(|((m, _, _), _)| *m == id)
                .collect();
            calls.sort_by_key(|(key, a)| (std::cmp::Reverse(a.total), **key));
            for ((_, hook, part), a) in calls {
                writeln!(
                    text,
                    "  {hook} [{}]: count={} total={} min={} max={}",
                    part.unwrap_or("-"),
                    a.count,
                    a.total,
                    a.min,
                    a.max
                )
                .unwrap();
            }
        }
        text
    }
}
#[derive(Clone, Copy)]
struct StoredRecord {
    record: Record,
    completed: bool,
}
struct Buffer {
    next: AtomicUsize,
    records: Vec<UnsafeCell<Option<StoredRecord>>>,
}
// Each index is reserved once with fetch_add. Only its reserving guard writes it; stop
// reads only after all guards have left. Buffers are never reused within an epoch.
#[allow(
    unsafe_code,
    reason = "Thread-owned buffers are read only after profiling drains."
)]
unsafe impl Sync for Buffer {}
struct Pool {
    next_thread: AtomicUsize,
    buffers: Vec<Buffer>,
    dropped: AtomicUsize,
    clock: fn() -> u64,
}
fn clock() -> u64 {
    #[cfg(test)]
    tests::CLOCK_READS.fetch_add(1, SeqCst);
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN
        .get_or_init(Instant::now)
        .elapsed()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64
}
pub fn is_enabled() -> bool {
    ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}
/// Allocate all thread buffers before enabling. Returns false if already running.
pub fn start() -> bool {
    // Initialize the monotonic origin on the control path, never in the first call.
    clock();
    start_with_clock(clock)
}
fn start_with_clock(clock: fn() -> u64) -> bool {
    let _control = CONTROL.lock().unwrap_or_else(|e| e.into_inner());
    if is_enabled() {
        return false;
    }
    let pool = Box::new(Pool {
        next_thread: AtomicUsize::new(0),
        dropped: AtomicUsize::new(0),
        clock,
        buffers: (0..THREADS)
            .map(|_| Buffer {
                next: AtomicUsize::new(0),
                records: (0..CALLS).map(|_| UnsafeCell::new(None)).collect(),
            })
            .collect(),
    });
    POOL.store(Box::into_raw(pool), SeqCst);
    POOL_EPOCH.store(EPOCH.fetch_add(1, SeqCst) + 1, SeqCst);
    ENABLED.store(true, SeqCst);
    true
}
static REPORT: Mutex<Option<Report>> = Mutex::new(None);
/// Stop outside any profiled guard, retaining the report privately for the command.
pub fn stop() {
    let mut report = REPORT.lock().unwrap_or_else(|e| e.into_inner());
    if is_enabled() {
        *report = Some(stop_report());
    }
}
/// The last stopped session's text report, or an empty string before the first stop.
pub fn summary() -> String {
    REPORT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map_or_else(String::new, Report::summary)
}
/// Export the last stopped session as raw Gecko/Firefox profile JSON, format version 37.
/// Output is limited to 8 MiB; missing reports or oversized output return an error,
/// never an empty or truncated file. Counters remain exact decimal strings. Markers
/// are aggregate snapshots, not sampled execution or an invocation timeline.
pub fn firefox_profile() -> Result<Vec<u8>, String> {
    let report = REPORT.lock().unwrap_or_else(|e| e.into_inner());
    firefox::encode(
        report
            .as_ref()
            .ok_or("No stopped mod profiler session is available")?,
    )
}
/// Disable, drain active calls, and merge every thread. Call outside any profiled guard.
pub(crate) fn stop_report() -> Report {
    let _control = CONTROL.lock().unwrap_or_else(|e| e.into_inner());
    if !ENABLED.swap(false, SeqCst) {
        return Report::default();
    }
    EPOCH.fetch_add(1, SeqCst);
    while ACTIVE.load(SeqCst) != 0 {
        std::thread::yield_now();
    }
    // No validated entrant can access the old pool after disable/epoch change and drain.
    #[allow(
        unsafe_code,
        reason = "The disabled and drained pool has one owning allocation."
    )]
    let pool = unsafe { Box::from_raw(POOL.swap(std::ptr::null_mut(), SeqCst)) };
    let mut report = Report {
        dropped: pool.dropped.load(SeqCst),
        ..Report::default()
    };
    for buffer in &pool.buffers {
        for record in buffer
            .records
            .iter()
            .take(buffer.next.load(SeqCst).min(CALLS))
        {
            // All writers have finished, including guards unwound by panics.
            #[allow(
                unsafe_code,
                reason = "Profiling has drained all writers before reading records."
            )]
            if let Some(record) = unsafe { *record.get() } {
                report.completed(record);
            }
        }
    }
    report
}
pub(crate) struct Guard {
    timing: Option<mod_vocab::dispatch_scope::Timing>,
}
impl Guard {
    pub(crate) fn into_timing(mut self) -> mod_vocab::dispatch_scope::Timing {
        self.timing
            .take()
            .expect("live profiler guard owns its timing")
    }
}
pub(crate) fn enter(call: ModCall) -> Option<Guard> {
    if !is_enabled() {
        return None;
    }
    let epoch = EPOCH.load(SeqCst);
    #[cfg(test)]
    tests::PAUSE.with(|p| {
        if let Some(pause) = p.take() {
            pause();
        }
    });
    ACTIVE.fetch_add(1, SeqCst);
    if !ENABLED.load(SeqCst) || EPOCH.load(SeqCst) != epoch {
        ACTIVE.fetch_sub(1, SeqCst);
        return None;
    }
    // ACTIVE pins this pool until the returned guard drops.
    #[allow(
        unsafe_code,
        reason = "ACTIVE pins the pool until the guard is dropped."
    )]
    let pool = unsafe { &*POOL.load(SeqCst) };
    let slot = SLOT.with(|s| {
        let (old_epoch, old_slot) = s.get();
        if old_epoch == epoch {
            old_slot
        } else {
            let slot = pool.next_thread.fetch_add(1, SeqCst);
            s.set((epoch, slot));
            slot
        }
    });
    // Each entry reserves one unique record before user code runs. The clock
    // field temporarily holds the start timestamp, and leave completes it.
    // Exhaustion is explicit: the sentinel still owns one ACTIVE reference.
    let record = match pool.buffers.get(slot) {
        Some(buffer) => {
            let record = buffer.next.fetch_add(1, SeqCst);
            if record < CALLS {
                #[allow(
                    unsafe_code,
                    reason = "Entry uniquely owns this reserved record; stop waits for ACTIVE."
                )]
                unsafe {
                    *buffer.records[record].get() = Some(StoredRecord {
                        record: Record {
                            mod_id: call.id,
                            hook: call.hook,
                            part: call.part,
                            nanoseconds: (pool.clock)(),
                        },
                        completed: false,
                    });
                }
                record
            } else {
                pool.dropped.fetch_add(1, SeqCst);
                OVERFLOW
            }
        }
        None => {
            pool.dropped.fetch_add(1, SeqCst);
            OVERFLOW
        }
    };
    Some(Guard {
        timing: Some(mod_vocab::dispatch_scope::Timing {
            generation: epoch,
            slot,
            record,
        }),
    })
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(timing) = self.timing.take() {
            leave(timing);
        }
    }
}
pub(crate) fn leave(timing: mod_vocab::dispatch_scope::Timing) {
    // The factory capability ensures only an authentic entry reaches this private
    // callback. Its ACTIVE reference pins the pool epoch until completion.
    if POOL_EPOCH.load(SeqCst) != timing.generation {
        return;
    }
    if timing.record == OVERFLOW {
        ACTIVE.fetch_sub(1, SeqCst);
        return;
    }
    if timing.slot >= THREADS || timing.record >= CALLS {
        return;
    }
    let pointer = POOL.load(SeqCst);
    if pointer.is_null() {
        return;
    }
    #[allow(
        unsafe_code,
        reason = "The authentic timing token owns an ACTIVE reference to this epoch."
    )]
    let pool = unsafe { &*pointer };
    if let Some(record) = pool
        .buffers
        .get(timing.slot)
        .and_then(|buffer| buffer.records.get(timing.record))
    {
        #[allow(
            unsafe_code,
            reason = "Only this token writes its reserved record; stop waits for ACTIVE."
        )]
        if let Some(record) = unsafe { &mut *record.get() } {
            record.record.nanoseconds = (pool.clock)().saturating_sub(record.record.nanoseconds);
            record.completed = true;
        }
    }
    ACTIVE.fetch_sub(1, SeqCst);
}

#[cfg(test)]
mod tests {
    use super::stop_report as stop;
    use super::*;
    use crate::content::crash::enter_native_mod;
    static SERIAL: Mutex<()> = Mutex::new(());
    type Pause = Box<dyn FnOnce()>;
    thread_local! {
        static TIME: Cell<u64> = const { Cell::new(0) };
        pub(super) static PAUSE: std::cell::RefCell<Option<Pause>> = const { std::cell::RefCell::new(None) };
    }
    pub(super) static CLOCK_READS: AtomicUsize = AtomicUsize::new(0);
    fn tick() -> u64 {
        TIME.with(|t| {
            let now = t.get();
            t.set(now + 1);
            now
        })
    }
    fn call(id: &'static str, hook: &'static str, part: Option<&'static str>) -> ModCall {
        ModCall { id, hook, part }
    }
    #[test]
    fn incomplete_records_are_skipped_and_counted() {
        let mut report = Report::default();
        report.completed(StoredRecord {
            record: Record {
                mod_id: "unfinished",
                hook: "reserved",
                part: None,
                nanoseconds: u64::MAX,
            },
            completed: false,
        });
        assert!(report.calls.is_empty());
        assert_eq!(report.dropped, 1);
    }

    #[test]
    #[ignore = "daily"]
    fn overflow_tokens_do_not_alias_records_or_lose_crash_owners() {
        if !isolated("overflow_tokens_do_not_alias_records_or_lose_crash_owners") {
            return;
        }
        let _serial = SERIAL.lock().unwrap();
        stop();
        TIME.with(|time| time.set(0));
        assert!(start_with_clock(tick));
        let mut guards: Vec<_> = (0..CALLS)
            .map(|_| enter_native_mod(call("nested", "frame", None)))
            .collect();
        let overflowing = enter_native_mod(call("overflow", "frame", None));
        assert_eq!(
            crate::content::crash::current_native_mod(),
            Some("overflow")
        );
        drop(overflowing);
        assert_eq!(crate::content::crash::current_native_mod(), Some("nested"));
        while let Some(guard) = guards.pop() {
            drop(guard);
        }
        assert_eq!(crate::content::crash::current_native_mod(), None);
        assert_eq!(ACTIVE.load(SeqCst), 0);
        let report = stop_report();
        assert_eq!(report.dropped, 1);
        let nested = &report.calls[&("nested", "frame", None)];
        assert_eq!(nested.count, CALLS as u64);
        // The deterministic clock yields odd inclusive durations 1 .. 8191.
        // A sentinel aliasing any real record breaks these bounds.
        assert_eq!(nested.min, 1);
        assert_eq!(nested.max, (2 * CALLS - 1) as u64);
        assert!(!report.calls.contains_key(&("overflow", "frame", None)));
    }

    #[test]
    #[ignore = "daily"]
    fn profiler_guards_and_threads() {
        if !isolated("profiler_guards_and_threads") {
            return;
        }
        let _serial = SERIAL.lock().unwrap();
        stop();
        *REPORT.lock().unwrap() = None;
        assert!(firefox_profile().unwrap_err().contains("No stopped"));
        TIME.with(|t| t.set(0));
        CLOCK_READS.store(0, SeqCst);
        drop(enter_native_mod(call("off", "off", None)));
        assert_eq!(TIME.with(Cell::get), 0);
        assert_eq!(CLOCK_READS.load(SeqCst), 0);
        assert!(start_with_clock(tick));
        assert!(!start());
        let outer = enter_native_mod(call("a", "outer", None));
        drop(enter_native_mod(call("b", "inner", Some("Part"))));
        drop(outer);
        let panic = std::panic::catch_unwind(|| {
            let _guard = enter_native_mod(call("c", "panic", None));
            panic!("mod panic");
        });
        assert!(panic.is_err());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let guard = enter_native_mod(call("d", "thread", None));
                    barrier.wait();
                    drop(guard);
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let report = stop();
        // Unrelated crate tests may enter other native hooks while this session runs.
        for (key, count, total) in [
            (("a", "outer", None), 1, 3),
            (("b", "inner", Some("Part")), 1, 1),
            (("c", "panic", None), 1, 1),
            (("d", "thread", None), 2, 2),
        ] {
            let a = &report.calls[&key];
            assert_eq!((a.count, a.total), (count, total));
        }
        assert_eq!(TIME.with(Cell::get), 6);
        assert_eq!(report.dropped, 0);
        assert!(stop().calls.is_empty());
        assert!(start_with_clock(tick));
        drop(enter_native_mod(call("fresh", "epoch", None)));
        super::stop();
        assert!(super::summary().contains("fresh:"));
        let profile = firefox_profile().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&profile).unwrap();
        assert!(
            json["threads"][0]["markers"]["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row[5]["mod"] == "fresh")
        );
        super::stop();
        assert_eq!(firefox_profile().unwrap(), profile);
        assert!(super::summary().contains("fresh:"));
    }
    #[test]
    #[ignore = "daily"]
    fn profiler_drains_in_flight_and_reports_overflow() {
        if !isolated("profiler_drains_in_flight_and_reports_overflow") {
            return;
        }
        let _serial = SERIAL.lock().unwrap();
        assert!(start_with_clock(tick));
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, exit) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _guard = enter_native_mod(call("drain", "waiting", None));
            entered.send(()).unwrap();
            exit.recv().unwrap();
        });
        ready.recv().unwrap();
        let stopper = std::thread::spawn(stop);
        while is_enabled() {
            std::thread::yield_now();
        }
        assert!(!stopper.is_finished());
        release.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(
            stopper.join().unwrap().calls[&("drain", "waiting", None)].count,
            1
        );
        assert!(start_with_clock(tick));
        for _ in 0..=CALLS {
            drop(enter_native_mod(call("capacity", "bounded", None)));
        }
        let report = stop();
        assert_eq!(
            report.calls[&("capacity", "bounded", None)].count,
            CALLS as u64
        );
        assert!(report.dropped >= 1);
    }
    #[test]
    #[ignore = "daily"]
    fn profiler_stale_epoch_is_rejected() {
        if !isolated("profiler_stale_epoch_is_rejected") {
            return;
        }
        let _serial = SERIAL.lock().unwrap();
        assert!(start_with_clock(tick));
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, exit) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            drop(enter(call("epoch", "warm", None)));
            PAUSE.with(|p| {
                *p.borrow_mut() = Some(Box::new(move || {
                    entered.send(()).unwrap();
                    exit.recv().unwrap();
                }))
            });
            let stale = enter(call("epoch", "stale", None));
            let rejected = stale.is_none();
            drop(stale);
            drop(enter(call("epoch", "fresh", None)));
            rejected
        });
        ready.recv().unwrap();
        stop();
        assert!(start_with_clock(tick));
        release.send(()).unwrap();
        let rejected = worker.join().unwrap();
        let report = stop();
        assert!(rejected, "stale snapshot attached to the restarted session");
        assert!(!report.calls.contains_key(&("epoch", "stale", None)));
        assert_eq!(report.calls[&("epoch", "fresh", None)].count, 1);
    }
    // Isolate process-wide service tests from unrelated crate tests entering native guards.
    fn isolated(name: &str) -> bool {
        if std::env::var_os("HORIZON_PROFILER_TEST_CHILD").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--include-ignored",
                    "--exact",
                    &format!("__private::profiler::tests::{name}"),
                ])
                .env("HORIZON_PROFILER_TEST_CHILD", "1")
                .output()
                .unwrap();
            // The child must run this one test: a success that ran nothing checks nothing.
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("test result: ok. 1 passed;"),
                "the isolated rerun of {name} did not pass exactly one test:\n{stdout}{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return false;
        }
        true
    }
    #[test]
    #[ignore = "daily"]
    fn profiler_empty_session_replaces_previous_report() {
        if !isolated("profiler_empty_session_replaces_previous_report") {
            return;
        }
        let _serial = SERIAL.lock().unwrap();
        *REPORT.lock().unwrap() = None;
        assert!(firefox_profile().is_err());
        assert!(super::start());
        super::stop();
        let empty = firefox_profile();
        assert!(
            empty.is_ok(),
            "a completed zero-call session must be exportable"
        );
        let marker_count = || {
            let json: serde_json::Value =
                serde_json::from_slice(&firefox_profile().unwrap()).unwrap();
            json["threads"][0]["markers"]["data"]
                .as_array()
                .unwrap()
                .len()
        };
        assert_eq!(marker_count(), 0);
        assert!(super::start());
        drop(enter_native_mod(call("replace", "old", None)));
        super::stop();
        assert_eq!(marker_count(), 1);
        assert!(super::start());
        super::stop();
        assert_eq!(
            marker_count(),
            0,
            "old rows must not survive an empty session"
        );
        let empty = firefox_profile().unwrap();
        super::stop();
        assert_eq!(firefox_profile().unwrap(), empty);
    }
    fn capacity(threads: usize, calls: usize, name: &str) {
        if !isolated(name) {
            return;
        }
        assert!(start_with_clock(tick));
        for _ in 0..threads {
            std::thread::spawn(move || {
                for _ in 0..calls {
                    drop(enter(call("capacity", "thread", None)));
                }
            })
            .join()
            .unwrap();
        }
        let report = stop();
        assert_eq!(
            report.calls[&("capacity", "thread", None)].count,
            (threads.min(THREADS) * calls) as u64
        );
        assert_eq!(report.dropped, threads.saturating_sub(THREADS) * calls);
    }
    #[test]
    #[ignore = "daily"]
    fn profiler_two_full_threads() {
        capacity(2, CALLS, "profiler_two_full_threads");
    }
    #[test]
    #[ignore = "daily"]
    fn profiler_thread_exhaustion() {
        capacity(THREADS + 1, 1, "profiler_thread_exhaustion");
    }
    #[test]
    fn profiler_histogram_and_summary() {
        let mut report = Report::default();
        for nanoseconds in [0, 1, 2, 3, 4, 7, 8, u64::MAX] {
            report.record(Record {
                mod_id: "m",
                hook: "h",
                part: None,
                nanoseconds,
            });
        }
        let a = &report.calls[&("m", "h", None)];
        assert_eq!((a.count, a.min, a.max), (8, 0, u64::MAX));
        assert_eq!(a.total, u128::from(u64::MAX) + 25);
        let mut expected = [0; 64];
        expected[0] = 2;
        expected[1] = 2;
        expected[2] = 2;
        expected[3] = 1;
        expected[63] = 1;
        assert_eq!(a.histogram, expected);
        report.record(Record {
            mod_id: "z",
            hook: "first",
            part: Some("P"),
            nanoseconds: u64::MAX,
        });
        report.record(Record {
            mod_id: "z",
            hook: "second",
            part: None,
            nanoseconds: 100,
        });
        let text = report.summary();
        assert!(text.find("z:").unwrap() < text.find("m:").unwrap());
        assert!(text.find("first [P]").unwrap() < text.find("second [-]").unwrap());
    }
}
