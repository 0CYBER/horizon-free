//! Protect prepared roots while the game or resource readers retain them.
//!
//! In-process pins guard a root against this process's prunes. Games sharing one cache
//! (two clients, or a client and a server, started from the same folder) also hold a shared
//! OS lock on `<root>.lock` beside it; a prune takes it exclusively before retiring a root.
//! The OS drops a dead process's locks, so a crash protects nothing for longer than it ran.
use super::*;
use horizon_assets::resource_source::ResourceSource;
type Pin = (PathBuf, Box<dyn Fn() -> bool + Send + Sync>);
static PINS: std::sync::Mutex<Vec<Pin>> = std::sync::Mutex::new(Vec::new());

/// A root held by this process: other games' prunes leave it while any clone lives.
pub struct RootHold {
    _lock: Option<fs::File>,
}

/// Keep a root protected from pruning until the final token clone is dropped.
/// The shared lock comes first: whoever then examines the root sees it whole or retired.
pub fn retain_root(path: &Path) -> std::sync::Arc<RootHold> {
    let token = std::sync::Arc::new(RootHold {
        _lock: lock_shared(path),
    });
    let weak = std::sync::Arc::downgrade(&token);
    register(path, Box::new(move || weak.strong_count() != 0));
    token
}
/// A reader of a root that protects it for as long as any clone lives. The source owns the
/// lock, so it is released the moment the last reader goes, not at a later sweep: Windows
/// can't remove a folder holding an open file (a cleared cache, a deleted `.horizon-cache`).
pub(super) fn retained_source(path: &Path) -> ResourceSource {
    let source = match lock_shared(path) {
        Some(lock) => ResourceSource::directory_holding(path.to_owned(), Box::new(lock)),
        None => ResourceSource::directory(path.to_owned()),
    };
    register(path, Box::new(source.liveness()));
    source
}

/// The lock file beside a root: outside it, so retiring the root renames no open file.
fn lock_path(root: &Path) -> PathBuf {
    root.with_extension("lock")
}

/// What a prune writes into a lock file whose root it retired, or before it unlinks a lock
/// whose root is gone. Live lock files are empty. A marked file is cleared only by a holder
/// that checked, under the exclusive lock, that it is still the file at the lock's name.
const DEAD: &[u8] = b"retired";

/// Whether `file` is the file now at `name`: an unlinked or replaced one protects nothing,
/// and neither does a link standing at the name.
fn is_current(file: &fs::File, name: &Path) -> bool {
    if !fs::symlink_metadata(name).is_ok_and(|m| m.is_file()) {
        return false;
    }
    let held = file.try_clone().and_then(same_file::Handle::from_file);
    let named = same_file::Handle::from_path(name);
    matches!((held, named), (Ok(held), Ok(named)) if held == named)
}

/// A shared hold on a root's lock file; `None` leaves only the in-process pin.
fn lock_shared(root: &Path) -> Option<fs::File> {
    lock_shared_from(root, None)
}

/// `opened`: a handle to the lock file opened earlier, possibly since unlinked or replaced.
fn lock_shared_from(root: &Path, mut opened: Option<fs::File>) -> Option<fs::File> {
    let name = root.file_name()?.to_str()?;
    if !is_root_name(name) {
        // The base resources, not a generated root: nothing prunes them.
        return None;
    }
    let lock = lock_path(root);
    let failed = |e: std::io::Error| {
        log::warn!(
            "Horizon: other games may prune mod content root {}: {e}",
            root.display()
        );
    };
    for _ in 0..50 {
        let file = match opened.take() {
            Some(file) => Ok(file),
            None => open_lock_file(&lock),
        };
        match file {
            Ok(file) => {
                // Blocks only while a prune holds it exclusively.
                if let Err(e) = file.lock_shared() {
                    failed(e);
                    return None;
                }
                // Unlinked or replaced while this game waited: open the name again. Names
                // change only under the exclusive lock, so a held current file stays current.
                if !is_current(&file, &lock) {
                    continue;
                }
                if file.metadata().is_ok_and(|m| m.len() == 0) {
                    return Some(file);
                }
                // Marked: its root was retired, so whatever now has the root's name was built
                // after, and the caller examines it as any other. Clear the mark under the
                // exclusive lock, only if the file is still the one at the name, then hold again.
                let _ = file.unlock();
                match file.try_lock() {
                    Ok(()) if is_current(&file, &lock) => {
                        let _ = file.set_len(0);
                    }
                    Ok(()) => {}
                    // Another game clearing it, or a prune: wait a moment for either.
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
                }
            }
            // A name whose deletion is still pending can refuse opens (Windows): retry.
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => {
                failed(e);
                return None;
            }
        }
    }
    failed(std::io::Error::other("its lock file stayed busy"));
    None
}

