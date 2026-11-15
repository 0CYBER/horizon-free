//! A bug report as a local file: what the player wrote, this machine, and optionally the end of
//! the game's log, with the player's names and worlds hidden. Nothing is sent: the player reads
//! the file and attaches it to the issue by hand (docs/telemetry.md).
use crate::account::Accounts;
use crate::backend;
use horizon_system::Machine;
use horizon_telemetry::scrub::Scrubber;
use std::path::PathBuf;

/// The newest log lines kept in a report.
const LOG_LINES: usize = 300;

/// Hides the player's account names and UUIDs and their worlds' folder names, besides what
/// `Scrubber` finds on its own.
fn scrubber() -> Scrubber {
    let mut s = Scrubber::new();
    for account in Accounts::load().accounts {
        s = s.term(account.name).term(account.uuid);
    }
    let saves = std::fs::read_dir(backend::game_home().join("saves"))
        .into_iter()
        .flatten()
        .flatten();
    for world in saves {
        s = s.term(world.file_name().to_string_lossy());
    }
    s
}

/// Writes `bug-reports/bug-report-<time>.txt` in the game folder and returns its path.
pub fn write(text: &str, with_log: bool) -> Result<PathBuf, String> {
    let home = backend::game_home();
    let scrubber = scrubber();
    let machine = Machine::detect_in(Some(&home));
    let mut report = format!(
        "{}\n\nHorizon launcher {}\n{} {} on {}, {} ({} cores, {} threads, {} GB)\n",
        scrubber.scrub(text.trim()),
        backend::VERSION,
        machine.os,
        machine.os_version,
        machine.arch,
        machine.cpu,
        machine.cores,
        machine.threads,
        machine.ram_gb,
    );
    if with_log {
        let (_, log) = backend::read_logs(LOG_LINES, false)?;
        let kept: Vec<&str> = log.lines().filter(|l| !l.contains("[CHAT]")).collect();
        report.push_str("\n--- end of the game's log ---\n");
        report.push_str(&scrubber.scrub(&kept.join("\n")));
        report.push('\n');
    }
    let dir = home.join("bug-reports");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!(
        "bug-report-{}.txt",
        chrono::Local::now().format("%Y-%m-%d_%H.%M.%S")
    ));
    std::fs::write(&path, report).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}
