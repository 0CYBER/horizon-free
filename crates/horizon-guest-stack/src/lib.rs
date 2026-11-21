//! Stack policy shared by guest callers and executable link settings.

#[path = "guest_stack_constants.rs"]
mod constants;
pub use constants::{GUEST_STACK_BYTES, HOST_HEADROOM};

/// Spawn guarantees the reservation even on platforms without a stack-size query.
pub fn builder() -> std::thread::Builder {
    std::thread::Builder::new().stack_size(GUEST_STACK_BYTES + HOST_HEADROOM)
}

/// Starts a thread that may enter a guest: guest-sized and named `name`, and in debug builds it
/// asserts at its start that the reservation took, so a test that starts it fails loudly if not.
pub fn spawn<T: Send + 'static>(
    name: String,
    body: impl FnOnce() -> T + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<T>> {
    builder().name(name).spawn(move || {
        debug_assert_eq!(
            check_current(),
            Ok(()),
            "a guest-capable thread must start on the guest stack"
        );
        body()
    })
}

/// Runs `test` on a guest-sized worker named like the calling thread, so libtest reports it under
/// its own name, and resumes its panic with the original payload. For tests that enter a guest:
/// libtest's own threads are too small.
pub fn run_on_guest_stack(test: impl FnOnce() + Send + 'static) {
    let mut worker = builder();
    if let Some(name) = std::thread::current().name() {
        worker = worker.name(name.to_owned());
    }
    if let Err(payload) = worker.spawn(test).unwrap().join() {
        std::panic::resume_unwind(payload);
    }
}

/// Runs an executable's whole `main` as `body`, after [`prepare_main`]: call it first, around
/// everything else. Where the main thread's stack is too small and nothing can enlarge it (an
/// x86_64 game under Rosetta 2 gets the soft limit's 8 MiB whatever `-stack_size` it links, and the
/// hard limit is below the guest stack), `body` runs on a guest-sized stack the main thread
/// allocates. It stays on the main thread, where AppKit needs the window and the event loop, and
/// where a guest's host imports find their caller's state.
pub fn run_main<R>(body: impl FnOnce() -> R) -> R {
    #[cfg(all(target_vendor = "apple", target_arch = "x86_64"))]
    {
        // SAFETY: pthread_main_np only reads the calling thread's identity.
        if inspect_current().is_err() && unsafe { libc::pthread_main_np() } != 0 {
            let switched = on_new_stack(REQUIRED + HOST_HEADROOM, |_| {
                // Judged on the new stack: refuses as before if even that is too small.
                prepare_main();
                body()
            });
            return switched.unwrap_or_else(|error| {
                eprintln!(
                    "GuestStackUnavailable: cannot allocate the main thread's stack: {error}"
                );
                std::process::exit(1);
            });
        }
    }
    prepare_main();
    body()
}

thread_local! {
    /// The stack `on_new_stack` switched this thread to, as (start, end), or (0, 0) on its own.
    /// It scopes `check_current`'s cache and tells `inspect_current` the switched stack's bounds.
    static ACTIVE_STACK: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    /// `check_current`'s answer, and the stack it was given on.
    static CHECK: std::cell::Cell<Option<((usize, usize), Result<(), &'static str>)>> =
        const { std::cell::Cell::new(None) };
}

/// Runs `body` on this same thread on a new stack of `size` bytes above a guard, then returns its
/// result here, or resumes its panic here. `body` gets the new stack's usable range.
#[cfg(all(
    target_arch = "x86_64",
    any(target_vendor = "apple", all(test, target_os = "linux"))
))]
fn on_new_stack<R>(
    size: usize,
    body: impl FnOnce(std::ops::Range<usize>) -> R,
) -> std::io::Result<R> {
    let mapped = MappedStack::new(size)?;
    Ok(on_stack(&mapped, body))
}

/// A stack mapping: a guard, the stack, and zeroed room above the stack's top, so a backtrace
/// taken on it that walks past its first frame by the stack pointer reads zeros, which end the
/// walk, not unmapped memory. Unmapped when dropped.
#[cfg(all(
    target_arch = "x86_64",
    any(target_vendor = "apple", all(test, target_os = "linux"))
))]
struct MappedStack {
    base: *mut libc::c_void,
    length: usize,
    stack: std::ops::Range<usize>,
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_vendor = "apple", all(test, target_os = "linux"))
))]
impl MappedStack {
    fn new(size: usize) -> std::io::Result<Self> {
        const GUARD: usize = 64 * 1024;
        const ABOVE: usize = 64 * 1024;
        let length = GUARD + size + ABOVE;
        // SAFETY: a new private anonymous mapping that nothing else refers to.
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if base == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        let mapped = Self {
            base,
            length,
            stack: base.addr() + GUARD..base.addr() + GUARD + size,
        };
        // SAFETY: the first GUARD bytes of the mapping just made.
        if unsafe { libc::mprotect(base, GUARD, libc::PROT_NONE) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(mapped)
    }
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_vendor = "apple", all(test, target_os = "linux"))
))]
impl Drop for MappedStack {
    fn drop(&mut self) {
        // SAFETY: the mapping `new` made; nothing runs on it once `on_stack` has returned.
        unsafe { libc::munmap(self.base, self.length) };
    }
}

