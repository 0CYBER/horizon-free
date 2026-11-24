//! Durable file publication, shared by the carrier ownership store (the
//! client's world save) and the server's region WAL.
//!
//! A file's contents are made durable by writing a temporary sibling,
//! `sync_all`ing it, and moving it over the destination with
//! `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)` on
//! Windows — which returns only once the rename metadata is flushed, and is
//! made again while another handle holds the file for a moment — or
//! `rename` plus an `fsync` of the directory on Unix. Every API failure is
//! an error; nothing silently falls back to a weaker write.
//!
//! The pattern is the server's write-ahead transaction
//! (`horizon-server/src/prune/transaction.rs`); this crate carries it
//! because it is the one place both the client and the server depend on
//! that does not forbid `unsafe`, which the direct Windows call needs.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// The directory of `path` synced, so a rename into it is on the disk
/// (POSIX; Windows has no directory handle to sync, and its write-through
/// move is journaled by NTFS).
fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Moves `from` onto `to` durably. Both must live in the same directory;
/// `to` may not exist yet. On Windows, a move refused while another handle
/// holds either file for a moment is made again ([`retry_busy`]).
pub fn replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        // The launcher's Windows suite compiles this file under edition 2021, which sorts
        // these names differently from the 2024 crates: keep one order for both.
        #[rustfmt::skip]
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        // std adds the extended-length prefix for long paths; retain that
        // behavior when calling Win32 directly, including a destination
        // that does not exist yet.
        let from = fs::canonicalize(from)?;
        let to = fs::canonicalize(parent_dir(to))?.join(to.file_name().unwrap());
        let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        retry_busy(&BUSY_WAITS_MS, || {
            // Both paths are owned, nul-terminated buffers valid for this
            // synchronous call.
            if unsafe {
                MoveFileExW(
                    from.as_ptr(),
                    to.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })?;
    }
    #[cfg(not(windows))]
    fs::rename(from, to)?;
    sync_dir(parent_dir(to))
}

/// How long [`retry_busy`] waits before each new try, in milliseconds: about 2 s in all.
#[cfg(windows)]
const BUSY_WAITS_MS: [u64; 11] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024];

/// `attempt`, made again after each of `waits_ms` while Windows refuses it because another handle
/// holds one of the files: access denied (OS error 5) while the file being replaced is open, a
/// sharing violation (OS error 32) while the one being moved is open without sharing its deletion.
/// Another save of the same file, or a scanner or indexer reading one just written, lets go within
/// moments. Once the waits are spent, the refusal is the error, so a file that stays refused (a
/// read-only one, also access denied) still fails; any other error is returned at once.
#[cfg(windows)]
fn retry_busy(waits_ms: &[u64], mut attempt: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut waits = waits_ms.iter();
    loop {
        match attempt() {
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) => match waits.next() {
                Some(&ms) => std::thread::sleep(std::time::Duration::from_millis(ms)),
                None => return Err(error),
            },
            done => return done,
        }
    }
}

