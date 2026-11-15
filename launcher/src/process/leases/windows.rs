//! Stable Win32 creation, used only inside the private one-game holder, never the GUI process.
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io, mem,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::*,
    Globalization::CompareStringOrdinal,
    System::{Console::*, Threading::*},
};

/// An owned process object; neither waiting nor termination guesses ownership from a PID.
pub struct WindowsChild(OwnedHandle);
impl WindowsChild {
    /// Wait for the owned process, retaining its full Windows exit code.
    pub fn wait(&self) -> io::Result<std::process::ExitStatus> {
        use std::os::windows::process::ExitStatusExt;
        // SAFETY: this object owns a live process handle; out points to initialized storage.
        unsafe {
            if WaitForSingleObject(self.0.as_raw_handle(), INFINITE) != WAIT_OBJECT_0 {
                return Err(io::Error::last_os_error());
            }
            let mut code = 0;
            if GetExitCodeProcess(self.0.as_raw_handle(), &mut code) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(std::process::ExitStatus::from_raw(code))
        }
    }
}
struct Attributes(Vec<usize>);
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: constructed only after InitializeProcThreadAttributeList succeeds.
        unsafe {
            DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast());
        }
    }
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let value: Vec<_> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(value)
}
fn argument(out: &mut Vec<u16>, value: &[u16]) {
    out.push(34);
    let mut slashes = 0;
    for &c in value {
        if c == 92 {
            slashes += 1;
            continue;
        }
        out.extend(std::iter::repeat_n(
            92,
            if c == 34 { slashes * 2 + 1 } else { slashes },
        ));
        out.push(c);
        slashes = 0;
    }
    out.extend(std::iter::repeat_n(92, slashes * 2));
    out.push(34);
}

pub(super) fn spawn(
    files: &[&File],
    data: &Path,
    program: &Path,
    args: &[OsString],
    environment: &[(OsString, OsString)],
    breakaway: bool,
) -> io::Result<WindowsChild> {
    let mut application = wide(program.as_os_str())?;
    let mut line = Vec::new();
    argument(&mut line, &application);
    application.push(0);
    for arg in args {
        line.push(32);
        argument(&mut line, &wide(arg)?);
    }
    line.push(0);
    let nul = File::options().read(true).write(true).open("NUL")?;
    let mut roots: Vec<_> = files.iter().map(|file| file.as_raw_handle()).collect();
    roots.sort_unstable_by_key(|h| *h as usize);
    roots.dedup();
    // SAFETY: Win32 calls borrow valid stdio/file handles; duplicates are owned immediately.
    unsafe {
        let own = GetCurrentProcess();
        let stdio = [
            GetStdHandle(STD_INPUT_HANDLE),
            GetStdHandle(STD_OUTPUT_HANDLE),
            GetStdHandle(STD_ERROR_HANDLE),
        ];
        let mut inherited = Vec::new();
        for source in stdio.into_iter().chain(roots) {
            let source = if source.is_null() || source == INVALID_HANDLE_VALUE {
                nul.as_raw_handle()
            } else {
                source
            };
            let mut duplicate = ptr::null_mut();
            if DuplicateHandle(
                own,
                source,
                own,
                &mut duplicate,
                0,
                1,
                DUPLICATE_SAME_ACCESS,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            inherited.push(OwnedHandle::from_raw_handle(duplicate));
        }
        let handles: Vec<_> = inherited.iter().map(AsRawHandle::as_raw_handle).collect();
        let numbers = handles[3..]
            .iter()
            .map(|h| (*h as usize).to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut env = Vec::new();
        for (key, value) in environment
            .iter()
            .map(|(k, v)| (k.as_os_str(), v.as_os_str()))
            .filter(|(key, _)| {
                !["HORIZON_RUNTIME_LEASE_HANDLES", "HORIZON_RUNTIME_DATA_ROOT"]
                    .iter()
                    .any(|k| key.to_string_lossy().eq_ignore_ascii_case(k))
            })
            .chain([
                (
                    OsStr::new("HORIZON_RUNTIME_LEASE_HANDLES"),
                    OsStr::new(&numbers),
                ),
                (OsStr::new("HORIZON_RUNTIME_DATA_ROOT"), data.as_os_str()),
            ])
        {
            let mut entry = wide(key)?;
            let start = usize::from(entry.first() == Some(&61)); // Preserve Windows =C: entries.
            if entry.len() == start || entry[start..].contains(&61) {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            let key_length = i32::try_from(entry.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
            entry.push(61);
            entry.extend(wide(value)?);
            entry.push(0);
            env.push((entry, key_length));
        }
        env.sort_unstable_by(|(a, a_length), (b, b_length)| {
            CompareStringOrdinal(a.as_ptr(), *a_length, b.as_ptr(), *b_length, 1).cmp(&2)
        });
        let mut environment: Vec<u16> = env.into_iter().flat_map(|(entry, _)| entry).collect();
        environment.push(0);
        let mut bytes = 0;
        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut bytes);
        let mut storage = vec![0usize; bytes.div_ceil(mem::size_of::<usize>())];
        let list = storage.as_mut_ptr().cast();
        if InitializeProcThreadAttributeList(list, 1, 0, &mut bytes) == 0 {
            return Err(io::Error::last_os_error());
        }
        let _attributes = Attributes(storage);
        if UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_ptr().cast(),
            mem::size_of_val(handles.as_slice()),
            ptr::null_mut(),
            ptr::null(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = handles[0];
        startup.StartupInfo.hStdOutput = handles[1];
        startup.StartupInfo.hStdError = handles[2];
        startup.lpAttributeList = list;
        let mut information = PROCESS_INFORMATION::default();
        let flags = EXTENDED_STARTUPINFO_PRESENT
            | CREATE_UNICODE_ENVIRONMENT
            | CREATE_NO_WINDOW
            | if breakaway {
                CREATE_BREAKAWAY_FROM_JOB
            } else {
                0
            };
        if CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            flags,
            environment.as_ptr().cast(),
            ptr::null(),
            &startup.StartupInfo,
            &mut information,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let process = OwnedHandle::from_raw_handle(information.hProcess);
        drop(OwnedHandle::from_raw_handle(information.hThread));
        // inherited closes without unlock; the game's file objects retain sharing exclusion.
        Ok(WindowsChild(process))
    }
}
