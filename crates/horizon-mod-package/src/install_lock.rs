//! Exclusive browser mutations per game home, shared by the game and launcher.
//!
//! Hold a guard across the entire load/modify/save transaction, including package
//! replacement. The persistent `.mod-browser.lock` file must never be removed:
//! locking the manifest itself would stop protecting it when it is replaced.
//! Nested operations on the owning thread reuse the same OS lock.
use std::collections::HashMap;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread::ThreadId;

struct LockState {
    holder: ThreadId,
    count: usize,
    file: Option<File>,
}

#[derive(Default)]
struct Registry {
    locks: Mutex<HashMap<PathBuf, LockState>>,
    condvar: Condvar,
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::default)
}

/// Normalize existing homes, including aliases of the same directory.
fn normalize_home(home: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(home)?;
    let path = home.canonicalize()?;
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        let clean = if let Some(stripped) = s.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{stripped}")
        } else {
            s.strip_prefix(r"\\?\").unwrap_or(&s).to_owned()
        };
        Ok(PathBuf::from(clean.to_lowercase()))
    }
    #[cfg(not(windows))]
    {
        Ok(path)
    }
}

/// Exclusive mutation access to one game home. Releases the OS lock when the
/// last nested guard drops. A guard must stay on the thread that acquired it.
#[must_use = "if unused the lock drops immediately"]
pub struct HomeLockGuard {
    path: PathBuf,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Drop for HomeLockGuard {
    fn drop(&mut self) {
        let reg = registry();
        let mut locks = reg.locks.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = locks.get_mut(&self.path) {
            state.count -= 1;
            if state.count == 0 {
                locks.remove(&self.path);
                reg.condvar.notify_all();
            }
        }
    }
}

fn acquire(home: &Path, blocking: bool) -> io::Result<Option<HomeLockGuard>> {
    let path = normalize_home(home)?;
    let current_tid = std::thread::current().id();
    let reg = registry();
    let mut locks = reg.locks.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        match locks.get_mut(&path) {
            Some(state) if state.holder == current_tid => {
                state.count += 1;
                return Ok(Some(HomeLockGuard {
                    path,
                    _not_send: std::marker::PhantomData,
                }));
            }
            Some(_) if !blocking => return Ok(None),
            Some(_) => {
                locks = reg.condvar.wait(locks).unwrap_or_else(|e| e.into_inner());
            }
            None => {
                locks.insert(
                    path.clone(),
                    LockState {
                        holder: current_tid,
                        count: 1,
                        file: None,
                    },
                );
                break;
            }
        }
    }
    // Reserve this home before waiting on another process, but do not hold the
    // registry mutex: unrelated homes must still be able to make progress.
    drop(locks);
    let guard = HomeLockGuard {
        path,
        _not_send: std::marker::PhantomData,
    };
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(guard.path.join(".mod-browser.lock"))?;
    if blocking {
        file.lock()?;
    } else {
        match file.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Ok(None),
            Err(fs::TryLockError::Error(e)) => return Err(e),
        }
    }
    reg.locks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(&guard.path)
        .expect("the guard reserves this home")
        .file = Some(file);
    Ok(Some(guard))
}

/// Acquires exclusive mutation access, waiting for other threads or processes.
/// Re-entrant on the owning thread. Filesystem errors refuse the operation.
pub fn lock_home(home: &Path) -> io::Result<HomeLockGuard> {
    acquire(home, true).map(|guard| guard.expect("blocking acquisition returns a guard"))
}

/// Acquires exclusive mutation access without waiting for another thread or
/// process. Returns `None` when busy or on an I/O error (logged); callers must
/// leave the home unchanged in either case.
pub fn try_lock_home(home: &Path) -> Option<HomeLockGuard> {
    match acquire(home, false) {
        Ok(guard) => guard,
        Err(e) => {
            log::warn!("Cannot lock mod browser in {}: {e}", home.display());
            None
        }
    }
}

#[cfg(test)]
mod tests;