/// The directory `path` is in: `.` for a bare file name, whose `parent` is empty (and an empty path
/// opens and canonicalizes nowhere).
fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Writes `bytes` to `path` so that, once this returns, the file on disk
/// holds exactly these bytes: a temporary sibling made for this call alone,
/// `sync_all`, then a durable [`replace`]. A path that is a link is written
/// through (the link stays, its target is replaced). A file that is replaced
/// keeps its permissions and, on Unix, its owner and group; a file that
/// cannot keep them is not replaced, and the error says so. A failed
/// publication removes the temporary.
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    // Through a link to the file it names, so the link is not replaced by a plain file. A link to
    // nothing is replaced itself.
    let resolved;
    let path = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => match fs::canonicalize(path) {
            Ok(target) => {
                resolved = target;
                resolved.as_path()
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => path,
            Err(error) => return Err(error),
        },
        _ => path,
    };
    // A missing file is a new one; failing to look at one is an error, not a new file.
    let old = match fs::metadata(path) {
        Ok(meta) => Some(meta),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    // With a file to keep the permissions of, the temporary one is owner-only until it has them, so it
    // is never more readable than what it replaces. A new file gets the default mode.
    let (temporary, mut file) = create_sibling(path, NEW, old.is_some())?;
    let written = (|| {
        file.write_all(bytes)?;
        if let Some(old) = &old {
            keep_owner(&file, old, path)?;
            // The file keeps the permissions it had, as an in-place write would.
            file.set_permissions(old.permissions())?;
        }
        file.sync_all()
    })();
    drop(file);
    match written.and_then(|()| replace(&temporary, path)) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

/// The suffixes of [`write`]'s temporary file and of [`write_in_place`]'s complete copy.
const NEW: &str = "durable-new";
const KEPT: &str = "durable-kept";

/// A sibling of `path` with a name of its own (`.NAME.PID.COUNTER.KIND`), made exclusively: another
/// save of this file, or something left at a name, is never written to, published or removed by
/// this one. `private` makes it owner-only on Unix.
fn create_sibling(path: &Path, kind: &str, private: bool) -> io::Result<(PathBuf, fs::File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static ATTEMPTS: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("data");
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut tries = 0;
    loop {
        let sibling = path.with_file_name(format!(
            ".{name}.{}.{}.{kind}",
            std::process::id(),
            ATTEMPTS.fetch_add(1, Ordering::Relaxed)
        ));
        match options.open(&sibling) {
            Ok(file) => return Ok((sibling, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && tries < 16 => tries += 1,
            Err(error) => return Err(error),
        }
    }
}

/// Held while a list or cache file is saved or loaded: an exclusive lock on its sibling
/// `.NAME.lock` (an advisory `flock` on Unix, `LockFileEx` on Windows), so the server and the
/// `horizon-server` CLI never read a file another of them is rewriting, nor remove the copy of a save
/// in progress. Released when dropped.
///
/// Unlocked, as before, only where no copy can have been made: a copy is only ever made under the
/// lock, after the lock file was made, and the lock file is never removed, so a folder where it does
/// not exist and cannot be made (closed to new files, a read-only file system), or a platform without
/// file locks, holds none. A lock file that exists but cannot be opened or locked is an error: a save
/// of another user or process may have left a copy there that an unlocked save would misread.
pub(crate) struct ListLock {
    held: Option<fs::File>,
}

impl ListLock {
    /// Whether this process holds the lock; without it, a save or load touches no copy.
    pub(crate) fn held(&self) -> bool {
        self.held.is_some()
    }
}

/// [`ListLock`] on `path`, waiting for whoever holds it.
pub(crate) fn lock(path: &Path) -> io::Result<ListLock> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("data");
    let at = path.with_file_name(format!(".{name}.lock"));
    let closed = |error: &io::Error| {
        matches!(
            error.kind(),
            io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
        )
    };
    let file = match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&at)
    {
        Ok(file) => file,
        // A lock file this process may not write (another user's) still takes a lock read-only.
        Err(error) if closed(&error) => match fs::File::open(&at) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ListLock { held: None });
            }
            Err(error) => return Err(error),
        },
        Err(error) => return Err(error),
    };
    match file.lock() {
        Ok(()) => Ok(ListLock { held: Some(file) }),
        Err(error) if error.kind() == io::ErrorKind::Unsupported => Ok(ListLock { held: None }),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("{} could not be locked: {error}", at.display()),
        )),
    }
}

