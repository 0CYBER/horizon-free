//! Scheduling hints for the game's threads: the few OS calls that the other
//! crates, which forbid unsafe code, can't make themselves.

use std::sync::atomic::{AtomicBool, Ordering};

/// Ask the OS to run the calling thread ahead of the process's other
/// threads, as vanilla's `MinecraftServer.spin` does for its server thread
/// (`Thread.setPriority(8)` when there are more than four processors), so
/// world generation and chunk encoding on every other core don't hold a tick
/// back (the client asks the same for its render thread, so they don't hold
/// a frame back either): user-interactive quality of service on macOS,
/// above-normal priority on Windows. Nothing elsewhere: Linux lets an
/// unprivileged thread lower its priority but not raise it (and the JVM
/// leaves it there too). Whether the OS took it.
pub fn favour_current() -> bool {
    if std::thread::available_parallelism().map_or(1, |n| n.get()) <= 4 {
        return false;
    }
    favour()
}

#[cfg(target_vendor = "apple")]
fn favour() -> bool {
    // SAFETY: sets the calling thread's own QoS class; no pointers.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0) == 0
    }
}

#[cfg(windows)]
fn favour() -> bool {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
    };
    // SAFETY: GetCurrentThread's pseudo handle needs no closing.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) != 0 }
}

#[cfg(not(any(target_vendor = "apple", windows)))]
fn favour() -> bool {
    false
}

/// Set once by a dedicated server, before its helper threads start: they
/// then go behind its server thread (`behind_server`).
static HELPERS_BEHIND: AtomicBool = AtomicBool::new(false);

/// How many nice levels a helper thread goes below the process on Linux.
#[cfg(target_os = "linux")]
const BEHIND: i32 = 10;

/// A dedicated server: from now on, the helper threads that call
/// `behind_server` as they start go behind the server thread. Call it
/// before they start.
pub fn put_helpers_behind() {
    HELPERS_BEHIND.store(true, Ordering::Relaxed);
}

/// Called by a helper thread as it starts (world generation and lighting,
/// a connection's packet writer): work the tick never waits for. In a
/// dedicated server (`put_helpers_behind`) on Linux, where the server thread
/// can't be raised (`favour_current`), the helper lowers itself instead,
/// which an unprivileged thread may: 10 nice levels below the process, or,
/// where nice runs out (a server started under nice 10 or more), the idle
/// class, which runs the thread only when no ordinary thread wants its core.
/// The server thread then gets a core when its tick starts instead of
/// waiting behind them. Nothing elsewhere. Whether the OS took it.
pub fn behind_server() -> bool {
    HELPERS_BEHIND.load(Ordering::Relaxed) && behind()
}

#[cfg(target_os = "linux")]
fn behind() -> bool {
    // SAFETY: these read and set the calling thread's own nice value or
    // policy (who 0, pid 0 is the calling thread on Linux); `param` lives
    // across the call.
    unsafe {
        let nice = libc::getpriority(libc::PRIO_PROCESS as _, 0);
        if nice + BEHIND <= 19 {
            libc::setpriority(libc::PRIO_PROCESS as _, 0, nice + BEHIND) == 0
        } else {
            let param = libc::sched_param { sched_priority: 0 };
            libc::sched_setscheduler(0, libc::SCHED_IDLE, &param) == 0
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn behind() -> bool {
    false
}

#[cfg(test)]
mod tests {
    /// The hint holds on the platforms that take it.
    #[test]
    fn favouring_a_thread_takes_where_the_os_allows_it() {
        let took = std::thread::spawn(super::favour).join().unwrap();
        assert_eq!(took, cfg!(any(target_vendor = "apple", windows)));
        #[cfg(target_vendor = "apple")]
        std::thread::spawn(|| {
            assert!(super::favour());
            let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
            let mut relative = 0;
            // SAFETY: reads the calling thread's own QoS class into locals.
            let read = unsafe {
                libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative)
            };
            assert_eq!(read, 0);
            assert!(matches!(
                class,
                libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE
            ));
        })
        .join()
        .unwrap();
    }

    /// A helper goes behind only once a dedicated server asks for it, and
    /// then, on Linux, 10 nice levels below the process or in the idle class.
    #[test]
    fn helpers_go_behind_only_when_asked() {
        assert!(!std::thread::spawn(super::behind_server).join().unwrap());
        super::put_helpers_behind();
        #[cfg(target_os = "linux")]
        std::thread::spawn(|| {
            // SAFETY: reads the calling thread's own nice value.
            let before = unsafe { libc::getpriority(libc::PRIO_PROCESS as _, 0) };
            assert!(super::behind_server());
            // SAFETY: reads the calling thread's own nice value and policy.
            let (after, policy) = unsafe {
                (
                    libc::getpriority(libc::PRIO_PROCESS as _, 0),
                    libc::sched_getscheduler(0),
                )
            };
            if before + super::BEHIND <= 19 {
                assert_eq!(after, before + super::BEHIND);
            } else {
                assert_eq!(policy, libc::SCHED_IDLE);
            }
        })
        .join()
        .unwrap();
        #[cfg(not(target_os = "linux"))]
        assert!(!std::thread::spawn(super::behind_server).join().unwrap());
    }
}
