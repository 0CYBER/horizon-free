//! What each kind of mob costs the server tick, for `horizon-report` (dedicated/report.rs): while
//! a report records, one tick in [`SAMPLE_EVERY`] times every mob's own tick and adds it up by
//! level and mob type.
//! Off, a mob costs one thread-local read; sampled, two clock reads and a map update.
//! A mob's riders tick inside its own tick, so they count for it. The server tick's
//! spans stay enabled while the report or slow-tick ring needs them ([`want_spans`]).
use horizon_client::{fast_hash::FxHashMap, mobs::MobType};
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Instant,
};

/// One server tick in this many is sampled while a report records.
pub(super) const SAMPLE_EVERY: usize = 4;

/// The report sampling bit.
const REPORT: u8 = 1;

/// The time and the ticks of one mob type in one level.
#[derive(Default)]
struct Cost {
    nanos: u64,
    mob_ticks: u64,
}

/// Sums by level, then by the type's registry id: one entry per type, found without allocating.
type Totals = FxHashMap<&'static str, FxHashMap<String, Cost>>;

thread_local! {
    /// The report bit, set while it samples.
    static SAMPLING: Cell<u8> = const { Cell::new(0) };
    /// The report's sums.
    static COSTS: RefCell<[Totals; 1]> = RefCell::default();
}

/// Who wants the tick's spans recorded besides nobody: `HORIZON_SPIKES` (the slow ring's steps),
/// a report's recording.
pub(super) const SPANS_SPIKES: u8 = 1;
pub(super) const SPANS_REPORT: u8 = 2;
#[cfg(test)]
const SPANS_MOD: u8 = 4;
static SPAN_USERS: AtomicU8 = AtomicU8::new(0);

/// `user` wants the spans recorded, or no longer does: they stay on while anyone does.
pub(super) fn want_spans(user: u8, on: bool) {
    let users = if on {
        SPAN_USERS.fetch_or(user, Ordering::Relaxed) | user
    } else {
        SPAN_USERS.fetch_and(!user, Ordering::Relaxed) & !user
    };
    horizon_client::spans::enable(users != 0);
}

#[cfg(test)]
pub(super) fn wanted(user: u8) -> bool {
    SPAN_USERS.load(Ordering::Relaxed) & user != 0
}

/// Sample (or stop sampling) the mob ticks this thread runs next, for the report.
pub(super) fn set_sampling(on: bool) {
    set(REPORT, on);
}

fn set(who: u8, on: bool) {
    SAMPLING.with(|s| s.set(if on { s.get() | who } else { s.get() & !who }));
}

/// Who samples on this thread: what a helper thread running part of its mob loop samples for.
#[cfg(any(test, feature = "upstream-parallel-mobs"))]
pub(super) fn sampling() -> u8 {
    SAMPLING.with(Cell::get)
}

/// A helper thread's sums, for the thread whose mob loop it ran part of ([`merge`]).
#[cfg(any(test, feature = "upstream-parallel-mobs"))]
pub(super) struct Sums([Totals; 1]);

/// This helper thread samples for `who` ([`sampling`] of the thread it helps), from empty sums.
#[cfg(any(test, feature = "upstream-parallel-mobs"))]
pub(super) fn begin_helper(who: u8) {
    SAMPLING.with(|s| s.set(who));
    COSTS.with(|c| *c.borrow_mut() = Default::default());
}

/// What this helper thread sampled since [`begin_helper`]; it samples nothing more.
#[cfg(any(test, feature = "upstream-parallel-mobs"))]
pub(super) fn end_helper() -> Sums {
    SAMPLING.with(|s| s.set(0));
    Sums(COSTS.with(|c| std::mem::take(&mut *c.borrow_mut())))
}

/// Add a helper thread's sums to this thread's.
#[cfg(any(test, feature = "upstream-parallel-mobs"))]
pub(super) fn merge(sums: Sums) {
    COSTS.with(|c| {
        let mut costs = c.borrow_mut();
        for (into, from) in costs.iter_mut().zip(sums.0) {
            for (dim, kinds) in from {
                let into = into.entry(dim).or_default();
                for (kind, cost) in kinds {
                    let sum = into.entry(kind).or_default();
                    sum.nanos += cost.nanos;
                    sum.mob_ticks += cost.mob_ticks;
                }
            }
        }
    });
}