/// Runs `body` on `mapped`. A guest-stack check cached for the outer stack is set aside, so the
/// body checks afresh, and whatever the body cached is dropped on the way out, so a later switch
/// to a mapping at the same address checks afresh too.
#[cfg(all(
    target_arch = "x86_64",
    any(target_vendor = "apple", all(test, target_os = "linux"))
))]
fn on_stack<R>(mapped: &MappedStack, body: impl FnOnce(std::ops::Range<usize>) -> R) -> R {
    let stack = mapped.stack.clone();
    let mut body = Some(body);
    let mut outcome = None;
    {
        let mut once = || {
            let body = body.take().expect("the new stack runs its body once");
            let outer = (ACTIVE_STACK.replace((stack.start, stack.end)), CHECK.take());
            outcome = Some(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || body(stack.clone()),
            )));
            // Also after a panic: the caught one is resumed only on the outer stack.
            ACTIVE_STACK.set(outer.0);
            CHECK.set(outer.1);
        };
        let mut run: &mut dyn FnMut() = &mut once;
        // SAFETY: the stack's end is the page-aligned end of the writable part of a mapping that
        // outlives the call, and `once` never unwinds.
        unsafe { call_on_stack(stack.end, &mut run) };
    }
    match outcome.expect("the new stack ran its body") {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Calls `run` with the stack pointer at `top`, and restores it after.
///
/// # Safety
/// `top` must be the 16-byte aligned end of writable memory deep enough for `run`, and `run`
/// must not unwind.
#[cfg(all(
    target_arch = "x86_64",
    any(target_vendor = "apple", all(test, target_os = "linux"))
))]
unsafe fn call_on_stack(top: usize, run: &mut &mut dyn FnMut()) {
    extern "C" fn enter(run: *mut std::ffi::c_void) {
        // SAFETY: call_on_stack passes its live `run`.
        let run = unsafe { &mut *run.cast::<&mut dyn FnMut()>() };
        run();
    }
    // SAFETY: the caller's contract; r12 is callee-saved, so it still holds the old stack pointer
    // when `enter` returns. A call needs rsp 16-byte aligned, which `top` is.
    unsafe {
        std::arch::asm!(
            "mov r12, rsp",
            "mov rsp, {top}",
            "call {enter}",
            "mov rsp, r12",
            top = in(reg) top,
            enter = sym enter,
            in("rdi") std::ptr::from_mut(run).cast::<std::ffi::c_void>(),
            out("r12") _,
            clobber_abi("C"),
        );
    }
}

/// Must run before compiler-child handling or any game initialization.
pub fn prepare_main() {
    if let Err(error) = prepare() {
        eprintln!("GuestStackUnavailable: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn prepare() -> std::io::Result<()> {
    // Executable link settings reserve the same stack on Windows and macOS.
    inspect_current().map_err(std::io::Error::other)
}

#[cfg(target_os = "linux")]
fn prepare() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let required = (GUEST_STACK_BYTES + HOST_HEADROOM) as libc::rlim_t;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: limit is writable and getrlimit initializes it on success.
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let inspected = inspect_current();
    if limit.rlim_cur >= required && inspected.is_ok() {
        return Ok(());
    }
    let describe = |bytes| {
        if bytes == libc::RLIM_INFINITY {
            "unlimited".to_owned()
        } else {
            format!("{} KiB", bytes / 1024)
        }
    };
    let limits = format!(
        "soft={}, hard={}",
        describe(limit.rlim_cur),
        describe(limit.rlim_max)
    );
    if std::env::var_os("HORIZON_STACK_REEXEC").is_some() {
        let why = inspected
            .err()
            .map_or("the soft limit is below the requirement", |why| {
                why.trim_start_matches("GuestStackUnavailable: ")
            });
        return Err(std::io::Error::other(format!(
            "stack re-exec marker already set but the stack is insufficient ({limits}): {why}"
        )));
    }
    if limit.rlim_max < required {
        return Err(std::io::Error::other(format!(
            "stack hard limit is too low ({limits}); the game needs {} KiB of stack, raise \
             ulimit -Hs to at least {} KiB",
            required / 1024,
            required.saturating_add(HOST_HEADROOM as libc::rlim_t) / 1024
        )));
    }
    // The argument and environment pages sit at the top of the main stack and count
    // against the limit. Reserve additional host headroom for those pages when the
    // hard limit permits it; guest entry still verifies the room left.
    limit.rlim_cur = required
        .saturating_add(HOST_HEADROOM as libc::rlim_t)
        .min(limit.rlim_max);
    // SAFETY: limit is initialized; this changes only this process's resource limit.
    if unsafe { libc::setrlimit(libc::RLIMIT_STACK, &limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Linux chooses its mmap base at exec using RLIMIT_STACK. Re-exec keeps the
    // enlarged stack's address range clear before libraries and workers start.
    Err(std::process::Command::new(std::env::current_exe()?)
        .arg0(std::env::args_os().next().unwrap_or_default())
        .args(std::env::args_os().skip(1))
        .env("HORIZON_STACK_REEXEC", "1")
        .exec())
}

/// Refuse guest entry when the calling thread's stack reservation is too small.
/// Cache the platform query once per thread and stack (`run_main` may switch the main thread's);
/// the hot path reads only thread-local state.
pub fn check_current() -> Result<(), &'static str> {
    let stack = ACTIVE_STACK.get();
    match CHECK.get() {
        Some((checked_on, result)) if checked_on == stack => result,
        _ => {
            let result = inspect_current();
            CHECK.set(Some((stack, result)));
            result
        }
    }
}

const REQUIRED: usize = GUEST_STACK_BYTES + HOST_HEADROOM;

/// A refusal naming what was measured. Leaked once per refusing thread: `check_current` keeps it
/// for the thread's life.
#[cfg(any(target_os = "linux", target_vendor = "apple", windows))]
fn refuse(why: String) -> Result<(), &'static str> {
    Err(Box::leak(
        format!("GuestStackUnavailable: {why}").into_boxed_str(),
    ))
}

/// A refusal where nothing could be measured: the reason, and the requirement.
#[cfg(target_os = "linux")]
fn unmeasured(why: &str) -> Result<(), &'static str> {
    refuse(format!("{why}; needs {} KiB", REQUIRED / 1024))
}

