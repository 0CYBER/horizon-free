//! Reserve an SMT core for the server and move cooperating workers off it.
//! Other threads and processes may still use that core. Reservations are planned
//! anew for each world. The game's own pool threads move themselves; on Linux a keeper
//! moves the threads that never chose their CPUs: the ones the server thread starts, which
//! inherit its mask, and the ones free to run anywhere (see `os::Keeper`). Every thread
//! moved is written down with the mask it had, and the release gives back exactly those,
//! to exactly that mask; a thread that changed its mask meanwhile keeps its own.
//! A thread pinned through [`pin_current`] is never touched. How the keeper tells an
//! inherited mask from a chosen one, and what it can't know about threads that other
//! code pins with its own calls: `os::Keeper`.
//! `HORIZON_SERVER_CORE=0` disables this; dedicated servers require `=1`.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Fewer cores with SMT than this, and no core is reserved: on a 4-core/8-thread laptop the
/// workers would lose a quarter of their CPUs. Counting only cores with SMT also keeps a hybrid
/// CPU with two performance cores (Intel's E-cores have no sibling) from giving one of its two
/// fast cores to the server and leaving the render thread the other.
pub const MIN_SMT_CORES: usize = 6;

/// One physical core: its logical CPUs and how fast it is (higher is faster: the maximum
/// frequency on Linux, the efficiency class on Windows; equal where the system doesn't say).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Core {
    pub cpus: Vec<usize>,
    pub rank: u64,
}

/// Where the threads go: the server thread on `reserved` (one core's CPUs, the server free to use
/// either sibling so another program on one doesn't stall it), cooperating workers on `others`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub reserved: Vec<usize>,
    pub others: Vec<usize>,
}

/// A CPU list as the kernel writes it (`0,16`, `0-1`, `0-3,8-11`); what doesn't parse is skipped.
pub fn parse_cpu_list(text: &str) -> Vec<usize> {
    let mut cpus = BTreeSet::new();
    for part in text
        .trim()
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let range = match part.split_once('-') {
            Some((a, b)) => a.trim().parse().ok().zip(b.trim().parse().ok()),
            None => part.parse::<usize>().ok().map(|n| (n, n)),
        };
        if let Some((a, b)) = range {
            cpus.extend(a..=b.min(a.saturating_add(4096)));
        }
    }
    cpus.into_iter().collect()
}

/// Which core the server thread takes on a machine of `cores` when the game may use `allowed`:
/// the fastest core with SMT whose CPUs are all allowed (the lowest-numbered of equals), `None`
/// where reserving doesn't pay (see [`MIN_SMT_CORES`]) or nothing would be left for the others.
pub fn plan(cores: &[Core], allowed: &[usize]) -> Option<Plan> {
    let allowed: BTreeSet<usize> = allowed.iter().copied().collect();
    let candidates: Vec<&Core> = cores
        .iter()
        .filter(|c| c.cpus.len() >= 2 && c.cpus.iter().all(|cpu| allowed.contains(cpu)))
        .collect();
    if candidates.len() < MIN_SMT_CORES {
        return None;
    }
    let core = candidates
        .iter()
        .max_by_key(|c| (c.rank, std::cmp::Reverse(c.cpus.iter().min().copied())))?;
    let reserved: Vec<usize> = core.cpus.clone();
    let others: Vec<usize> = allowed
        .iter()
        .copied()
        .filter(|cpu| !reserved.contains(cpu))
        .collect();
    (!others.is_empty()).then_some(Plan { reserved, others })
}

/// `HORIZON_SERVER_CORE=0` (or `off`, `false`) turns the reservation off.
fn switched_off(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some("0" | "off" | "false"))
}

/// Hosted servers opt in explicitly because multiple processes can choose the same core.
pub fn dedicated_enabled() -> bool {
    dedicated_enabled_value(std::env::var("HORIZON_SERVER_CORE").ok().as_deref())
}

fn dedicated_enabled_value(value: Option<&str>) -> bool {
    value == Some("1")
}

// The lock serializes publication with self-applied changes. The generation makes the
// common worker path one relaxed load; workers reconcile when they next run a job.
static ACTIVE: Mutex<Option<Plan>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);
// How many CPUs the active plan was taken from (0: none).
static WHOLE: AtomicUsize = AtomicUsize::new(0);

/// Serializes the tests that pin threads or start a keeper (which moves every thread of the
/// test process that never chose its CPUs) with the tests that read a thread's CPUs.
#[cfg(test)]
pub(crate) static RESERVATION_TEST: Mutex<()> = Mutex::new(());

/// The CPUs the process may use while a reservation narrows its threads' own masks: the
/// server thread's is its core alone and the threads moved off it have the rest, so
/// `std::thread::available_parallelism` asked on any of them undercounts. `None` without one.
pub(crate) fn whole_set() -> Option<NonZeroUsize> {
    NonZeroUsize::new(WHOLE.load(Ordering::Relaxed))
}

fn whole_count(plan: &Plan) -> usize {
    plan.reserved
        .iter()
        .chain(&plan.others)
        .collect::<BTreeSet<_>>()
        .len()
}

struct WorkerState {
    generation: u64,
    previous: Option<os::State>,
}

impl Drop for WorkerState {
    fn drop(&mut self) {
        if let Some(state) = self.previous.take() {
            os::restore(&state);
        }
    }
}

