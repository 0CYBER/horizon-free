//! Durable file publication, shared by the carrier ownership store (the
//! client's world save) and the server's region WAL.
//!
//! A file's contents are made durable by writing a temporary sibling,
//! `sync_all`ing it, and moving it over the destination with
//! `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)` on
//! Windows — which returns only once the rename metadata is flushed — or
//! `rename` plus an `fsync` of the directory on Unix. Every API failure is
//! an error; nothing silently falls back to a weaker write.
//!
//! The pattern is the server's write-ahead transaction
//! (`horizon-server/src/prune/transaction.rs`); this crate carries it
//! because it is the one place both the client and the server depend on
//! that does not forbid `unsafe`, which the direct Windows call needs.

use std::{fs, io, path::Path};

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
/// `to` may not exist yet.
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
        let to = fs::canonicalize(to.parent().unwrap())?.join(to.file_name().unwrap());
        let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
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
    }
    #[cfg(not(windows))]
    fs::rename(from, to)?;
    sync_dir(to.parent().unwrap())
}

/// Writes `bytes` to `path` so that, once this returns, the file on disk
/// holds exactly these bytes: a temporary sibling, `sync_all`, then a
/// durable [`replace`]. A failed publication removes the temporary.
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("data");
    let temporary = path.with_file_name(format!(".{name}.durable-new"));
    let written = fs::File::create(&temporary).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    match written {
        Ok(()) => match replace(&temporary, path) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                Err(error)
            }
        },
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

/// Removes `path`, then makes the directory entry's removal durable the
/// same way [`write`] makes a replacement durable. A missing file is
/// already gone.
pub fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => sync_dir(path.parent().unwrap_or(path)),
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
}