#[cfg(any(target_os = "linux", target_vendor = "apple", windows))]
fn too_small(size: usize, measured_by: &str) -> Result<(), &'static str> {
    refuse(format!(
        "the calling thread has {} KiB of stack, needs {} KiB ({measured_by})",
        size / 1024,
        REQUIRED / 1024
    ))
}

fn inspect_current() -> Result<(), &'static str> {
    #[cfg(all(
        target_arch = "x86_64",
        any(target_vendor = "apple", all(test, target_os = "linux"))
    ))]
    {
        // On a stack on_new_stack allocated, its bounds are known: no platform query describes it.
        let (start, end) = ACTIVE_STACK.get();
        let anchor = 0u8;
        let anchor = std::hint::black_box(&raw const anchor).addr();
        if (start..end).contains(&anchor) {
            return if anchor - start >= REQUIRED {
                Ok(())
            } else {
                too_small(
                    anchor - start,
                    "the stack run_main allocated, below this frame",
                )
            };
        }
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: gettid only reads this thread's id.
        let tid = unsafe { libc::syscall(libc::SYS_gettid) };
        let tid_error = std::io::Error::last_os_error();
        // SAFETY: getpid only reads this process's id and cannot fail.
        let pid = libc::c_long::from(unsafe { libc::getpid() });
        match is_main_thread(tid, pid) {
            None => {
                return unmeasured(&format!(
                    "cannot tell whether this is the main thread: gettid failed ({tid_error})"
                ));
            }
            Some(true) => {
                return match main_thread_room() {
                    Ok((room, _)) if room >= REQUIRED => Ok(()),
                    Ok((room, bound)) => refuse(format!(
                        "the main thread has {} KiB of stack, needs {} KiB ({bound})",
                        room / 1024,
                        REQUIRED / 1024
                    )),
                    Err(why) => {
                        unmeasured(&format!("cannot measure the main thread's stack: {why}"))
                    }
                };
            }
            Some(false) => {}
        }
        let mut attr = std::mem::MaybeUninit::<libc::pthread_attr_t>::uninit();
        // SAFETY: pthread_self is live and attr is writable. Destroy only on success.
        unsafe {
            if libc::pthread_getattr_np(libc::pthread_self(), attr.as_mut_ptr()) != 0 {
                return unmeasured(
                    "cannot inspect the calling thread stack: pthread_getattr_np failed",
                );
            }
            let mut attr = attr.assume_init();
            let mut size = 0;
            let result = libc::pthread_attr_getstacksize(&attr, &mut size);
            libc::pthread_attr_destroy(&mut attr);
            if result != 0 {
                return unmeasured(
                    "cannot inspect the calling thread stack: pthread_attr_getstacksize failed",
                );
            }
            if size < REQUIRED {
                return too_small(size, "pthread_getattr_np");
            }
        }
    }
    #[cfg(target_vendor = "apple")]
    {
        // SAFETY: pthread_self names the live calling thread.
        let size = unsafe { libc::pthread_get_stacksize_np(libc::pthread_self()) };
        // SAFETY: pthread_main_np only reads the calling thread's identity.
        if size < REQUIRED && unsafe { libc::pthread_main_np() } == 0 {
            return too_small(size, "pthread_get_stacksize_np");
        }
        if size < REQUIRED {
            // For the main thread libpthread answers the soft stack limit unless the stack it was
            // told about has exactly one guard page: an x86_64 game under Rosetta 2 got 8176 KiB
            // (the default soft limit) though it links a larger `-stack_size`. The mapping decides:
            // the room below this frame, which is why macOS links more than REQUIRED.
            let anchor = 0u8;
            let anchor = std::hint::black_box(&raw const anchor).addr();
            let pthread = format!(
                "pthread_get_stacksize_np says {} KiB, the soft stack limit is {}",
                size / 1024,
                soft_stack_limit()
            );
            match mapped_stack(anchor, REQUIRED, vm_region) {
                Ok(mapped) if mapped >= REQUIRED => {}
                Ok(mapped) => {
                    return refuse(format!(
                        "the main thread has {} KiB of stack below this frame, needs {} KiB (its \
                         mapped stack regions; {pthread})",
                        mapped / 1024,
                        REQUIRED / 1024
                    ));
                }
                Err(why) => {
                    return refuse(format!(
                        "cannot measure the main thread's stack: {why}; needs {} KiB ({pthread})",
                        REQUIRED / 1024
                    ));
                }
            }
        }
    }
    #[cfg(windows)]
    {
        let mut low = 0;
        let mut high = 0;
        // SAFETY: both pointers refer to writable stack bounds.
        unsafe { get_current_thread_stack_limits(&mut low, &mut high) };
        if high - low < REQUIRED {
            return too_small(high - low, "GetCurrentThreadStackLimits");
        }
    }
    #[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
    return Err("GuestStackUnavailable: stack inspection is unsupported on this platform");
    #[cfg(any(target_os = "linux", target_vendor = "apple", windows))]
    Ok(())
}

