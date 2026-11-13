//! The helper threads of the server tick's parallel phases (`HORIZON_PARALLEL_TICK`): one pool, made
//! once, instead of threads started for every phase of every tick. A phase hands it work that gives
//! the same result in whatever order the threads take it, and each part's results stay in order.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

/// `HORIZON_PARALLEL_TICK`: off when unset (0). `1` turns the server tick's parallel phases on (1),
/// and `verify` does too (2), also checking the phases that can be checked against the serial code.
/// Each phase gives the serial result either way.
pub fn parallel_tick() -> u8 {
    static ON: OnceLock<u8> = OnceLock::new();
    *ON.get_or_init(|| match std::env::var("HORIZON_PARALLEL_TICK").as_deref() {
        Ok("1" | "on" | "true") => 1,
        Ok("verify") => 2,
        _ => 0,
    })
}

/// Whether a villager's brain draws from the villager's own random instead of the level's.
///
/// The level random is one stream, so a draw has to wait for every mob before it (`OrderedDraws`),
/// and a villager's brain draws on nearly every turn (behaviour durations, one-in-N rolls).
/// With the parallel tick on, those draws come from a stream seeded from the villager's own, so
/// no villager waits on another mob and the lanes run apart. That is the one place the parallel
/// tick departs from vanilla's draw order: the villagers' rolls have the same odds but not the
/// same values a vanilla level would give them. The seed is derived from the mob's id and the game
/// time, so the villager's entity random keeps vanilla's sequence. The serial loop does the same while the parallel
/// tick is on, so which path a tick takes never changes the result.
pub fn villagers_own_random() -> bool {
    OWN_RANDOM_TEST.load(Ordering::Relaxed) || parallel_tick() > 0
}

/// A test's switch for [`villagers_own_random`], process-wide because the lanes are threads: any
/// gameplay test running at the same time in the same binary sees villagers on their own random, so
/// the tests that set it run alone (`--test-threads=1`).
#[doc(hidden)]
pub static OWN_RANDOM_TEST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// How many threads a phase may share its work among: `horizon-threads`' count (at most 8) while
/// the parallel tick is on, else 1.
pub fn lanes() -> usize {
    if parallel_tick() > 0 {
        horizon_threads::worker_count().clamp(1, 8)
    } else {
        1
    }
}

/// The pool: one thread fewer than a phase's lanes (the server thread takes parts too).
fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(
                horizon_threads::worker_count()
                    .clamp(1, 8)
                    .saturating_sub(1)
                    .max(1),
            )
            .thread_name(|i| format!("tick-helper-{i}"))
            .build()
            .expect("the tick's helper threads start")
    })
}

/// `f` over `items` in parts on the pool, or all on this thread while the parallel tick is off or
/// for fewer than `min` items: every part's results, in the order of `items`, as one thread would
/// give them.
pub fn par_map<T: Sync, R: Send>(
    items: &[T],
    min: usize,
    f: impl Fn(&[T], &mut Vec<R>) + Sync,
) -> Vec<R> {
    par_map_on(items, min, lanes(), f)
}

/// `f` over `items` in chunks of `chunk`, in place, on the pool: the same sharing out as
/// `horizon_threads::par_chunks_mut`, without starting threads for every call. A chunk runs on
/// whatever thread takes it, so `f` must not depend on the order. Unlike [`par_map`] it does not
/// wait for the parallel tick's switch (the phases that use it ran shared out before the pool
/// existed, #8158): it shares out unless `HORIZON_SERIAL_TICK=all` or one thread is all there is.
/// The thread-locals the tick relies on do not follow a helper.
pub fn par_chunks_mut<T: Send>(items: &mut [T], chunk: usize, f: impl Fn(&mut [T]) + Send + Sync) {
    let lanes = if horizon_threads::serial_tick_all() {
        1
    } else {
        horizon_threads::worker_count().clamp(1, 8)
    };
    par_chunks_mut_on(items, chunk, lanes, f);
}

/// [`par_chunks_mut`] on `lanes` threads.
fn par_chunks_mut_on<T: Send>(
    items: &mut [T],
    chunk: usize,
    lanes: usize,
    f: impl Fn(&mut [T]) + Send + Sync,
) {
    let chunk = chunk.max(1);
    let count = items.len().div_ceil(chunk);
    if lanes <= 1 || count < 2 {
        for slice in items.chunks_mut(chunk) {
            f(slice);
        }
        return;
    }
    // A thread takes a chunk whole; the lock is only held while a chunk is taken, never while
    // it runs: the chunk is popped in a statement of its own, so the guard is gone before `f`
    // (a guard in a `while let` scrutinee would live through the loop body).
    let pending = Mutex::new(items.chunks_mut(chunk).collect::<Vec<_>>());
    let work = || {
        loop {
            let next = pending.lock().unwrap_or_else(PoisonError::into_inner).pop();
            let Some(slice) = next else { break };
            f(slice);
        }
    };
    pool().in_place_scope(|s| {
        for _ in 1..lanes.min(count) {
            s.spawn(|_| work());
        }
        work();
    });
}