thread_local! {
    static WORKER: std::cell::RefCell<WorkerState> = const { std::cell::RefCell::new(WorkerState { generation: u64::MAX, previous: None }) };
    static SERVER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A thread-bound reservation, released by the server thread itself.
pub struct ServerCore {
    plan: Plan,
    state: os::State,
    keeper: os::Keeper,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl ServerCore {
    /// The CPUs the server thread runs on.
    pub fn cpus(&self) -> &[usize] {
        &self.plan.reserved
    }
}

impl Drop for ServerCore {
    fn drop(&mut self) {
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        self.keeper.stop();
        os::restore(&self.state);
        SERVER.with(|server| server.set(false));
        WHOLE.store(0, Ordering::Relaxed);
        *active = None;
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Recompute the plan for each world, then pin only this thread.
pub fn take() -> Option<ServerCore> {
    if switched_off(std::env::var("HORIZON_SERVER_CORE").ok().as_deref()) {
        return None;
    }
    let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    if active.is_some() {
        return None;
    }
    let (cores, allowed) = os::topology()?;
    let plan = plan(&cores, &allowed)?;
    // A lazily initialized pool must not size itself from the server's temporary pin.
    let _ = crate::worker_count();
    let core = begin(plan)?;
    *active = Some(core.plan.clone());
    GENERATION.fetch_add(1, Ordering::Relaxed);
    Some(core)
}

/// What `available_parallelism()` answers while `plan` holds: what std says before the pin, a
/// CPU quota (cgroup `cpu.max`) included, so the sizes asked meanwhile stay what they were.
fn whole(plan: &Plan) -> usize {
    std::thread::available_parallelism()
        .map_or(whole_count(plan), |n| n.get().min(whole_count(plan)))
}

/// Pin this thread to the plan's core, then publish the count of [`whole`], then start the
/// keeper: a pin that fails publishes nothing, and no thread is moved (each move takes the
/// keeper's lock, which `Keeper::start` takes first) before the count is there to ask.
fn begin(plan: Plan) -> Option<ServerCore> {
    let whole = whole(&plan);
    let state = os::reserve(&plan)?;
    WHOLE.store(whole, Ordering::Relaxed);
    SERVER.with(|server| server.set(true));
    let keeper = os::Keeper::start(&plan, &state);
    Some(ServerCore {
        plan,
        state,
        keeper,
        _thread: std::marker::PhantomData,
    })
}

/// Pin the calling thread to `cpus` as its own choice: no reservation moves it or gives it
/// anything back, and [`leave_reserved`] leaves it alone. The way for Horizon's code to pin a
/// thread, so the keeper knows the mask was chosen (Linux; elsewhere nothing, `false`).
/// Whether the system took it.
///
/// The threads it starts inherit the mask and keep it, through the release too, unless the
/// mask is exactly the whole set the plan was made from or one a reservation hands out (the
/// server's core, the other CPUs): a thread born with those bits may as well have inherited
/// them from the server thread or a thread the keeper moved, Linux keeps no record of which
/// thread started which, and the keeper takes them for the reservation's (moved, then given
/// what that mask came from). Children that must keep such a mask pin it themselves.
pub fn pin_current(cpus: &[usize]) -> bool {
    os::pin(cpus)
}

/// Reconcile this owned helper's selection with the current reservation: off the server's
/// core while one is held, the CPUs it had given back once it isn't. A mask inherited from
/// the pinned server thread is widened to what that thread had before; any other mask is only
/// narrowed, and one inside the reserved core alone is kept. On Linux the reservation gives the
/// CPUs back as it ends (its ledger, `os::Keeper`); on Windows the thread does at its next call.
pub fn leave_reserved() {
    let generation = GENERATION.load(Ordering::Relaxed);
    if SERVER.with(|server| server.get()) {
        return;
    }
    WORKER.with(|worker| {
        let mut worker = worker.borrow_mut();
        if worker.generation == generation {
            return;
        }
        let active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = worker.previous.take() {
            os::restore(&previous);
        }
        worker.previous = active.as_ref().and_then(os::leave);
        worker.generation = GENERATION.load(Ordering::Relaxed);
    });
}

/// Remove reserved CPUs only when at least one previously allowed CPU remains.
#[cfg(any(target_os = "linux", windows, test))]
fn narrowed(previous: &[usize], reserved: &[usize]) -> Option<Vec<usize>> {
    let remaining: Vec<_> = previous
        .iter()
        .copied()
        .filter(|cpu| !reserved.contains(cpu))
        .collect();
    (!remaining.is_empty() && remaining.len() != previous.len()).then_some(remaining)
}

#[cfg(target_os = "linux")]
mod os {
    use super::{Core, Plan};
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Mutex;
    use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
    use std::thread::JoinHandle;
    use std::time::Duration;

    pub struct State {
        pub(super) original: Vec<usize>,
    }

    fn read(path: String) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    pub fn affinity(tid: i32) -> Option<Vec<usize>> {
        // SAFETY: a zeroed cpu_set_t is the empty set; the size passed is its own.
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            if libc::sched_getaffinity(tid, std::mem::size_of::<libc::cpu_set_t>(), &mut set) != 0 {
                return None;
            }
            Some(
                (0..libc::CPU_SETSIZE as usize)
                    .filter(|&cpu| libc::CPU_ISSET(cpu, &set))
                    .collect(),
            )
        }
    }

    pub fn set_affinity(tid: i32, cpus: &[usize]) -> bool {
        // SAFETY: as in `affinity`; CPU_SET ignores nothing in range, and every id is checked.
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            for &cpu in cpus.iter().filter(|&&c| c < libc::CPU_SETSIZE as usize) {
                libc::CPU_SET(cpu, &mut set);
            }
            libc::sched_setaffinity(tid, std::mem::size_of::<libc::cpu_set_t>(), &set) == 0
        }
    }

    /// The cores of the CPUs this thread may use, from sysfs, and those CPUs.
    pub fn topology() -> Option<(Vec<Core>, Vec<usize>)> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let allowed = super::parse_cpu_list(
            status
                .lines()
                .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))?,
        );
        let mut cores: Vec<Core> = Vec::new();
        for &cpu in &allowed {
            let base = format!("/sys/devices/system/cpu/cpu{cpu}");
            let cpus = read(format!("{base}/topology/thread_siblings_list"))
                .map(|s| super::parse_cpu_list(&s))
                .filter(|cpus| cpus.contains(&cpu))
                .unwrap_or_else(|| vec![cpu]);
            if cores.iter().any(|c| c.cpus == cpus) {
                continue;
            }
            // AMD's preferred cores and Intel's Turbo Boost Max cores report a higher maximum.
            let rank = read(format!("{base}/cpufreq/cpuinfo_max_freq"))
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            cores.push(Core { cpus, rank });
        }
        Some((cores, allowed))
    }

    /// A thread: its id and when it started (`starttime` in `/proc`), so that an id the kernel
    /// hands out again is another thread.
    type Id = (i32, u64);

    fn tid() -> i32 {
        // SAFETY: gettid takes nothing and can't fail.
        unsafe { libc::gettid() }
    }

    fn id(tid: i32) -> Option<Id> {
        let stat = read(format!("/proc/self/task/{tid}/stat"))?;
        // The name before them is in parentheses and may hold anything: count from the last ')'.
        let start = stat.rsplit_once(')')?.1.split_whitespace().nth(19)?;
        Some((tid, start.parse().ok()?))
    }

    fn threads() -> Vec<i32> {
        std::fs::read_dir("/proc/self/task")
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
            .collect()
    }

    /// A thread Horizon moved off the reserved core: the mask it had, and the one it was given.
    struct Moved {
        id: Id,
        before: Vec<usize>,
        given: Vec<usize>,
    }

    /// What one reservation changed, so that its release gives back exactly that.
    struct Ledger {
        reserved: Vec<usize>,
        /// The CPUs the plan was made from: a thread with this mask never chose its own.
        whole: Vec<usize>,
        server: i32,
        keeper: Option<i32>,
        /// The threads alive when the reservation began, which inherited nothing from it.
        born_before: BTreeSet<Id>,
        /// Every mask the reservation gave a thread (the server's core included), with the mask
        /// that thread had before: what a thread born with one of them would have had.
        handed_out: Vec<(Vec<usize>, Vec<usize>)>,
        moved: BTreeMap<i32, Moved>,
        /// Threads whose mask the reservation can't account for: chosen by them (or their
        /// creator), never touched.
        theirs: BTreeSet<Id>,
    }

    impl Ledger {
        /// What a thread the reservation hasn't moved had without it, when its mask says it
        /// never chose one: the whole set (never narrowed), or a mask the reservation handed
        /// out, which a thread born since inherited from the thread it was given to. A thread
        /// born to a `pin` on one of these masks reads the same (its bits don't say who started
        /// it): taking it for the reservation's keeps the server's own helpers off its core.
        fn before(&self, id: Id, mask: &[usize]) -> Option<Vec<usize>> {
            if mask == self.whole {
                return Some(mask.to_vec());
            }
            if self.born_before.contains(&id) {
                return None;
            }
            self.handed_out
                .iter()
                .find(|(given, _)| given == mask)
                .map(|(_, before)| before.clone())
        }

        fn record(&mut self, id: Id, before: Vec<usize>, given: Vec<usize>) {
            if !self.handed_out.iter().any(|(g, _)| *g == given) {
                self.handed_out.push((given.clone(), before.clone()));
            }
            self.moved.insert(id.0, Moved { id, before, given });
        }

        /// Move `id` (now on `mask`) off the reserved core if the reservation may, and write
        /// it down; otherwise it is the thread's own.
        fn take_over(&mut self, id: Id, mask: Vec<usize>) {
            let target = self
                .before(id, &mask)
                .and_then(|before| Some((super::narrowed(&before, &self.reserved)?, before)));
            match target {
                Some((given, before)) if given == mask || set_affinity(id.0, &given) => {
                    self.record(id, before, given)
                }
                Some(_) => {}
                None => {
                    self.theirs.insert(id);
                }
            }
        }
    }

    struct Book {
        /// Threads pinned through `pin`: no reservation moves them or gives them anything.
        pinned: BTreeSet<Id>,
        /// The reservation held, if any.
        ledger: Option<Ledger>,
    }

    /// Every change of mask Horizon makes happens under this lock, so none of its own pins can
    /// land between the keeper reading a thread's mask and setting it.
    static BOOK: Mutex<Book> = Mutex::new(Book {
        pinned: BTreeSet::new(),
        ledger: None,
    });

    fn book() -> std::sync::MutexGuard<'static, Book> {
        BOOK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Holds the lock every move takes, so no thread is moved until it is dropped.
    #[cfg(test)]
    pub(super) fn hold_moves() -> impl Sized {
        book()
    }

    pub fn pin(cpus: &[usize]) -> bool {
        let Some(id) = id(tid()) else {
            return false;
        };
        let mut book = book();
        if !set_affinity(0, cpus) {
            return false;
        }
        book.pinned.insert(id);
        if let Some(ledger) = &mut book.ledger {
            ledger.moved.remove(&id.0);
        }
        true
    }

    pub fn reserve(plan: &Plan) -> Option<State> {
        let original = affinity(0)?;
        if !plan.reserved.iter().all(|cpu| original.contains(cpu))
            || !set_affinity(0, &plan.reserved)
        {
            return None;
        }
        Some(State { original })
    }

    /// The calling thread asks to leave the reserved core: its mask narrowed, or, when it was
    /// born with a mask the reservation handed out (the server's core, for a helper the server
    /// thread started), the mask that came from narrowed. The ledger gives it back, so there is
    /// no state for the thread to keep.
    pub fn leave(_plan: &Plan) -> Option<State> {
        let id = id(tid())?;
        let mut book = book();
        let Book { pinned, ledger } = &mut *book;
        let ledger = ledger.as_mut()?;
        if pinned.contains(&id) || ledger.moved.get(&id.0).is_some_and(|m| m.id == id) {
            return None;
        }
        let mask = affinity(0)?;
        let before = ledger.before(id, &mask).unwrap_or_else(|| mask.clone());
        let given = super::narrowed(&before, &ledger.reserved)?;
        if given == mask || set_affinity(0, &given) {
            ledger.theirs.remove(&id);
            ledger.record(id, before, given);
        }
        None
    }

    pub fn restore(state: &State) {
        set_affinity(0, &state.original);
    }

    /// Look at every thread once: move the ones that never chose their CPUs, and note the ones
    /// that chose since they were moved.
    pub(super) fn scan() {
        let mut book = book();
        let Book { pinned, ledger } = &mut *book;
        let Some(ledger) = ledger else {
            return;
        };
        let alive: BTreeSet<i32> = threads().into_iter().collect();
        ledger.moved.retain(|tid, _| alive.contains(tid));
        pinned.retain(|(tid, _)| alive.contains(tid));
        for &tid in &alive {
            if tid == ledger.server || Some(tid) == ledger.keeper {
                continue;
            }
            let (Some(id), Some(mask)) = (id(tid), affinity(tid)) else {
                continue;
            };
            if pinned.contains(&id) || ledger.theirs.contains(&id) {
                continue;
            }
            match ledger.moved.get(&tid) {
                Some(moved) if moved.id == id => {
                    // It set a mask of its own since: from now on that is its choice.
                    if mask != moved.given {
                        ledger.moved.remove(&tid);
                        ledger.theirs.insert(id);
                    }
                    continue;
                }
                Some(_) => {
                    ledger.moved.remove(&tid);
                }
                None => {}
            }
            ledger.take_over(id, mask);
        }
    }

    /// Two kinds of thread would undo the reservation, measured on a 16-core/32-thread laptop:
    /// - threads the pinned server thread starts (chunk encoders, save compressors, a network
    ///   thread...) begin on its CPUs only, as Linux threads inherit their creator's mask: with
    ///   31 encoders and 16 compressors left there, ticks took three times as long;
    /// - threads free to run anywhere (the render, main and network threads) drift onto the
    ///   reserved core once the workers have left it, as it is then the machine's idlest:
    ///   one run in three took three times as long, the other two gained a third of the win.
    ///
    /// Once a second the keeper looks at every thread. It moves to the other CPUs only the ones
    /// whose mask shows they never chose one: the whole allowed set, or, for a thread born after
    /// the reservation began, a mask the reservation itself handed out (the server's core, the
    /// other CPUs), which such a thread can only have inherited from the thread it was given to.
    /// Each thread it moves goes in the ledger with the mask it had: for an inherited mask,
    /// what the thread it came from had. Any other mask is the thread's own (or its creator's
    /// own, inherited), and so is a mask a moved thread sets later: never touched again. When
    /// the world closes the keeper looks once more, then gives each thread in the ledger that
    /// still has the mask it was given back the one it had; a thread that changed it keeps its.
    ///
    /// Horizon's own threads choose a mask only through this module (the server thread's
    /// `reserve`, `leave`, `pin`), which writes each choice down under the keeper's lock, so for
    /// every thread Horizon pins the reading is exact, and for the threads those start, but one
    /// born to a pin on exactly the whole set or a mask the reservation handed out, which reads
    /// as the reservation's (see `pin_current`). What it can't know about a thread other
    /// code pins with its own `sched_setaffinity` (a driver's, a library's, `taskset -p`): one
    /// that picks, before the keeper first sees it, exactly the whole set or a mask the
    /// reservation handed out looks as if it never chose, is moved and given that mask back at
    /// the end; and a pin that lands between the keeper reading a mask and setting it is lost
    /// until the world closes, as the kernel has no compare-and-set for a mask. Any other pin,
    /// at any time, is kept.
    pub struct Keeper {
        stop: Option<Sender<()>>,
        thread: Option<JoinHandle<()>>,
    }

    /// How often the keeper looks.
    const EVERY: Duration = Duration::from_secs(1);

    impl Keeper {
        /// Opens the reservation's ledger (on the server thread, pinned by `server`) and starts
        /// the keeper.
        pub fn start(plan: &Plan, server: &State) -> Keeper {
            let whole: Vec<usize> = plan
                .others
                .iter()
                .chain(&plan.reserved)
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            book().ledger = Some(Ledger {
                reserved: plan.reserved.clone(),
                whole,
                server: tid(),
                keeper: None,
                born_before: threads().into_iter().filter_map(id).collect(),
                handed_out: vec![(plan.reserved.clone(), server.original.clone())],
                moved: BTreeMap::new(),
                theirs: BTreeSet::new(),
            });
            let (stop, stopped) = channel::<()>();
            let others = plan.others.clone();
            let thread = std::thread::Builder::new()
                .name("Server core keeper".into())
                .spawn(move || {
                    if let Some(ledger) = &mut book().ledger {
                        ledger.keeper = Some(tid());
                    }
                    set_affinity(0, &others);
                    loop {
                        scan();
                        if !matches!(stopped.recv_timeout(EVERY), Err(RecvTimeoutError::Timeout)) {
                            return;
                        }
                    }
                })
                .ok();
            Keeper {
                stop: Some(stop),
                thread,
            }
        }

        /// Stops the keeper and gives back what the reservation moved.
        pub fn stop(&mut self) {
            self.stop.take();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            // The threads born since the last look (the server's newest helpers) are moved
            // here only to be given back their mask with the rest.
            scan();
            let mut book = book();
            let Some(ledger) = book.ledger.take() else {
                return;
            };
            for moved in ledger.moved.values() {
                let tid = moved.id.0;
                if id(tid) == Some(moved.id) && affinity(tid).as_ref() == Some(&moved.given) {
                    set_affinity(tid, &moved.before);
                }
            }
        }
    }
}

