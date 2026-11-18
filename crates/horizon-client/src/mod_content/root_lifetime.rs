//! Protect prepared roots while the game or resource readers retain them.
use super::*;
use horizon_assets::resource_source::ResourceSource;
type Pin = (PathBuf, Box<dyn Fn() -> bool + Send + Sync>);
static PINS: std::sync::Mutex<Vec<Pin>> = std::sync::Mutex::new(Vec::new());

/// Keep a root protected from pruning until the final token clone is dropped.
pub fn retain_root(path: &Path) -> std::sync::Arc<()> {
    let token = std::sync::Arc::new(());
    let weak = std::sync::Arc::downgrade(&token);
    register(path, Box::new(move || weak.strong_count() != 0));
    token
}
pub(super) fn retain_source(path: &Path, source: &ResourceSource) {
    register(path, Box::new(source.liveness()));
}

/// Extend a pinned root's life through the handoff to its readers.
pub(super) fn renew_root(path: &Path) {
    if fs::write(used_path(path), b"").is_err() {
        // A cache that could be read but not marked used still works for this process.
        let renewed = std::time::Instant::now();
        register(path, Box::new(move || renewed.elapsed() <= KEEP_ROOTS));
    }
}

fn register(path: &Path, alive: Box<dyn Fn() -> bool + Send + Sync>) {
    PINS.lock().unwrap().push((path.to_owned(), alive));
}
pub(super) struct StagingRoot(pub(super) PathBuf);
impl Drop for StagingRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Remove stale, unretained roots other than the current candidate.
/// Game tokens protect the active root through preparation failures; resource
/// source observers protect older generations for as long as readers own them.
pub(super) fn prune_roots(roots: &Path, current: &Path, keep: std::time::Duration) {
    let Ok(entries) = fs::read_dir(roots) else {
        return;
    };
    let old_layout = roots.join(STAMP).exists();
    for entry in entries.flatten() {
        let path = entry.path();
        if path == current {
            continue;
        }
        let doomed = match retire_root(&path, old_layout, keep) {
            Ok(Some(doomed)) => doomed,
            Ok(None) => continue,
            Err(e) => {
                log::warn!(
                    "Horizon: an old mod content root stays: {}: {e}",
                    path.display()
                );
                continue;
            }
        };
        let removed = if doomed.is_dir() {
            fs::remove_dir_all(&doomed)
        } else {
            fs::remove_file(&doomed)
        };
        if let Err(e) = removed
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!(
                "Horizon: an old mod content root stays: {}: {e}",
                doomed.display()
            );
        }
    }
}

const PRUNING: &str = ".pruning-";