/// [`write`] for a file whose access controls must stay as they are (the server's lists and caches),
/// under [`lock`]: an existing file is written in place, the same file, so its owner, its Windows or
/// Unix access list and any link to it are untouched, as `fs::write` would leave them; only a file
/// that does not exist yet is created whole through [`write`].
///
/// With the lock held and a copy beside the file, an earlier save cut off while it rewrote the file is
/// finished first
/// ([`cut_off_copy`], with `valid` the load's own check of a whole file); a failure to finish it fails
/// this save. Then the file is only touched once [`stage_copy`] has made this save's copy and removed
/// the copies of earlier saves; the copy is removed once the file holds the bytes. A save cut off
/// while the file is rewritten so leaves exactly one copy, its own, for [`kept_copies`]. Without the
/// lock (see [`ListLock`]) the file is rewritten as before, and no copy is made or removed.
pub fn write_in_place(path: &Path, bytes: &[u8], valid: impl Fn(&[u8]) -> bool) -> io::Result<()> {
    let lock = lock(path)?;
    let file = match fs::OpenOptions::new().write(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return write(path, bytes),
        Err(error) => return Err(error),
    };
    let copy = if lock.held() {
        // The file is only read when there is a copy to compare it with, so a list the server may
        // write but not read is saved as on main. A folder whose copies cannot be listed, a copy that
        // cannot be read, or, with a copy beside it, a file that cannot be read fails the save, with
        // the file and the copies untouched: whether a copy is the only complete list cannot be
        // told, and staging this save would remove it.
        let copies = kept_copies(path)?;
        if !copies.is_empty() {
            let raw = fs::read(path)?;
            // A file that already holds all of a copy's bytes needs nothing finished.
            let cut = cut_off_copy(&raw, &copies, valid)?
                .filter(|(_, saved)| trimmed(&raw) != saved.as_slice());
            if let Some((cut, saved)) = cut {
                finish_cut_off_save(path, &cut, &saved)?;
            }
        }
        stage_copy(path, bytes, copies)?
    } else {
        None
    };
    // From here a failure keeps the copy: the file may be cut short.
    rewrite(file, bytes)?;
    if let Some(copy) = copy {
        // The file is whole; a copy that stays holds the same bytes and goes at the next load or save.
        let _ = remove(&copy);
    }
    Ok(())
}

