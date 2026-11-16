//! Automatic cleanup uses the same capability-bounded inventory as explicit Clear.
use super::*;
use crate::backend;
use std::sync::Mutex;
use std::time::SystemTime;
static TOAST: Mutex<Option<String>> = Mutex::new(None);
pub fn take_toast() -> Option<String> {
    TOAST.lock().unwrap_or_else(|e| e.into_inner()).take()
}
fn announce(report: &Report) {
    if let Some(message) = freed_message(report) {
        *TOAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(message);
    }
}

impl Storage {
    /// Run inside the existing build lease, never briefly releasing its exclusion.
    pub(crate) fn after_build(cache: &Cache) -> Result<Report, Error> {
        let root = backend::build_dir().canonicalize()?;
        let storage = Self {
            directory: Dir::open_ambient_dir(&root, cap_std::ambient_authority())?,
            root,
            lease: cache.share()?,
        };
        storage.stamp_build()?;
        storage.evict()
    }

    pub fn evict(&self) -> Result<Report, Error> {
        let report = self.evict_inventory(SystemTime::now())?;
        announce(&report);
        Ok(report)
    }
}

/// A lifecycle trigger is best effort, never a retry of a busy unit.
/// Storage::open acquires the exclusive build lock before inventory: concurrent
/// triggers cannot both measure and free the same unit.
pub fn request_cleanup() {
    #[cfg(not(test))]
    std::thread::spawn(|| {
        if let Err(error) = cleanup() {
            eprintln!("{}", crate::native_mods::cache_error(error));
        }
    });
}

/// After a build, inside its lease: what was freed goes to the build log, and each kept unit
/// with its reason and size as a warning.
pub(crate) fn after_build_logged(
    cache: &Cache,
    log: impl Fn(&str) -> Result<(), String>,
    warn: impl Fn(&str),
) {
    match Storage::after_build(cache) {
        Ok(report) => {
            if let Some(message) = freed_message(&report) {
                let _ = log(&message);
            }
            for kept in report.rows.iter().flat_map(|row| &row.kept) {
                warn(&format!("{} ({} bytes)", kept.reason, kept.bytes));
            }
        }
        Err(error) => warn(&crate::native_mods::cache_error(error)),
    }
}

/// A minute after the launcher starts, once: the launch-time trigger, off the UI thread.
pub fn cleanup_after_start() {
    #[cfg(not(test))]
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(60));
        if let Err(error) = cleanup() {
            // Busy is a refusal, not a retry loop. Keep the held build untouched.
            eprintln!("{}", crate::native_mods::cache_error(error));
        }
    });
}

pub fn cleanup() -> Result<Report, Error> {
    Storage::open()?.evict()
}

pub fn freed_message(report: &Report) -> Option<String> {
    let bytes: u64 = report.rows.iter().map(|row| row.freed_bytes).sum();
    (bytes > 0).then(|| {
        crate::i18n::trf(
            "Freed {size} of old caches",
            &[("size", &super::format_bytes(bytes))],
        )
    })
}