/// When this tick is sampled, the start of a mob's tick.
#[inline]
pub(super) fn start() -> Option<Instant> {
    (SAMPLING.with(Cell::get) != 0).then(Instant::now)
}

/// A sampled mob of type `kind` in level `dim` ticked since `started`.
#[inline]
pub(super) fn add(dim: &'static str, kind: &MobType, started: Instant) {
    add_kind(dim, &kind.id, started);
}

/// A mob of type `kind` starts its tick: the CPU sampler's work, and `horizon-report`'s cost by mob type when sampled.
pub(super) fn start_mob(kind: &Arc<MobType>) -> Option<(Instant, Arc<MobType>)> {
    horizon_threads::work::set_entity(horizon_threads::work::entity_id(&kind.id));
    start().map(|t| (t, Arc::clone(kind)))
}

/// The mob [`start_mob`] started ended its tick in level `dim`.
pub(super) fn end_mob(dim: &'static str, sampled: Option<(Instant, Arc<MobType>)>) {
    horizon_threads::work::clear_work();
    if let Some((started, kind)) = sampled {
        add(dim, &kind, started);
    }
}

/// [`add`] for the type with registry id `id`.
fn add_kind(dim: &'static str, id: &str, started: Instant) {
    let nanos = started.elapsed().as_nanos() as u64;
    let who = SAMPLING.with(Cell::get);
    COSTS.with(|c| {
        let mut costs = c.borrow_mut();
        for (sums, bit) in costs.iter_mut().zip([REPORT]) {
            if who & bit == 0 {
                continue;
            }
            let kinds = sums.entry(dim).or_default();
            // The id is copied once, for a type's first sample in a level.
            if !kinds.contains_key(id) {
                kinds.insert(id.to_owned(), Cost::default());
            }
            let cost = kinds.get_mut(id).expect("just inserted");
            cost.nanos += nanos;
            cost.mob_ticks += 1;
        }
    });
}

/// One level's mob type: its sampled time and mob ticks.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct MobCost {
    pub dimension: &'static str,
    pub kind: String,
    pub nanos: u64,
    pub mob_ticks: u64,
}

/// The report's costs sampled since the last call, costliest first; clears them.
pub(super) fn take() -> Vec<MobCost> {
    take_of(0)
}

fn take_of(sums: usize) -> Vec<MobCost> {
    let costs = COSTS.with(|c| std::mem::take(&mut c.borrow_mut()[sums]));
    let mut out: Vec<MobCost> = costs
        .into_iter()
        .flat_map(|(dimension, kinds)| {
            kinds.into_iter().map(move |(kind, c)| MobCost {
                dimension,
                kind,
                nanos: c.nanos,
                mob_ticks: c.mob_ticks,
            })
        })
        .collect();
    out.sort_by(|a, b| b.nanos.cmp(&a.nanos).then_with(|| a.kind.cmp(&b.kind)));
    out
}

