//! A bounded request/reply channel to one initialized exporter JVM.
use crate::*;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime};

const REPLY_TIMEOUT: Duration = Duration::from_secs(3600);
/// How long a JVM that stopped replying gets to exit on its own, so the status it reports is its
/// own and not the kill's (on Windows both are exit code 1).
const EXIT_GRACE: Duration = Duration::from_secs(10);
/// How many lines of the JVM's log a stopped worker's error quotes.
const TAIL_LINES: usize = 12;
/// How many lines of a stopped JVM's stdout are kept: HotSpot's longest banner fits.
const PRINTED_LINES: usize = 64;

pub(crate) struct Worker {
    child: Child,
    input: ChildStdin,
    replies: Receiver<String>,
    /// When it started: a crash report older than that is another process's.
    started: SystemTime,
    /// Its stderr: Java's exceptions, and vanilla's log while it starts.
    log: PathBuf,
    /// Its working directory, where HotSpot writes a crash report (`hs_err_pid<pid>.log`).
    dir: PathBuf,
}

impl Worker {
    pub(crate) fn new(run: &Run, name: &str, classes: &Path, cp: &str) -> Result<Self, String> {
        let mut command = java::command(&run.jdk.java);
        command.args(java::BATCH_OPTIONS).args([
            "-Xmx2G",
            "-cp",
            &format!(
                "{}{}{}",
                classes.display(),
                if cfg!(windows) { ";" } else { ":" },
                cp
            ),
            "dev.horizonlab.parity.PrepareWorker",
        ]);
        let log = run.local.join(format!("{name}-worker.log"));
        // Vanilla's code logs relative to the working directory (logs/): in the scratch, as
        // `Run::run`.
        Self::spawn(command, &log, &run.local)
    }

    fn spawn(mut command: Command, log: &Path, dir: &Path) -> Result<Self, String> {
        let file = fs::File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
        let started = SystemTime::now();
        let mut child = command
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(file)
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (send, replies) = mpsc::channel();
        std::thread::spawn(move || forward_lines(BufReader::new(output), &send));
        Ok(Self {
            child,
            input,
            replies,
            started,
            log: log.to_path_buf(),
            dir: dir.to_path_buf(),
        })
    }

    pub(crate) fn run(&mut self, main: &str, args: &[String], log: &Path) -> Result<(), String> {
        let task = Value::Array(vec![
            Value::Str(arg(log)),
            Value::Str(main.into()),
            Value::Array(args.iter().cloned().map(Value::Str).collect()),
        ]);
        let sent = writeln!(self.input, "{}", pyjson::dumps(&task, &pyjson::COMPACT))
            .and_then(|()| self.input.flush());
        // A JVM that stopped before it read the task closed its input: why is in its log.
        let reply = match sent.map(|()| self.replies.recv_timeout(REPLY_TIMEOUT)) {
            Ok(Ok(reply)) => reply,
            Ok(Err(RecvTimeoutError::Timeout)) => {
                let _ = self.child.kill();
                let what = format!("Exporter JVM took over an hour on {main} and was stopped");
                return Err(self.stopped(&what, Vec::new(), log));
            }
            Err(_) | Ok(Err(RecvTimeoutError::Disconnected)) => {
                return Err(self.stopped(&before(main), Vec::new(), log));
            }
        };
        match pyjson::loads(&reply) {
            Ok(value) => match value.as_str() {
                Some("") => Ok(()),
                _ => Err(format!("Exporter {main} failed: {reply}")),
            },
            // Not a reply: HotSpot prints why it stops (a VM that cannot start, a crash) to
            // stdout, ahead of the protocol.
            Err(_) => Err(self.stopped(&before(main), vec![reply], log)),
        }
    }

