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
fn register(path: &Path, alive: Box<dyn Fn() -> bool + Send + Sync>) {
    PINS.lock().unwrap().push((path.to_owned(), alive));
}
fn protected(path: &Path) -> bool {
    let mut pins = PINS.lock().unwrap();
    pins.retain(|(_, alive)| alive());
    pins.iter().any(|(root, _)| root == path)
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
        if path == current || protected(&path) {
            continue;
        }
        let name = entry.file_name();
        let is_root = name.len() == ROOT_NAME
            && name
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            && path.is_dir();
        let age = |file: PathBuf| {
            fs::metadata(file)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
        };
        let stale = match is_root {
            // Its build (the stamp) or its last reuse, whichever is newer.
            true => [age(path.join(STAMP)), age(used_path(&path))]
                .into_iter()
                .flatten()
                .min()
                .is_none_or(|age| age > keep),
            // The old single root's files and folders.
            false => old_layout,
        };
        if !stale {
            continue;
        }
        let removed = match path.is_dir() {
            true => fs::remove_dir_all(&path).inspect(|_| {
                let _ = fs::remove_file(used_path(&path));
            }),
            false => fs::remove_file(&path),
        };
        if let Err(e) = removed {
            log::warn!(
                "Horizon: an old mod content root stays: {}: {e}",
                path.display()
            );
        }
    }
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
