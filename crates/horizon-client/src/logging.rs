//! The game's log, as vanilla's log4j configuration writes it (client-1.21.2.xml, the one
//! 26.3's version manifest names): `logs/latest.log` in the game folder with the pattern
//! `[%d{HH:mm:ss}] [%t/%level]: %msg%n`, rolled over at start (OnStartupTriggeringPolicy, a
//! non-empty file only) and when the day changes (TimeBasedTriggeringPolicy) to
//! `logs/%d{yyyy-MM-dd}-%i.log.gz`, dated by the old file's last write. DefaultRolloverStrategy
//! keeps 7 archives a day, the newest with the highest index.
//!
//! Messages go through the `log` facade. Horizon's own still print their plain text on stderr,
//! as they did before the log existed; other crates' errors only reach the file (their
//! warnings are mostly the Vulkan loader's startup chatter).
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Local, NaiveDate};

/// DefaultRolloverStrategy's default `max`: archives kept for one date.
pub const MAX_ARCHIVES_PER_DAY: u32 = 7;
/// Formatted lines kept in memory for crash reports.
const RECENT: usize = 50;

/// The platform data folder (what the `dirs` crate's data_dir gives).
pub fn data_dir() -> PathBuf {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let home = env("HOME").or_else(|| env("USERPROFILE"));
    if cfg!(windows) {
        if let Some(d) = env("APPDATA") {
            return d;
        }
    } else if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            return h.join("Library/Application Support");
        }
    } else if let Some(d) = env("XDG_DATA_HOME") {
        return d;
    } else if let Some(h) = &home {
        return h.join(".local/share");
    }
    std::env::temp_dir()
}

/// Where the launcher runs the game and keeps its saves: `HORIZON_HOME`, else `horizon` in the
/// platform data folder (launcher/src/backend.rs `game_home`).
pub fn game_home() -> PathBuf {
    match std::env::var_os("HORIZON_HOME").filter(|h| !h.is_empty()) {
        Some(h) => PathBuf::from(h),
        None => data_dir().join("horizon"),
    }
}

/// `logs/` and `crash-reports/` of a game folder.
pub fn logs_dir(home: &Path) -> PathBuf {
    home.join("logs")
}
pub fn crash_reports_dir(home: &Path) -> PathBuf {
    home.join("crash-reports")
}

/// `2026-10-14-3.log.gz`
pub fn archive_name(date: NaiveDate, index: u32) -> String {
    format!("{}-{index}.log.gz", date.format("%Y-%m-%d"))
}

/// The archive index of `name` if it is one of `date`'s archives.
fn archive_index(name: &str, date: NaiveDate) -> Option<u32> {
    let prefix = format!("{}-", date.format("%Y-%m-%d"));
    name.strip_prefix(&prefix)?
        .strip_suffix(".log.gz")?
        .parse()
        .ok()
        .filter(|&i| i > 0)
}

/// What one rollover does to the archives already there.
#[derive(Debug, PartialEq, Eq)]
pub struct Rollover {
    pub delete: Option<String>,
    /// In order: each target is free when its rename runs.
    pub renames: Vec<(String, String)>,
    pub archive: String,
}

/// DefaultRolloverStrategy with fileIndex "max": the new archive takes the next index; once
/// `max` are there the lowest goes and the others move down one.
pub fn rollover(existing: &[String], date: NaiveDate, max: u32) -> Rollover {
    let mut present: Vec<u32> = existing
        .iter()
        .filter_map(|n| archive_index(n, date))
        .collect();
    present.sort_unstable();
    present.dedup();
    let highest = present.last().copied().unwrap_or(0);
    if highest < max {
        return Rollover {
            delete: None,
            renames: Vec::new(),
            archive: archive_name(date, highest + 1),
        };
    }
    let delete = Some(archive_name(date, present[0]));
    let kept = &present[1..];
    let renames = kept
        .iter()
        .enumerate()
        .map(|(i, &from)| (from, i as u32 + 1))
        .filter(|(from, to)| from != to)
        .map(|(from, to)| (archive_name(date, from), archive_name(date, to)))
        .collect();
    Rollover {
        delete,
        renames,
        archive: archive_name(date, kept.len() as u32 + 1),
    }
}

/// `[12:04:31] [Render thread/INFO]: message`
pub fn format_line(
    time: &DateTime<Local>,
    thread: &str,
    level: log::Level,
    message: &str,
) -> String {
    format!(
        "[{}] [{thread}/{level}]: {message}",
        time.format("%H:%M:%S")
    )
}

/// Gzips `text` into the next archive for `date` in `dir`.
fn archive(dir: &Path, date: NaiveDate, text: &[u8]) -> io::Result<()> {
    let names: Vec<String> = fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    let plan = rollover(&names, date, MAX_ARCHIVES_PER_DAY);
    if let Some(name) = &plan.delete {
        fs::remove_file(dir.join(name))?;
    }
    for (from, to) in &plan.renames {
        fs::rename(dir.join(from), dir.join(to))?;
    }
    let mut gz = flate2::write::GzEncoder::new(
        File::create(dir.join(&plan.archive))?,
        flate2::Compression::default(),
    );
    gz.write_all(text)?;
    gz.finish()?.sync_all()
}