#[cfg(windows)]
mod os {
    use super::{Core, Plan};
    use std::sync::Mutex;
    use windows_sys::Win32::System::SystemInformation::{
        CpuSetInformation, GetSystemCpuSetInformation, SYSTEM_CPU_SET_INFORMATION,
    };
    use windows_sys::Win32::System::Threading::{
        GetActiveProcessorGroupCount, GetCurrentProcess, GetCurrentThread, GetProcessAffinityMask,
        GetProcessDefaultCpuSets, GetThreadGroupAffinity, GetThreadSelectedCpuSets,
        SetThreadSelectedCpuSets,
    };

    /// Logical processor index → CPU set id, for the machine's one processor group.
    static IDS: Mutex<Vec<(usize, u32)>> = Mutex::new(Vec::new());
    /// The CPU set ids a reservation selected for a thread, sorted. The selection it replaced
    /// was empty: a thread with a selection of its own is never touched.
    pub struct State {
        given: Vec<u32>,
    }

    fn ids(cpus: &[usize]) -> Vec<u32> {
        let ids = IDS.lock().unwrap_or_else(|e| e.into_inner());
        cpus.iter()
            .filter_map(|cpu| ids.iter().find(|(c, _)| c == cpu).map(|&(_, id)| id))
            .collect()
    }