/// The durable, owner-only copy of `bytes` [`write_in_place`] writes before it touches `path`, with
/// every `older` copy (all of them, listed under the same lock) removed; when they cannot be removed, the error, and `path` is not touched.
/// `None` in a folder closed to new files that holds no older copy, and on Windows, where a new file
/// takes the folder's inherited access list, which may be wider than the list's own: there older
/// copies are only removed, and a cut-off save is not recovered.
fn stage_copy(path: &Path, bytes: &[u8], older: Vec<PathBuf>) -> io::Result<Option<PathBuf>> {
    use std::io::Write;
    if !cfg!(unix) {
        older.iter().try_for_each(|older| remove(older))?;
        return Ok(None);
    }
    let (copy, mut kept) = match create_sibling(path, KEPT, true) {
        Ok(made) => made,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied && older.is_empty() => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let staged = kept
        .write_all(bytes)
        .and_then(|()| kept.sync_all())
        .and_then(|()| sync_dir(parent_dir(path)))
        .and_then(|()| older.iter().try_for_each(|older| remove(older)));
    drop(kept);
    match staged {
        Ok(()) => Ok(Some(copy)),
        Err(error) => {
            let _ = remove(&copy);
            Err(error)
        }
    }
}

/// `file` emptied and given `bytes`, synced.
fn rewrite(mut file: fs::File, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    file.set_len(0)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// `raw` without the zero bytes a crash may leave after what was written.
fn trimmed(raw: &[u8]) -> &[u8] {
    &raw[..raw.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1)]
}

/// The copy, and its bytes, of the save that cut off the file now holding `raw`: of `copies` (newest
/// first), the first whose bytes start with the file's and that `valid` takes whole, every entry
/// included. Every copy is looked at, so a copy a later save made before it could remove this one
/// does not hide it.
/// A copy that cannot be read is an error, not a copy that does not fit: it may be the one.
pub(crate) fn cut_off_copy(
    raw: &[u8],
    copies: &[PathBuf],
    valid: impl Fn(&[u8]) -> bool,
) -> io::Result<Option<(PathBuf, Vec<u8>)>> {
    let start = trimmed(raw);
    for copy in copies {
        // Not `NotFound` itself, which a load takes for a missing list.
        let saved = fs::read(copy)
            .map_err(|error| io::Error::other(format!("{}: {error}", copy.display())))?;
        if saved.starts_with(start) && valid(&saved) {
            return Ok(Some((copy.clone(), saved)));
        }
    }
    Ok(None)
}

/// Finishes the save that `copy` was written for and that was cut off while it rewrote `path`: the
/// file gets the copy's `bytes`, then the copy is removed. Called under [`lock`].
pub(crate) fn finish_cut_off_save(path: &Path, copy: &Path, bytes: &[u8]) -> io::Result<()> {
    rewrite(fs::OpenOptions::new().write(true).open(path)?, bytes)?;
    remove(copy)
}

/// What a crash in the middle of [`write_in_place`] leaves: the complete copy of `bytes`, and `path`
/// holding only their first half.
#[cfg(test)]
pub(crate) fn cut_off_write_in_place(path: &Path, bytes: &[u8]) -> PathBuf {
    use std::io::Write;
    let (copy, mut kept) = create_sibling(path, KEPT, true).unwrap();
    kept.write_all(bytes).unwrap();
    fs::write(path, &bytes[..bytes.len() / 2]).unwrap();
    copy
}

/// The complete copies [`write_in_place`] left beside `path` when a save of it was cut off, newest
/// first. A folder or an entry that cannot be read, or a copy whose time cannot, is an error, never
/// "no copy": the caller then leaves the file and the copies as they are. So is a file name that is
/// not UTF-8, whose copies [`create_sibling`] would name after `data`.
pub(crate) fn kept_copies(path: &Path) -> io::Result<Vec<PathBuf>> {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} has no UTF-8 file name", path.display()),
        ));
    };
    let (prefix, suffix) = (format!(".{name}."), format!(".{KEPT}"));
    // Not `NotFound` itself, which a load takes for a missing list.
    let failed =
        |error: io::Error| io::Error::other(format!("{}: {error}", parent_dir(path).display()));
    let mut copies = Vec::new();
    for entry in fs::read_dir(parent_dir(path)).map_err(failed)? {
        let entry = entry.map_err(failed)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // `.NAME.PID.COUNTER.durable-kept`: the middle is two numbers, so `NAME.bak`'s copies are not
        // taken for `NAME`'s.
        let Some(middle) = name
            .strip_prefix(prefix.as_str())
            .and_then(|rest| rest.strip_suffix(suffix.as_str()))
        else {
            continue;
        };
        let mut parts = middle.split('.');
        let number = |part: Option<&str>| {
            part.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        };
        if number(parts.next()) && number(parts.next()) && parts.next().is_none() {
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .map_err(failed)?;
            copies.push((modified, path.with_file_name(entry.file_name())));
        }
    }
    copies.sort_by_key(|copy| std::cmp::Reverse(copy.0));
    Ok(copies.into_iter().map(|(_, copy)| copy).collect())
}

/// A new, empty scratch folder for a test, made exclusively under a name of its own: nothing that
/// was already there is used or removed.
#[cfg(test)]
pub(crate) fn scratch_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let dir = std::env::temp_dir().join(format!(
            "horizon-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&dir) {
            Ok(()) => return dir,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!("{}: {error}", dir.display()),
        }
    }
}

/// A bare file name in the working directory that nothing else holds: made exclusively, then removed
/// by the test that made it, so the test starts from a missing file it owns the name of.
#[cfg(test)]
pub(crate) fn claim_bare_name(tag: &str) -> String {
    let mut n = 0u32;
    loop {
        let name = format!("{tag}-{}-{n}.json", std::process::id());
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&name)
        {
            Ok(_) => {
                fs::remove_file(&name).unwrap();
                return name;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => n += 1,
            Err(error) => panic!("{name}: {error}"),
        }
    }
}

/// Gives `file`, the staged copy of `old`, the owner and group `old` has, when it has others than the
/// staged file's. A process that may not do that (it is not root) cannot replace the file by renaming
/// without handing it to another owner: that is an error, and the file is left as it is.
#[cfg(unix)]
fn keep_owner(file: &fs::File, old: &fs::Metadata, path: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let staged = file.metadata()?;
    if (staged.uid(), staged.gid()) == (old.uid(), old.gid()) {
        return Ok(());
    }
    std::os::unix::fs::fchown(file, Some(old.uid()), Some(old.gid())).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "{} is owned by another user or group, which a new file made here cannot take over: {error}",
                path.display()
            ),
        )
    })
}