/// Whether the caller is the main thread (its thread id is the process id); `None` when the
/// thread-id query failed (a negative result, as under a seccomp filter that refuses it).
#[cfg(target_os = "linux")]
fn is_main_thread(tid: libc::c_long, pid: libc::c_long) -> Option<bool> {
    (tid >= 0).then_some(tid == pid)
}

/// The main thread's stack grows on demand up to `RLIMIT_STACK`, so its size is the limit, not the
/// pages mapped so far (what musl's `pthread_getattr_np` reports for it). Reads the stack's top and
/// the mapping below it from `/proc/self/maps`, and where the arguments and environment end (the
/// initial stack pointer, `startstack`) from `/proc/self/stat`. The answer doesn't depend on how
/// deep the caller is, so `check_current` may cache it. Returns the room and what bounds it.
#[cfg(target_os = "linux")]
fn main_thread_room() -> Result<(usize, String), String> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: limit is writable and getrlimit initializes it on success.
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut limit) } != 0 {
        return Err(format!(
            "getrlimit(RLIMIT_STACK) failed ({})",
            std::io::Error::last_os_error()
        ));
    }
    // RLIM_INFINITY is the largest value, so it stays unbounded here.
    let soft_limit = usize::try_from(limit.rlim_cur).unwrap_or(usize::MAX);
    // SAFETY: sysconf only reads a system setting.
    let page = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) })
        .map_err(|_| "sysconf(_SC_PAGESIZE) failed".to_owned())?;
    // The kernel's default `stack_guard_gap`: 256 pages kept free above the mapping below.
    let guard_gap = 256 * page;
    let stat = std::fs::read("/proc/self/stat")
        .map_err(|error| format!("/proc/self/stat unreadable ({error})"))?;
    let start_stack = start_stack(&stat).ok_or("no startstack in /proc/self/stat")?;
    let maps = std::fs::read("/proc/self/maps")
        .map_err(|error| format!("/proc/self/maps unreadable ({error})"))?;
    let (below, top) = stack_row(&maps, start_stack).ok_or_else(|| {
        format!(
            "cannot read the stack's mapping: no valid [stack] row of /proc/self/maps holds \
             startstack {start_stack:#x}"
        )
    })?;
    // stack_row guarantees below <= start_stack < top, so these differences cannot wrap.
    let (room, bound) = main_stack_room(soft_limit, below, guard_gap, top, start_stack);
    let used = (top - start_stack) / 1024;
    let bound = match bound {
        StackBound::Limit => format!(
            "soft stack limit {} KiB, less {used} KiB of arguments and environment",
            soft_limit / 1024
        ),
        StackBound::MappingBelow => format!(
            "the mapping below ends at {below:#x}, {} KiB under the stack's top {top:#x}, less the \
             {} KiB guard gap and {used} KiB of arguments and environment",
            (top - below) / 1024,
            guard_gap / 1024
        ),
    };
    Ok((room, bound))
}

/// `startstack`, field 28 of `/proc/self/stat`. The fields after the command name, which ends at
/// the last ')', start at field 3.
#[cfg(target_os = "linux")]
fn start_stack(stat: &[u8]) -> Option<usize> {
    let after_name = &stat[stat.iter().rposition(|&byte| byte == b')')? + 1..];
    std::str::from_utf8(after_name)
        .ok()?
        .split_ascii_whitespace()
        .nth(25)?
        .parse()
        .ok()
}

