//! Windows file publication and removal through open handles.

use std::io;
use std::mem::{offset_of, size_of, size_of_val};
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle};
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_RENAME_INFORMATION, FileRenameInformation, NtSetInformationFile,
};
use windows_sys::Win32::Foundation::{HANDLE, RtlNtStatusToDosError};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_DISPOSITION_INFO, FileDispositionInfo, SetFileInformationByHandle,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

// FILE_RENAME_INFORMATION ends in a variable-length UTF-16 name.
#[repr(C)]
struct RenameBuffer {
    replace: u32,
    root: HANDLE,
    name_bytes: u32,
    name: [u16; 256],
}

const _: () =
    assert!(offset_of!(RenameBuffer, root) == offset_of!(FILE_RENAME_INFORMATION, RootDirectory));
const _: () = assert!(
    offset_of!(RenameBuffer, name_bytes) == offset_of!(FILE_RENAME_INFORMATION, FileNameLength)
);
const _: () =
    assert!(offset_of!(RenameBuffer, name) == offset_of!(FILE_RENAME_INFORMATION, FileName));

/// Rename the file relative to an open destination directory. The source handle
/// needs DELETE access. The destination name must be one file name.
pub fn rename_new(
    file: BorrowedHandle<'_>,
    target: BorrowedHandle<'_>,
    name: &str,
) -> io::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['\\', '/', ':', '\0']) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut info = RenameBuffer {
        replace: 0,
        root: target.as_raw_handle(),
        name_bytes: 0,
        name: [0; 256],
    };
    for (index, col) in name.encode_utf16().enumerate() {
        *info
            .name
            .get_mut(index)
            .ok_or(io::ErrorKind::InvalidInput)? = col;
        info.name_bytes += size_of::<u16>() as u32;
    }
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: both handles and the correctly laid out buffers are live during
    // the synchronous call. The kernel receives a basename relative to target.
    let result = unsafe {
        NtSetInformationFile(
            file.as_raw_handle(),
            &mut status,
            (&raw const info).cast(),
            size_of_val(&info) as u32,
            FileRenameInformation,
        )
    };
    if result < 0 {
        // SAFETY: this maps the status value returned by NtSetInformationFile.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32,
        ));
    }
    Ok(())
}

/// Delete the file or empty directory when its remaining handles close.
pub fn delete_on_close(file: &impl AsHandle) -> io::Result<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the borrow keeps the handle open and the disposition stays live
    // with the size required for this information class.
    let result = unsafe {
        SetFileInformationByHandle(
            file.as_handle().as_raw_handle(),
            FileDispositionInfo,
            (&raw const disposition).cast(),
            size_of_val(&disposition) as u32,
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
