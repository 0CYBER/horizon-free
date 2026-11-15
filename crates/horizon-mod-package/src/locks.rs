//! Shared kernel-lock ownership for Cargo and runtime leases.
use std::fs::File;

/// A held lock that is released before its file closes. A child process spawned at that moment
/// (Cargo, git) holds a copy of the descriptor until its exec, and closing ours alone would leave
/// the lock taken through that copy: the next lease or cleanup would read Busy.
pub struct Locked {
    file: File,
    release: Release,
}

#[derive(Clone, Copy)]
pub enum Release {
    Unlock,
    Close,
}

impl Locked {
    pub fn close_only(mut self) -> Self {
        self.release = Release::Close;
        self
    }

    pub fn file(&self) -> &File {
        &self.file
    }
}

pub enum LockMode {
    Shared,
    Exclusive,
}

impl Drop for Locked {
    fn drop(&mut self) {
        if matches!(self.release, Release::Unlock) {
            let _ = self.file.unlock();
        }
    }
}

pub fn locked(file: File, mode: LockMode) -> Result<Locked, std::fs::TryLockError> {
    match mode {
        LockMode::Shared => file.try_lock_shared(),
        LockMode::Exclusive => file.try_lock(),
    }?;
    Ok(Locked {
        file,
        release: Release::Unlock,
    })
}