/// The `[stack]` row of `/proc/<pid>/maps` that holds `address`, as (end of the row before it,
/// its own end), so `below <= address < end`; `None` also when a row before it is malformed or out
/// of order. The path field must be exactly the pseudo-name: a file may be called
/// `/opt/horizon-[stack]`. Mapped file names need not be UTF-8, so the rows are read as bytes.
#[cfg(target_os = "linux")]
fn stack_row(maps: &[u8], address: usize) -> Option<(usize, usize)> {
    let mut below = 0;
    for line in maps
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        // Range, permissions, offset, device, inode, then the padded path (if any).
        let mut fields = line.splitn(6, |&byte| byte == b' ');
        let (start, end) = std::str::from_utf8(fields.next()?).ok()?.split_once('-')?;
        let start = usize::from_str_radix(start, 16).ok()?;
        let end = usize::from_str_radix(end, 16).ok()?;
        // Linux lists increasing, non-overlapping ranges; anything else is unreadable.
        if start >= end || start < below {
            return None;
        }
        let path = fields.nth(4).map_or(&b""[..], <[u8]>::trim_ascii_start);
        if path == b"[stack]" && (start..end).contains(&address) {
            return Some((below, end));
        }
        below = end;
    }
    None
}

#[cfg(target_os = "linux")]
#[derive(Debug, PartialEq)]
enum StackBound {
    /// `RLIMIT_STACK`: mm/mmap.c `acct_stack_growth` counts the whole stack, top to lowest address.
    Limit,
    /// The next lower mapping plus the guard gap: mm/mmap.c `expand_downwards`.
    MappingBelow,
}

/// Room left below `used_from` on a stack whose top is `top`. The kernel lets the stack grow to
/// `soft_limit` bytes in all, the pages above `used_from` included, and never to within
/// `guard_gap` of the mapping that ends at `below`. The gap bounds only the mapping, not the limit.
#[cfg(target_os = "linux")]
fn main_stack_room(
    soft_limit: usize,
    below: usize,
    guard_gap: usize,
    top: usize,
    used_from: usize,
) -> (usize, StackBound) {
    let to_mapping = top.saturating_sub(below.saturating_add(guard_gap));
    let (reach, bound) = if to_mapping < soft_limit {
        (to_mapping, StackBound::MappingBelow)
    } else {
        (soft_limit, StackBound::Limit)
    };
    (reach.saturating_sub(top.saturating_sub(used_from)), bound)
}

/// A region of the address space as `mach_vm_region` answers it.
#[cfg(any(target_vendor = "apple", test))]
#[derive(Clone, Copy, Debug)]
struct Region {
    start: usize,
    end: usize,
    read_write: bool,
    /// Who made it (<mach/vm_statistics.h>): stacks are `VM_MEMORY_STACK`.
    user_tag: u32,
}

#[cfg(any(target_vendor = "apple", test))]
const VM_MEMORY_STACK: u32 = 30;

/// The room below `anchor` on the stack holding it: through the readable and writable stack region
/// that holds it and the ones directly below it, to the first gap, guard or region that isn't a
/// stack's. What lies above `anchor` is in use, so it never counts. Stops early once `enough` is
/// reached. `region(address)` is the region holding `address`, or the next one above it when none
/// does.
#[cfg(any(target_vendor = "apple", test))]
fn mapped_stack(
    anchor: usize,
    enough: usize,
    mut region: impl FnMut(usize) -> Result<Region, String>,
) -> Result<usize, String> {
    let holding = region(anchor)?;
    if !(holding.start..holding.end).contains(&anchor) || !holding.read_write {
        return Err(format!(
            "no readable and writable region holds the stack address {anchor:#x}"
        ));
    }
    if holding.user_tag != VM_MEMORY_STACK {
        return Err(format!(
            "the region holding the stack address {anchor:#x} has user tag {}, not a stack's",
            holding.user_tag
        ));
    }
    let mut bottom = holding.start;
    // Relies on XNU's guard below a main stack: no other stack's region can abut it.
    while anchor - bottom < enough && bottom > 0 {
        let below = region(bottom - 1)?;
        // A region that doesn't end at `bottom` doesn't hold `bottom - 1`: a gap.
        if below.end != bottom
            || below.start >= bottom
            || !below.read_write
            || below.user_tag != VM_MEMORY_STACK
        {
            break;
        }
        bottom = below.start;
    }
    Ok(anchor - bottom)
}

