use std::io;

#[cfg(unix)]
mod platform {
    use super::*;
    use std::{
        fs::File,
        os::fd::{AsRawFd, FromRawFd},
    };
    pub struct Restore {
        saved: File,
        target: i32,
        flags: i32,
    }
    pub fn redirect(target: i32, writer: &os_pipe::PipeWriter) -> io::Result<Restore> {
        // SAFETY: target is a caller-owned stdio slot and writer owns its descriptor.
        let flags = unsafe { libc::fcntl(target, libc::F_GETFD) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        // Keep saved copies above all stdio targets even when another standard slot is closed.
        let fd = unsafe { libc::fcntl(target, libc::F_DUPFD_CLOEXEC, 3) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful fcntl returned a new owned descriptor.
        let original = Restore {
            saved: unsafe { File::from_raw_fd(fd) },
            target,
            flags,
        };
        // SAFETY: both descriptors remain owned through the atomic replacement.
        if unsafe { libc::dup2(writer.as_raw_fd(), target) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(original)
    }
    #[cfg(test)]
    impl Restore {
        pub(super) fn private_copy(&self) -> bool {
            self.saved.as_raw_fd() >= 3
                && unsafe { libc::fcntl(self.saved.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC
                    != 0
        }
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: saved remains owned and target is this guard's standard slot.
            unsafe {
                if libc::dup2(self.saved.as_raw_fd(), self.target) >= 0 {
                    libc::fcntl(self.target, libc::F_SETFD, self.flags);
                }
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::io::{AsRawHandle, IntoRawHandle};
    use windows_sys::Win32::Foundation::SetHandleInformation;
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
    struct Crt(i32);
    impl Drop for Crt {
        fn drop(&mut self) {
            unsafe {
                libc::close(self.0);
            }
        }
    }
    impl Crt {
        fn above_stdio(fd: i32) -> io::Result<Self> {
            let mut low = Vec::new();
            loop {
                // SAFETY: only duplicate a borrowed CRT descriptor; every result is owned.
                let copy = unsafe { libc::dup(fd) };
                if copy < 0 {
                    return Err(io::Error::last_os_error());
                }
                let owned = Self(copy);
                if copy >= 3 {
                    // SAFETY: owned keeps this CRT handle alive; private copies never inherit.
                    let handle = unsafe { libc::get_osfhandle(copy) };
                    if unsafe { SetHandleInformation(handle as _, 1, 0) } == 0 {
                        return Err(io::Error::last_os_error());
                    }
                    return Ok(owned);
                }
                low.push(owned);
            }
        }
    }
    pub struct Restore {
        saved: Crt,
        target: i32,
        standard: u32,
    }
    pub fn redirect(target: i32, writer: &os_pipe::PipeWriter) -> io::Result<Restore> {
        let original = Restore {
            saved: Crt::above_stdio(target)?,
            target,
            standard: if target == 1 {
                STD_OUTPUT_HANDLE
            } else {
                STD_ERROR_HANDLE
            },
        };
        let pipe = writer.try_clone()?;
        // SAFETY: pipe remains owned until the CRT accepts ownership on success.
        let fd = unsafe {
            libc::open_osfhandle(
                pipe.as_raw_handle() as isize,
                libc::O_WRONLY | libc::O_BINARY,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let _ = pipe.into_raw_handle();
        let pipe = Crt(fd);
        // SAFETY: these are owned CRT descriptors, and target is a fixed standard slot.
        unsafe {
            if libc::dup2(pipe.0, target) < 0 {
                return Err(io::Error::last_os_error());
            }
            if SetStdHandle(original.standard, libc::get_osfhandle(target) as _) == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(original)
    }
    #[cfg(test)]
    impl Restore {
        pub(super) fn private_copy(&self) -> bool {
            let mut flags = 0;
            self.saved.0 >= 3
                && unsafe {
                    windows_sys::Win32::Foundation::GetHandleInformation(
                        libc::get_osfhandle(self.saved.0) as _,
                        &mut flags,
                    )
                } != 0
                && flags & 1 == 0
        }
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: restoring an owned saved CRT stream also provides the new Win32 handle.
            unsafe {
                if libc::dup2(self.saved.0, self.target) >= 0 {
                    SetStdHandle(self.standard, libc::get_osfhandle(self.target) as _);
                }
            }
        }
    }
}

pub(super) use platform::{Restore, redirect};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "daily"]
    fn saved_copy_never_uses_stdio_or_inherits() {
        if std::env::var_os("HORIZON_CAPTURE_FD_TEST").is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--include-ignored",
                    "--exact",
                    "output::stdio::tests::saved_copy_never_uses_stdio_or_inherits",
                    "--nocapture",
                ])
                .env("HORIZON_CAPTURE_FD_TEST", "1")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let (_reader, writer) = os_pipe::pipe().unwrap();
        unsafe {
            libc::close(0);
        }
        let restore = redirect(1, &writer).unwrap();
        assert!(
            restore.private_copy(),
            "saved copy clobbered a stdio slot or can leak across exec"
        );
    }
}
