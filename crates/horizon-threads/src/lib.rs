//! Scheduling hints for the game's threads: the few OS calls that the other
//! crates, which forbid unsafe code, can't make themselves.

/// Ask the OS to run the calling thread ahead of the process's other
/// threads, as vanilla's `MinecraftServer.spin` does for its server thread
/// (`Thread.setPriority(8)` when there are more than four processors), so
/// world generation and chunk encoding on every other core don't hold a tick
/// back: user-interactive quality of service on macOS, above-normal priority
/// on Windows. Nothing elsewhere: Linux lets an unprivileged thread lower its
/// priority but not raise it (and the JVM leaves it there too). Whether the
/// OS took it.
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
}
