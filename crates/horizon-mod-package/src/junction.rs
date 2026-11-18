//! A directory linked as an NTFS junction, for the mod content root's overlay
//! (`horizon-client/src/mod_content.rs`): a directory of the overlay to the base's directory,
//! where unix links it with one symlink. Without it each of the base's ~28,000 files is
//! hard-linked one by one: 25 to 35 s on Windows (the file system makes one name at a time, and
//! the antivirus looks at each), at every start after a mod or the game changes. A
//! junction needs no privilege, unlike a directory symlink, and reads as a symlink to `std`
//! (`FileType::is_symlink`), as unix's do: the overlay replaces it with a real directory before it
//! writes inside (`Overlay::dir`), and `remove_dir_all` removes it without going in.
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;

/// `IO_REPARSE_TAG_MOUNT_POINT`: a junction.
const MOUNT_POINT: u32 = 0xA000_0003;

fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    text.encode_wide().collect()
}

/// The reparse data of a junction to `target` (an absolute local path, without the `\\?\` form).
fn reparse_data(target: &str) -> Vec<u8> {
    let substitute = wide(std::ffi::OsStr::new(&format!("\\??\\{target}")));
    let print = wide(std::ffi::OsStr::new(target));
    // REPARSE_DATA_BUFFER with a MountPointReparseBuffer: the two names, each ending in a NUL.
    let names_bytes = (substitute.len() + 1 + print.len() + 1) * 2;
    let data_length = 8 + names_bytes;
    let mut buffer: Vec<u8> = Vec::with_capacity(8 + data_length);
    buffer.extend_from_slice(&MOUNT_POINT.to_le_bytes());
    buffer.extend_from_slice(&(data_length as u16).to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&((substitute.len() * 2) as u16).to_le_bytes());
    buffer.extend_from_slice(&(((substitute.len() + 1) * 2) as u16).to_le_bytes());
    buffer.extend_from_slice(&((print.len() * 2) as u16).to_le_bytes());
    for unit in substitute
        .iter()
        .chain([&0])
        .chain(print.iter())
        .chain([&0])
    {
        buffer.extend_from_slice(&unit.to_le_bytes());
    }
    buffer
}

/// Makes `at` (which must not exist) a junction to the directory `target`. On error nothing is
/// left at `at`.
pub fn junction(target: &Path, at: &Path) -> std::io::Result<()> {
    let target = std::path::absolute(target)?;
    let plain = target.as_os_str().to_string_lossy();
    let plain = plain.strip_prefix("\\\\?\\").unwrap_or(&plain).to_owned();
    if plain.starts_with("\\\\") {
        // A share: junctions point only to local volumes.
        return Err(std::io::Error::other(
            "a junction cannot point to a network path",
        ));
    }
    let buffer = reparse_data(&plain);
    if buffer.len() > 16 * 1024 {
        return Err(std::io::Error::other("the junction's target is too long"));
    }
    std::fs::create_dir(at)?;
    let made = (|| {
        let mut name = wide(at.as_os_str());
        name.push(0);
        // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let mut returned = 0u32;
        // SAFETY: `buffer` is a well-formed reparse buffer of the length given; `handle` is open.
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_SET_REPARSE_POINT,
                buffer.as_ptr().cast(),
                buffer.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        let error = (ok == 0).then(std::io::Error::last_os_error);
        // SAFETY: `handle` came from `CreateFileW` above and is closed once.
        unsafe { CloseHandle(handle) };
        error.map_or(Ok(()), Err)
    })();
    if made.is_err() {
        let _ = std::fs::remove_dir(at);
    }
    made
}

/// Removes a junction or a link at `path`, leaving what it points to alone.
pub fn remove_link(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        // A directory reparse point is removed as a directory, its target untouched.
        Ok(meta) if meta.file_type().is_symlink() => {
            std::fs::remove_dir(path).or_else(|_| std::fs::remove_file(path))
        }
        _ => std::fs::remove_file(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_junction_shows_the_directory_and_goes_without_touching_it() {
        let root = std::env::temp_dir().join(format!("horizon-junction-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base = root.join("base");
        std::fs::create_dir_all(base.join("inner")).unwrap();
        std::fs::write(base.join("inner/file.txt"), b"base").unwrap();
        let at = root.join("out");
        junction(&base, &at).unwrap();
        assert_eq!(std::fs::read(at.join("inner/file.txt")).unwrap(), b"base");
        assert!(
            std::fs::symlink_metadata(&at)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        remove_link(&at).unwrap();
        assert!(!at.exists());
        assert_eq!(std::fs::read(base.join("inner/file.txt")).unwrap(), b"base");
        // remove_dir_all of a folder holding a junction leaves the target too.
        std::fs::create_dir(&at).unwrap();
        junction(&base, &at.join("linked")).unwrap();
        std::fs::remove_dir_all(&at).unwrap();
        assert_eq!(std::fs::read(base.join("inner/file.txt")).unwrap(), b"base");
        let _ = std::fs::remove_dir_all(&root);
    }
}
