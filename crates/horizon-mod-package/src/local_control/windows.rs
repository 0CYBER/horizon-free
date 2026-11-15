use super::{Marker, check_deadline, pause};
use sha2::{Digest, Sha256};
use std::ptr::{null, null_mut};
use std::{
    ffi::c_void,
    fs::File,
    io::{self, Read, Write},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::{Pipes::*, Threading::*},
};

struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: each Local exclusively owns a buffer allocated by the LocalAlloc-family APIs.
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn current_user_acl() -> io::Result<Local> {
    let mut token = null_mut();
    // SAFETY: pseudo-handle is valid and token points to a writable output slot.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcessToken returned a new valid, exclusively owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut size = 0;
    // SAFETY: null zero-length buffer requests the required size, written to size.
    unsafe {
        GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut size);
    }
    let mut storage = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    // SAFETY: aligned storage covers size bytes and the token remains owned during the call.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            storage.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: TokenUser initialized aligned storage; SID pointers stay inside its lifetime.
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    let mut string = null_mut();
    // SAFETY: the SID lies in live TokenUser storage; string is a writable output slot.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut string) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let string_owner = Local(string.cast());
    let mut length = 0;
    // SAFETY: the API returned a null-terminated allocation retained by string_owner.
    while unsafe { *string.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: length was bounded by the allocation's terminator, and string_owner is live.
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(string, length) })
        .map_err(|_| io::ErrorKind::InvalidData)?;
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})")
        .encode_utf16()
        .chain([0])
        .collect();
    let mut descriptor = null_mut();
    // SAFETY: sddl is null-terminated and descriptor is a writable output pointer.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    drop(string_owner);
    Ok(Local(descriptor))
}
fn name(marker: &Marker) -> Vec<u16> {
    let bytes: Vec<u8> = marker
        .directory
        .as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect();
    format!(r"\\.\pipe\horizon-runtime-{:x}", Sha256::digest(bytes))
        .encode_utf16()
        .chain([0])
        .collect()
}
pub struct Stream(File);
impl Stream {
    pub fn set_nonblocking(&self, enabled: bool) -> io::Result<()> {
        let mode = PIPE_READMODE_BYTE | if enabled { PIPE_NOWAIT } else { PIPE_WAIT };
        // SAFETY: the owned pipe handle and mode pointer remain valid throughout the call.
        if unsafe { SetNamedPipeHandleState(self.0.as_raw_handle(), &mode, null(), null()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
impl Read for Stream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let mut count = 0;
        // SAFETY: synchronous handle, writable byte/count buffers, and no overlapped operation.
        let ok = unsafe {
            ReadFile(
                self.0.as_raw_handle(),
                bytes.as_mut_ptr(),
                bytes.len().min(u32::MAX as usize) as u32,
                &mut count,
                null_mut(),
            )
        };
        if ok != 0 {
            return if count == 0 && !bytes.is_empty() {
                Err(io::ErrorKind::WouldBlock.into())
            } else {
                Ok(count as usize)
            };
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(code) if code == ERROR_NO_DATA as i32 => Err(io::ErrorKind::WouldBlock.into()),
            Some(code) if code == ERROR_BROKEN_PIPE as i32 => Ok(0),
            _ => Err(error),
        }
    }
}
impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self.0.write(bytes) {
            Ok(0) if !bytes.is_empty() => Err(io::ErrorKind::WouldBlock.into()),
            result => result.map_err(|error| {
                if error.raw_os_error() == Some(ERROR_NO_DATA as i32) {
                    io::ErrorKind::WouldBlock.into()
                } else {
                    error
                }
            }),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub struct Endpoint(Option<Stream>);
impl Endpoint {
    pub fn bind(marker: &Marker) -> io::Result<Self> {
        let descriptor = current_user_acl()?;
        let security = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        // One first-instance pipe, current SID only, non-inheritable, no remote clients.
        // SAFETY: live null-terminated name/security descriptor; API returns a new owned handle.
        let handle = unsafe {
            CreateNamedPipeW(
                name(marker).as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65536,
                65536,
                0,
                &security,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful CreateNamedPipeW returned an exclusively owned valid handle.
        Ok(Self(Some(Stream(unsafe { File::from_raw_handle(handle) }))))
    }
    pub fn accept(&mut self, deadline: Instant) -> io::Result<Stream> {
        let stream = self.0.take().ok_or(io::ErrorKind::NotConnected)?;
        loop {
            check_deadline(deadline)?;
            // NOWAIT can report a listening instance as successful: confirm an actual peer.
            // SAFETY: the owned NOWAIT pipe does not use an overlapped structure.
            unsafe {
                ConnectNamedPipe(stream.0.as_raw_handle(), null_mut());
            }
            let mut peer = 0;
            // SAFETY: the owned pipe is live and peer is a writable output slot.
            if unsafe { GetNamedPipeClientProcessId(stream.0.as_raw_handle(), &mut peer) } != 0 {
                return Ok(stream);
            }
            let error = io::Error::last_os_error();
            if !matches!(error.raw_os_error(), Some(code) if code == ERROR_PIPE_LISTENING as i32 || code == ERROR_PIPE_NOT_CONNECTED as i32 || code == ERROR_NOT_FOUND as i32)
            {
                return Err(error);
            }
            pause(deadline)?;
        }
    }
}
#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
pub fn connect(marker: &Marker, deadline: Instant) -> io::Result<Stream> {
    loop {
        check_deadline(deadline)?;
        // SAFETY: name is null-terminated; all optional pointers are null for this synchronous open.
        let handle = unsafe {
            CreateFileW(
                name(marker).as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            // SAFETY: successful CreateFileW returned a new exclusively owned valid handle.
            let stream = Stream(unsafe { File::from_raw_handle(handle) });
            stream.set_nonblocking(true)?;
            return Ok(stream);
        }
        let error = io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(code) if code == ERROR_FILE_NOT_FOUND as i32 || code == ERROR_PIPE_BUSY as i32)
        {
            return Err(error);
        }
        pause(deadline)?;
    }
}
