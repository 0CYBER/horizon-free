//! Attached Unix supervision. The target is an owned unreaped child, never a PID liveness guess.
use std::{
    io,
    process::{Child, ExitStatus},
    sync::atomic::{AtomicU32, Ordering},
};
static PENDING: AtomicU32 = AtomicU32::new(0);
const SIGNALS: [i32; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];
extern "C" fn record(signal: i32) {
    if let Some(index) = SIGNALS.iter().position(|&value| value == signal) {
        PENDING.fetch_or(1 << index, Ordering::SeqCst);
    }
}

/// One per attached CLI/holder process. Install before spawn, before other signal consumers.
pub struct Forward(Vec<(i32, libc::sigaction)>);
impl Forward {
    /// Preserve prior actions and restore partial installation on failure.
    pub fn install() -> io::Result<Self> {
        PENDING.store(0, Ordering::SeqCst);
        let mut guard = Self(Vec::new());
        for signal in SIGNALS {
            // SAFETY: zero is valid sigaction storage; only the atomic handler runs on signals.
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = record as *const () as usize;
            action.sa_flags = libc::SA_RESTART;
            let mut previous = std::mem::MaybeUninit::uninit();
            unsafe {
                libc::sigemptyset(&mut action.sa_mask);
                if libc::sigaction(signal, &action, previous.as_mut_ptr()) != 0 {
                    return Err(io::Error::last_os_error());
                }
                guard.0.push((signal, previous.assume_init()));
            }
        }
        Ok(guard)
    }
    /// Forward exact requests and reap. Set group only for a child spawned with process_group(0).
    /// Ownership stays here until reaping, so the delivery id cannot be reused in this interval.
    pub fn wait(&self, child: &mut Child, group: bool) -> io::Result<i32> {
        loop {
            let pending = PENDING.swap(0, Ordering::SeqCst);
            if let Some(status) = child.try_wait()? {
                return exit_code(status);
            }
            for (index, signal) in SIGNALS.into_iter().enumerate() {
                if pending & (1 << index) == 0 {
                    continue;
                }
                let id = child.id() as i32;
                // SAFETY: child is owned and unreaped; group delivery additionally requires the
                // caller's documented process_group(0) spawn, not an arbitrary process lookup.
                if unsafe { libc::kill(if group { -id } else { id }, signal) } != 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::ESRCH) {
                        return Err(error);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
/// Preserve ordinary codes and the conventional 128 + signal for Unix termination.
pub fn exit_code(status: ExitStatus) -> io::Result<i32> {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .ok_or_else(|| io::ErrorKind::InvalidData.into())
}

#[cfg(test)]
mod tests;

impl Drop for Forward {
    fn drop(&mut self) {
        for (signal, previous) in self.0.iter().rev() {
            // SAFETY: these exact prior actions were returned by successful sigaction calls.
            unsafe {
                libc::sigaction(*signal, previous, std::ptr::null_mut());
            }
        }
    }
}
