//! Only known local lock implementations are accepted; remote/process-owned emulations are not.
use super::Error;
use cap_std::fs::Dir;

pub(super) fn local(directory: &Dir) -> Result<(), Error> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::{mem::MaybeUninit, os::fd::AsRawFd};
        let mut status = MaybeUninit::<libc::statfs>::zeroed();
        // SAFETY: directory owns the fd and status provides writable statfs storage.
        if unsafe { libc::fstatfs(directory.as_raw_fd(), status.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: a successful fstatfs initialized the result.
        let status = unsafe { status.assume_init() };
        #[cfg(target_os = "linux")]
        // ext*, tmpfs, overlayfs, XFS, Btrfs, ZFS, ramfs, F2FS, FAT and exFAT. Unknown
        // implementations (including NFS, SMB and FUSE) must not silently inherit trust.
        let supported = supported_linux(status.f_type as u64);
        #[cfg(target_os = "macos")]
        let supported = status.f_flags & libc::MNT_LOCAL as u32 != 0 && {
            // SAFETY: the kernel's fixed f_fstypename is NUL-terminated.
            let kind = unsafe { std::ffi::CStr::from_ptr(status.f_fstypename.as_ptr()) };
            matches!(kind.to_bytes(), b"apfs" | b"hfs" | b"msdos" | b"exfat")
        };
        if supported {
            Ok(())
        } else {
            Err(Error::UnsupportedFilesystem)
        }
    }
    #[cfg(windows)]
    {
        use std::{
            os::windows::{
                ffi::{OsStrExt, OsStringExt},
                io::AsRawHandle,
            },
            path::PathBuf,
        };
        use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetFinalPathNameByHandleW};
        let mut name = vec![0u16; 256];
        loop {
            // SAFETY: the directory handle is owned and name is writable for this length.
            let length = unsafe {
                GetFinalPathNameByHandleW(
                    directory.as_raw_handle(),
                    name.as_mut_ptr(),
                    name.len() as u32,
                    0,
                )
            };
            if length == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            if length as usize >= name.len() {
                name.resize(length as usize + 1, 0);
                continue;
            }
            name.truncate(length as usize);
            break;
        }
        let path = PathBuf::from(std::ffi::OsString::from_wide(&name));
        let root = path
            .ancestors()
            .last()
            .ok_or(Error::UnsupportedFilesystem)?;
        let root: Vec<_> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: root is a NUL-terminated volume path from the owned directory handle.
        if matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3 | 5 | 6) {
            Ok(())
        } else {
            Err(Error::UnsupportedFilesystem)
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = directory;
        Err(Error::UnsupportedFilesystem)
    }
}

#[cfg(target_os = "linux")]
fn supported_linux(kind: u64) -> bool {
    matches!(
        kind,
        0xef53
            | 0x01021994
            | 0x794c7630
            | 0x58465342
            | 0x9123683e
            | 0x2fc12fc1
            | 0x858458f6
            | 0xf2f52010
            | 0x4d44
            | 0x2011bab0
    )
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn local_filesystem_contract() {
    for kind in [0xef53, 0x01021994, 0x794c7630, 0x58465342, 0x9123683e] {
        assert!(supported_linux(kind));
    }
    for kind in [
        0x6969, 0xff534d42, 0x517b, 0x65735546, 0x01021997, 0x9fa0, 0,
    ] {
        assert!(!supported_linux(kind), "unsafe filesystem {kind:x}");
    }
}
