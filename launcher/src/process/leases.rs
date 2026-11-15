//! Child-owned kernel claims. Not wired into managed launch until the complete safety stack.
use crate::{
    native_cache::{Error, LockMode, Release},
    run_leases::{Claim, Registry},
};
use cap_std::fs::Dir;
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

/// Independent open descriptions: dropping the creator never explicitly unlocks the child.
pub struct ChildLeases {
    claims: Vec<Claim>,
    data: PathBuf,
}
impl ChildLeases {
    /// Acquire the shared gate, then each held tree. Fails immediately on contention.
    pub fn acquire(data: &Path, trees: &[Dir]) -> io::Result<Self> {
        let registry = Registry::with_release(data, Release::Close).map_err(error)?;
        let claims = trees
            .iter()
            .map(|tree| registry.lock(tree, LockMode::Shared).map_err(error))
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self {
            claims,
            data: data.canonicalize()?,
        })
    }
    /// Create exactly one native child inside the isolated holder. Inherits that holder's cwd
    /// and stdio; the supplied environment is complete, not an overlay. No launcher-global
    /// handle is made inheritable. A refused breakaway is returned for the caller's job policy.
    #[cfg(windows)]
    pub fn spawn_windows(
        self,
        program: &Path,
        args: &[std::ffi::OsString],
        environment: &[(std::ffi::OsString, std::ffi::OsString)],
        breakaway: bool,
    ) -> io::Result<WindowsChild> {
        let child = windows::spawn(
            &self.files(),
            &self.data,
            program,
            args,
            environment,
            breakaway,
        );
        drop(self);
        child
    }

    fn files(&self) -> Vec<&File> {
        self.claims.iter().flat_map(Claim::files).collect()
    }
    /// Only the chosen fork clears CLOEXEC. Consume the command, so stale pre_exec fds cannot
    /// be reused after these owners drop. No allocation or lock is taken in the child callback.
    #[cfg(unix)]
    pub fn spawn(self, mut command: std::process::Command) -> io::Result<std::process::Child> {
        use std::os::{
            fd::{AsRawFd, FromRawFd},
            unix::process::CommandExt,
        };
        let transfer = self
            .files()
            .iter()
            .map(|file| {
                // SAFETY: file is owned; duplicate above stdio so Command cannot replace it.
                let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
                if fd < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    // SAFETY: fcntl returned a new owned descriptor; File closes without unlock.
                    Ok(unsafe { File::from_raw_fd(fd) })
                }
            })
            .collect::<io::Result<Vec<_>>>()?;
        let fds: Vec<_> = transfer.iter().map(AsRawFd::as_raw_fd).collect();
        command.env("HORIZON_RUNTIME_DATA_ROOT", &self.data).env(
            "HORIZON_RUNTIME_LEASE_FDS",
            fds.iter().map(i32::to_string).collect::<Vec<_>>().join(","),
        );
        // SAFETY: only async-signal-safe fcntl calls run after fork. transfer owns every fd until
        // spawn returns, and the creator's copies keep their original CLOEXEC flags.
        unsafe {
            command.pre_exec(move || {
                // Command's fork path can allocate a pipe at a closed stdio slot. Its
                // dup2(fd, fd) leaves CLOEXEC set, so exec would discard that pipe.
                // Stdio setup is complete here; preserve every configured stdio handle.
                for fd in 0..3 {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 {
                        let error = io::Error::last_os_error();
                        if error.raw_os_error() == Some(libc::EBADF) {
                            continue;
                        }
                        return Err(error);
                    }
                    if flags & libc::FD_CLOEXEC != 0
                        && libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                }
                for &fd in &fds {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let child = command.spawn();
        drop(transfer); // File closes only: inherited descriptions are never explicitly unlocked.
        drop(self); // Close-only, including the gate: never unlock the transferred set.
        child
    }
}
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::WindowsChild;

#[cfg(test)]
mod tests;

fn error(error: Error) -> io::Error {
    match error {
        Error::Busy | Error::UnprovenExclusion | Error::IncompleteUnit => {
            io::ErrorKind::WouldBlock.into()
        }
        Error::UnsupportedFilesystem => io::ErrorKind::Unsupported.into(),
        Error::Unsafe(_) => io::ErrorKind::InvalidInput.into(),
        Error::Io(error) => error,
    }
}
