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
/// Cache the platform query once per thread; the hot path reads only thread-local state.
pub fn check_current() -> Result<(), &'static str> {
    thread_local! {
        static CHECK: std::cell::OnceCell<Result<(), &'static str>> = const { std::cell::OnceCell::new() };
    }
    CHECK.with(|check| *check.get_or_init(inspect_current))
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
        if size < REQUIRED {
            return too_small(size, "pthread_get_stacksize_np");
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
    fn guest_worker_reserves_the_required_stack() {
        super::builder()
            .spawn(|| super::check_current().unwrap())
            .unwrap()
            .join()
            .unwrap();
    }
}