enum Exclusive {
    /// Another game holds the root, or its lock file changed under this prune.
    Held,
    /// Nobody does: the current lock file, held exclusively.
    Free(fs::File),
}

/// Created when missing: a root built before roots had locks is retired only under one too,
/// so a game that starts using it meanwhile finds it gone or marked, never half removed.
fn lock_exclusive(root: &Path) -> std::io::Result<Exclusive> {
    let lock = lock_path(root);
    let file = open_lock_file(&lock)?;
    match file.try_lock() {
        // Another prune may have unlinked it first: then the name is not this file's.
        Ok(()) if is_current(&file, &lock) => Ok(Exclusive::Free(file)),
        Ok(()) | Err(fs::TryLockError::WouldBlock) => Ok(Exclusive::Held),
        Err(fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Opens or creates a lock file, never through a link: a holder may clear it and a prune
/// writes into it, which must not reach whatever file a link at the name points to.
fn open_lock_file(lock: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: the link itself, rejected below.
        options.custom_flags(0x0020_0000);
    }
    let file = options.open(lock)?;
    let meta = file.metadata()?;
    #[cfg(windows)]
    let link = {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT
        meta.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let link = false;
    if !meta.is_file() || link {
        return Err(std::io::Error::other("the lock file is not a regular file"));
    }
    Ok(file)
}

/// Marks a held lock file dead; dropping the handle then releases it. Written over the start,
/// never truncated: a live file is empty and a marked one already holds exactly this.
fn mark_dead(lock: &mut fs::File) {
    use std::io::{Seek, Write};
    let _ = lock.rewind().and_then(|()| lock.write_all(DEAD));
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
    let mut pins = PINS.lock().unwrap();
    pins.retain(|(_, alive)| alive());
    pins.push((path.to_owned(), alive));
}
pub(super) struct StagingRoot(pub(super) PathBuf);
impl Drop for StagingRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// How many unretained roots besides the current one outlive a prune: enough to switch
/// back to the previous mod sets, few enough that repeated starts don't fill the drive.
const MAX_RECENT_ROOTS: usize = 2;
/// How long after its last build or use the cap leaves a root alone.
const CAP_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Remove stale or surplus unretained roots other than the current candidate.
/// Game tokens protect the active root through preparation failures; resource
/// source observers protect older generations for as long as readers own them.
pub(super) fn prune_roots(roots: &Path, current: &Path, keep: std::time::Duration) {
    let Ok(entries) = fs::read_dir(roots) else {
        return;
    };
    let old_layout = roots.join(STAMP).exists();
    // Newest first, so the roots the cap keeps are the most recently built or used. Lock
    // files last, so the locks of roots this prune retires go in the same prune.
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path != current)
        .map(|path| {
            let lock = path.extension().is_some_and(|e| e == "lock");
            (lock, last_use_age(&path), path)
        })
        .collect();
    paths.sort_by_key(|(lock, age, _)| (*lock, age.unwrap_or(std::time::Duration::MAX)));
    let mut recent = MAX_RECENT_ROOTS;
    for (_, _, path) in paths {
        let doomed = match retire_root(&path, old_layout, keep, &mut recent) {
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
const STAGING: &str = "-tables";

fn is_root_name(name: &str) -> bool {
    name.len() == ROOT_NAME && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A directory itself, never a link to one: only those are roots or staging.
fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// How long ago a root was last built or used; failed builds leave unstamped roots.
fn last_use_age(path: &Path) -> Option<std::time::Duration> {
    let age = |file: &Path| {
        fs::metadata(file)
            .and_then(|m| m.modified())
            .ok()
            // A time ahead of the clock (skew) is fresh, not unknown and so stale.
            .map(|t| t.elapsed().unwrap_or_default())
    };
    [age(&path.join(STAMP)), age(&used_path(path))]
        .into_iter()
        .flatten()
        .min()
        .or_else(|| age(path))
}

/// Checks retention and age, then removes the loadable name while still holding the pin lock.
/// The worker can delete the renamed directory without blocking new loads.
/// `recent` counts the fresh unretained roots still allowed to stay.
fn retire_root(
    path: &Path,
    old_layout: bool,
    keep: std::time::Duration,
    recent: &mut usize,
) -> std::io::Result<Option<PathBuf>> {
    let mut pins = PINS.lock().unwrap();
    pins.retain(|(_, alive)| alive());
    if pins.iter().any(|(root, _)| root == path) {
        return Ok(None);
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if let Some((stem, id)) = name.split_once(PRUNING)
        && is_root_name(stem.strip_suffix(STAGING).unwrap_or(stem))
        && id.split_once('-').is_some_and(|(pid, serial)| {
            pid.parse::<u32>().is_ok() && serial.parse::<u64>().is_ok()
        })
    {
        // Retry a retirement interrupted after the rename.
        return Ok(is_real_dir(path).then(|| path.to_owned()));
    }
    if let Some(root) = name.strip_suffix(".lock")
        && is_root_name(root)
    {
        // A lock whose root is gone, unless a game holds it to build that root.
        let root = path.with_file_name(root);
        let gone = || fs::symlink_metadata(&root).is_err();
        if gone()
            && let Exclusive::Free(mut lock) = lock_exclusive(&root)?
            && gone()
        {
            // Marked first; a game that opened it before the unlink finds it no longer current.
            mark_dead(&mut lock);
            fs::remove_file(path)?;
        }
        return Ok(None);
    }
    if let Some(root) = name.strip_suffix(STAGING)
        && is_root_name(root)
    {
        // A build that ended without dropping its staging (a crash, a kill) leaves it
        // behind; a day is far longer than any build still writing it.
        let root = path.with_file_name(root);
        let building = pins.iter().any(|(pinned, _)| *pinned == root);
        let stale = last_use_age(path).is_none_or(|age| age > keep);
        if building || !stale || !is_real_dir(path) {
            return Ok(None);
        }
        // A builder in another game holds its root's lock for the whole build.
        let Exclusive::Free(_lock) = lock_exclusive(&root)? else {
            return Ok(None);
        };
        return aside(path).map(Some);
    }
    if !is_root_name(&name) {
        // Leave last-use markers to their root's retirement, even during legacy cleanup.
        // Unknown entries of the old single-root layout go, whatever they are.
        let used = name.strip_suffix(".used").is_some_and(is_root_name);
        return Ok((old_layout && !used).then(|| path.to_owned()));
    }
    if !is_real_dir(path) {
        return Ok(None);
    }
    let Exclusive::Free(mut lock) = lock_exclusive(path)? else {
        return Ok(None);
    };
    // Read under the lock: a game that renewed the root before it is counted.
    let used = used_path(path);
    let age = last_use_age(path);
    let stale = age.is_none_or(|age| age > keep);
    // The cap leaves a root built or used moments ago to a later prune: file times can't
    // order roots made within one tick of the clock, and a game may be about to hold it.
    if !stale && age.is_some_and(|age| age < CAP_GRACE) {
        return Ok(None);
    }
    if !stale && *recent > 0 {
        *recent -= 1;
        return Ok(None);
    }
    let doomed = aside(path)?;
    let _ = fs::remove_file(used);
    mark_dead(&mut lock);
    Ok(Some(doomed))
}

/// Renames an entry to a name no load looks for, to be deleted outside the pin lock.
fn aside(path: &Path) -> std::io::Result<PathBuf> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let doomed = path.with_file_name(format!("{name}{PRUNING}{}-{serial}", std::process::id()));
    fs::rename(path, &doomed)?;
    Ok(doomed)
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
        let source = retained_source(&old);
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
        assert!(retire_root(&root, false, keep, &mut 0).unwrap().is_none());
        drop(pin);

        let (retired_tx, retired_rx) = channel();
        let (reloaded_tx, reloaded_rx) = channel();
        std::thread::scope(|scope| {
            let reloaded_tx = reloaded_tx;
            let root = &root;
            scope.spawn(move || {
                let retired = retire_root(root, false, keep, &mut 0).unwrap().unwrap();
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
    fn only_the_newest_unretained_roots_outlive_a_prune() {
        use std::time::{Duration, SystemTime};
        let scratch = tempfile::tempdir().unwrap();
        let root = |name: &str, age_s: u64| {
            let dir = scratch.path().join(name);
            fs::create_dir(&dir).unwrap();
            fs::write(dir.join(STAMP), b"x").unwrap();
            fs::write(used_path(&dir), b"").unwrap();
            for file in [dir.join(STAMP), used_path(&dir)] {
                fs::File::options()
                    .write(true)
                    .open(file)
                    .unwrap()
                    .set_modified(SystemTime::now() - Duration::from_secs(age_s))
                    .unwrap();
            }
            dir
        };
        let current = root("00000000000000aa", 500);
        let newest = root("00000000000000bb", 10);
        let newer = root("00000000000000cc", 20);
        let older = root("00000000000000dd", 30);
        let oldest = root("00000000000000ee", 40);
        let held = root("00000000000000ff", 50);
        let _token = retain_root(&held);
        prune_roots(scratch.path(), &current, KEEP_ROOTS);
        for kept in [&current, &newest, &newer, &held] {
            assert!(
                kept.is_dir() && used_path(kept).exists(),
                "{}",
                kept.display()
            );
        }
        for gone in [&older, &oldest] {
            assert!(
                !gone.exists() && !used_path(gone).exists(),
                "{}",
                gone.display()
            );
        }
    }

    fn aged_root(dir: &Path, name: &str, age: std::time::Duration) -> PathBuf {
        let root = dir.join(name);
        fs::create_dir(&root).unwrap();
        fs::write(root.join(STAMP), b"x").unwrap();
        fs::File::options()
            .write(true)
            .open(root.join(STAMP))
            .unwrap()
            .set_modified(std::time::SystemTime::now() - age)
            .unwrap();
        root
    }

    /// Another game's hold is a second open of the lock file. flock (Unix) and LockFileEx
    /// (Windows) lock per open file description or handle, so this process's own prune
    /// conflicts with it exactly as with another process's.
    #[test]
    fn a_root_another_game_holds_outlives_prunes_by_age_and_count() {
        use std::time::Duration;
        let scratch = tempfile::tempdir().unwrap();
        let current = scratch.path().join("00000000000000aa");
        let stale = aged_root(scratch.path(), "00000000000000bb", 2 * KEEP_ROOTS);
        let surplus = aged_root(scratch.path(), "00000000000000cc", Duration::from_secs(300));
        for name in ["00000000000000dd", "00000000000000ee"] {
            aged_root(scratch.path(), name, Duration::from_secs(60));
        }
        let hold = |root: &Path| {
            let file = fs::File::create(lock_path(root)).unwrap();
            file.lock_shared().unwrap();
            file
        };
        let holds = [hold(&stale), hold(&surplus)];
        prune_roots(scratch.path(), &current, KEEP_ROOTS);
        assert!(stale.is_dir() && surplus.is_dir());
        drop(holds);
        prune_roots(scratch.path(), &current, KEEP_ROOTS);
        for gone in [&stale, &surplus] {
            assert!(!gone.exists() && !lock_path(gone).exists());
        }
    }

    #[test]
    fn a_reuse_that_waits_on_a_prune_finds_the_root_gone_and_builds_anew() {
        let scratch = tempfile::tempdir().unwrap();
        let root = aged_root(scratch.path(), "0123456789abcdef", KEEP_ROOTS);
        let pruning = fs::File::create(lock_path(&root)).unwrap();
        pruning.try_lock().unwrap();
        std::thread::scope(|scope| {
            let reuse = scope.spawn(|| {
                let hold = retain_root(&root);
                // What resource_root_with_conditions checks once it holds the root.
                if !root.join(STAMP).exists() {
                    fs::create_dir_all(&root).unwrap();
                    fs::write(root.join(STAMP), b"new").unwrap();
                }
                hold
            });
            let doomed = aside(&root).unwrap();
            let mut pruning = pruning;
            mark_dead(&mut pruning);
            drop(pruning);
            fs::remove_dir_all(doomed).unwrap();
            let _hold = reuse.join().unwrap();
            assert_eq!(fs::read(root.join(STAMP)).unwrap(), b"new");
            assert!(matches!(lock_exclusive(&root).unwrap(), Exclusive::Held));
        });
    }

    #[test]
    fn a_root_rebuilt_under_a_retired_name_is_held_again() {
        let scratch = tempfile::tempdir().unwrap();
        let current = scratch.path().join("00000000000000aa");
        let root = aged_root(scratch.path(), "00000000000000bb", 2 * KEEP_ROOTS);
        fs::File::create(lock_path(&root)).unwrap();
        let doomed = retire_root(&root, false, KEEP_ROOTS, &mut 0)
            .unwrap()
            .unwrap();
        fs::remove_dir_all(doomed).unwrap();
        assert_eq!(fs::read(lock_path(&root)).unwrap(), DEAD);
        // Another game takes the same stamp and builds the root again.
        let other = lock_shared(&root).unwrap();
        aged_root(scratch.path(), "00000000000000bb", 2 * KEEP_ROOTS);
        prune_roots(scratch.path(), &current, KEEP_ROOTS);
        assert!(root.is_dir());
        drop(other);
        prune_roots(scratch.path(), &current, KEEP_ROOTS);
        assert!(!root.exists() && !lock_path(&root).exists());
    }

    /// A second handle stands in for another game, as in the test above: it gets the exclusive
    /// lock only once nothing in this process still holds a shared one.
    #[test]
    fn the_last_hold_of_a_root_releases_its_lock_at_once() {
        let scratch = tempfile::tempdir().unwrap();
        let root = aged_root(scratch.path(), "0123456789abcdef", KEEP_ROOTS);
        let free = || matches!(lock_exclusive(&root).unwrap(), Exclusive::Free(_));
        let token = retain_root(&root);
        let source = retained_source(&root);
        let reader = source.clone();
        drop(token);
        drop(source);
        assert!(!free());
        drop(reader);
        assert!(free());
    }

    /// Games A and B opened a lock file whose root is gone; another game's cleanup marked and
    /// unlinked it before either got the shared lock. A, then B, take it: both must end on the
    /// file at the lock's name, not B on the unlinked one A cleared.
    #[test]
    fn two_games_racing_a_lock_cleanup_hold_the_same_file() {
        let scratch = tempfile::tempdir().unwrap();
        let current = scratch.path().join("00000000000000aa");
        let root = scratch.path().join("00000000000000bb");
        let open = || {
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(lock_path(&root))
                .unwrap()
        };
        let (mut cleanup, opened_a, opened_b) = (open(), open(), open());
        cleanup.try_lock().unwrap();
        mark_dead(&mut cleanup);
        fs::remove_file(lock_path(&root)).unwrap();
        drop(cleanup);
        let a = lock_shared_from(&root, Some(opened_a)).unwrap();
        let b = lock_shared_from(&root, Some(opened_b)).unwrap();
        let named = same_file::Handle::from_path(lock_path(&root)).unwrap();
        for held in [&a, &b] {
            assert_eq!(
                same_file::Handle::from_file(held.try_clone().unwrap()).unwrap(),
                named
            );
        }
        aged_root(scratch.path(), "00000000000000bb", 2 * KEEP_ROOTS);
        drop(a);
        prune_roots(scratch.path(), &current, KEEP_ROOTS);
        assert!(root.is_dir());
        drop(b);
        prune_roots(scratch.path(), &current, KEEP_ROOTS);
        assert!(!root.exists());
    }

    /// A link at a lock's name: no hold or prune opens, clears or marks the file it points to.
    #[cfg(unix)]
    #[test]
    fn a_lock_name_that_is_a_link_leaves_its_target_alone() {
        let scratch = tempfile::tempdir().unwrap();
        let current = scratch.path().join("00000000000000aa");
        let root = aged_root(scratch.path(), "00000000000000bb", 2 * KEEP_ROOTS);
        let sentinel = scratch.path().join("sentinel");
        fs::write(&sentinel, b"keep me").unwrap();
        std::os::unix::fs::symlink(&sentinel, lock_path(&root)).unwrap();
        assert!(lock_shared(&root).is_none());
        assert!(retire_root(&root, false, KEEP_ROOTS, &mut 0).is_err());
        prune_roots(scratch.path(), &current, KEEP_ROOTS);
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep me");
        assert!(root.is_dir());
    }

    /// Roots made within one tick of the file clock have no order the cap could go by.
    #[test]
    fn the_cap_leaves_roots_built_moments_ago() {
        let scratch = tempfile::tempdir().unwrap();
        let current = scratch.path().join("00000000000000aa");
        let fresh: Vec<_> = ["00000000000000b1", "00000000000000b2", "00000000000000b3"]
            .into_iter()
            .map(|name| aged_root(scratch.path(), name, std::time::Duration::ZERO))
            .collect();
        let older = aged_root(scratch.path(), "00000000000000c1", 4 * CAP_GRACE);
        let oldest = aged_root(scratch.path(), "00000000000000c2", 8 * CAP_GRACE);
        let surplus = aged_root(scratch.path(), "00000000000000c3", 16 * CAP_GRACE);
        prune_roots(scratch.path(), &current, KEEP_ROOTS);
        assert!(fresh.iter().all(|root| root.is_dir()));
        assert!(older.is_dir() && oldest.is_dir() && !surplus.exists());
    }

    /// Roots built before locks have no lock file: the prune makes one and retires under it.
    #[test]
    fn a_root_without_a_lock_file_is_retired_under_one() {
        let scratch = tempfile::tempdir().unwrap();
        let root = aged_root(scratch.path(), "00000000000000bb", 2 * KEEP_ROOTS);
        let doomed = retire_root(&root, false, KEEP_ROOTS, &mut 0)
            .unwrap()
            .unwrap();
        assert!(doomed.is_dir() && !root.exists());
        assert_eq!(fs::read(lock_path(&root)).unwrap(), DEAD);
        // A game that comes to the root now holds the marked file only after clearing it.
        let hold = lock_shared(&root).unwrap();
        assert_eq!(hold.metadata().unwrap().len(), 0);
    }

    #[test]
    fn a_root_stamped_ahead_of_the_clock_counts_as_fresh() {
        let scratch = tempfile::tempdir().unwrap();
        let ahead = scratch.path().join("00000000000000bb");
        fs::create_dir(&ahead).unwrap();
        fs::write(ahead.join(STAMP), b"x").unwrap();
        fs::File::options()
            .write(true)
            .open(ahead.join(STAMP))
            .unwrap()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600))
            .unwrap();
        let current = scratch.path().join("00000000000000aa");
        prune_roots(scratch.path(), &current, std::time::Duration::from_secs(60));
        assert!(ahead.is_dir());
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