    /// The cores from the system's CPU sets (siblings share a core index) and the logical
    /// processors the process's affinity mask allows. `None` past 64 CPUs (more than one
    /// processor group), which this keeps out of.
    pub fn topology() -> Option<(Vec<Core>, Vec<usize>)> {
        // SAFETY: plain calls; the out pointers are live locals, the buffer is as long as said,
        // and each record is read within the length the system returned, by its own size.
        unsafe {
            if GetActiveProcessorGroupCount() != 1 {
                return None;
            }
            let mut count = 0;
            if GetProcessDefaultCpuSets(GetCurrentProcess(), std::ptr::null_mut(), 0, &mut count)
                == 0
                || count != 0
            {
                return None;
            }
            let (mut process_mask, mut system_mask) = (0usize, 0usize);
            if GetProcessAffinityMask(GetCurrentProcess(), &mut process_mask, &mut system_mask) == 0
            {
                return None;
            }
            let mut length = 0u32;
            GetSystemCpuSetInformation(
                std::ptr::null_mut(),
                0,
                &mut length,
                GetCurrentProcess(),
                0,
            );
            if length == 0 {
                return None;
            }
            let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
            if GetSystemCpuSetInformation(
                buffer.as_mut_ptr().cast(),
                length,
                &mut length,
                GetCurrentProcess(),
                0,
            ) == 0
            {
                return None;
            }
            if length as usize > buffer.len() * std::mem::size_of::<u64>() {
                return None;
            }
            let bytes = buffer.as_ptr().cast::<u8>();
            let (mut offset, mut cores, mut allowed) = (0usize, Vec::<(u8, Core)>::new(), vec![]);
            let mut ids = Vec::new();
            while offset + std::mem::size_of::<u32>() * 2 <= length as usize {
                let size = bytes.add(offset).cast::<u32>().read_unaligned() as usize;
                if size < std::mem::size_of::<u32>() * 2 || size > length as usize - offset {
                    break;
                }
                let kind = bytes.add(offset + 4).cast::<i32>().read_unaligned();
                if kind == CpuSetInformation
                    && size >= std::mem::size_of::<SYSTEM_CPU_SET_INFORMATION>()
                {
                    let info = bytes
                        .add(offset)
                        .cast::<SYSTEM_CPU_SET_INFORMATION>()
                        .read_unaligned();
                    let set = info.Anonymous.CpuSet;
                    let cpu = set.LogicalProcessorIndex as usize;
                    if set.Group != 0 || cpu >= usize::BITS as usize {
                        return None;
                    }
                    ids.push((cpu, set.Id));
                    if process_mask & (1usize << cpu) != 0 {
                        allowed.push(cpu);
                    }
                    match cores.iter_mut().find(|(index, _)| *index == set.CoreIndex) {
                        Some((_, core)) => core.cpus.push(cpu),
                        None => cores.push((
                            set.CoreIndex,
                            Core {
                                cpus: vec![cpu],
                                rank: set.EfficiencyClass as u64,
                            },
                        )),
                    }
                }
                offset += size;
            }
            *IDS.lock().unwrap_or_else(|e| e.into_inner()) = ids;
            allowed.sort_unstable();
            Some((cores.into_iter().map(|(_, c)| c).collect(), allowed))
        }
    }

