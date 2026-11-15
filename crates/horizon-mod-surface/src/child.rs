//! Parsing a mod's source in a child process, so that a parser that runs out of stack ends the
//! child and not the program that asked.
//!
//! The parser reads nesting by recursion. [`crate::syntax::too_deep`] refuses deep source before
//! it is parsed, but a count of tokens can be fooled, and a stack overflow cannot be caught: it
//! aborts the process. So [`crate::uses::uses_contained`] first parses every file once in a child,
//! the program itself started again ([`std::env::current_exe`]) with [`CHILD_VAR`] set, its
//! sources on stdin and never on the command line. A child that dies, by a signal, an overflow or
//! a limit, or that outlives [`CHILD_WAIT`], leaves the source "cannot judge". Only once the
//! child parsed every file, on half the scan's parser stack, does the scan parse them in the
//! program itself, with megabytes to spare.
//!
//! Every program that runs the scan calls [`child_entry`] first in `main`, and hands what it
//! returns to [`crate::uses::uses_contained`], so one that does not cannot compile. The child also
//! answers with [`HANDSHAKE`] before anything else: a program started that prints anything else
//! first is no scan's child, and is stopped at once.
//!
//! Limits on the child: on Linux its address space is capped ([`CHILD_MEMORY`]); on Unix its
//! processor time is capped too, so a child whose parent died still ends. Windows and macOS have
//! no memory cap (macOS does not enforce one), and on Windows a child outlives a dead parent until
//! it ends by itself; there the wait in the parent is the only bound.
use std::io::{BufRead, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Marks a process as the scan's child; its value is the parser's stack size in bytes.
pub const CHILD_VAR: &str = "HORIZON_MOD_SCAN_CHILD";

/// How long a child may parse a mod's source: about ten times the 1.5 s a release build takes
/// on 200 000 lines in 1749 files.
pub const CHILD_WAIT: Duration = Duration::from_secs(15);

/// The address space a child may use, where the system enforces it (Linux).
pub const CHILD_MEMORY: u64 = 4 << 30;

/// The first bytes of a child's input.
const MAGIC: &[u8] = b"horizon-mod-scan 1\n";

/// The first line a child writes, before it reads its input.
pub const HANDSHAKE: &str = "horizon-mod-scan child 1";

/// What [`child_entry`] returns in a program that is not the scan's child: proof for
/// [`crate::uses::uses_contained`] that the program runs the entry first.
#[derive(Clone, Copy, Debug)]
pub struct ChildEntry(());

/// Why the child did not parse the sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Contained {
    /// It stopped, by a signal, an overflow or its memory limit, while parsing this source (its
    /// index among those given), or before it started.
    Died(Option<usize>),
    /// It was still parsing this source when [`CHILD_WAIT`] ran out.
    TimedOut(Option<usize>),
    /// It could not be started, or the program started never answered as a scan's child.
    NoChild,
}

/// In the scan's child, parses the sources on stdin and exits; anywhere else, returns at once.
/// A process started with [`CHILD_VAR`] whose stdin is not a scan's input, or holds anything
/// after the last source, exits with status 3, so the entry never runs outside a scan.
pub fn child_entry() -> ChildEntry {
    let Some(stack) = std::env::var_os(CHILD_VAR) else {
        return ChildEntry(());
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{HANDSHAKE}");
    let _ = out.flush();
    let Some(stack) = stack.to_str().and_then(|s| s.parse::<usize>().ok()) else {
        std::process::exit(3)
    };
    limit();
    let Some(texts) = read_input(&mut std::io::stdin().lock()) else {
        std::process::exit(3)
    };
    for (index, text) in texts.iter().enumerate() {
        // Which source is being parsed, so the parent can name the one the child died on.
        let _ = writeln!(out, "{index}");
        let _ = out.flush();
        let parsed = std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(stack)
                .spawn_scoped(scope, || {
                    let _ = crate::syntax::parse(text);
                })
                .map(|thread| thread.join().is_ok())
        });
        if !matches!(parsed, Ok(true)) {
            std::process::exit(4)
        }
    }
    let _ = writeln!(out, "done");
    let _ = out.flush();
    std::process::exit(0)
}