fn is_root_name(name: &str) -> bool {
    name.len() == ROOT_NAME && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Checks retention and age, then removes the loadable name while still holding the pin lock.
/// The worker can delete the renamed directory without blocking new loads.
fn retire_root(
    path: &Path,
    old_layout: bool,
    keep: std::time::Duration,
) -> std::io::Result<Option<PathBuf>> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let mut pins = PINS.lock().unwrap();
    pins.retain(|(_, alive)| alive());
    if pins.iter().any(|(root, _)| root == path) {
        return Ok(None);
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if let Some((root, id)) = name.split_once(PRUNING)
        && is_root_name(root)
        && id.split_once('-').is_some_and(|(pid, serial)| {
            pid.parse::<u32>().is_ok() && serial.parse::<u64>().is_ok()
        })
    {
        // Retry a retirement interrupted after the rename.
        return Ok(Some(path.to_owned()));
    }
    if !is_root_name(&name) {
        // Leave last-use markers to their root's retirement, even during legacy cleanup.
        let used = name.strip_suffix(".used").is_some_and(is_root_name);
        return Ok((old_layout && !used).then(|| path.to_owned()));
    }
    if !path.is_dir() {
        return Ok(None);
    }
    let age = |file: &Path| {
        fs::metadata(file)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
    };
    // Prefer the newest of the generation's creation or last use. Unstamped
    // generations can be left by failed builds.
    let used = used_path(path);
    let stale = [age(&path.join(STAMP)), age(&used)]
        .into_iter()
        .flatten()
        .min()
        .or_else(|| age(path))
        .is_none_or(|age| age > keep);
    if !stale {
        return Ok(None);
    }
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let doomed = path.with_file_name(format!("{name}{PRUNING}{}-{serial}", std::process::id()));
    fs::rename(path, &doomed)?;
    let _ = fs::remove_file(used);
    Ok(Some(doomed))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_and_retained_readers_protect_stale_roots_until_both_release() {
        let scratch = tempfile::tempdir().unwrap();
        let old = scratch.path().join("0123456789abcdef");
        fs::create_dir(&old).unwrap();
        fs::write(old.join(STAMP), b"old").unwrap();
        let token = retain_root(&old);
        let source = ResourceSource::directory(old.clone());
        retain_source(&old, &source);
        let current = scratch.path().join("fedcba9876543210");
        prune_roots(scratch.path(), &current, std::time::Duration::ZERO);
        assert!(old.is_dir());
        drop(token);
        prune_roots(scratch.path(), &current, std::time::Duration::ZERO);
        assert!(old.is_dir());
        drop(source);
        prune_roots(scratch.path(), &current, std::time::Duration::ZERO);
        assert!(!old.exists());
    }

    #[test]
    fn an_admitted_prune_cannot_delete_the_root_a_reload_builds() {
        use std::sync::mpsc::channel;
        use std::time::{Duration, SystemTime};

        let scratch = tempfile::tempdir().unwrap();
        let base = scratch.path().join("base");
        let roots = scratch.path().join("mod-content");
        let root = roots.join("0123456789abcdef");
        fs::create_dir_all(base.join("assets")).unwrap();
        fs::write(base.join("assets/unchanged"), b"base").unwrap();
        let overlay = Overlay::new(&base, &root).unwrap();
        #[cfg(windows)]
        assert!(
            fs::symlink_metadata(root.join("assets"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        overlay.write(STAMP, b"old").unwrap();
        fs::File::options()
            .write(true)
            .open(root.join(STAMP))
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(7200))
            .unwrap();
        let keep = Duration::from_secs(3600);
        let pin = retain_root(&root);
        assert!(retire_root(&root, false, keep).unwrap().is_none());
        drop(pin);

        let (retired_tx, retired_rx) = channel();
        let (reloaded_tx, reloaded_rx) = channel();
        std::thread::scope(|scope| {
            let reloaded_tx = reloaded_tx;
            let root = &root;
            scope.spawn(move || {
                let retired = retire_root(root, false, keep).unwrap().unwrap();
                retired_tx.send(()).unwrap();
                // Wait until a reload uses this same cache name before deleting the old overlay.
                if reloaded_rx.recv().is_ok() {
                    fs::remove_dir_all(retired).unwrap();
                }
            });
            retired_rx.recv().unwrap();
            let _reload = retain_root(root);
            assert!(!root.join(STAMP).exists());
            let overlay = Overlay::new(&base, root).unwrap();
            overlay.write(STAMP, b"new").unwrap();
            reloaded_tx.send(()).unwrap();
        });
        assert_eq!(fs::read(root.join(STAMP)).unwrap(), b"new");
        assert_eq!(fs::read(root.join("assets/unchanged")).unwrap(), b"base");
        assert_eq!(fs::read(base.join("assets/unchanged")).unwrap(), b"base");
    }

    #[test]
    fn unsuccessful_staging_is_removed_but_previous_root_survives() {
        let scratch = tempfile::tempdir().unwrap();
        let old = scratch.path().join("old");
        fs::create_dir(&old).unwrap();
        let stage = scratch.path().join("candidate-tables");
        let failure = || -> Result<()> {
            let _stage = StagingRoot(stage.clone());
            fs::create_dir(&stage)?;
            fs::write(stage.join("partial.json"), b"{}")?;
            Err("handler refused".into())
        };
        assert!(failure().is_err());
        assert!(!stage.exists());
        assert!(old.exists());
    }
}