#[cfg(target_vendor = "apple")]
fn vm_region(address: usize) -> Result<Region, String> {
    // <mach/vm_region.h>: the flavor, and its info's size in ints (vm_region_extended_info is
    // packed to 4 bytes: 36 bytes). It starts with the current protection, then the user tag.
    const VM_REGION_EXTENDED_INFO: libc::c_int = 13;
    let mut info: [libc::c_int; 9] = [0; 9];
    let mut count = info.len() as u32;
    let mut start = address as u64;
    let mut size: u64 = 0;
    let mut object_name: u32 = 0;
    // SAFETY: every out pointer is writable, and count is info's length in ints.
    let result = unsafe {
        mach_vm_region(
            mach_task_self_,
            &mut start,
            &mut size,
            VM_REGION_EXTENDED_INFO,
            info.as_mut_ptr(),
            &mut count,
            &mut object_name,
        )
    };
    if result != 0 {
        return Err(format!("mach_vm_region failed (kern_return_t {result})"));
    }
    let read_write = libc::VM_PROT_READ | libc::VM_PROT_WRITE;
    Ok(Region {
        start: start as usize,
        end: start.saturating_add(size) as usize,
        read_write: info[0] & read_write == read_write,
        user_tag: info[1] as u32,
    })
}

#[cfg(target_vendor = "apple")]
fn soft_stack_limit() -> String {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: limit is writable and getrlimit initializes it on success.
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut limit) } != 0 {
        "unknown".to_owned()
    } else if limit.rlim_cur == libc::RLIM_INFINITY {
        "unlimited".to_owned()
    } else {
        format!("{} KiB", limit.rlim_cur / 1024)
    }
}

#[cfg(target_vendor = "apple")]
unsafe extern "C" {
    // Written again in a forked child, so mutable: read it by value.
    static mut mach_task_self_: u32;
    fn mach_vm_region(
        target_task: u32,
        address: *mut u64,
        size: *mut u64,
        flavor: libc::c_int,
        info: *mut libc::c_int,
        count: *mut u32,
        object_name: *mut u32,
    ) -> libc::c_int;
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetCurrentThreadStackLimits"]
    fn get_current_thread_stack_limits(low: *mut usize, high: *mut usize);
}

