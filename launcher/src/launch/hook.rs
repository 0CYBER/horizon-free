//! The pre-launch command on Windows ([`super::wrap`]), run to its end before the game starts.
//! It runs in a job object, so the start's Stop ends it with everything it started, and so
//! does the launcher's exit, a crash included: the job ends what is in it when its last handle
//! closes. Of its output only the end is kept, for the message when it fails.
use std::collections::VecDeque;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How much of the end of each of the command's streams is kept.
pub(super) const KEPT: usize = 8 * 1024;
/// How many of those last lines the failure message shows.
const SHOWN_LINES: usize = 20;

/// The end of a stream, at most [`KEPT`] bytes.
#[derive(Default)]
pub(super) struct Tail {
    bytes: VecDeque<u8>,
    cut: bool,
}

impl Tail {
    pub(super) fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend(chunk);
        let over = self.bytes.len().saturating_sub(KEPT);
        if over > 0 {
            self.bytes.drain(..over);
            self.cut = true;
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Its last lines as text, without a line whose start was dropped.
    pub(super) fn text(&self) -> String {
        let bytes: Vec<u8> = self.bytes.iter().copied().collect();
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if self.cut {
            let first = text.find('\n').map_or(text.len(), |i| i + 1);
            text.drain(..first);
        }
        let lines: Vec<_> = text.trim().lines().collect();
        lines[lines.len().saturating_sub(SHOWN_LINES)..].join("\n")
    }
}

/// Output readers running now, for the tests.
#[cfg(test)]
pub(super) static READING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A thread reading one of the command's streams.
struct Reader {
    thread: JoinHandle<()>,
    /// The stream's handle, and whether the thread still holds it open: the thread closes it
    /// only while it holds this lock, so [`interrupt`] never acts on a closed handle.
    open: Arc<Mutex<Option<usize>>>,
}

/// The threads that read the command's output.
#[derive(Default)]
struct Readers {
    threads: Vec<Reader>,
    stop: Arc<AtomicBool>,
}

#[cfg(windows)]
fn raw(stream: &impl std::os::windows::io::AsRawHandle) -> usize {
    stream.as_raw_handle() as usize
}

#[cfg(not(windows))]
fn raw<T>(_: &T) -> usize {
    0
}

impl Readers {
    /// Reads `stream` (its handle `handle`) into `tail` on its own thread, until it ends or
    /// [`Readers::finish`].
    fn keep(
        &mut self,
        handle: usize,
        mut stream: impl Read + Send + 'static,
        tail: Arc<Mutex<Tail>>,
    ) {
        let stop = Arc::clone(&self.stop);
        let open = Arc::new(Mutex::new(Some(handle)));
        let closing = Arc::clone(&open);
        #[cfg(test)]
        READING.fetch_add(1, Ordering::SeqCst);
        let thread = std::thread::spawn(move || {
            let mut chunk = [0; 8192];
            while !stop.load(Ordering::Relaxed) {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => tail
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(&chunk[..n]),
                }
            }
            let mut open = closing.lock().unwrap_or_else(|e| e.into_inner());
            *open = None;
            drop(stream);
            #[cfg(test)]
            READING.fetch_sub(1, Ordering::SeqCst);
        });
        self.threads.push(Reader { thread, open });
    }

    fn done(&self) -> bool {
        self.threads.iter().all(|r| r.thread.is_finished())
    }