/// Caps the child's address space (enforced on Linux) and processor time (Unix), so it ends
/// even when its parent is gone. A limit the system refuses leaves the wait in the parent.
fn limit() {
    #[cfg(unix)]
    {
        let memory = libc::rlimit {
            rlim_cur: CHILD_MEMORY as libc::rlim_t,
            rlim_max: CHILD_MEMORY as libc::rlim_t,
        };
        let seconds = 2 * CHILD_WAIT.as_secs();
        let processor = libc::rlimit {
            rlim_cur: seconds as libc::rlim_t,
            rlim_max: seconds as libc::rlim_t,
        };
        // SAFETY: `setrlimit` reads the struct it is given and touches nothing else.
        unsafe {
            libc::setrlimit(libc::RLIMIT_AS, &memory);
            libc::setrlimit(libc::RLIMIT_CPU, &processor);
        }
    }
}

fn read_input(input: &mut impl Read) -> Option<Vec<String>> {
    let mut magic = vec![0; MAGIC.len()];
    input.read_exact(&mut magic).ok()?;
    if magic != MAGIC {
        return None;
    }
    let mut count = [0; 8];
    input.read_exact(&mut count).ok()?;
    let count = u64::from_le_bytes(count);
    let mut texts = Vec::new();
    for _ in 0..count {
        let mut length = [0; 8];
        input.read_exact(&mut length).ok()?;
        let length = u64::from_le_bytes(length);
        let mut bytes = Vec::new();
        input.take(length).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 != length {
            return None;
        }
        texts.push(String::from_utf8(bytes).ok()?);
    }
    // Nothing may follow the last source.
    let mut rest = [0; 1];
    if input.read(&mut rest).ok()? != 0 {
        return None;
    }
    Some(texts)
}

fn write_input(texts: &[&str]) -> Vec<u8> {
    let mut input = MAGIC.to_vec();
    input.extend((texts.len() as u64).to_le_bytes());
    for text in texts {
        input.extend((text.len() as u64).to_le_bytes());
        input.extend(text.as_bytes());
    }
    input
}

/// How a child ended, seen from the parent.
enum End {
    Exited(std::process::ExitStatus),
    Late,
    /// It wrote something other than [`HANDSHAKE`] first: not a scan's child.
    Foreign,
}

/// Parses `texts` in a child started from `program`, the parser on a stack of `stack` bytes,
/// waiting at most `wait`.
pub fn parse_in_child(
    program: &std::path::Path,
    texts: &[&str],
    stack: usize,
    wait: Duration,
) -> Result<(), Contained> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(program);
    command
        .env(CHILD_VAR, stack.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: a console child of a program without a console opens no window.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|_| Contained::NoChild)?;
    let input = write_input(texts);
    let mut stdin = child.stdin.take().ok_or(Contained::NoChild)?;
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let stdout = child.stdout.take().ok_or(Contained::NoChild)?;
    let foreign = Arc::new(AtomicBool::new(false));
    let reader = std::thread::spawn({
        let foreign = Arc::clone(&foreign);
        move || {
            let mut lines = std::io::BufReader::new(stdout).lines();
            match lines.next() {
                Some(Ok(first)) if first == HANDSHAKE => {}
                Some(_) => {
                    foreign.store(true, Ordering::Relaxed);
                    return None;
                }
                None => return None,
            }
            Some(lines.map_while(Result::ok).collect::<Vec<String>>())
        }
    });
    let deadline = Instant::now() + wait;
    let end = loop {
        if foreign.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            break End::Foreign;
        }
        match child.try_wait() {
            Ok(Some(status)) => break End::Exited(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break End::Late;
            }
        }
    };
    let _ = writer.join();
    // `None`: the program never answered as a scan's child.
    let said = reader.join().ok().flatten();
    let lines = said.as_deref().unwrap_or_default();
    let at = lines
        .iter()
        .rev()
        .find_map(|line| line.parse::<usize>().ok());
    let done = lines.last().map(String::as_str) == Some("done");
    match end {
        End::Foreign => Err(Contained::NoChild),
        End::Late => Err(Contained::TimedOut(at)),
        End::Exited(_) if said.is_none() => Err(Contained::NoChild),
        End::Exited(status) if status.success() && done => Ok(()),
        End::Exited(_) => Err(Contained::Died(at)),
    }
}
