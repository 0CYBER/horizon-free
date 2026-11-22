//! macOS extended ACL inspection through an open file descriptor.

use std::ffi::{c_int, c_uint, c_void};
use std::io;
use std::os::fd::{AsRawFd as _, BorrowedFd};
use std::ptr;

// Darwin's <sys/acl.h>.
const ACL_TYPE_EXTENDED: c_uint = 0x100;
const ACL_FIRST_ENTRY: c_int = 0;

unsafe extern "C" {
    fn acl_get_fd_np(fd: c_int, acl_type: c_uint) -> *mut c_void;
    fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
    fn acl_free(acl: *mut c_void) -> c_int;
}

/// Check whether the open file or directory has any entries in its extended ACL.
pub fn has_extended_acl_entries(fd: BorrowedFd<'_>) -> io::Result<bool> {
    // SAFETY: The borrowed descriptor stays open throughout the call.
    let acl = unsafe { acl_get_fd_np(fd.as_raw_fd(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        let err = io::Error::last_os_error();
        return match err.raw_os_error() {
            // Darwin reports ENOENT for an absent ACL.
            Some(libc::ENOENT | libc::ENOTSUP | libc::EOPNOTSUPP) => Ok(false),
            _ => Err(err),
        };
    }

    let mut entry = ptr::null_mut();
    // SAFETY: The system returned a live ACL and entry points to writable storage.
    let result = if unsafe { acl_get_entry(acl, ACL_FIRST_ENTRY, &mut entry) } == 0 {
        Ok(true)
    } else {
        let err = io::Error::last_os_error();
        // Darwin reports EINVAL when ACL_FIRST_ENTRY on a valid ACL finds no entry.
        match err.raw_os_error() {
            Some(libc::EINVAL) => Ok(false),
            _ => Err(err),
        }
    };
    // SAFETY: This releases the ACL returned by acl_get_fd_np exactly once.
    unsafe { acl_free(acl) };
    result
}