    /// `what` stopped the JVM: the error says how it exited, the last thing it said, and where
    /// its log and crash report are. The task's `log`, which the JVM may have stopped before it
    /// wrote, gets that and the ends of what it printed and logged.
    fn stopped(&mut self, what: &str, printed: Vec<String>, log: &Path) -> String {
        let deadline = Instant::now() + EXIT_GRACE;
        let mut printed = VecDeque::from(printed);
        while let Ok(line) = self
            .replies
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            if printed.len() == PRINTED_LINES {
                printed.pop_front();
            }
            printed.push_back(line);
            if Instant::now() >= deadline {
                break;
            }
        }
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status.to_string(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Ok(None) => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break "still running, so it was stopped".into();
                }
                Err(e) => break e.to_string(),
            }
        };
        // All of it for the headline: HotSpot's comes early in its banner.
        let all_printed = Vec::from(printed).join("\n");
        let printed = tail(&all_printed);
        let logged = read_tail(&self.log);
        let report = format!("hs_err_pid{}.log", self.child.id());
        // HotSpot writes the report to the working directory, or the temporary one if it can't.
        // Windows soon gives a pid to another process: an older report isn't this JVM's.
        let report = [self.dir.join(&report), std::env::temp_dir().join(&report)]
            .into_iter()
            .find(|path| {
                fs::metadata(path)
                    .and_then(|m| m.modified())
                    .is_ok_and(|modified| modified >= self.started)
            });
        // One line: the launcher shows a preparer's last line, and the caller adds the step.
        // HotSpot's own words come first, else Java's last exception.
        let mut error = format!("{what} ({status})");
        if let Some(line) = headline(&all_printed).or_else(|| logged.lines().next_back()) {
            error += &format!(": {}", line.trim());
        }
        error += &format!("; its log: {}", self.log.display());
        if let Some(report) = &report {
            error += &format!("; Java's crash report: {}", report.display());
        }
        let mut details = error.clone();
        if !printed.is_empty() {
            details += &format!("\nIt printed:\n{printed}");
        }
        details += &if logged.is_empty() {
            "\nIts log is empty.".to_string()
        } else {
            format!("\nIts log ends:\n{logged}")
        };
        let _ = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(log)
            .and_then(|mut file| writeln!(file, "{details}"));
        error
    }
}

/// Sends each line `output` prints, decoded lossily: HotSpot prints paths in the system's code
/// page, and a line that isn't UTF-8 must not cut off the lines after it.
fn forward_lines(mut output: impl BufRead, send: &Sender<String>) {
    let mut line = vec![];
    while output
        .read_until(b'\n', &mut line)
        .is_ok_and(|read| read > 0)
    {
        let text = String::from_utf8_lossy(&line);
        if send
            .send(text.trim_end_matches(['\r', '\n']).to_string())
            .is_err()
        {
            break;
        }
        line.clear();
    }
}

/// The first line of what a JVM printed that says what happened, without HotSpot's `#`: not a
/// heading, a link, or where the crash report is (the error names that itself).
fn headline(printed: &str) -> Option<&str> {
    printed
        .lines()
        .map(|line| line.trim_start_matches('#').trim())
        .find(|line| {
            !(line.is_empty()
                || line.contains("://")
                || line.contains("hs_err_pid")
                || line.starts_with("A fatal error has been detected")
                || line.starts_with("An error report file")
                || *line == "Error occurred during initialization of VM")
        })
}

fn before(main: &str) -> String {
    format!("Exporter JVM stopped before {main} finished")
}

/// The last [`TAIL_LINES`] lines of `text` that say something. A Java stack trace's frames
/// would push its exceptions out of them.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| {
            let line = line.trim();
            // HotSpot's banner frames its lines with bare `#`.
            !(line.is_empty()
                || line == "#"
                || line.starts_with("at ")
                || (line.starts_with("... ") && line.ends_with(" more")))
        })
        .collect();
    lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}