#[cfg(not(unix))]
fn keep_owner(_: &fs::File, _: &fs::Metadata, _: &Path) -> io::Result<()> {
    Ok(())
}

/// Removes `path`, then makes the directory entry's removal durable the
/// same way [`write`] makes a replacement durable. A missing file is
/// already gone.
pub fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => sync_dir(parent_dir(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_whole_and_leaves_no_temporary_file() {
        let name = "durable-whole";
        let dir =
            std::env::temp_dir().join(format!("horizon-private-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("api.json");
        write(&path, b"one").unwrap();
        write(&path, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        remove(&path).unwrap();
        assert!(!path.exists());
        remove(&path).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn keeps_the_old_permissions_and_writes_through_a_link() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::durable::scratch_dir("private-durable-mode");
        let path = dir.join("ops.json");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        write(&path, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // A list reached through a link stays a link: the file it names is the one replaced.
        let real = dir.join("real.json");
        std::fs::write(&real, "old").unwrap();
        let link = dir.join("link.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write(&link, b"through").unwrap();
        let is_link = std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink();
        assert!(is_link);
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "through");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_file_written_in_place_keeps_its_inode_and_a_new_one_is_created() {
        use std::os::unix::fs::MetadataExt;
        let dir = crate::durable::scratch_dir("private-durable-inplace");
        let path = dir.join("ops.json");
        std::fs::write(&path, "old and longer").unwrap();
        let inode = std::fs::metadata(&path).unwrap().ino();
        write_in_place(&path, b"new", |_| true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
        let fresh = dir.join("fresh.json");
        write_in_place(&fresh, b"x", |_| true).unwrap();
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "x");
        // The two files and their lock files, no copy.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 4);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saves_at_once_leave_one_whole_file_and_no_temporary() {
        let dir = crate::durable::scratch_dir("private-durable-race");
        let path = dir.join("bans.json");
        std::fs::write(&path, "old").unwrap();
        std::thread::scope(|scope| {
            for n in 0..8 {
                let path = &path;
                scope.spawn(move || {
                    for round in 0..25 {
                        write(path, format!("writer {n} round {round}").as_bytes()).unwrap();
                    }
                });
            }
        });
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("writer ") && text.contains(" round "),
            "{text}"
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A move Windows refuses while another handle holds the file is made again until it goes
    /// through; one still refused once the waits are spent fails with the refusal, and any other
    /// error at once.
    #[cfg(windows)]
    #[test]
    fn a_busy_move_is_made_again_and_gives_up_after_its_waits() {
        let refused = |code: i32| -> io::Result<()> { Err(io::Error::from_raw_os_error(code)) };
        let mut tries = 0;
        let moved = retry_busy(&[0, 0, 0], || {
            tries += 1;
            match tries {
                1 => refused(5),
                2 => refused(32),
                _ => Ok(()),
            }
        });
        assert!(moved.is_ok(), "{moved:?}");
        assert_eq!(tries, 3);
        tries = 0;
        let error = retry_busy(&[0, 0], || {
            tries += 1;
            refused(5)
        })
        .unwrap_err();
        assert_eq!((error.raw_os_error(), tries), (Some(5), 3));
        tries = 0;
        let error = retry_busy(&[0, 0], || {
            tries += 1;
            refused(2)
        })
        .unwrap_err();
        assert_eq!((error.raw_os_error(), tries), (Some(2), 1));
    }

    /// A bare file name (the lists' defaults, in the working directory) has an empty parent.
    #[test]
    fn a_bare_file_name_is_written_and_replaced_in_the_working_directory() {
        let name = claim_bare_name("durable-bare");
        let path = Path::new(&name);
        struct Cleanup<'a>(&'a Path);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0);
            }
        }
        let _cleanup = Cleanup(path);
        write(path, b"one").unwrap();
        write(path, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "two");
        remove(path).unwrap();
        assert!(!path.exists());
    }
}