    /// The CPU set ids this thread has selected, sorted (empty: none); `None` if the system
    /// doesn't say.
    fn selection() -> Option<Vec<u32>> {
        // SAFETY: this thread's own handle; the buffer holds as many ids as the count passed.
        unsafe {
            let mut count = 0;
            if GetThreadSelectedCpuSets(GetCurrentThread(), std::ptr::null_mut(), 0, &mut count)
                != 0
            {
                return (count == 0).then(Vec::new);
            }
            if count == 0 {
                return None;
            }
            let mut selected = vec![0u32; count as usize];
            if GetThreadSelectedCpuSets(
                GetCurrentThread(),
                selected.as_mut_ptr(),
                count,
                &mut count,
            ) == 0
            {
                return None;
            }
            selected.truncate(count as usize);
            selected.sort_unstable();
            Some(selected)
        }
    }

    fn select(cpus: &[usize]) -> Option<State> {
        // A non-empty selection is foreign: leave it untouched.
        if !selection()?.is_empty() {
            return None;
        }
        let mut given = ids(cpus);
        if given.len() != cpus.len() || given.is_empty() {
            return None;
        }
        given.sort_unstable();
        // SAFETY: this thread's own handle; the ids are live for the call.
        unsafe {
            (SetThreadSelectedCpuSets(GetCurrentThread(), given.as_ptr(), given.len() as u32) != 0)
                .then_some(State { given })
        }
    }
    fn thread_mask() -> Option<Vec<usize>> {
        // SAFETY: the output is initialized and belongs to this thread.
        unsafe {
            let mut group = std::mem::zeroed();
            if GetThreadGroupAffinity(GetCurrentThread(), &mut group) == 0 || group.Group != 0 {
                return None;
            }
            Some(
                (0..usize::BITS as usize)
                    .filter(|cpu| group.Mask & (1usize << cpu) != 0)
                    .collect(),
            )
        }
    }
    pub fn reserve(plan: &Plan) -> Option<State> {
        let mask = thread_mask()?;
        if !plan.reserved.iter().all(|cpu| mask.contains(cpu)) {
            return None;
        }
        select(&plan.reserved)
    }
    pub fn leave(plan: &Plan) -> Option<State> {
        let mask = thread_mask()?;
        select(&super::narrowed(&mask, &plan.reserved)?)
    }
    /// Not on Windows yet.
    pub fn pin(_cpus: &[usize]) -> bool {
        false
    }