/// Moves what `file` holds into an archive dated `date` and empties it. The file stays open
/// (and locked), so this works on Windows too.
fn roll(file: &mut File, dir: &Path, date: NaiveDate) -> io::Result<()> {
    let mut text = Vec::new();
    file.seek(SeekFrom::Start(0))?;
    file.read_to_end(&mut text)?;
    if !text.is_empty() {
        archive(dir, date, &text)?;
    }
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

/// The open `latest.log` and the lock that keeps it this game's.
pub struct Latest {
    pub file: File,
    _lock: File,
}

/// Opens `dir/latest.log` for this run, archiving the last run's. None when another Horizon
/// using the same folder is writing it: that game keeps it, this one logs to stderr only. The
/// lock is a separate file (`.lock`): Windows locks are mandatory, and a locked latest.log could
/// not be read while the game runs.
pub fn open_latest(dir: &Path) -> io::Result<Option<Latest>> {
    fs::create_dir_all(dir)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(".lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => return Ok(None),
        Err(fs::TryLockError::Error(e)) => return Err(e),
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("latest.log"))?;
    let modified = file.metadata()?.modified()?;
    let date = DateTime::<Local>::from(modified).date_naive();
    roll(&mut file, dir, date)?;
    Ok(Some(Latest { file, _lock: lock }))
}

struct State {
    file: Option<Latest>,
    dir: PathBuf,
    /// The day the open file's lines belong to.
    date: NaiveDate,
    recent: VecDeque<String>,
}

struct Logger {
    state: Mutex<State>,
    /// What the main thread is called in the log: "Render thread", or "Server thread" for the
    /// dedicated server, as in vanilla.
    main_thread: &'static str,
}

static LOGGER: std::sync::OnceLock<Logger> = std::sync::OnceLock::new();

fn ours(target: &str) -> bool {
    target.starts_with("horizon")
}

impl log::Log for Logger {
    fn enabled(&self, meta: &log::Metadata) -> bool {
        meta.level() <= log::Level::Info
            && (ours(meta.target()) || meta.level() <= log::Level::Error)
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = record.args().to_string();
        if ours(record.target()) {
            eprintln!("{message}");
        }
        let current = std::thread::current();
        let thread = match current.name() {
            Some("main") => self.main_thread,
            Some(name) => name,
            None => "unnamed",
        };
        self.write(thread, record.level(), &message);
    }

    fn flush(&self) {
        if let Some(latest) = &mut self.state.lock().unwrap_or_else(|e| e.into_inner()).file {
            let _ = latest.file.flush();
        }
    }
}

impl Logger {
    fn write(&self, thread: &str, level: log::Level, message: &str) {
        let now = Local::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let state = &mut *state;
        if now.date_naive() != state.date {
            if let Some(latest) = &mut state.file
                && let Err(e) = roll(&mut latest.file, &state.dir, state.date)
            {
                eprintln!("Horizon: could not roll over the log: {e}");
            }
            state.date = now.date_naive();
        }
        // As log4j writes a multi-line message: the pattern once, the rest as it is.
        for (i, line) in message.lines().enumerate() {
            let line = if i == 0 {
                format_line(&now, thread, level, line)
            } else {
                line.to_owned()
            };
            if let Some(latest) = &mut state.file {
                let _ = writeln!(latest.file, "{line}");
            }
            state.recent.push_back(line);
            if state.recent.len() > RECENT {
                state.recent.pop_front();
            }
        }
    }
}

/// Starts the log in `home/logs`. The main thread shows as `main_thread` in it. Messages are
/// still printed when the file cannot be opened.
pub fn init(home: &Path, main_thread: &'static str) {
    let dir = logs_dir(home);
    let file = match open_latest(&dir) {
        Ok(Some(latest)) => Some(latest),
        Ok(None) => {
            eprintln!(
                "Horizon: another game is writing {}; this one logs to the terminal only",
                dir.join("latest.log").display()
            );
            None
        }
        Err(e) => {
            eprintln!(
                "Horizon: could not open {}: {e}",
                dir.join("latest.log").display()
            );
            None
        }
    };
    let logger = LOGGER.get_or_init(|| Logger {
        state: Mutex::new(State {
            file,
            dir,
            date: Local::now().date_naive(),
            recent: VecDeque::new(),
        }),
        main_thread,
    });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// A line in the log from code that prints it elsewhere itself (the dedicated server console).
pub fn write_only(level: log::Level, message: &str) {
    if let Some(logger) = LOGGER.get() {
        let current = std::thread::current();
        let thread = match current.name() {
            Some("main") | None => logger.main_thread,
            Some(name) => name,
        };
        logger.write(thread, level, message);
    }
}

/// The last lines written, oldest first (for crash reports).
pub fn recent_lines() -> Vec<String> {
    LOGGER.get().map_or_else(Vec::new, |l| {
        l.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recent
            .iter()
            .cloned()
            .collect()
    })
}

/// Flushes the log file (before the process exits).
pub fn flush() {
    if let Some(logger) = LOGGER.get() {
        log::Log::flush(logger);
    }
}

/// The last `lines` lines of a text file.
pub fn tail(path: &Path, lines: usize) -> io::Result<Vec<String>> {
    let text = fs::read(path)?;
    let text = String::from_utf8_lossy(&text);
    let all: Vec<&str> = text.lines().collect();
    Ok(all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|l| l.to_string())
        .collect())
}

/// The newest `crash-*.txt` in a game folder's crash-reports (names sort by time).
pub fn newest_crash_report(home: &Path) -> Option<PathBuf> {
    fs::read_dir(crash_reports_dir(home))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .max_by_key(|p| p.file_name().map(|n| n.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn archives_are_named_like_vanillas() {
        assert_eq!(archive_name(day(4), 1), "2026-10-04-1.log.gz");
        let none: Vec<String> = Vec::new();
        assert_eq!(
            rollover(&none, day(14), 7),
            Rollover {
                delete: None,
                renames: vec![],
                archive: "2026-10-14-1.log.gz".into()
            }
        );
        // Another day's archives and unrelated files do not count.
        let names: Vec<String> = [
            "2026-10-13-1.log.gz",
            "2026-10-13-2.log.gz",
            "2026-10-14-1.log.gz",
            "2026-10-14-2.log.gz",
            "latest.log",
            "debug-1.log.gz",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(rollover(&names, day(14), 7).archive, "2026-10-14-3.log.gz");
        assert_eq!(rollover(&names, day(13), 7).archive, "2026-10-13-3.log.gz");
        assert_eq!(rollover(&names, day(15), 7).archive, "2026-10-15-1.log.gz");
    }

    #[test]
    fn the_eighth_archive_of_a_day_drops_the_oldest() {
        let names: Vec<String> = (1..=7).map(|i| archive_name(day(14), i)).collect();
        let plan = rollover(&names, day(14), 7);
        assert_eq!(plan.delete.as_deref(), Some("2026-10-14-1.log.gz"));
        assert_eq!(plan.renames.len(), 6);
        assert_eq!(
            plan.renames[0],
            ("2026-10-14-2.log.gz".into(), "2026-10-14-1.log.gz".into())
        );
        assert_eq!(
            plan.renames[5],
            ("2026-10-14-7.log.gz".into(), "2026-10-14-6.log.gz".into())
        );
        assert_eq!(plan.archive, "2026-10-14-7.log.gz");
    }

    #[test]
    fn lines_look_like_log4js_pattern() {
        let time = chrono::TimeZone::with_ymd_and_hms(&Local, 2026, 10, 14, 9, 5, 7).unwrap();
        assert_eq!(
            format_line(
                &time,
                "Render thread",
                log::Level::Info,
                "load assets: 1.20s"
            ),
            "[09:05:07] [Render thread/INFO]: load assets: 1.20s"
        );
        assert_eq!(
            format_line(&time, "Worker-1", log::Level::Warn, "x"),
            "[09:05:07] [Worker-1/WARN]: x"
        );
    }

    #[test]
    fn startup_archives_the_last_runs_log() {
        let dir = std::env::temp_dir().join(format!("horizon-logs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Empty: nothing is archived (OnStartupTriggeringPolicy's minSize is 1 byte).
        drop(open_latest(&dir).unwrap().unwrap());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::write(
            dir.join("latest.log"),
            "[10:00:00] [Render thread/INFO]: first run\n",
        )
        .unwrap();
        let mut latest = open_latest(&dir).unwrap().unwrap();
        // A second game in the same folder leaves the file alone.
        assert!(open_latest(&dir).unwrap().is_none());
        writeln!(latest.file, "second run").unwrap();
        drop(latest);
        let today = Local::now().date_naive();
        let archive = dir.join(archive_name(today, 1));
        let mut text = String::new();
        flate2::read::GzDecoder::new(File::open(&archive).unwrap())
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "[10:00:00] [Render thread/INFO]: first run\n");
        assert_eq!(
            fs::read_to_string(dir.join("latest.log")).unwrap(),
            "second run\n"
        );
        assert_eq!(
            tail(&dir.join("latest.log"), 100).unwrap(),
            vec!["second run"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_newest_crash_report_is_found_by_name() {
        let home = std::env::temp_dir().join(format!("horizon-crash-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        assert!(newest_crash_report(&home).is_none());
        let dir = crash_reports_dir(&home);
        fs::create_dir_all(&dir).unwrap();
        for name in [
            "crash-2026-10-13_23.59.59-client.txt",
            "crash-2026-10-14_08.00.01-client.txt",
            "notes.txt",
        ] {
            fs::write(dir.join(name), "x").unwrap();
        }
        assert_eq!(
            newest_crash_report(&home).unwrap(),
            dir.join("crash-2026-10-14_08.00.01-client.txt")
        );
        let _ = fs::remove_dir_all(&home);
    }
}
