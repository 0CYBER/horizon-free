//! Stop requests from outside: Ctrl-C in the terminal, SIGTERM from systemd or launchd, SIGHUP when the
//! terminal closes, the console window closing on Windows. The handler only counts them; the supervisor
//! sees the count and stops the game cleanly (a second request stops it at once).

use std::sync::atomic::{AtomicU32, Ordering};

static REQUESTS: AtomicU32 = AtomicU32::new(0);

/// How many stop requests arrived.
pub fn requests() -> u32 {
    REQUESTS.load(Ordering::SeqCst)
}

#[cfg(unix)]
pub fn install() {
    extern "C" fn handle(_: libc::c_int) {
        REQUESTS.fetch_add(1, Ordering::SeqCst);
    }
    let handler = handle as extern "C" fn(libc::c_int);
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: the handler only increments an atomic, which is async-signal-safe.
        unsafe {
            libc::signal(signal, handler as libc::sighandler_t);
        }
    }
}

#[cfg(windows)]
pub fn install() {
    unsafe extern "system" fn handle(_: u32) -> windows_sys::core::BOOL {
        REQUESTS.fetch_add(1, Ordering::SeqCst);
        1
    }
    // SAFETY: the handler only increments an atomic; it stays valid for the life of the process.
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(handle), 1);
    }
}

#[cfg(not(any(unix, windows)))]
pub fn install() {}
