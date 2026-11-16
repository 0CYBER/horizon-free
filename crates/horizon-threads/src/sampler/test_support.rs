//! Linux sampling-test calibration, independent of rings, unwinding and aggregation.
//! The sampler itself is Linux-only. Dependent test crates opt in with the
//! `sampling-test-support` feature and gate their sampling tests on Linux.

use std::sync::OnceLock;

/// Effective CPU timer deliveries per second when requesting 1 kHz on this kernel.
/// A dedicated unregistered thread counts raw signals using the sampler's existing
/// once-installed handler. It never replaces handlers owned by concurrent tests.
pub fn cpu_hz() -> f64 {
    static HZ: OnceLock<f64> = OnceLock::new();
    *HZ.get_or_init(|| {
        std::thread::spawn(super::os::measure_cpu_hz)
            .join()
            .expect("CPU timer calibration worker panicked")
            .expect("CPU timer calibration failed")
    })
}

/// CPU workload duration targeting the number of timer deliveries that `nominal_ms`
/// would produce at 1 kHz. Saturation fixtures keep their fixed ring sizes and drop
/// bounds, but run enough CPU work to exercise the same number of signal deliveries.
pub fn cpu_ms_for_samples(nominal_ms: u64) -> u64 {
    (nominal_ms as f64 * 1000.0 / cpu_hz()).ceil() as u64
}

/// Scale a CPU sample-count bound specified for a requested 1 kHz timer.
/// Ceil and a nonzero minimum prevent calibration from erasing small bounds.
/// Wall-clock sample bounds and behavioral proportions must not use this scale.
pub fn cpu_count(nominal_at_1000_hz: u64) -> u64 {
    let scaled = (nominal_at_1000_hz as f64 * cpu_hz() / 1000.0).ceil() as u64;
    if nominal_at_1000_hz == 0 {
        0
    } else {
        scaled.max(1)
    }
}

/// The share of wall time, in thousandths, that this thread ran while `spin` burned `cpu_ms` of
/// CPU. On a loaded runner a spinning thread is preempted, and its wall samples then rightly say
/// waiting; tests judge the running label against this share.
pub fn spin_share_permille(cpu_ms: u64, spin: fn(u64)) -> u64 {
    let (wall, cpu) = (std::time::Instant::now(), super::os::thread_cpu_ns());
    spin(cpu_ms);
    let ran = super::os::thread_cpu_ns() - cpu;
    (ran * 1000 / (wall.elapsed().as_nanos() as u64).max(1)).min(1000)
}

/// Runs a live sampling measurement up to five times and passes on the first run whose checks all
/// hold, panicking with every run's failure otherwise. A thread's CPU timer fires on the
/// scheduler's tick while it runs and its wall timer while the runner gets to deliver it, so on a
/// loaded runner one run can see far fewer samples, or skewed shares, than its work is worth. A
/// sampler fault fails the same check in every run and still fails the test.
pub fn first_passing_run(mut run: impl FnMut() -> Result<(), String>) {
    let mut failures = Vec::new();
    for _ in 0..5 {
        match run() {
            Ok(()) => return,
            Err(failure) => failures.push(failure),
        }
    }
    panic!("no run of five passed:\n{}", failures.join("\n"));
}

/// One check of a [`first_passing_run`] measurement: `Err(why())` when `ok` is false.
pub fn ensure(ok: bool, why: impl FnOnce() -> String) -> Result<(), String> {
    if ok { Ok(()) } else { Err(why()) }
}
