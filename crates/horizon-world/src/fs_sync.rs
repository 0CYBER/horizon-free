//! Making a rename durable: the folder that holds the renamed file synced.
use std::io;
use std::path::Path;

/// `folder` synced, so the renames into it are on the disk before what
/// depends on them is written. A no-op on Windows: a directory cannot be
/// opened and synced there the way POSIX allows, so a rename there stays
/// only as durable as NTFS's journal makes it.
pub fn sync_dir(folder: &Path) -> io::Result<()> {
    // A bare file name's parent is "": the working directory.
    let folder = if folder.as_os_str().is_empty() {
        Path::new(".")
    } else {
        folder
    };
    #[cfg(unix)]
    match std::fs::File::open(folder)?.sync_all() {
        // Some filesystems (network and FUSE ones among them) refuse to sync
        // a directory. A rename there is as durable as they make it; failing
        // every upgrade and new world over it would be worse.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
            ) => {}
        result => result?,
    }
    #[cfg(not(unix))]
    let _ = folder;
    Ok(())
}