/// [`tail`] of the file at `path`, read from its last 64 KiB.
fn read_tail(path: &Path) -> String {
    let mut bytes = Vec::new();
    let _ = fs::File::open(path).and_then(|mut file| {
        let length = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::Start(length.saturating_sub(64 * 1024)))?;
        file.read_to_end(&mut bytes)
    });
    tail(&String::from_utf8_lossy(&bytes))
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in JVM that prints like HotSpot on stdout, like Java on stderr, and exits 3
    /// without reading its task.
    #[cfg(windows)]
    fn stand_in() -> Command {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("cmd");
        command.raw_arg(
            "/C \"echo Error occurred during initialization of VM& \
             echo Could not reserve enough space for object heap& \
             echo java.lang.OutOfMemoryError: Java heap space 1>&2& exit /b 3\"",
        );
        command
    }

    #[cfg(not(windows))]
    fn stand_in() -> Command {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "echo Error occurred during initialization of VM; \
             echo Could not reserve enough space for object heap; \
             echo 'java.lang.OutOfMemoryError: Java heap space' >&2; exit 3",
        ]);
        command
    }

    #[test]
    #[ignore = "daily"]
    fn a_jvm_that_stops_early_says_how_in_the_error_and_the_task_log() {
        let dir = std::env::temp_dir().join(format!("horizon-worker-stop-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let task_log = dir.join("export.log");
        fs::File::create(&task_log).unwrap();
        let mut worker = Worker::spawn(stand_in(), &dir.join("server-worker.log"), &dir).unwrap();
        let error = worker
            .run("VanillaHeightmapOracle", &[], &task_log)
            .unwrap_err();
        assert!(
            error.starts_with("Exporter JVM stopped before VanillaHeightmapOracle finished"),
            "{error}"
        );
        assert_eq!(error.lines().count(), 1, "{error}");
        assert!(error.contains(": 3)"), "the exit status: {error}");
        // What HotSpot printed says more than what Java logged before it.
        assert!(
            error.contains(": Could not reserve enough space for object heap;"),
            "{error}"
        );
        assert!(error.contains("server-worker.log"), "{error}");
        let logged = fs::read_to_string(&task_log).unwrap();
        assert!(logged.starts_with(&format!("{error}\n")), "{logged}");
        assert!(
            logged.contains("OutOfMemoryError: Java heap space"),
            "{logged}"
        );
        drop(worker);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_line_that_isnt_utf8_keeps_the_lines_after_it() {
        let (send, lines) = mpsc::channel();
        // A Cyrillic path in Windows-1251, as HotSpot prints it on a Russian system.
        let output: &[u8] = b"# An error report file with more information is saved as:\r\n\
                              # C:\\Users\\\xc4\xee\xec\\hs_err_pid7.log\r\n\
                              #\r\n";
        forward_lines(output, &send);
        drop(send);
        let lines: Vec<String> = lines.iter().collect();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[1].ends_with(r"\hs_err_pid7.log"), "{lines:?}");
        assert_eq!(lines[2], "#");
    }

    #[test]
    fn the_headline_is_what_happened_not_hotspots_banner() {
        let out_of_memory = "#\n\
            # There is insufficient memory for the Java Runtime Environment to continue.\n\
            # Native memory allocation (mmap) failed to map 65536 bytes.\n\
            # An error report file with more information is saved as:\n\
            # C:\\x\\.local\\hs_err_pid7.log\n";
        assert_eq!(
            headline(out_of_memory),
            Some("There is insufficient memory for the Java Runtime Environment to continue.")
        );
        let crash = "#\n\
            # A fatal error has been detected by the Java Runtime Environment:\n\
            #\n\
            #  EXCEPTION_ACCESS_VIOLATION (0xc0000005) at pc=0x0, pid=7, tid=8\n\
            # If you would like to submit a bug report, please visit:\n\
            #   https://bugreport.java.com/bugreport/crash.jsp\n";
        assert_eq!(
            headline(crash),
            Some("EXCEPTION_ACCESS_VIOLATION (0xc0000005) at pc=0x0, pid=7, tid=8")
        );
        assert_eq!(
            headline(
                "Error occurred during initialization of VM\n\
                 Could not reserve enough space for object heap"
            ),
            Some("Could not reserve enough space for object heap")
        );
        assert_eq!(headline("#\n# https://bugreport.java.com\n"), None);
    }

    #[test]
    fn tails_keep_the_last_lines_that_say_something() {
        let text: String = (1..=20)
            .map(|n| format!("line {n}\n\n\tat a.B.c(B.java:{n})\n\t... {n} more\n"))
            .collect();
        let kept = tail(&text);
        assert_eq!(kept.lines().count(), TAIL_LINES);
        assert!(
            kept.starts_with("line 9\n") && kept.ends_with("line 20"),
            "{kept}"
        );
    }
}