/// What sampling one mob costs on this machine, in nanoseconds: the report's own overhead.
pub(super) fn sample_cost_nanos() -> f64 {
    const N: u32 = 20_000;
    let was = SAMPLING.with(Cell::get);
    let kept = COSTS.with(|c| std::mem::take(&mut c.borrow_mut()[0]));
    SAMPLING.with(|s| s.set(REPORT));
    let timer = Instant::now();
    for _ in 0..N {
        if let Some(started) = start() {
            add_kind("calibration", "calibration", started);
        }
    }
    let per = timer.elapsed().as_nanos() as f64 / f64::from(N);
    SAMPLING.with(|s| s.set(was));
    COSTS.with(|c| c.borrow_mut()[0] = kept);
    per
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(dim: &'static str, id: &str, started: Instant) {
        add_kind(dim, id, started);
    }

    /// Mobs add up by level and type, only on sampled ticks; taking them clears them.
    #[test]
    fn costs_add_up_by_level_and_type_while_sampling() {
        let (zombie, cow) = ("minecraft:zombie", "minecraft:cow");
        set_sampling(false);
        assert!(start().is_none(), "nothing is timed while off");
        set_sampling(true);
        for _ in 0..3 {
            let t = start().expect("sampled");
            std::thread::sleep(std::time::Duration::from_millis(1));
            add("minecraft:overworld", zombie, t);
        }
        add("minecraft:overworld", cow, Instant::now());
        add("minecraft:the_nether", zombie, Instant::now());
        set_sampling(false);
        let costs = take();
        assert_eq!(costs.len(), 3, "{costs:?}");
        assert_eq!(costs[0].kind, "minecraft:zombie");
        assert_eq!(costs[0].dimension, "minecraft:overworld");
        assert_eq!(costs[0].mob_ticks, 3);
        assert!(costs[0].nanos >= 3_000_000, "{}", costs[0].nanos);
        let nether = costs
            .iter()
            .find(|c| c.dimension == "minecraft:the_nether")
            .unwrap();
        assert_eq!(
            (nether.kind.as_str(), nether.mob_ticks),
            ("minecraft:zombie", 1)
        );
        assert!(take().is_empty());
    }

    /// A helper thread running part of the mob loop samples for whoever samples on the thread
    /// it helps, and its sums join that thread's.
    #[test]
    fn a_helper_threads_samples_join_the_server_threads() {
        let zombie = "minecraft:zombie";
        set_sampling(true);
        add("minecraft:overworld", zombie, Instant::now());
        let who = sampling();
        let helped = std::thread::spawn(move || {
            begin_helper(who);
            for _ in 0..2 {
                add("minecraft:overworld", zombie, start().expect("sampled"));
            }
            let sums = end_helper();
            assert!(start().is_none(), "the helper stops sampling");
            sums
        })
        .join()
        .unwrap();
        merge(helped);
        set_sampling(false);
        let costs = take();
        assert_eq!(costs.len(), 1, "{costs:?}");
        assert_eq!(costs[0].mob_ticks, 3);
    }

    /// Many mobs of one type over several ticks, some ticked on helper threads and merged back,
    /// make one row for the type in each level, with every tick counted.
    #[test]
    fn one_row_per_type_however_many_mobs_and_threads() {
        let who = {
            set_sampling(true);
            sampling()
        };
        let chicken = "minecraft:chicken";
        let mut helpers = Vec::new();
        for _ in 0..4 {
            // A sampled server tick: mobs on this thread, mobs on two helpers.
            for _ in 0..5 {
                add("minecraft:overworld", chicken, start().unwrap());
            }
            add("minecraft:the_nether", chicken, Instant::now());
            add("minecraft:overworld", "minecraft:bee", Instant::now());
            for _ in 0..2 {
                helpers.push(
                    std::thread::spawn(move || {
                        begin_helper(who);
                        for _ in 0..3 {
                            add("minecraft:overworld", chicken, start().unwrap());
                        }
                        end_helper()
                    })
                    .join()
                    .unwrap(),
                );
            }
            for sums in helpers.drain(..) {
                merge(sums);
            }
        }
        set_sampling(false);
        let costs = take();
        let ticks = |dim: &str, kind: &str| {
            let rows: Vec<_> = costs
                .iter()
                .filter(|c| c.dimension == dim && c.kind == kind)
                .collect();
            assert_eq!(rows.len(), 1, "{costs:?}");
            rows[0].mob_ticks
        };
        assert_eq!(ticks("minecraft:overworld", chicken), 4 * (5 + 2 * 3));
        assert_eq!(ticks("minecraft:the_nether", chicken), 4);
        assert_eq!(ticks("minecraft:overworld", "minecraft:bee"), 4);
        assert_eq!(costs.len(), 3, "{costs:?}");
    }

    /// Spans stay on while anyone wants them.
    #[test]
    fn spans_stay_on_while_anyone_wants_them() {
        // Only the mod's bit: other tests may want them as the report meanwhile.
        want_spans(SPANS_MOD, true);
        assert!(horizon_client::spans::enabled());
        assert_ne!(SPAN_USERS.load(Ordering::Relaxed) & SPANS_MOD, 0);
        want_spans(SPANS_MOD, false);
        assert_eq!(SPAN_USERS.load(Ordering::Relaxed) & SPANS_MOD, 0);
    }

    /// Calibrating leaves what was sampled, and the switch, as they were.
    #[test]
    fn calibration_keeps_the_samples() {
        set_sampling(true);
        add("minecraft:overworld", "minecraft:pig", Instant::now());
        let per = sample_cost_nanos();
        assert!(per > 0.0 && per < 100_000.0, "{per}");
        assert!(start().is_some(), "still sampling");
        set_sampling(false);
        let costs = take();
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].kind, "minecraft:pig");
    }
}
