use std::io::{self, Read as _};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy)]
enum Phase {
    Idle,
    Writing(Instant),
    Closing(Instant),
    Cancelled,
    Done,
}

struct Watch {
    phase: Mutex<Phase>,
    changed: Condvar,
}

impl Watch {
    fn set(&self, next: Phase) -> bool {
        let mut phase = self.phase.lock().unwrap();
        match *phase {
            Phase::Done => return false,
            // Do not reset a deadline that the supervisor has yet to act on.
            Phase::Writing(deadline)
                if !matches!(next, Phase::Cancelled) && Instant::now() >= deadline =>
            {
                return false;
            }
            _ => {}
        }
        *phase = next;
        self.changed.notify_one();
        true
    }
}

/// The supervisor owns the child so it can kill it while the writer is in a
/// blocking pipe write. The writer only holds the pipe and reports progress.
pub(super) struct EncoderChild {
    watch: Arc<Watch>,
    worker: Option<JoinHandle<io::Result<ExitStatus>>>,
    timeout: Duration,
}

impl EncoderChild {
    pub(super) fn spawn(command: Command, timeout: Duration) -> io::Result<(Self, ChildStdin)> {
        let mut child = ReapChild::spawn(command)?;
        let stdin = child.child.stdin.take().expect("encoder stdin is piped");
        let watch = Arc::new(Watch {
            phase: Mutex::new(Phase::Idle),
            changed: Condvar::new(),
        });
        let watcher = Arc::clone(&watch);
        let worker = thread::Builder::new()
            .name("export-encoder".into())
            .spawn(move || supervise(child, &watcher))?;
        Ok((
            Self {
                watch,
                worker: Some(worker),
                timeout,
            },
            stdin,
        ))
    }

    pub(super) fn writing(&self) -> bool {
        self.watch
            .set(Phase::Writing(Instant::now() + self.timeout))
    }

    pub(super) fn idle(&self) -> bool {
        self.watch.set(Phase::Idle)
    }

    pub(super) fn finish(mut self) -> io::Result<ExitStatus> {
        self.watch
            .set(Phase::Closing(Instant::now() + self.timeout));
        self.join()
    }

    fn join(&mut self) -> io::Result<ExitStatus> {
        self.worker
            .take()
            .expect("encoder supervisor joins once")
            .join()
            .map_err(|_| io::Error::other("encoder supervisor panicked"))?
    }
}

impl Drop for EncoderChild {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.watch.set(Phase::Cancelled);
            let _ = self.join();
        }
    }
}

/// Read a short protocol listing under a single deadline for both output and exit.
pub(super) fn probe_output(mut command: Command, timeout: Duration) -> io::Result<Vec<u8>> {
    const MAX_OUTPUT: u64 = 64 * 1024;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let deadline = Instant::now() + timeout;
    let mut child = ReapChild::spawn(command)?;
    let stdout = child.child.stdout.take().expect("probe stdout is piped");
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("export-encoder-probe".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take(MAX_OUTPUT + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send(result);
        })?;

    // Read before reaping the leader so a helper holding stdout open is still
    // covered by the process-tree cleanup on timeout.
    let bytes = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|err| match err {
            RecvTimeoutError::Timeout => probe_timeout(),
            RecvTimeoutError::Disconnected => io::Error::other("encoder protocol reader stopped"),
        })??;
    if bytes.len() as u64 > MAX_OUTPUT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "encoder protocol listing is too large",
        ));
    }
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(bytes)
            } else {
                Err(io::Error::other(format!(
                    "encoder protocol probe failed ({status})"
                )))
            };
        }
        let wait = time_left(deadline).ok_or_else(probe_timeout)?;
        thread::sleep(wait.min(EXIT_POLL_INTERVAL));
    }
}

fn probe_timeout() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "encoder protocol probe timed out")
}

struct ReapChild {
    child: Child,
    reaped: bool,
    #[cfg(windows)]
    job: Option<horizon_threads::process_job::ProcessJob>,
}

impl ReapChild {
    fn spawn(command: Command) -> io::Result<Self> {
        #[cfg(windows)]
        let (child, job) = horizon_threads::process_job::ProcessJob::spawn(command)?;
        #[cfg(not(windows))]
        let child = {
            let mut command = command;
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt as _;
                command.process_group(0);
            }
            command.spawn()?
        };
        Ok(Self {
            child,
            reaped: false,
            #[cfg(windows)]
            job: Some(job),
        })
    }

    #[cfg(unix)]
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        use rustix::io::Errno;
        use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

        // Keep the leader's PID reserved until its group has been signalled.
        let pid = Pid::from_child(&self.child);
        let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
        match waitid(WaitId::Pid(pid), options) {
            Ok(Some(_)) => {
                self.kill_group();
                self.reaped = true;
                self.child.wait().map(Some)
            }
            Ok(None) | Err(Errno::INTR) => Ok(None),
            Err(err) => {
                self.reaped = err == Errno::CHILD;
                Err(err.into())
            }
        }
    }

    #[cfg(not(unix))]
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        self.reaped = status.is_some();
        Ok(status)
    }

    #[cfg(unix)]
    fn kill_group(&self) {
        use rustix::process::{Pid, Signal, kill_process_group};
        let _ = kill_process_group(Pid::from_child(&self.child), Signal::KILL);
    }
}

impl Drop for ReapChild {
    fn drop(&mut self) {
        // Close the job before waiting for the leader and even if it already exited.
        #[cfg(windows)]
        drop(self.job.take());
        if self.reaped {
            return;
        }
        #[cfg(unix)]
        self.kill_group();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn supervise(mut child: ReapChild, watch: &Watch) -> io::Result<ExitStatus> {
    let mut phase = watch.phase.lock().unwrap();
    let result = loop {
        let wait = match *phase {
            Phase::Idle => {
                phase = watch.changed.wait(phase).unwrap();
                continue;
            }
            Phase::Writing(deadline) => {
                let Some(wait) = time_left(deadline) else {
                    break Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "encoder stopped reading frames",
                    ));
                };
                wait
            }
            Phase::Closing(deadline) => {
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Err(err) => break Err(err),
                    Ok(None) => {}
                }
                let Some(wait) = time_left(deadline) else {
                    break Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "encoder did not exit after stdin closed",
                    ));
                };
                wait.min(EXIT_POLL_INTERVAL)
            }
            Phase::Cancelled => break Err(io::ErrorKind::Interrupted.into()),
            Phase::Done => unreachable!(),
        };
        (phase, _) = watch.changed.wait_timeout(phase, wait).unwrap();
    };
    *phase = Phase::Done;
    drop(phase);
    // Drop kills and reaps the child on timeout, closing its pipe before join returns.
    drop(child);
    result
}

fn time_left(deadline: Instant) -> Option<Duration> {
    deadline.checked_duration_since(Instant::now())
}