/// [`par_map`] on `lanes` threads.
///
/// The parts are small (four a thread) and go to whichever thread is free, this one too: a helper
/// the machine is slow to run (the chunk workers compete for the same cores) holds the phase up by
/// one part at most instead of an eighth of the work, and the server thread works instead of only
/// waiting.
fn par_map_on<T: Sync, R: Send>(
    items: &[T],
    min: usize,
    lanes: usize,
    f: impl Fn(&[T], &mut Vec<R>) + Sync,
) -> Vec<R> {
    if lanes <= 1 || items.len() < min.max(2) {
        let mut out = Vec::with_capacity(items.len());
        f(items, &mut out);
        return out;
    }
    let size = items.len().div_ceil(lanes * 4);
    let count = items.len().div_ceil(size);
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Vec<R>>> = (0..count).map(|_| Mutex::new(Vec::new())).collect();
    let work = || {
        loop {
            let k = next.fetch_add(1, Ordering::Relaxed);
            if k >= count {
                break;
            }
            let part = &items[k * size..((k + 1) * size).min(items.len())];
            let mut done = Vec::with_capacity(part.len());
            f(part, &mut done);
            *slots[k].lock().unwrap_or_else(PoisonError::into_inner) = done;
        }
    };
    pool().in_place_scope(|s| {
        for _ in 1..lanes {
            s.spawn(|_| work());
        }
        work();
    });
    let mut out = Vec::with_capacity(items.len());
    for slot in slots {
        out.extend(slot.into_inner().unwrap_or_else(PoisonError::into_inner));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On several threads, every item's result comes back once, in the items' order, for item
    /// counts that don't divide into the parts evenly; a part may give several results or none.
    #[test]
    fn parts_come_back_in_order() {
        for n in [0usize, 1, 7, 33, 1000, 4097] {
            let items: Vec<u64> = (0..n as u64).collect();
            let f = |part: &[u64], out: &mut Vec<u64>| {
                for &x in part {
                    // Spin a little so the threads overlap.
                    let mut h = x;
                    for _ in 0..200 {
                        h = h.wrapping_mul(6364136223846793005).wrapping_add(1);
                    }
                    if x % 3 != 0 {
                        out.push(x * 10);
                    }
                    if x % 5 == 0 {
                        out.push(h);
                    }
                }
            };
            let mut serial = Vec::new();
            f(&items, &mut serial);
            assert_eq!(par_map_on(&items, 1, 4, f), serial, "{n} items");
        }
    }

    /// Every chunk is handled once, whole, and its items changed in place, for item counts that
    /// don't divide into chunks evenly.
    #[test]
    fn chunks_are_each_handled_once() {
        for n in [0usize, 1, 15, 16, 17, 79, 80, 1000] {
            let mut items: Vec<u64> = (0..n as u64).collect();
            par_chunks_mut_on(&mut items, 16, 4, |chunk| {
                assert!(chunk.len() <= 16);
                for x in chunk {
                    *x = *x * 3 + 1;
                }
            });
            assert_eq!(items, (0..n as u64).map(|x| x * 3 + 1).collect::<Vec<_>>());
        }
    }

    /// Chunks really run at the same time: one waits (bounded, so a serial run fails instead of
    /// hanging) until another has started, and each item is still handled exactly once.
    #[test]
    fn chunks_overlap_and_run_once() {
        use std::sync::atomic::AtomicBool;
        use std::time::{Duration, Instant};
        let active = AtomicUsize::new(0);
        let overlapped = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(10);
        let runs: Vec<AtomicUsize> = (0..64).map(|_| AtomicUsize::new(0)).collect();
        let mut items: Vec<usize> = (0..64).collect();
        par_chunks_mut_on(&mut items, 4, 4, |chunk| {
            for &i in chunk.iter() {
                runs[i].fetch_add(1, Ordering::SeqCst);
            }
            if active.fetch_add(1, Ordering::SeqCst) >= 1 {
                overlapped.store(true, Ordering::SeqCst);
            }
            while !overlapped.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            active.fetch_sub(1, Ordering::SeqCst);
        });
        assert!(
            overlapped.load(Ordering::SeqCst),
            "no two chunks ran at once"
        );
        assert!(runs.iter().all(|r| r.load(Ordering::SeqCst) == 1));
    }

    /// A panic in a part reaches the caller instead of being lost on a helper.
    #[test]
    fn a_panic_reaches_the_caller() {
        let items: Vec<u32> = (0..256).collect();
        let caught = std::panic::catch_unwind(|| {
            par_map_on(&items, 1, 4, |part, out: &mut Vec<u32>| {
                assert!(!part.contains(&200), "item 200");
                out.extend(part);
            })
        });
        assert!(caught.is_err());
    }
}
