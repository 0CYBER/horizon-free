//! `horizon-server run`: writes `server.properties` from `server.toml`, runs the engine with the console on
//! stdin, stops it cleanly on Ctrl-C or SIGTERM, and restarts it when it crashes (docs/server.md, section 7).

use crate::{config, config::Config, engine, signals};
use horizon_server_files::java_properties::now_millis;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ExitStatus},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// After this many crashes within [`CRASH_WINDOW`] the supervisor stops restarting the game.
const MAX_CRASHES: usize = 5;
const CRASH_WINDOW: Duration = Duration::from_secs(600);
/// The first wait before a restart; it doubles after each crash, up to [`MAX_BACKOFF`].
const FIRST_BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(300);
/// How long `stop` may take before the game is killed.
const STOP_TIMEOUT: Duration = Duration::from_secs(120);

/// A line from the supervisor, in the game's log format (`[HH:mm:ss] [thread/LEVEL]: message`, UTC).
pub fn note(level: &str, message: &str) {
    let secs = now_millis().div_euclid(1000).rem_euclid(86_400);
    println!(
        "[{:02}:{:02}:{:02}] [Supervisor/{level}]: {message}",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    );
}

/// The folder's lock: one supervisor per server folder. Released when the process ends, even by a crash.
pub fn lock(dir: &Path) -> Result<File, String> {
    let state = dir.join(".horizon");
    std::fs::create_dir_all(&state).map_err(|e| format!("{}: {e}", state.display()))?;
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state.join("lock"))
        .map_err(|e| e.to_string())?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => {
            Err(format!("a server is already running in {}", dir.display()))
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e.to_string()),
    }
}

enum Exit {
    /// Asked to stop, or the game ended by itself without an error (`stop` typed in its console, the EULA
    /// not agreed, the port taken): not restarted.
    Stopped,
    Crashed(ExitStatus),
}

pub fn run(dir: &Path) -> Result<(), String> {
    signals::install();
    let config = Config::load(dir)?;
    let program = engine::program(&config, dir)?;
    let resources = engine::resources(&config, dir, Some(&program))?;
    let _lock = lock(dir)?;
    let console = console_lines();
    let mut crashes: Vec<Instant> = Vec::new();
    loop {
        // Read again before every start: edits to server.toml apply on the next restart.
        let config = Config::load(dir)?;
        let written = config
            .write_properties(dir)
            .map_err(|e| format!("server.properties: {e}"))?;
        note(
            "INFO",
            &format!(
                "Starting {} with the game files in {}",
                program.display(),
                resources.display()
            ),
        );
        let mut child = engine::spawn(&program, &resources, dir)
            .map_err(|e| format!("{}: {e}", program.display()))?;
        let exit = supervise(&mut child, &console);
        match config::sync_back(dir, &written) {
            Ok(keys) if !keys.is_empty() => note(
                "INFO",
                &format!(
                    "Copied into server.toml what the game changed: {}",
                    keys.join(", ")
                ),
            ),
            Ok(_) => {}
            Err(e) => note("WARN", &format!("Could not update server.toml: {e}")),
        }
        let status = match exit {
            Exit::Stopped => return Ok(()),
            Exit::Crashed(status) => status,
        };
        if !config.engine.restart_on_crash {
            return Err(format!(
                "the game stopped unexpectedly ({status}); engine.restart_on_crash is off"
            ));
        }
        let now = Instant::now();
        crashes.retain(|t| now.duration_since(*t) < CRASH_WINDOW);
        crashes.push(now);
        if crashes.len() >= MAX_CRASHES {
            return Err(format!(
                "the game stopped unexpectedly {MAX_CRASHES} times in {} minutes: not restarting it. \
                 See logs/latest.log and crash-reports/",
                CRASH_WINDOW.as_secs() / 60
            ));
        }
        let wait = backoff(crashes.len());
        note(
            "WARN",
            &format!(
                "The game stopped unexpectedly ({status}); restarting in {} s",
                wait.as_secs()
            ),
        );
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if signals::requests() > 0 {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// The wait before the restart that follows the `crashes`th recent crash.
fn backoff(crashes: usize) -> Duration {
    let doublings = u32::try_from(crashes.saturating_sub(1))
        .unwrap_or(u32::MAX)
        .min(16);
    (FIRST_BACKOFF * 2u32.pow(doublings)).min(MAX_BACKOFF)
}

/// The supervisor's own stdin, line by line (nothing when it has none, as under a service manager).
fn console_lines() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let _ = thread::Builder::new()
        .name("console".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    rx
}

/// Copies the game's output to ours, line by line.
fn pump(from: impl Read + Send + 'static, to_stderr: bool) {
    let _ = thread::Builder::new()
        .name("engine-log".into())
        .spawn(move || {
            for line in BufReader::new(from).lines() {
                let Ok(line) = line else { break };
                if to_stderr {
                    eprintln!("{line}");
                } else {
                    println!("{line}");
                }
            }
        });
}

/// Runs one engine process to its end: console lines in, the log out, and a clean `stop` when asked.
fn supervise(child: &mut Child, console: &mpsc::Receiver<String>) -> Exit {
    if let Some(out) = child.stdout.take() {
        pump(out, false);
    }
    if let Some(err) = child.stderr.take() {
        pump(err, true);
    }
    let mut stdin = child.stdin.take();
    let mut stopping: Option<Instant> = None;
    let mut handled = signals::requests();
    loop {
        while let Ok(line) = console.try_recv() {
            let command = line.trim().trim_start_matches('/');
            if command == "stop" && stopping.is_none() {
                stopping = Some(Instant::now());
            }
            if let Some(input) = stdin.as_mut()
                && writeln!(input, "{line}")
                    .and_then(|()| input.flush())
                    .is_err()
            {
                stdin = None;
            }
        }
        let requests = signals::requests();
        if requests > handled {
            handled = requests;
            if stopping.is_none() {
                note("INFO", "Stopping the server");
                stopping = Some(Instant::now());
                if let Some(input) = stdin.as_mut() {
                    let _ = writeln!(input, "stop").and_then(|()| input.flush());
                }
            } else {
                note("WARN", "Stopping the game at once, without saving");
                let _ = child.kill();
            }
        }
        if stopping.is_some_and(|t| t.elapsed() > STOP_TIMEOUT) {
            note(
                "WARN",
                &format!(
                    "The game did not stop within {} s: killing it",
                    STOP_TIMEOUT.as_secs()
                ),
            );
            let _ = child.kill();
        }
        match child.try_wait() {
            Ok(Some(status)) if stopping.is_some() || status.success() => return Exit::Stopped,
            Ok(Some(status)) => return Exit::Crashed(status),
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                note("ERROR", &format!("Lost track of the game: {e}"));
                return Exit::Stopped;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_five_minutes() {
        let waits: Vec<u64> = (1..=8).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(waits, [5, 10, 20, 40, 80, 160, 300, 300]);
    }

    #[test]
    fn one_supervisor_per_folder() {
        let dir = std::env::temp_dir().join(format!("horizon-server-lock-{}", std::process::id()));
        let first = lock(&dir).unwrap();
        assert!(lock(&dir).unwrap_err().contains("already running"));
        drop(first);
        lock(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