    /// A Windows thread doesn't inherit its creator's CPU set, so nothing needs moving back.
    pub struct Keeper;
    impl Keeper {
        pub fn start(_plan: &Plan, _server: &State) -> Keeper {
            Keeper
        }
        pub fn stop(&mut self) {}
    }

    /// Clears the selection `state` made, if the thread still has it: one it selected since
    /// is its own and stays, as a Linux thread keeps a mask it set after it was moved.
    pub fn restore(state: &State) {
        if selection().as_ref() != Some(&state.given) {
            return;
        }
        // SAFETY: this is the calling thread; no ids, which clears its selection.
        unsafe {
            SetThreadSelectedCpuSets(GetCurrentThread(), std::ptr::null(), 0);
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod os {
    use super::{Core, Plan};

    pub struct State;

    /// No hard affinity here (macOS's affinity tags are hints the scheduler may ignore).
    pub fn topology() -> Option<(Vec<Core>, Vec<usize>)> {
        None
    }
    pub fn reserve(_plan: &Plan) -> Option<State> {
        None
    }
    pub fn leave(_plan: &Plan) -> Option<State> {
        None
    }
    pub fn restore(_state: &State) {}
    pub fn pin(_cpus: &[usize]) -> bool {
        false
    }

    pub struct Keeper;
    impl Keeper {
        pub fn start(_plan: &Plan, _server: &State) -> Keeper {
            Keeper
        }
        pub fn stop(&mut self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `physical` cores of `smt` CPUs each, numbered as Linux does (core n's CPUs n, n +
    /// physical...), all equally fast.
    fn machine(physical: usize, smt: usize) -> Vec<Core> {
        (0..physical)
            .map(|n| Core {
                cpus: (0..smt).map(|t| n + t * physical).collect(),
                rank: 0,
            })
            .collect()
    }

    fn all(cores: &[Core]) -> Vec<usize> {
        let mut cpus: Vec<usize> = cores.iter().flat_map(|c| c.cpus.clone()).collect();
        cpus.sort_unstable();
        cpus
    }

    #[test]
    fn cpu_lists_read_as_the_kernel_writes_them() {
        assert_eq!(parse_cpu_list("0,16\n"), vec![0, 16]);
        assert_eq!(parse_cpu_list("0-1"), vec![0, 1]);
        assert_eq!(parse_cpu_list("0-3,8-11"), vec![0, 1, 2, 3, 8, 9, 10, 11]);
        assert_eq!(parse_cpu_list("5"), vec![5]);
        assert_eq!(parse_cpu_list(""), Vec::<usize>::new());
        assert_eq!(parse_cpu_list("x,2"), vec![2]);
        assert_eq!(parse_cpu_list(&usize::MAX.to_string()), vec![usize::MAX]);
    }

    #[test]
    fn dedicated_servers_require_explicit_opt_in() {
        assert!(!dedicated_enabled_value(None));
        assert!(!dedicated_enabled_value(Some("0")));
        assert!(!dedicated_enabled_value(Some("true")));
        assert!(dedicated_enabled_value(Some("1")));
    }

    #[test]
    fn sixteen_cores_two_ccds_reserve_a_core_and_its_sibling() {
        let cores = machine(16, 2);
        let plan = plan(&cores, &all(&cores)).unwrap();
        assert_eq!(plan.reserved, vec![0, 16]);
        assert_eq!(plan.others.len(), 30);
        assert!(!plan.others.contains(&0) && !plan.others.contains(&16));
    }

    #[test]
    fn eight_cores_reserve_one() {
        let cores = machine(8, 2);
        let plan = plan(&cores, &all(&cores)).unwrap();
        assert_eq!(plan.reserved, vec![0, 8]);
        assert_eq!(plan.others, (1..8).chain(9..16).collect::<Vec<_>>());
    }

    #[test]
    fn four_cores_and_no_smt_reserve_nothing() {
        let cores = machine(4, 2);
        assert_eq!(plan(&cores, &all(&cores)), None);
        let cores = machine(16, 1);
        assert_eq!(plan(&cores, &all(&cores)), None);
    }

    #[test]
    fn an_affinity_mask_is_worked_inside() {
        let cores = machine(16, 2);
        // CPU 0 taken away by the player: core 0 is out, core 1 (1 and 17) is the first whole one.
        let allowed: Vec<usize> = (1..32).collect();
        let plan = plan(&cores, &allowed).unwrap();
        assert_eq!(plan.reserved, vec![1, 17]);
        assert_eq!(plan.others.len(), 29);
        assert!(plan.others.iter().all(|cpu| allowed.contains(cpu)));
        // Limited to the first CCD's 8 cores and their siblings: still enough.
        let ccd: Vec<usize> = (0..8).chain(16..24).collect();
        assert_eq!(super::plan(&cores, &ccd).unwrap().others.len(), 14);
        // Limited to 4 cores: not enough left to give one away.
        let four: Vec<usize> = (0..4).chain(16..20).collect();
        assert_eq!(super::plan(&cores, &four), None);
        // One CPU per core (siblings excluded): no SMT inside the mask.
        assert_eq!(super::plan(&cores, &(0..16).collect::<Vec<_>>()), None);
    }

    #[test]
    fn a_hybrid_cpu_reserves_its_fastest_core_with_smt() {
        // 6 performance cores with SMT (CPUs 0-11, sibling pairs), 8 efficiency cores without
        // (12-19); performance core 2 boosts highest.
        let mut cores: Vec<Core> = (0..6)
            .map(|n| Core {
                cpus: vec![2 * n, 2 * n + 1],
                rank: if n == 2 { 5_000_000 } else { 4_700_000 },
            })
            .collect();
        cores.extend((12..20).map(|cpu| Core {
            cpus: vec![cpu],
            rank: 3_500_000,
        }));
        let plan = plan(&cores, &all(&cores)).unwrap();
        assert_eq!(plan.reserved, vec![4, 5]);
        assert_eq!(plan.others.len(), 18);
        // Two performance cores and eight efficiency cores: no reservation.
        let small: Vec<Core> = cores[..2].iter().chain(&cores[6..]).cloned().collect();
        assert_eq!(super::plan(&small, &all(&small)), None);
    }

    #[test]
    fn narrowing_never_widens_and_skips_an_empty_result() {
        assert_eq!(narrowed(&[0, 2, 4], &[0, 1]), Some(vec![2, 4]));
        assert_eq!(narrowed(&[0], &[0, 1]), None);
        assert_eq!(narrowed(&[2], &[0, 1]), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn worker_reconciles_each_generation() {
        use std::sync::mpsc::channel;
        let _test = RESERVATION_TEST.lock().unwrap();
        // The count is kept from its first ask: make that before this test narrows any thread.
        let _ = crate::worker_count();
        let original = os::affinity(0).unwrap();
        if original.len() < 3 {
            return;
        }
        let (go, receive) = channel::<()>();
        let (send, result) = channel();
        let thread = std::thread::spawn(move || {
            for _ in 0..4 {
                receive.recv().unwrap();
                leave_reserved();
                send.send(os::affinity(0).unwrap()).unwrap();
            }
        });
        // This thread stands in for the server: the reservation opens and ends here.
        let server = os::State {
            original: original.clone(),
        };
        let mut keeper: Option<os::Keeper> = None;
        let mut publish = |reserved: Option<usize>| {
            let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(mut keeper) = keeper.take() {
                keeper.stop();
            }
            *active = reserved.map(|cpu| Plan {
                reserved: vec![cpu],
                others: original.iter().copied().filter(|c| *c != cpu).collect(),
            });
            keeper = active.as_ref().map(|plan| os::Keeper::start(plan, &server));
            GENERATION.fetch_add(1, Ordering::Relaxed);
        };
        publish(Some(original[0]));
        go.send(()).unwrap();
        assert_eq!(result.recv().unwrap(), original[1..]);
        go.send(()).unwrap();
        assert_eq!(result.recv().unwrap(), original[1..]);
        publish(Some(original[1]));
        go.send(()).unwrap();
        assert_eq!(
            result.recv().unwrap(),
            original
                .iter()
                .copied()
                .filter(|c| *c != original[1])
                .collect::<Vec<_>>()
        );
        publish(None);
        go.send(()).unwrap();
        assert_eq!(result.recv().unwrap(), original);
        thread.join().unwrap();
    }

    /// A thread that does what it is told and answers with its mask.
    #[cfg(target_os = "linux")]
    struct Probe {
        ask: std::sync::mpsc::Sender<Ask>,
        answer: std::sync::mpsc::Receiver<(Vec<usize>, Option<Probe>)>,
    }

    #[cfg(target_os = "linux")]
    enum Ask {
        Mask,
        /// Set its own mask with its own call, as code outside Horizon would.
        Set(Vec<usize>),
        Pin(Vec<usize>),
        Leave,
        /// Start a probe of its own, which inherits its mask.
        Spawn,
    }

    #[cfg(target_os = "linux")]
    impl Probe {
        fn start() -> Probe {
            let (ask, asked) = std::sync::mpsc::channel();
            let (answer, answers) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                while let Ok(what) = asked.recv() {
                    let child = match what {
                        Ask::Mask => None,
                        Ask::Set(cpus) => {
                            assert!(os::set_affinity(0, &cpus));
                            None
                        }
                        Ask::Pin(cpus) => {
                            assert!(pin_current(&cpus));
                            None
                        }
                        Ask::Leave => {
                            leave_reserved();
                            None
                        }
                        Ask::Spawn => Some(Probe::start()),
                    };
                    answer.send((os::affinity(0).unwrap(), child)).unwrap();
                }
            });
            Probe {
                ask,
                answer: answers,
            }
        }

        fn ask(&self, what: Ask) -> (Vec<usize>, Option<Probe>) {
            self.ask.send(what).unwrap();
            self.answer.recv().unwrap()
        }

        fn mask(&self) -> Vec<usize> {
            self.ask(Ask::Mask).0
        }

        fn with(self, what: Ask) -> Probe {
            self.ask(what);
            self
        }
    }

    /// The keeper and the release touch only what the reservation moved, and give back to each
    /// exactly what it had; a mask a thread chose, before or during the reservation, is kept,
    /// whether it is the other CPUs, inside the reserved core or anything else.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_reservation_gives_back_only_what_it_moved() {
        let _test = RESERVATION_TEST.lock().unwrap();
        // The count is kept from its first ask: make that before this test narrows any thread.
        let _ = crate::worker_count();
        let seen = std::thread::spawn(|| {
            let original = os::affinity(0).unwrap();
            if original.len() < 4 {
                return None;
            }
            let (o, plan) = (
                original.clone(),
                Plan {
                    reserved: original[..2].to_vec(),
                    others: original[2..].to_vec(),
                },
            );
            // Born before the reservation.
            let free = Probe::start();
            let changes_mind = Probe::start();
            let chose_before = Probe::start().with(Ask::Set(vec![o[1], o[2]]));
            let inside_before = Probe::start().with(Ask::Set(vec![o[0]]));
            let state = os::reserve(&plan).unwrap();
            let mut keeper = os::Keeper::start(&plan, &state);
            *ACTIVE.lock().unwrap() = Some(plan.clone());
            GENERATION.fetch_add(1, Ordering::Relaxed);
            // Born on the pinned server thread, so on the reserved core.
            let from_server = Probe::start();
            let pinned_others = Probe::start().with(Ask::Pin(plan.others.clone()));
            let pinned_inside = Probe::start().with(Ask::Pin(vec![o[0]]));
            let pinned_reserved = Probe::start().with(Ask::Pin(plan.reserved.clone()));
            let helper = Probe::start().with(Ask::Leave);
            // Born to a pin on a mask the reservation never hands out: its creator's choice.
            let from_pinned = Probe::start()
                .with(Ask::Pin(vec![o[1], o[2]]))
                .ask(Ask::Spawn)
                .1
                .unwrap();
            inside_before.ask(Ask::Leave);
            os::scan();
            // Started by a thread the keeper moved, after it did: born on the other CPUs.
            let from_moved = free.ask(Ask::Spawn).1.unwrap();
            // Seen as moved, then a choice of its own.
            changes_mind.ask(Ask::Set(vec![o[2]]));
            os::scan();
            let probes = [
                ("free", &free),
                ("changes_mind", &changes_mind),
                ("chose_before", &chose_before),
                ("inside_before", &inside_before),
                ("from_server", &from_server),
                ("pinned_others", &pinned_others),
                ("pinned_inside", &pinned_inside),
                ("pinned_reserved", &pinned_reserved),
                ("helper", &helper),
                ("from_moved", &from_moved),
                ("from_pinned", &from_pinned),
            ];
            let masks = || -> Vec<(&str, Vec<usize>)> {
                probes.iter().map(|(name, p)| (*name, p.mask())).collect()
            };
            let during = masks();
            let server_during = os::affinity(0).unwrap();
            {
                let mut active = ACTIVE.lock().unwrap();
                keeper.stop();
                *active = None;
                GENERATION.fetch_add(1, Ordering::Relaxed);
            }
            os::restore(&state);
            let after = masks();
            let server_after = os::affinity(0).unwrap();
            Some((o, plan, during, after, server_during, server_after))
        })
        .join()
        .unwrap();
        let Some((o, plan, during, after, server_during, server_after)) = seen else {
            return;
        };
        let (others, reserved) = (plan.others.clone(), plan.reserved.clone());
        let expected = |moved: &Vec<usize>| {
            vec![
                ("free", moved.clone()),
                ("changes_mind", vec![o[2]]),
                ("chose_before", vec![o[1], o[2]]),
                ("inside_before", vec![o[0]]),
                ("from_server", moved.clone()),
                ("pinned_others", others.clone()),
                ("pinned_inside", vec![o[0]]),
                ("pinned_reserved", reserved.clone()),
                ("helper", moved.clone()),
                ("from_moved", moved.clone()),
                ("from_pinned", vec![o[1], o[2]]),
            ]
        };
        assert_eq!(during, expected(&others));
        assert_eq!(after, expected(&o));
        assert_eq!((server_during, server_after), (reserved, o.clone()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reservation_restores_the_calling_thread() {
        let _test = RESERVATION_TEST.lock().unwrap();
        // The count is kept from its first ask: make that before this test narrows any thread.
        let _ = crate::worker_count();
        std::thread::spawn(|| {
            let original = os::affinity(0).unwrap();
            if original.len() < 2 {
                return;
            }
            let plan = Plan {
                reserved: vec![original[0]],
                others: original[1..].to_vec(),
            };
            let state = os::reserve(&plan).unwrap();
            assert_eq!(os::affinity(0).unwrap(), plan.reserved);
            *ACTIVE.lock().unwrap() = Some(plan.clone());
            SERVER.with(|server| server.set(true));
            let keeper = os::Keeper::start(&plan, &state);
            let guard = ServerCore {
                plan,
                state,
                keeper,
                _thread: std::marker::PhantomData,
            };
            drop(guard);
            assert_eq!(os::affinity(0).unwrap(), original);
            let missing = Plan {
                reserved: vec![libc::CPU_SETSIZE as usize],
                others: original.clone(),
            };
            assert!(os::reserve(&missing).is_none());
        })
        .join()
        .unwrap();
    }

    /// The count `available_parallelism()` answers while reserved is there before the keeper
    /// may move a thread: one moved first would size a pool from its narrowed mask.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_count_is_published_before_any_thread_moves() {
        use std::time::{Duration, Instant};
        let _test = RESERVATION_TEST.lock().unwrap();
        // The count is kept from its first ask: make that before this test narrows any thread.
        let _ = crate::worker_count();
        let original = os::affinity(0).unwrap();
        if original.len() < 2 {
            return;
        }
        let plan = Plan {
            reserved: vec![original[0]],
            others: original[1..].to_vec(),
        };
        let expected = whole(&plan);
        let moves = os::hold_moves();
        let server = std::thread::spawn(move || drop(begin(plan)));
        let deadline = Instant::now() + Duration::from_secs(5);
        while whole_set().is_none() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let published = whole_set().map(NonZeroUsize::get);
        drop(moves);
        server.join().unwrap();
        assert_eq!(published, Some(expected));
        assert_eq!(whole_set(), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pools_sized_while_reserved_count_the_whole_set() {
        let _test = RESERVATION_TEST.lock().unwrap();
        // The count is kept from its first ask: make that before this test narrows any thread.
        let _ = crate::worker_count();
        std::thread::spawn(|| {
            let original = os::affinity(0).unwrap();
            if original.len() < 3 {
                return;
            }
            let plan = Plan {
                reserved: vec![original[0]],
                others: original[1..].to_vec(),
            };
            // What std says before the pin, a CPU quota included: never more than the mask.
            let before = std::thread::available_parallelism().unwrap().get();
            let core = begin(plan.clone()).unwrap();
            let on_server = crate::available_parallelism().unwrap().get();
            let moved = std::thread::spawn({
                let others = plan.others.clone();
                move || {
                    os::set_affinity(0, &others);
                    crate::available_parallelism().unwrap().get()
                }
            })
            .join()
            .unwrap();
            drop(core);
            let after = crate::available_parallelism().unwrap().get();
            assert_eq!((on_server, moved, after), (before, before, before));
            let missing = Plan {
                reserved: vec![libc::CPU_SETSIZE as usize],
                others: original.clone(),
            };
            assert!(begin(missing).is_none());
            assert_eq!(whole_set(), None);
        })
        .join()
        .unwrap();
    }
}
