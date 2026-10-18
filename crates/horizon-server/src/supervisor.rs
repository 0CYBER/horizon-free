//! `horizon-server run`: writes `server.properties` from `server.toml`, runs the engine with the console on
//! stdin, serves the control API, stops the game cleanly on Ctrl-C, SIGTERM or `stop`, and restarts it when
//! it crashes (docs/server.md, sections 2 and 7).

use crate::{
    api::{self, Phase, Shared},
    backup, config,
    config::Config,
    engine, signals,
};
use serde_json::json;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ExitStatus},
    sync::{Arc, atomic::Ordering, mpsc},
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

/// A line from the supervisor, in the game's log format (`[HH:mm:ss] [thread/LEVEL]: message`, local time
/// as the game's own lines), on stdout and in the log the API serves.
fn note(shared: &Shared, level: &str, message: &str) {
    let line = format!(
        "[{}] [Supervisor/{level}]: {message}",
        chrono::Local::now().format("%H:%M:%S")
    );
    println!("{line}");
    shared.log.lock().unwrap().push(line);
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
    /// Asked to restart (`horizon-server restart`).
    Restart,
    Crashed(ExitStatus),
}

pub fn run(dir: &Path) -> Result<(), String> {
    signals::install();
    let config = Config::load(dir)?;
    let program = engine::program(&config, dir)?;
    let resources = engine::resources(&config, dir, Some(&program))?;
    // A game the system can't load would only crash again at every restart: say why at once.
    engine::check_starts(&program)?;
    let _lock = lock(dir)?;
    let shared = Shared::new(dir);
    let _endpoint = api::start(shared.clone())?;
    match config.panel.address() {
        Ok(Some(address)) => match crate::panel::start(shared.clone(), address) {
            Ok(()) => note(
                &shared,
                "INFO",
                &format!(
                    "Web panel on http://{address} (`horizon-server panel` prints a login link)"
                ),
            ),
            Err(e) => note(
                &shared,
                "WARN",
                &format!("The web panel can't listen on {address}: {e}"),
            ),
        },
        Ok(None) => {}
        Err(e) => note(&shared, "WARN", &e),
    }
    sample(shared.clone());
    schedule_backups(shared.clone());
    let console = console_lines();
    let mut crashes: Vec<Instant> = Vec::new();
    loop {
        // Read again before every start: edits to server.toml apply on the next restart.
        let config = Config::load(dir)?;
        let written = config
            .write_properties(dir)
            .map_err(|e| format!("server.properties: {e}"))?;
        note(
            &shared,
            "INFO",
            &format!(
                "Starting {} with the game files in {}",
                program.display(),
                resources.display()
            ),
        );
        shared.set_phase(Phase::Starting);
        let mut child = engine::spawn(&program, &resources, dir)
            .map_err(|e| format!("{}: {e}", program.display()))?;
        {
            let mut state = shared.state.lock().unwrap();
            state.engine_pid = Some(child.id());
            state.engine_started = Some(Instant::now());
        }
        let exit = supervise(&shared, &mut child, &console);
        {
            let mut state = shared.state.lock().unwrap();
            state.engine_pid = None;
            state.engine_started = None;
        }
        match config::sync_back(dir, &written) {
            Ok(keys) if !keys.is_empty() => note(
                &shared,
                "INFO",
                &format!(
                    "Copied into server.toml what the game changed: {}",
                    keys.join(", ")
                ),
            ),
            Ok(_) => {}
            Err(e) => note(
                &shared,
                "WARN",
                &format!("Could not update server.toml: {e}"),
            ),
        }
        let status = match exit {
            Exit::Stopped => return Ok(()),
            Exit::Restart => {
                let restore = shared.restore.lock().unwrap().take();
                if let Some(name) = restore {
                    match backup::restore(dir, &config, &name) {
                        Ok(Some(old)) => note(
                            &shared,
                            "INFO",
                            &format!(
                                "Restored {name}; the world it replaced is in {}",
                                old.display()
                            ),
                        ),
                        Ok(None) => note(&shared, "INFO", &format!("Restored {name}")),
                        Err(e) => note(&shared, "ERROR", &format!("Could not restore {name}: {e}")),
                    }
                }
                continue;
            }
            Exit::Crashed(status) => status,
        };
        {
            let mut state = shared.state.lock().unwrap();
            state.crashes += 1;
            state.last_crash = Some(status.to_string());
        }
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
            &shared,
            "WARN",
            &format!(
                "The game stopped unexpectedly ({status}); restarting in {} s",
                wait.as_secs()
            ),
        );
        shared.set_phase(Phase::Restarting);
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if signals::requests() > 0 || shared.stop.load(Ordering::SeqCst) {
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

/// Copies the game's output to ours and to the API's log, line by line; the game's `Done` line means it
/// is up.
fn pump(shared: &Arc<Shared>, from: impl Read + Send + 'static, to_stderr: bool) {
    let shared = shared.clone();
    let _ = thread::Builder::new()
        .name("engine-log".into())
        .spawn(move || {
            for line in BufReader::new(from).lines() {
                let Ok(line) = line else { break };
                if to_stderr {
                    eprintln!("{line}");
                } else {
                    println!("{line}");
                    if line.contains("/INFO]: Done (") {
                        let mut state = shared.state.lock().unwrap();
                        if state.phase == Phase::Starting {
                            state.phase = Phase::Running;
                        }
                    }
                }
                shared.log.lock().unwrap().push(line);
            }
        });
}

/// Every second, the game's RAM, CPU and status, for `status` and the panel's graphs.
fn sample(shared: Arc<Shared>) {
    let _ = thread::Builder::new()
        .name("sampler".into())
        .spawn(move || {
            let mut control: Option<engine::Control> = None;
            let mut previous: Option<(u32, Instant, f64)> = None;
            loop {
                thread::sleep(Duration::from_secs(1));
                let (pid, running) = {
                    let state = shared.state.lock().unwrap();
                    (state.engine_pid, state.phase == Phase::Running)
                };
                let Some(pid) = pid else {
                    control = None;
                    previous = None;
                    shared.state.lock().unwrap().sample = serde_json::Value::Null;
                    continue;
                };
                let now = Instant::now();
                let cpu = horizon_bench::cpu_seconds(pid);
                let cpu_percent = match (previous, cpu) {
                    (Some((p, at, before)), Some(cpu)) if p == pid => {
                        Some((cpu - before) / now.duration_since(at).as_secs_f64() * 100.0)
                    }
                    _ => None,
                };
                previous = cpu.map(|c| (pid, now, c));
                if !running {
                    control = None;
                } else if control.is_none() {
                    control = engine::Control::connect(&shared.dir).ok();
                }
                let game = control
                    .as_mut()
                    .map(|c| c.ask("server-status"))
                    .transpose()
                    .unwrap_or_else(|_| {
                        control = None;
                        None
                    });
                let rss = horizon_bench::rss_bytes(pid);
                shared.record(json!({
                    "t": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs()),
                    "tps": game.as_ref().and_then(|g| g["tps"].as_f64()),
                    "mspt": game.as_ref().and_then(|g| g["mspt"]["average"].as_f64()),
                    "mspt_p95": game.as_ref().and_then(|g| g["mspt"]["p95"].as_f64()),
                    "rss_bytes": rss,
                    "cpu_percent": cpu_percent,
                    "players": game.as_ref().and_then(|g| g["players"].as_array().map(Vec::len)),
                }));
                shared.state.lock().unwrap().sample = json!({
                    "rss_bytes": rss,
                    "cpu_percent": cpu_percent,
                    "game": game,
                });
            }
        });
}

/// Automatic backups every `backups.every` while the game runs, keeping the last `backups.keep`.
fn schedule_backups(shared: Arc<Shared>) {
    let _ = thread::Builder::new()
        .name("backups".into())
        .spawn(move || {
            let mut last = Instant::now();
            loop {
                thread::sleep(Duration::from_secs(30));
                let Ok(config) = Config::load(&shared.dir) else {
                    continue;
                };
                let Ok(Some(every)) = backup::interval(&config.backups.every) else {
                    continue;
                };
                if last.elapsed() < every || shared.state.lock().unwrap().phase != Phase::Running {
                    continue;
                }
                last = Instant::now();
                match shared.back_up(&backup::auto_name(std::time::SystemTime::now())) {
                    Ok(path) => {
                        note(&shared, "INFO", &format!("Backed up to {}", path.display()));
                        let pruned = backup::prune(&shared.dir, config.backups.keep as usize);
                        if !pruned.is_empty() {
                            note(
                                &shared,
                                "INFO",
                                &format!("Deleted older backups: {}", pruned.join(", ")),
                            );
                        }
                    }
                    Err(e) => note(
                        &shared,
                        "WARN",
                        &format!("The automatic backup failed: {e}"),
                    ),
                }
            }
        });
}

/// Runs one engine process to its end: console lines in, the log out, and a clean `stop` when asked.
fn supervise(shared: &Arc<Shared>, child: &mut Child, console: &mpsc::Receiver<String>) -> Exit {
    if let Some(out) = child.stdout.take() {
        pump(shared, out, false);
    }
    if let Some(err) = child.stderr.take() {
        pump(shared, err, true);
    }
    let mut stdin = child.stdin.take();
    let mut stopping: Option<Instant> = None;
    let mut restart = false;
    let mut handled = signals::requests();
    loop {
        while let Ok(line) = console.try_recv() {
            let command = line.trim().trim_start_matches('/');
            if command == "stop" && stopping.is_none() {
                stopping = Some(Instant::now());
                shared.set_phase(Phase::Stopping);
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
        let asked = requests > handled || shared.stop.swap(false, Ordering::SeqCst);
        handled = requests;
        if shared.restart.swap(false, Ordering::SeqCst) && stopping.is_none() {
            restart = true;
            note(shared, "INFO", "Restarting the server");
            stopping = Some(Instant::now());
            shared.set_phase(Phase::Restarting);
            send_stop(&mut stdin);
        }
        if asked {
            restart = false;
            if stopping.is_none() {
                note(shared, "INFO", "Stopping the server");
                stopping = Some(Instant::now());
                shared.set_phase(Phase::Stopping);
                send_stop(&mut stdin);
            } else {
                note(shared, "WARN", "Stopping the game at once, without saving");
                let _ = child.kill();
            }
        }
        if stopping.is_some_and(|t| t.elapsed() > STOP_TIMEOUT) {
            note(
                shared,
                "WARN",
                &format!(
                    "The game did not stop within {} s: killing it",
                    STOP_TIMEOUT.as_secs()
                ),
            );
            let _ = child.kill();
            stopping = Some(Instant::now());
        }
        match child.try_wait() {
            Ok(Some(_)) if restart => return Exit::Restart,
            Ok(Some(status)) if stopping.is_some() || status.success() => return Exit::Stopped,
            Ok(Some(status)) => return Exit::Crashed(status),
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                note(shared, "ERROR", &format!("Lost track of the game: {e}"));
                return Exit::Stopped;
            }
        }
    }
}

fn send_stop(stdin: &mut Option<std::process::ChildStdin>) {
    if let Some(input) = stdin.as_mut() {
        let _ = writeln!(input, "stop").and_then(|()| input.flush());
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
