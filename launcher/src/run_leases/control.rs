//! GUI process-group control and startup lifetime registration.
use crate::backend::{self, Running};
use std::io::Write;

impl Running {
    pub fn send(&mut self, line: &str) {
        if let Some(stdin) = &mut self.stdin {
            let _ = writeln!(stdin, "{line}");
            let _ = stdin.flush();
        }
    }
    pub fn kill(&self) {
        let mut child = self.child.lock().unwrap();
        #[cfg(unix)]
        {
            // GUI children own their group. Never signal a cached, already-reaped PID.
            if matches!(child.try_wait(), Ok(None)) {
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
            }
        }
        #[cfg(not(unix))]
        let _ = child.kill();
    }
}

pub fn startup_cleanup() {
    if let Err(error) = super::publish_existing() {
        eprintln!("{}", crate::native_mods::cache_error(error));
    }
    backend::cleanup_launcher_updates();
}