    /// Lets the readers reach the end of the output for `grace`, then stops them, which closes
    /// the launcher's ends of the pipes: a program the command left running can hold the
    /// output open for as long as it runs. Never waits longer than `grace` and a second.
    fn finish(self, grace: Duration) {
        let until = Instant::now() + grace;
        while !self.done() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.stop.store(true, Ordering::Relaxed);
        let until = Instant::now() + Duration::from_secs(1);
        while !self.done() && Instant::now() < until {
            for reader in &self.threads {
                if let Some(handle) = *reader.open.lock().unwrap_or_else(|e| e.into_inner()) {
                    interrupt(handle);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Ends the read waiting on the open stream `handle`, if any. The standard library reads a
/// child's pipe as overlapped I/O, which only `CancelIoEx` ends.
#[cfg(windows)]
fn interrupt(handle: usize) {
    // SAFETY: the caller holds the lock that keeps `handle` open; with no read waiting, it does
    // nothing.
    unsafe { windows_sys::Win32::System::IO::CancelIoEx(handle as _, std::ptr::null()) };
}

#[cfg(not(windows))]
fn interrupt(_: usize) {}

pub(super) fn stopped() -> String {
    "The pre-launch command was stopped, so the game did not start.".into()
}

/// Runs `pre` to its end: Ok when it succeeds, else why the game does not start. `stop` ends
/// it with what it started.
pub(super) fn run(mut pre: Command, stop: &AtomicBool) -> Result<(), String> {
    if stop.load(Ordering::Relaxed) {
        return Err(stopped());
    }
    pre.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (mut child, tree) =
        tree::spawn(pre).map_err(|e| format!("The pre-launch command didn't run: {e}"))?;
    let (out, err) = (Arc::default(), Arc::default());
    let mut readers = Readers::default();
    if let Some(stream) = child.stdout.take() {
        readers.keep(raw(&stream), stream, Arc::clone(&out));
    }
    if let Some(stream) = child.stderr.take() {
        readers.keep(raw(&stream), stream, Arc::clone(&err));
    }
    let status = loop {
        if stop.load(Ordering::Relaxed) {
            tree.kill(&mut child);
            let _ = child.wait();
            readers.finish(Duration::ZERO);
            return Err(stopped());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                tree.kill(&mut child);
                readers.finish(Duration::ZERO);
                return Err(format!("The pre-launch command didn't run: {e}"));
            }
        }
    };
    // It ended on its own: what it started in the background (`start`) goes on.
    tree.release();
    if status.success() {
        readers.finish(Duration::ZERO);
        return Ok(());
    }
    readers.finish(Duration::from_millis(500));
    let printed = [&err, &out]
        .into_iter()
        .map(|tail| tail.lock().unwrap_or_else(|e| e.into_inner()).text())
        .find(|text| !text.is_empty())
        .map(|text| format!(":\n{text}"))
        .unwrap_or_default();
    let exit = status
        .code()
        .map_or_else(|| status.to_string(), |code| code.to_string());
    Err(format!(
        "The pre-launch command failed (exit {exit}), so the game did not start{printed}"
    ))
}

#[cfg(windows)]
mod tree {
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenThread, ResumeThread, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
    };

    /// The job the command and everything it starts run in.
    pub(super) struct Tree(HANDLE);

    impl Drop for Tree {
        fn drop(&mut self) {
            // SAFETY: the handle is this job's, opened by `spawn` and closed only here.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn limit(job: HANDLE, flags: u32) -> io::Result<()> {
        // SAFETY: all-zero is a valid value of this plain C struct; the call reads it whole.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = flags;
        // SAFETY: `job` is an open job handle, and the size is the struct's.
        let set = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        match set {
            0 => Err(io::Error::last_os_error()),
            _ => Ok(()),
        }
    }

    /// Starts `command` in a new job that ends what is in it when its last handle closes. It
    /// starts suspended and runs once it is in the job, so nothing it starts escapes it.
    pub(super) fn spawn(mut command: Command) -> io::Result<(Child, Tree)> {
        // SAFETY: no security attributes and no name: a new anonymous job.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let tree = Tree(job);
        limit(job, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)?;
        command.creation_flags(crate::process::CREATE_NO_WINDOW | CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        // SAFETY: both handles are open: the job's and the child's, which `child` owns.
        let joined = unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) };
        let started = match joined {
            0 => Err(io::Error::last_os_error()),
            _ => resume(child.id()),
        };
        if let Err(e) = started {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        Ok((child, tree))
    }

    /// Resumes the threads of the suspended process `pid` (a new one has one).
    fn resume(pid: u32) -> io::Result<()> {
        // SAFETY: a snapshot of every thread; closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: all-zero is a valid value of this plain C struct; its size is set as the
        // calls require.
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut resumed = false;
        // SAFETY: the snapshot is open and `entry` is sized; the thread handle is closed after
        // use.
        unsafe {
            let mut more = Thread32First(snapshot, &mut entry) != 0;
            while more {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !thread.is_null() {
                        resumed |= ResumeThread(thread) != u32::MAX;
                        CloseHandle(thread);
                    }
                }
                more = Thread32Next(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
        }
        match resumed {
            true => Ok(()),
            false => Err(io::Error::other("its thread could not be resumed")),
        }
    }

    impl Tree {
        /// Ends the command and everything it started.
        pub(super) fn kill(&self, _child: &mut Child) {
            // SAFETY: the job's handle is open.
            unsafe { TerminateJobObject(self.0, 1) };
        }

        /// Lets what the command left running go on once the job closes.
        pub(super) fn release(&self) {
            let _ = limit(self.0, 0);
        }
    }
}

/// Elsewhere [`super::wrap`] runs the pre-launch command in the game's own shell: only tests
/// come here.
#[cfg(not(windows))]
mod tree {
    use std::process::{Child, Command};

    pub(super) struct Tree;

    pub(super) fn spawn(mut command: Command) -> std::io::Result<(Child, Tree)> {
        Ok((command.spawn()?, Tree))
    }

    impl Tree {
        pub(super) fn kill(&self, child: &mut Child) {
            let _ = child.kill();
        }

        pub(super) fn release(&self) {}
    }
}