#[cfg(test)]
#[test]
fn audited_guest_workers_use_the_stack_policy() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in [
        "crates/horizon-threads/src/jobs.rs",
        "crates/horizon-threads/src/lib.rs",
        "crates/horizon-client/src/lan/io_pool.rs",
        "crates/horizon-client/src/vanillagen.rs",
        "crates/horizon-client/src/vanillagen/lod.rs",
        "crates/horizon-client/src/game/runtime/resource_load.rs",
        "crates/horizon-client/src/game/interface/startup_menu.rs",
        "crates/horizon-client/src/game/world/world_loading.rs",
        "crates/horizon-client/src/game/world/world_entry.rs",
        "crates/horizon-client/src/game/world/packs/mod.rs",
    ] {
        let source = std::fs::read_to_string(root.join(file)).unwrap();
        let source = source.split("\n#[cfg(test)]").next().unwrap();
        assert!(
            ![
                "thread::Builder::new()",
                "thread::spawn(",
                "scope.spawn(",
                "s.spawn("
            ]
            .iter()
            .any(|form| source.contains(form)),
            "{file}: route guest workers through horizon_guest_stack::builder"
        );
    }
    let server =
        std::fs::read_to_string(root.join("crates/horizon-client/src/game/world/integrated.rs"))
            .unwrap();
    // Production code only: a test may start its own server thread with the same policy.
    let server = server.split("\n#[cfg(test)]").next().unwrap();
    assert_eq!(
        server.matches(".stack_size(server_thread_stack())").count(),
        2
    );
    assert_eq!(server.matches("thread::Builder::new()").count(), 2);
    for form in ["thread::spawn(", "scope.spawn(", "s.spawn("] {
        assert!(
            !server.contains(form),
            "server: route guest workers through the stack policy"
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn small_worker_refuses_guest_entry() {
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                assert!(
                    super::check_current()
                        .unwrap_err()
                        .contains("GuestStackUnavailable")
                )
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn main_stack_is_judged_by_its_limit() {
        use super::StackBound::{Limit, MappingBelow};
        const MIB: usize = 1024 * 1024;
        let needed = super::GUEST_STACK_BYTES + super::HOST_HEADROOM;
        let gap = MIB;
        // Fits a 32-bit address space too.
        let top: usize = 0xc000_0000;
        let far = top - 1024 * MIB;
        let env = 64 * 1024;
        let room = |soft, below, used| super::main_stack_room(soft, below, gap, top, top - used);
        // A re-executed Nightly on musl: soft limit 144 MiB.
        let (bytes, bound) = room(144 * MIB, far, env);
        assert!(bytes >= needed && bound == Limit);
        // A hard limit of 140 MiB caps the soft one there, which still leaves 136 MiB.
        assert!(room(140 * MIB, far, env).0 >= needed);
        // Unlimited: the mapping far below bounds it.
        let (bytes, bound) = room(usize::MAX, far, env);
        assert!(bytes >= needed && bound == MappingBelow);
        // Exactly 136 MiB: the kernel counts the arguments and environment against the limit.
        assert!(room(136 * MIB, far, env).0 < needed);
        // A mapping 136 MiB + the environment + less than the gap below the top is too close...
        let (bytes, bound) = room(144 * MIB, top - (136 * MIB + env + gap - 4096), env);
        assert!(bytes < needed && bound == MappingBelow);
        // ...and one a full gap further down leaves exactly enough: the gap isn't taken twice.
        assert_eq!(
            room(144 * MIB, top - (136 * MIB + env + gap), env).0,
            needed
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_the_stack_pseudo_row_is_the_stack() {
        let maps: &[u8] = b"00400000-00500000 r-xp 00000000 08:01 1234       /opt/horizon-[stack]
10000000-10100000 rw-p 00000000 00:00 0
7f000000-7f021000 rw-p 00000000 00:00 0                          [stack]
7f100000-7f101000 r-xp 00000000 00:00 0                          [vdso]
";
        assert_eq!(
            super::stack_row(maps, 0x7f02_0000),
            Some((0x1010_0000, 0x7f02_1000))
        );
        // The file row never counts, even when it holds the address.
        assert_eq!(super::stack_row(maps, 0x0040_1000), None);
        // Out-of-order or inverted rows before the stack make the table unreadable: no answer
        // rather than a floor above the stack.
        let out_of_order: &[u8] = b"7f100000-7f101000 r-xp 00000000 00:00 0
7f000000-7f021000 rw-p 00000000 00:00 0                          [stack]
";
        assert_eq!(super::stack_row(out_of_order, 0x7f02_0000), None);
        let inverted: &[u8] = b"10100000-10000000 rw-p 00000000 00:00 0
7f000000-7f021000 rw-p 00000000 00:00 0                          [stack]
";
        assert_eq!(super::stack_row(inverted, 0x7f02_0000), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_failed_thread_id_query_is_not_a_worker() {
        assert_eq!(super::is_main_thread(-1, 42), None);
        assert_eq!(super::is_main_thread(42, 42), Some(true));
        assert_eq!(super::is_main_thread(43, 42), Some(false));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startstack_is_read_after_the_command_name() {
        // A command name may hold spaces and ')'.
        let mut stat = b"7 (a) b) S".to_vec();
        for field in 4..=27 {
            stat.extend_from_slice(format!(" {field}").as_bytes());
        }
        stat.extend_from_slice(b" 3221225472 29\n");
        assert_eq!(super::start_stack(&stat), Some(0xc000_0000));
    }

    #[test]
    fn the_mapped_stack_ends_at_a_guard_a_gap_or_another_mapping() {
        use super::Region;
        const MIB: usize = 1024 * 1024;
        let needed = super::GUEST_STACK_BYTES + super::HOST_HEADROOM;
        let top: usize = 0xc000_0000;
        // `mach_vm_region`: the region holding the address, or the next one above it.
        let query = |regions: Vec<Region>| {
            move |address: usize| {
                regions
                    .iter()
                    .copied()
                    .find(|region| address < region.end)
                    .ok_or_else(|| "none above".to_owned())
            }
        };
        // A stack's region; `other` is an anonymous mapping (user tag 0).
        let region = |start, end, read_write| Region {
            start,
            end,
            read_write,
            user_tag: super::VM_MEMORY_STACK,
        };
        let other = |start, end| Region {
            start,
            end,
            read_write: true,
            user_tag: 0,
        };
        // Startup's frames use the top 64 KiB; only the room below them counts.
        let used = 64 * 1024;
        let anchor = top - used;
        // macOS links the requirement plus the host headroom (packaging/executable_link.rs).
        let linked = needed + super::HOST_HEADROOM;
        let stack = |size| {
            vec![
                region(top - size - 16 * 1024, top - size, false),
                region(top - size, top, true),
            ]
        };
        assert_eq!(
            super::mapped_stack(anchor, needed, query(stack(linked))),
            Ok(linked - used)
        );
        // A stack of exactly the requirement has less than that below a frame: refused...
        assert_eq!(
            super::mapped_stack(anchor, needed, query(stack(needed))),
            Ok(needed - used)
        );
        // ...and one with exactly the requirement below the frame is enough.
        assert_eq!(
            super::mapped_stack(anchor, needed, query(stack(needed + used))),
            Ok(needed)
        );
        // The same stack in two regions counts whole.
        let split = vec![
            region(top - linked - 16 * 1024, top - linked, false),
            region(top - linked, top - 8 * MIB, true),
            region(top - 8 * MIB, top, true),
        ];
        assert_eq!(
            super::mapped_stack(anchor, needed, query(split)),
            Ok(linked - used)
        );
        // An 8 MiB stack above a guard stops there, whatever lies below the guard.
        let guarded = vec![
            region(0x1000, top - 64 * MIB, true),
            region(top - 64 * MIB, top - 8 * MIB, false),
            region(top - 8 * MIB, top, true),
        ];
        assert_eq!(
            super::mapped_stack(anchor, needed, query(guarded)),
            Ok(8 * MIB - used)
        );
        // A gap below stops it too: the region the query answers lies above the gap.
        let gap = vec![
            region(0x1000, top - 9 * MIB, true),
            region(top - 8 * MIB, top, true),
        ];
        assert_eq!(
            super::mapped_stack(anchor, needed, query(gap)),
            Ok(8 * MIB - used)
        );
        // So does a writable mapping right below that isn't a stack's.
        let abutting = vec![
            other(0x1000, top - 8 * MIB),
            region(top - 8 * MIB, top, true),
        ];
        assert_eq!(
            super::mapped_stack(anchor, needed, query(abutting)),
            Ok(8 * MIB - used)
        );
        // An anchor in a region that isn't a stack's is not measured.
        let untagged = vec![other(top - needed, top)];
        assert!(super::mapped_stack(anchor, needed, query(untagged)).is_err());
        // The anchor in no region, or in a guard, is not measured.
        let below = vec![region(top - 8 * MIB, top, true)];
        assert!(super::mapped_stack(top - 9 * MIB, needed, query(below)).is_err());
        let in_guard = vec![region(top - 8 * MIB, top, false)];
        assert!(super::mapped_stack(anchor, needed, query(in_guard)).is_err());
        // A failed query below is not measured either.
        let mut calls = 0;
        let failing = |address: usize| {
            calls += 1;
            if calls == 1 {
                Ok(region(top - 8 * MIB, top, true))
            } else {
                Err(format!("no answer for {address:#x}"))
            }
        };
        assert!(super::mapped_stack(anchor, needed, failing).is_err());
    }

    #[cfg(all(
        target_arch = "x86_64",
        any(target_vendor = "apple", target_os = "linux")
    ))]
    #[test]
    fn a_body_runs_on_the_new_stack_and_its_panic_comes_back() {
        const MIB: usize = 1024 * 1024;
        // Each level keeps a 4 KiB frame alive across the call below it.
        fn deep(depth: usize) -> usize {
            let frame = std::hint::black_box([1u8; 4096]);
            if depth == 0 {
                return 0;
            }
            deep(depth - 1) + usize::from(std::hint::black_box(&frame)[0])
        }
        let answer = super::on_new_stack(64 * MIB, |stack| {
            assert_eq!(stack.len(), 64 * MIB);
            let local = 0u8;
            let here = std::hint::black_box(&raw const local).addr();
            assert!(stack.contains(&here), "{here:#x} is not in {stack:x?}");
            // At least 16 MiB deep: far past this test thread's own stack.
            deep(4096)
        })
        .unwrap();
        assert_eq!(answer, 4096);
        let panicked = std::panic::catch_unwind(|| {
            super::on_new_stack(MIB, |_| -> u8 { panic!("on the new stack") })
        })
        .unwrap_err();
        assert_eq!(panicked.downcast_ref::<&str>(), Some(&"on the new stack"));
    }

    #[cfg(all(
        target_arch = "x86_64",
        any(target_vendor = "apple", target_os = "linux")
    ))]
    #[test]
    fn the_check_follows_the_thread_onto_the_new_stack_and_back() {
        // A thread too small for a guest, as the main thread is under Rosetta 2.
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                let guest_sized = super::REQUIRED + super::HOST_HEADROOM;
                // Refused first, and that cached refusal doesn't stick on the new stack...
                assert!(super::check_current().is_err());
                let inside = super::on_new_stack(guest_sized, |_| super::check_current());
                assert_eq!(inside.unwrap(), Ok(()));
                // ...nor the approval there once the thread is back on its own.
                assert!(super::check_current().is_err());
                // Also when the body panicked after an approved check.
                let panicked = std::panic::catch_unwind(|| {
                    super::on_new_stack(guest_sized, |_| -> u8 {
                        super::check_current().unwrap();
                        panic!("after an approved check")
                    })
                });
                assert!(panicked.is_err());
                assert!(super::check_current().is_err());
                // A new stack smaller than the guest needs is refused.
                let small =
                    super::on_new_stack(super::GUEST_STACK_BYTES, |_| super::check_current());
                assert!(small.unwrap().is_err());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(all(
        target_arch = "x86_64",
        any(target_vendor = "apple", target_os = "linux")
    ))]
    #[test]
    fn a_later_switch_to_the_same_stack_checks_afresh() {
        // Checks at least 2 MiB below where it starts.
        fn deep_check(depth: usize) -> Result<(), &'static str> {
            let frame = std::hint::black_box([1u8; 4096]);
            if depth == 0 {
                return super::check_current();
            }
            let result = deep_check(depth - 1);
            std::hint::black_box(&frame);
            result
        }
        // One mapping, so both switches have the same address and size.
        let mapped = super::MappedStack::new(super::REQUIRED + 1024 * 1024).unwrap();
        // Near its top there is room enough...
        assert_eq!(super::on_stack(&mapped, |_| super::check_current()), Ok(()));
        // ...and with no check on the outer stack in between, the next switch deep below it must
        // not reuse that approval.
        assert!(super::on_stack(&mapped, |_| deep_check(512)).is_err());
    }

    #[test]
    fn guest_worker_reserves_the_required_stack() {
        super::builder()
            .spawn(|| super::check_current().unwrap())
            .unwrap()
            .join()
            .unwrap();
    }
}
