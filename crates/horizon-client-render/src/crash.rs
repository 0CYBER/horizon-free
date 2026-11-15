//! Crash reports, laid out like vanilla's CrashReport.getFriendlyReport: a comment, the time,
//! a description, the error with its stack, then the details of the game and the machine.
//! Written to `crash-reports/crash-<yyyy-MM-dd_HH.mm.ss>-client.txt` in the game folder when
//! the main thread panics or `main` gets an error back. It stays a local file: the player
//! reads it and shares it by hand.
pub use mod_api::content::crash::{
    ConvertedCall, ConvertedPackage, ConvertedPackageGuard, ConvertedPackages, ModCall, ModGuard,
    current_converted_call, current_native_mod,
};
// Tests run without the executable's startup: the platform's versions install the crash
// backend on first use and keep details, as the started game's do.
#[cfg(not(test))]
use mod_api::content::crash::details;
#[cfg(not(test))]
pub use mod_api::content::crash::{clear_detail, enter_native_mod, set_detail};
#[cfg(test)]
use mod_platform::content::crash::details;
#[cfg(test)]
pub use mod_platform::content::crash::{clear_detail, enter_native_mod, set_detail};
use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use horizon_system::{command_output, machine_arch};

static REPORTED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// How many `contain` calls this thread is inside.
    static CONTAINED: Cell<u32> = const { Cell::new(0) };
    /// The message of the last panic `contain` caught on this thread, from the hook.
    static CAUGHT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// `message`, with the native mod hook this thread is inside, if any, and the converted package
/// the hook's runtime is executing.
fn attributed(message: String) -> String {
    let Some(call) = mod_api::content::crash::current_native_call() else {
        return message;
    };
    let part = call
        .part
        .map(|part| format!(", part {part}"))
        .unwrap_or_default();
    let mut out = format!(
        "{message}\nin native mod {}, hook {}{part}",
        call.id, call.hook
    );
    if let Some(scoped) = mod_api::content::crash::current_converted_call()
        && scoped.runtime == call.id
    {
        // `export` and `site` come from the runtime, and for wasm from module bytes: a name
        // with a newline could forge another line of this report, so keep each on one line.
        let _ = write!(
            out,
            "\nin converted package {}, export {}, call site {}",
            scoped.package,
            scoped.export.escape_debug(),
            scoped.site.escape_debug()
        );
    }
    out
}

/// Runs `f`, and returns a panic in it as an error with the panic's message instead of letting
/// it end the game. For work that is not the game's own and whose failure the caller recovers
/// from, such as handing a shader pack's programs to the GPU driver's compiler: the panic goes
/// to the log with its stack, and no crash report is made for it.
pub fn contain<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    CONTAINED.with(|c| c.set(c.get() + 1));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    CONTAINED.with(|c| c.set(c.get() - 1));
    result.map_err(|payload| {
        CAUGHT
            .with(|c| c.borrow_mut().take())
            .unwrap_or_else(|| payload_text(&*payload))
    })
}

/// For the hook: a panic inside `contain` is logged with its stack and kept for `contain` to
/// return. False for any other panic.
fn contained_panic(info: &std::panic::PanicHookInfo) -> bool {
    if CONTAINED.with(Cell::get) == 0 {
        return false;
    }
    let message = panic_message(info);
    let stack = stack_lines(
        &std::backtrace::Backtrace::force_capture().to_string(),
        true,
    );
    log::error!(
        "Recovered from an error: {message}\n{}",
        stack
            .lines()
            .map(|l| format!("\tat {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    CAUGHT.with(|c| *c.borrow_mut() = Some(message));
    true
}

/// Vanilla picks a line from a list of jokes; these only say what happened.
const COMMENTS: &[&str] = &[
    "Something went wrong.",
    "The game stopped here.",
    "This should not have happened.",
    "Details below.",
];

/// One crash, ready to be written.
pub struct Report {
    pub time: DateTime<Local>,
    /// The lines mods put under the title (`Registrar::crash_header`), `<mod id>: <text>`.
    pub headers: Vec<String>,
    pub description: String,
    /// The error's message, first line of the stack section.
    pub error: String,
    /// Where it happened and how it got there: one frame a line (`function (file:line:col)`).
    pub stack: String,
    pub thread: String,
    /// `client` or `server`, the end of the file name.
    pub side: &'static str,
    pub details: Vec<(String, String)>,
    pub log: Vec<String>,
}

/// `crash-2026-10-14_09.05.07-client.txt`
pub fn file_name(time: &DateTime<Local>, side: &str) -> String {
    format!("crash-{}-{side}.txt", time.format("%Y-%m-%d_%H.%M.%S"))
}

impl Report {
    /// The report's text.
    pub fn text(&self) -> String {
        let comment = COMMENTS[self.time.timestamp().rem_euclid(COMMENTS.len() as i64) as usize];
        let mut out = String::new();
        let _ = writeln!(out, "---- Horizon Crash Report ----");
        for header in &self.headers {
            let _ = writeln!(out, "{header}");
        }
        let _ = writeln!(out, "// {comment}\n");
        let _ = writeln!(out, "Time: {}", self.time.format("%Y-%m-%d %H:%M:%S"));
        let _ = writeln!(out, "Description: {}\n", self.description);
        let _ = writeln!(out, "{}", self.error);
        for line in self.stack.lines() {
            let _ = writeln!(out, "\tat {line}");
        }
        let _ = writeln!(
            out,
            "\n\nA detailed walkthrough of the error, its code path and all known details is as follows:"
        );
        let _ = writeln!(out, "{}\n", "-".repeat(87));
        let _ = writeln!(out, "-- Head --");
        let _ = writeln!(out, "Thread: {}\n", self.thread);
        let _ = writeln!(out, "-- System Details --");
        let _ = writeln!(out, "Details:");
        for (key, value) in &self.details {
            let _ = writeln!(out, "\t{key}: {value}");
        }
        let _ = writeln!(out, "\n-- Last {} log lines --", self.log.len());
        for line in &self.log {
            let _ = writeln!(out, "{line}");
        }
        out
    }

    /// Writes the report in `home/crash-reports` and returns its path.
    pub fn save(&self, home: &Path) -> std::io::Result<PathBuf> {
        let dir = crate::logging::crash_reports_dir(home);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(file_name(&self.time, self.side));
        std::fs::write(&path, self.text())?;
        Ok(path)
    }
}

/// The release this binary is (`v0.2.3`); a nightly's label (`Nightly 2026-10-28 · 0c02067`); for a
/// release from before 0.2, its tag and the client's crate version; for a build from source, that
/// version alone.
pub fn version() -> String {
    if let Some(label) = horizon_core::release::nightly_label() {
        return label;
    }
    let version = env!("CARGO_PKG_VERSION");
    match (horizon_core::release::RELEASE, release_tag()) {
        (Some(_), Some(tag)) => tag,
        (None, Some(tag)) => format!("{tag} (client {version})"),
        (_, None) => version.to_owned(),
    }
}

/// The release this binary came with: the version release.yml built in (docs/versioning.md), else
/// the folder the launcher installed it in, named after the tag (releases from before 0.2). A
/// nightly is its version as the launcher installs it (`nightly-2026-10-28-0c02067`).
pub fn release_tag() -> Option<String> {
    if let Some(nightly) = horizon_core::release::nightly_version() {
        return Some(nightly);
    }
    if let Some(version) = horizon_core::release::RELEASE {
        return Some(format!("v{version}"));
    }
    let exe = std::env::current_exe().ok()?;
    let folder = exe.parent()?.file_name()?.to_str()?;
    (folder.starts_with('v') && folder[1..].starts_with(|c: char| c.is_ascii_digit()))
        .then(|| folder.to_owned())
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// The build's architecture, and the machine's after "on" when it emulates the build.
fn arch_label(build: &str, machine: &str) -> String {
    if build == machine {
        build.to_owned()
    } else {
        format!("{build} on {machine}")
    }
}

/// `PRETTY_NAME` of /etc/os-release and the kernel, or the system's own version string.
pub(crate) fn operating_system() -> String {
    let arch = arch_label(std::env::consts::ARCH, machine_arch());
    let version = if cfg!(target_os = "linux") {
        let name = read("/etc/os-release").and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_owned())
        });
        let kernel = read("/proc/sys/kernel/osrelease").map(|k| k.trim().to_owned());
        match (name, kernel) {
            (Some(n), Some(k)) => Some(format!("{n}, kernel {k}")),
            (n, k) => n.or(k),
        }
    } else if cfg!(target_os = "macos") {
        command_output("sw_vers", &["-productVersion"]).map(|v| format!("macOS {v}"))
    } else if cfg!(windows) {
        horizon_system::windows_version()
    } else {
        None
    };
    format!(
        "{} ({arch}) version {}",
        std::env::consts::OS,
        version.unwrap_or_else(|| "unknown".into())
    )
}

/// Logical processors and the CPU's name (`12x AMD Ryzen 5 5600X ...`).
pub fn cpu() -> String {
    let threads = std::thread::available_parallelism().map_or(0, |n| n.get());
    let name = if cfg!(target_os = "linux") {
        read("/proc/cpuinfo").and_then(|t| horizon_system::linux_cpu_name(&t))
    } else if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else if cfg!(windows) {
        horizon_system::windows_cpu()
    } else {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    };
    format!(
        "{threads}x {}",
        name.unwrap_or_else(|| std::env::consts::ARCH.into())
    )
}

/// Total memory and what this process uses, in MiB where the system says.
fn memory() -> String {
    const MIB: u64 = 1024 * 1024;
    let kib = |text: &str, key: &str| -> Option<u64> {
        text.lines()
            .find(|l| l.starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    let (total, used) = if cfg!(target_os = "linux") {
        (
            read("/proc/meminfo")
                .and_then(|t| kib(&t, "MemTotal:"))
                .map(|k| k * 1024),
            read("/proc/self/status")
                .and_then(|t| kib(&t, "VmRSS:"))
                .map(|k| k * 1024),
        )
    } else if cfg!(target_os = "macos") {
        (
            command_output("sysctl", &["-n", "hw.memsize"]).and_then(|v| v.parse().ok()),
            None,
        )
    } else if cfg!(windows) {
        (
            command_output(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
                ],
            )
            .and_then(|v| v.parse().ok()),
            None,
        )
    } else {
        (None, None)
    };
    match (total, used) {
        (Some(t), Some(u)) => format!(
            "{} MiB used by the game, {} MiB installed",
            u / MIB,
            t / MIB
        ),
        (Some(t), None) => format!("{} MiB installed", t / MIB),
        _ => "unknown".into(),
    }
}

/// The report's details: Horizon's and the machine's, then what the game set as it ran. A dedicated
/// server renders nothing: it leaves out the render settings it read from options.txt.
pub fn system_details(side: &str) -> Vec<(String, String)> {
    let mut out = vec![
        ("Horizon Version".to_owned(), version()),
        (
            "Minecraft Version".to_owned(),
            horizon_core::VANILLA_VERSION.to_owned(),
        ),
        ("Operating System".to_owned(), operating_system()),
        ("CPU".to_owned(), cpu()),
        ("Memory".to_owned(), memory()),
    ];
    out.extend(
        details()
            .into_iter()
            .filter(|(k, _)| side != "server" || *k != "Render Settings")
            .map(|(k, v)| (k.to_owned(), v)),
    );
    out.extend(mod_lines(false));
    out
}

/// The lines mods put under a report's title, `<mod id>: <text>`.
fn mod_headers() -> Vec<String> {
    mod_lines(true)
        .into_iter()
        .map(|(name, text)| format!("{name}: {text}"))
        .collect()
}

/// How long a report waits for the mods' lines, all of them together: a line waiting on a lock the
/// crashing thread holds must not keep the report from being written.
const MOD_LINES_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// The mods' `Registrar::crash_header` lines (`headers`) or `crash_section` lines, in registration
/// order, as `(<mod id>: <label>, text)`, or `(<mod id>, text)` for a header. Each is asked once,
/// under [`contain`], on a guest-sized thread of its own: this runs inside the panic hook, where a
/// second panic would end the process before any catch could. A line that panics, traps or misses
/// [`MOD_LINES_WAIT`] reads `~~ERROR~~`; one that answers `None` is left out, as are the lines of
/// the mod this thread's crash is attributed to. No mod line, no thread.
fn mod_lines(headers: bool) -> Vec<(String, String)> {
    let faulted = current_native_mod();
    let lines: Vec<_> = mod_api::__private::game::mod_crash_lines()
        .into_iter()
        .filter(|(owner, label, _)| label.is_none() == headers && Some(*owner) != faulted)
        .collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let (send, answers) = std::sync::mpsc::channel();
    let asked: Vec<_> = lines
        .iter()
        .map(|(owner, _, line)| (*owner, line.clone()))
        .collect();
    // A thread that doesn't start drops `send`: every line then reads as an error.
    let _ = horizon_guest_stack::spawn("Crash report lines".into(), move || {
        let failures = horizon_mod_runtime::dispatch_failures::dispatch_failure_counter();
        for (owner, line) in asked {
            let before = failures.get();
            let answer = contain(|| {
                let _mod = enter_native_mod(ModCall {
                    id: owner,
                    hook: "crash_section",
                    part: None,
                });
                line.value()
            });
            // A wasm line that traps answers `None` through its proxy; the failed call tells.
            if send
                .send(answer.ok().filter(|_| failures.get() == before))
                .is_err()
            {
                break;
            }
        }
    });
    let deadline = std::time::Instant::now() + MOD_LINES_WAIT;
    let mut waiting = true;
    lines
        .into_iter()
        .filter_map(|(owner, label, _)| {
            // After one line misses the wait, a later answer would belong to that line, not this.
            let answer = waiting
                .then(|| {
                    answers
                        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                        .ok()
                })
                .flatten();
            waiting = answer.is_some();
            let text = match answer {
                Some(Some(None)) => return None,
                Some(Some(Some(text))) => one_line(&text),
                _ => "~~ERROR~~".to_owned(),
            };
            let name = match label {
                Some(label) => format!("{owner}: {}", one_line(&label)),
                None => owner.to_owned(),
            };
            Some((name, text))
        })
        .collect()
}

/// `text` on one line, so no mod line passes for one of the report's own: control characters
/// become spaces, and it is cut to 4096 bytes at a character boundary.
fn one_line(text: &str) -> String {
    let mut end = text.len().min(4096);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end]
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// The thread's name as vanilla's reports say it: the main thread is the client's "Render thread"
/// and the dedicated server's "Server thread".
fn thread_name(side: &str) -> String {
    let current = std::thread::current();
    match current.name() {
        Some("main") | None if side == "server" => "Server thread".into(),
        Some("main") | None => "Render thread".into(),
        Some(name) => name.into(),
    }
}

/// What starts the stderr line naming a saved report, the path follows: the Launch API's
/// promise to launchers (docs/api/launch.md), so it never changes within a line of that API.
pub const SAVED_LINE: &str = "Horizon: crash report saved to ";

/// What runs on each report before it is saved ([`on_report`]).
pub type ReportHook = fn(&Path, &Report);

static REPORT_HOOKS: std::sync::RwLock<Vec<ReportHook>> = std::sync::RwLock::new(Vec::new());

/// Runs `hook` with every crash report, before it is saved: the game folder and the report. An
/// extension point for what lives above the engine (Horizon's opt-in telemetry keeps the
/// report). A hook runs inside the crash handler: it must not panic or block.
pub fn on_report(hook: ReportHook) {
    REPORT_HOOKS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push(hook);
}

type EmergencyHook = std::sync::Arc<dyn Fn() + Send + Sync>;
static EMERGENCY_SAVE: Mutex<Option<EmergencyHook>> = Mutex::new(None);
const SAVE_NOT_STARTED: u8 = 0;
const SAVE_RUNNING: u8 = 1;
const SAVE_FINISHED: u8 = 2;
// Taking the hook must not let a second crashing worker exit before the save finishes.
static EMERGENCY_SAVE_STATE: AtomicU8 = AtomicU8::new(SAVE_NOT_STARTED);

/// Unregisters this emergency save, without removing a newer registration.
#[must_use]
pub struct EmergencySave(EmergencyHook);

impl Drop for EmergencySave {
    fn drop(&mut self) {
        let mut hook = EMERGENCY_SAVE.lock().unwrap_or_else(|e| e.into_inner());
        if hook
            .as_ref()
            .is_some_and(|f| std::sync::Arc::ptr_eq(f, &self.0))
        {
            *hook = None;
        }
    }
}

/// Replaces the process's emergency save. The callback must not panic and must
/// bound its wait: it runs on the crashing worker, before the process exits.
pub fn on_emergency_save(f: impl Fn() + Send + Sync + 'static) -> EmergencySave {
    let hook: EmergencyHook = std::sync::Arc::new(f);
    *EMERGENCY_SAVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook.clone());
    EmergencySave(hook)
}

fn emergency_save() {
    // Leave room for the integrated server's own 30 s save timeout.
    emergency_save_with_timeout(Duration::from_secs(35));
}

/// Run once per process; other crashing workers wait rather than cutting the save short.
fn emergency_save_with_timeout(timeout: Duration) {
    if EMERGENCY_SAVE_STATE
        .compare_exchange(
            SAVE_NOT_STARTED,
            SAVE_RUNNING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        let started = Instant::now();
        while EMERGENCY_SAVE_STATE.load(Ordering::Acquire) == SAVE_RUNNING {
            let Some(left) = timeout.checked_sub(started.elapsed()) else {
                break;
            };
            std::thread::sleep(left.min(Duration::from_millis(5)));
        }
        return;
    }
    // Never wait on the registry in a panic hook, or hold it while stopping the server.
    let hook = EMERGENCY_SAVE.try_lock().ok().and_then(|mut h| h.take());
    if let Some(hook) = hook {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook()));
    }
    EMERGENCY_SAVE_STATE.store(SAVE_FINISHED, Ordering::Release);
}

/// Writes the report and tells where it is; the log gets the error too.
fn deliver(home: &Path, report: Report) -> Option<PathBuf> {
    if REPORTED.swap(true, Ordering::SeqCst) {
        return None;
    }
    // The panic is on stderr already: the stack goes to the log only.
    crate::logging::write_only(
        log::Level::Error,
        &format!(
            "Horizon crashed: {}\n{}",
            report.error,
            report
                .stack
                .lines()
                .map(|l| format!("\tat {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );
    crate::logging::flush();
    for hook in REPORT_HOOKS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
    {
        hook(home, &report);
    }
    // A player of a shared game folder (`--player-dir`) finds the report beside its own log.
    match report.save(crate::logging::player_dir_override().unwrap_or(home)) {
        Ok(path) => {
            eprintln!("{SAVED_LINE}{}", path.display());
            Some(path)
        }
        Err(e) => {
            eprintln!("Horizon: could not save a crash report: {e}");
            eprintln!("{}", report.text());
            None
        }
    }
}

/// The report for a hung server tick, written by the watchdog thread (`ServerWatchdog`): the
/// stack is the server thread's, read from the watchdog (`horizon_threads::stack_dump`), as
/// vanilla's report carries the server thread's stack; without it, the details say why.
pub fn report_watchdog(
    home: &Path,
    side: &'static str,
    message: String,
    server_stack: Result<String, horizon_threads::stack_dump::NoStack>,
) -> Option<PathBuf> {
    let mut details = system_details(side);
    let stack = match server_stack {
        Ok(stack) => stack_lines(&stack, false),
        Err(why) => {
            details.push(("Server thread stack".into(), why.to_string()));
            String::new()
        }
    };
    deliver(
        home,
        Report {
            time: Local::now(),
            headers: mod_headers(),
            description: "Watching Server".into(),
            error: format!("{message} (Watching Server)"),
            stack,
            thread: thread_name(side),
            side,
            details,
            log: crate::logging::recent_lines(),
        },
    )
}

/// The report for a game that went over the player's RAM limit (`ram_limit`), written by the
/// watcher thread: it says nothing about where the memory went, so it carries no stack. The player
/// chose this limit, and the game keeps running, over its limit, until the report is written.
pub fn report_memory_limit(home: &Path, side: &'static str, message: String) {
    if REPORTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let report = Report {
        time: Local::now(),
        headers: mod_headers(),
        description: "RAM limit reached".into(),
        error: message,
        stack: String::new(),
        thread: thread_name(side),
        side,
        details: system_details(side),
        log: crate::logging::recent_lines(),
    };
    match report.save(home) {
        Ok(path) => eprintln!("{SAVED_LINE}{}", path.display()),
        Err(e) => eprintln!("Horizon: could not save a crash report: {e}"),
    }
}

fn payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "Box<dyn Any>".into())
}

fn panic_message(info: &std::panic::PanicHookInfo) -> String {
    let text = payload_text(info.payload());
    attributed(match info.location() {
        Some(at) => format!("panicked at {at}:\n{text}"),
        None => format!("panicked:\n{text}"),
    })
}

/// A `std::backtrace` as one line a frame, `function (file:line:col)`. Captured in a panic hook
/// (`from_panic`), it starts at the frame that panicked: the hook's and the panic machinery's
/// frames go. Paths inside the workspace lose the build machine's prefix.
pub fn stack_lines(backtrace: &str, from_panic: bool) -> String {
    // The workspace as it was on the machine that built this binary (it need not exist here).
    // A report names workspace files the same way on every host, whatever
    // separator the build machine used.
    let manifest = env!("CARGO_MANIFEST_DIR").replace('\\', "/");
    let root = manifest
        .strip_suffix("crates/horizon-client-render")
        .unwrap_or(&manifest);
    let mut frames: Vec<(String, Option<String>)> = Vec::new();
    for line in backtrace.lines() {
        let line = line.trim();
        if let Some(at) = line.strip_prefix("at ") {
            if let Some(frame) = frames.last_mut() {
                frame.1 = Some(short_path(&at.replace('\\', "/"), root).to_owned());
            }
        } else if let Some((n, name)) = line.split_once(": ")
            && n.chars().all(|c| c.is_ascii_digit())
        {
            frames.push((name.to_owned(), None));
        }
    }
    let machinery = |name: &str| {
        [
            "std::panicking::",
            "core::panicking::",
            "std::sys::backtrace::",
            "__rustc::",
        ]
        .iter()
        .any(|p| name.starts_with(p))
            || name.contains("rust_begin_unwind")
    };
    // The hook's frames, then the panic machinery's, then the code that panicked.
    let top = frames.iter().take(30).position(|(name, _)| machinery(name));
    let first = match top.filter(|_| from_panic) {
        Some(start) => {
            start
                + frames[start..]
                    .iter()
                    .take_while(|(name, _)| machinery(name))
                    .count()
        }
        None => 0,
    };
    // Some platforms keep std's own capture frames (`<std::backtrace::Backtrace>::create`).
    let first = first
        + frames[first..]
            .iter()
            .take_while(|(name, _)| name.trim_start_matches('<').starts_with("std::backtrace"))
            .count();
    frames[first..]
        .iter()
        .map(|(name, at)| match at {
            Some(at) => format!("{name} ({at})\n"),
            None => format!("{name}\n"),
        })
        .collect()
}

/// A source path without the build machine's folders: relative to the workspace, or from the
/// crate's folder for dependencies (`winit-0.30.13/src/event_loop.rs:642:56`).
fn short_path<'a>(path: &'a str, root: &str) -> &'a str {
    if let Some(rest) = path.strip_prefix(root) {
        return rest;
    }
    if let Some(i) = path.find("/registry/src/") {
        let rest = &path[i + "/registry/src/".len()..];
        if let Some((_index, krate)) = rest.split_once('/') {
            return krate;
        }
    }
    path
}

/// A report for a panic, with the stack captured now (RUST_BACKTRACE or not).
pub fn panic_report(info: &std::panic::PanicHookInfo, side: &'static str) -> Report {
    Report {
        time: Local::now(),
        headers: mod_headers(),
        description: "Unexpected error".into(),
        error: panic_message(info),
        stack: stack_lines(
            &std::backtrace::Backtrace::force_capture().to_string(),
            true,
        ),
        thread: thread_name(side),
        side,
        details: system_details(side),
        log: crate::logging::recent_lines(),
    }
}

/// Shared with the integrated server spawn so reporting follows its exact name.
pub const INTEGRATED_SERVER_THREAD: &str = "Server thread";

static SERVER_CRASHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the server has already delivered the fatal report.
pub fn server_crashed() -> bool {
    SERVER_CRASHED.load(std::sync::atomic::Ordering::Acquire)
}

/// Whether `name` is one of the shared worker pool's threads (`horizon_threads::jobs`), vanilla's
/// `Util` worker threads.
fn is_worker_thread(name: Option<&str>) -> bool {
    name.is_some_and(|n| n.starts_with("horizon-worker-"))
}

/// `Util.onThreadException`: a panic on a worker thread is made a report titled "Exception on
/// worker thread", with the thread's name, as `BlockableEventLoop.relayDelayCrash` hands it to
/// the main thread.
pub fn worker_panic_report(info: &std::panic::PanicHookInfo, side: &'static str) -> Report {
    Report {
        description: "Exception on worker thread".into(),
        ..panic_report(info, side)
    }
}

/// Reports panics: on the main thread a crash report (the panic then ends the game, exit code
/// 101); on the integrated server thread a server report relayed to the window; on a worker thread (`Util.onThreadException`) a crash report too, and the game ends
/// with it; on other threads the error goes to the log, as vanilla's uncaught exception handler
/// does. The default hook still prints the panic. A panic inside `contain` is only logged. A
/// panic inside a native mod's hook names the mod and the hook.
pub fn install(home: PathBuf, side: &'static str) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if contained_panic(info) {
            return;
        }
        default(info);
        let main = std::thread::current().name() == Some("main");
        if std::thread::current().name() == Some(INTEGRATED_SERVER_THREAD) {
            // `MinecraftServer.runServer` saves its own `-server.txt` report before
            // `onServerCrash` hands it to the client, which then only prints its path.
            let report = Report {
                description: "Exception in server tick loop".into(),
                ..panic_report(info, "server")
            };
            deliver(&home, report);
            SERVER_CRASHED.store(true, std::sync::atomic::Ordering::Release);
        } else if main {
            deliver(&home, panic_report(info, side));
        } else if is_worker_thread(std::thread::current().name()) {
            deliver(&home, worker_panic_report(info, side));
            emergency_save();
            // The crash ends the game, as the relayed report does on vanilla's main thread.
            std::process::exit(101);
        } else {
            log::error!(
                "Caught previously unhandled exception in thread {}: {}",
                thread_name(side),
                panic_message(info)
            );
        }
    }));
}

/// Vanilla's debug crash (`key.debug.crash` held for ten seconds): the error `main` gets back,
/// with the stack where it was raised.
#[derive(Debug)]
pub struct DebugCrash {
    stack: String,
}

impl DebugCrash {
    pub const MESSAGE: &str = "Manually triggered debug crash";
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            stack: stack_lines(
                &std::backtrace::Backtrace::force_capture().to_string(),
                false,
            ),
        }
    }
}

impl std::fmt::Display for DebugCrash {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(Self::MESSAGE)
    }
}

impl std::error::Error for DebugCrash {}

/// A report for an error `main` got back: the game ends with it.
pub fn report_error(
    home: &Path,
    side: &'static str,
    error: &(dyn std::error::Error + 'static),
) -> Option<PathBuf> {
    if let Some(debug) = error.downcast_ref::<DebugCrash>() {
        return deliver(
            home,
            Report {
                time: Local::now(),
                headers: mod_headers(),
                description: DebugCrash::MESSAGE.into(),
                error: DebugCrash::MESSAGE.into(),
                stack: debug.stack.clone(),
                thread: thread_name(side),
                side,
                details: system_details(side),
                log: crate::logging::recent_lines(),
            },
        );
    }
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(e) = source {
        let _ = write!(message, "\nCaused by: {e}");
        source = e.source();
    }
    deliver(
        home,
        Report {
            time: Local::now(),
            headers: mod_headers(),
            description: "Fatal error".into(),
            error: message,
            stack: String::new(),
            thread: thread_name(side),
            side,
            details: system_details(side),
            log: crate::logging::recent_lines(),
        },
    )
}

/// Called every frame: panics once `HORIZON_DEBUG_CRASH` asks (development builds only).
pub fn debug_crash_tick() {
    static AT: std::sync::OnceLock<Option<(std::time::Instant, std::time::Duration)>> =
        std::sync::OnceLock::new();
    if let Some((first, after)) =
        AT.get_or_init(|| debug_crash_after().map(|d| (std::time::Instant::now(), d)))
        && first.elapsed() >= *after
    {
        panic!("Manually triggered debug crash");
    }
}

/// Development builds crash on purpose when `HORIZON_DEBUG_CRASH` is set (vanilla has F3+C held
/// for ten seconds): the number is seconds after the first frame (default 5).
pub fn debug_crash_after() -> Option<std::time::Duration> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let value = std::env::var("HORIZON_DEBUG_CRASH").ok()?;
    let seconds = value.parse::<f64>().unwrap_or(5.0).max(0.0);
    Some(std::time::Duration::from_secs_f64(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::__private::game::*;

    /// The panic hook is process-wide: tests that swap it hold this so they never overlap.
    static PANIC_HOOK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn reporting_threads_write_once_and_contained_server_panics_write_nothing() {
        let _lock = PANIC_HOOK.lock().unwrap_or_else(|e| e.into_inner());
        for (name, native, contained, count, title) in [
            (
                "Server thread",
                false,
                false,
                1,
                "Exception in server tick loop",
            ),
            (
                "Server thread",
                true,
                false,
                1,
                "Exception in server tick loop",
            ),
            ("Server thread", false, true, 0, ""),
            ("main", false, false, 1, "Unexpected error"),
        ] {
            let scratch = tempfile::tempdir().unwrap();
            let home = scratch.path().to_path_buf();
            let previous = std::panic::take_hook();
            REPORTED.store(false, Ordering::SeqCst);
            install(
                home.clone(),
                if name == "main" { "server" } else { "client" },
            );
            let result = std::thread::Builder::new()
                .name(name.into())
                .spawn(move || {
                    let crash = || {
                        let _guard = native.then(|| {
                            enter_native_mod(ModCall {
                                id: "example:broken",
                                hook: "server_tick",
                                part: None,
                            })
                        });
                        panic!("posted server crash 3494");
                    };
                    if contained {
                        assert!(contain(crash).is_err());
                    } else {
                        crash();
                    }
                })
                .unwrap()
                .join();
            std::panic::set_hook(previous);
            assert_eq!(result.is_ok(), contained);
            let reports: Vec<_> = std::fs::read_dir(home.join("crash-reports"))
                .into_iter()
                .flatten()
                .map(|p| p.unwrap().path())
                .collect();
            assert_eq!(reports.len(), count);
            if count == 1 {
                // Both are server reports: `main` is installed as a dedicated server here,
                // and the integrated server thread writes its own even under a client install.
                let file = reports[0]
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                assert!(file.ends_with("-server.txt"), "{file}");
                let text = std::fs::read_to_string(&reports[0]).unwrap();
                assert!(text.contains(&format!("Description: {title}")));
                assert!(text.contains("posted server crash 3494"));
                if name == "Server thread" {
                    assert!(text.contains("Server thread"));
                }
                if native {
                    assert!(text.contains("in native mod example:broken, hook server_tick"));
                }
            }
            REPORTED.store(false, Ordering::SeqCst);
            SERVER_CRASHED.store(false, std::sync::atomic::Ordering::Release);
        }
    }

    static EMERGENCY_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn reset_emergency_save() {
        *EMERGENCY_SAVE.lock().unwrap() = None;
        EMERGENCY_SAVE_STATE.store(SAVE_NOT_STARTED, Ordering::Release);
    }

    #[test]
    fn emergency_save_second_caller_waits_for_save() {
        let _serial = EMERGENCY_TEST_LOCK.lock().unwrap();
        reset_emergency_save();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let saved = std::sync::Arc::new(AtomicBool::new(false));
        let (started, running) = std::sync::mpsc::channel();
        let (count, flag) = (calls.clone(), saved.clone());
        let _hook = on_emergency_save(move || {
            count.fetch_add(1, Ordering::Relaxed);
            started.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::Release);
        });
        let first = std::thread::spawn(emergency_save);
        running.recv_timeout(Duration::from_secs(2)).unwrap();
        let second = std::thread::spawn(move || {
            emergency_save();
            assert!(saved.load(Ordering::Acquire));
        });
        first.join().unwrap();
        second.join().unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn emergency_save_waiter_times_out() {
        let _serial = EMERGENCY_TEST_LOCK.lock().unwrap();
        reset_emergency_save();
        EMERGENCY_SAVE_STATE.store(SAVE_RUNNING, Ordering::Release);
        let bound = Duration::from_millis(20);
        let started = Instant::now();
        emergency_save_with_timeout(bound);
        assert!(started.elapsed() >= bound);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn emergency_save_registry_calls_once_replaces_and_unregisters() {
        use std::sync::{Arc, atomic::AtomicUsize};
        let _serial = EMERGENCY_TEST_LOCK.lock().unwrap();
        reset_emergency_save();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let old = on_emergency_save(move || {
            count.fetch_add(1, Ordering::Relaxed);
        });
        emergency_save();
        emergency_save();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        reset_emergency_save();
        let count = calls.clone();
        let replaced = on_emergency_save(move || {
            count.fetch_add(10, Ordering::Relaxed);
        });
        let count = calls.clone();
        let latest = on_emergency_save(move || {
            count.fetch_add(100, Ordering::Relaxed);
        });
        drop(old);
        drop(replaced);
        emergency_save();
        assert_eq!(calls.load(Ordering::Relaxed), 101);
        reset_emergency_save();
        let count = calls.clone();
        let removed = on_emergency_save(move || {
            count.fetch_add(1000, Ordering::Relaxed);
        });
        drop(removed);
        emergency_save();
        assert_eq!(calls.load(Ordering::Relaxed), 101);
        drop(latest);
    }

    #[test]
    fn an_emulated_build_names_both_architectures() {
        // A Windows ARM64 PC playing the x64 build: its reports say so (#7513 read "x86_64").
        assert_eq!(arch_label("x86_64", "aarch64"), "x86_64 on aarch64");
        assert_eq!(arch_label("aarch64", "aarch64"), "aarch64");
        assert_eq!(arch_label("x86_64", "x86_64"), "x86_64");
        // On the machine running the tests: native builds name one architecture.
        let label = arch_label(std::env::consts::ARCH, machine_arch());
        assert!(label.starts_with(std::env::consts::ARCH));
    }

    #[test]
    fn a_release_reports_its_built_in_version() {
        // release.yml builds with HORIZON_VERSION (docs/versioning.md): `v0.2.3`, wherever it is installed.
        if let Some(label) = horizon_core::release::nightly_label() {
            assert_eq!(super::version(), label);
            assert_eq!(release_tag(), horizon_core::release::nightly_version());
            return;
        }
        match horizon_core::release::RELEASE {
            Some(version) => {
                assert_eq!(release_tag(), Some(format!("v{version}")));
                assert_eq!(super::version(), format!("v{version}"));
            }
            None => assert!(release_tag().is_none() || super::version().contains("(client ")),
        }
    }

    #[test]
    fn a_panic_in_a_native_mods_hook_names_it() {
        let outer = enter_native_mod(ModCall {
            id: "outer",
            hook: "render",
            part: None,
        });
        let inner = enter_native_mod(ModCall {
            id: "b",
            hook: "tick",
            part: None,
        });
        assert!(attributed("boom".into()).ends_with("\nin native mod b, hook tick"));
        drop(inner);
        assert!(attributed("boom".into()).ends_with("\nin native mod outer, hook render"));
        drop(outer);
        assert_eq!(attributed("boom".into()), "boom");
        let part = enter_native_mod(ModCall {
            id: "c",
            hook: "analog_output",
            part: Some("c::Gauge"),
        });
        assert!(
            attributed("boom".into())
                .ends_with("\nin native mod c, hook analog_output, part c::Gauge")
        );
        drop(part);
    }

    #[test]
    fn a_panic_in_a_converted_package_names_the_runtime_package_export_and_site() {
        let runtime = enter_native_mod(ModCall {
            id: "converter_runtime",
            hook: "client_tick_pre",
            part: None,
        });
        let packages = ConvertedPackages::new("converter_runtime", ["example:mod"]);
        let package = packages.package("example:mod").unwrap();
        let scope = package
            .enter("tick", "com.example.Foo.tick(Foo.java:42)")
            .unwrap();
        assert!(attributed("boom".into()).ends_with(
            "\nin native mod converter_runtime, hook client_tick_pre\n\
             in converted package example:mod, export tick, \
             call site com.example.Foo.tick(Foo.java:42)"
        ));
        // Another mod's hook inside the package keeps its own attribution: the runtime's
        // package is not named for a crash in somebody else's code.
        let other = enter_native_mod(ModCall {
            id: "other",
            hook: "tick",
            part: None,
        });
        assert_eq!(
            attributed("boom".into()),
            "boom\nin native mod other, hook tick"
        );
        drop(other);
        assert!(
            attributed("boom".into()).contains("\nin converted package example:mod, export tick")
        );
        drop(scope);
        assert!(
            attributed("boom".into())
                .ends_with("\nin native mod converter_runtime, hook client_tick_pre")
        );
        drop(runtime);
        assert_eq!(attributed("boom".into()), "boom");
    }

    #[test]
    fn a_guest_export_cannot_forge_a_report_line() {
        let runtime = enter_native_mod(ModCall {
            id: "converter_runtime",
            hook: "client_tick_pre",
            part: None,
        });
        let packages = ConvertedPackages::new("converter_runtime", ["example:mod"]);
        let scope = packages
            .package("example:mod")
            .unwrap()
            .enter(
                "tick\nin native mod forged, hook render",
                "Foo.tick(\nforged)",
            )
            .unwrap();
        let report = attributed("boom".into());
        // One attributed line each: the guest's text stays inside them, escaped.
        assert_eq!(
            report
                .lines()
                .filter(|line| line.starts_with("in native mod"))
                .count(),
            1
        );
        assert_eq!(
            report
                .lines()
                .filter(|line| line.starts_with("in converted package"))
                .count(),
            1
        );
        assert!(report.contains(r"export tick\nin native mod forged, hook render"));
        assert!(report.contains(r"call site Foo.tick(\nforged)"));
        drop(scope);
        drop(runtime);
    }

    #[test]
    fn a_panic_in_a_converted_package_becomes_a_report() {
        let caught = std::sync::Arc::new(std::sync::Mutex::new(None));
        let sink = caught.clone();
        let _hook = PANIC_HOOK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if panic_message(info).contains("the guest crash") {
                *sink.lock().unwrap() = Some(panic_report(info, "client"));
            }
        }));
        let _runtime = enter_native_mod(ModCall {
            id: "runtime",
            hook: "dispatch",
            part: None,
        });
        let packages = ConvertedPackages::new("runtime", ["example:mod"]);
        let package = packages.package("example:mod").unwrap();
        let _ = std::panic::catch_unwind(|| {
            let _scope = package.enter("tick", "Foo.tick(Foo.java:42)").unwrap();
            panic!("the guest crash");
        });
        std::panic::set_hook(previous);
        let report = caught.lock().unwrap().take().expect("the hook saw it");
        assert!(
            report.error.ends_with(
                "\nin native mod runtime, hook dispatch\n\
                 in converted package example:mod, export tick, call site Foo.tick(Foo.java:42)"
            ),
            "{}",
            report.error
        );
        // The scope is gone once the report is written.
        assert_eq!(
            attributed("boom".into()),
            "boom\nin native mod runtime, hook dispatch"
        );
    }

    fn report() -> Report {
        Report {
            time: chrono::TimeZone::with_ymd_and_hms(&Local, 2026, 10, 14, 9, 5, 7).unwrap(),
            headers: Vec::new(),
            description: "Unexpected error".into(),
            error: "panicked at src/game.rs:10:5:\nboom".into(),
            stack: "horizon_client::game::frame (src/game.rs:10:5)\n".into(),
            thread: "Render thread".into(),
            side: "client",
            details: vec![
                ("Horizon Version".into(), "0.1.0".into()),
                ("Minecraft Version".into(), "26.3".into()),
                ("Graphics".into(), "Test GPU (Vulkan), driver 1.0".into()),
            ],
            log: vec!["[09:05:06] [Render thread/INFO]: load assets: 1.20s".into()],
        }
    }

    #[test]
    fn file_names_are_vanillas() {
        assert_eq!(
            file_name(&report().time, "client"),
            "crash-2026-10-14_09.05.07-client.txt"
        );
    }

    #[test]
    fn the_report_reads_like_vanillas() {
        let text = report().text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "---- Horizon Crash Report ----");
        assert!(lines[1].starts_with("// "));
        assert_eq!(lines[3], "Time: 2026-10-14 09:05:07");
        assert_eq!(lines[4], "Description: Unexpected error");
        assert_eq!(lines[6], "panicked at src/game.rs:10:5:");
        assert_eq!(lines[7], "boom");
        assert_eq!(
            lines[8],
            "\tat horizon_client::game::frame (src/game.rs:10:5)"
        );
        assert!(text.contains("-- Head --\nThread: Render thread\n"));
        assert!(text.contains(
            "-- System Details --\nDetails:\n\tHorizon Version: 0.1.0\n\tMinecraft Version: 26.3\n\tGraphics: Test GPU (Vulkan), driver 1.0\n"
        ));
        assert!(text.ends_with(
            "-- Last 1 log lines --\n[09:05:06] [Render thread/INFO]: load assets: 1.20s\n"
        ));
    }

    #[test]
    fn a_dedicated_servers_report_names_its_server_thread_and_no_render_settings() {
        let unnamed = |side: &'static str| {
            std::thread::spawn(move || thread_name(side))
                .join()
                .unwrap()
        };
        assert_eq!(unnamed("server"), "Server thread");
        assert_eq!(unnamed("client"), "Render thread");
        set_detail("Render Settings", "render distance 12");
        let has = |side| {
            system_details(side)
                .iter()
                .any(|(k, _)| k == "Render Settings")
        };
        assert!(has("client"));
        assert!(!has("server"));
        clear_detail("Render Settings");
    }

    #[test]
    fn stacks_start_at_the_panicking_frame() {
        let bt = format!(
            "   0: horizon_client::crash::panic_report\n             at {root}/src/crash.rs:296:16\n   1: std::panicking::panic_with_hook\n             at /rustc/abc/library/std/src/panicking.rs:823:13\n   2: core::panicking::panic_fmt\n             at /rustc/abc/library/core/src/panicking.rs:80:14\n   3: horizon_client::crash::debug_crash_tick\n             at {root}/src/crash.rs:359:9\n   4: winit::dispatch\n             at /home/someone/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/winit-0.30.13/src/event_loop.rs:642:56\n   5: main\n",
            root = env!("CARGO_MANIFEST_DIR")
        );
        assert_eq!(
            stack_lines(&bt, true),
            "horizon_client::crash::debug_crash_tick (crates/horizon-client-render/src/crash.rs:359:9)\nwinit::dispatch (winit-0.30.13/src/event_loop.rs:642:56)\nmain\n"
        );
    }

    #[test]
    fn contained_panics_come_back_as_errors() {
        assert_eq!(contain(|| 7), Ok(7));
        let error = contain(|| -> u8 { panic!("a pack's program") }).unwrap_err();
        assert!(error.contains("a pack's program"), "{error}");
        // Nested, and the thread is no longer inside one after.
        let outer = contain(|| contain(|| -> u8 { panic!("inner") }));
        assert!(outer.unwrap().unwrap_err().contains("inner"));
        assert_eq!(CONTAINED.with(Cell::get), 0);
    }

    #[test]
    fn the_debug_crash_is_reported_as_vanillas() {
        let error: Box<dyn std::error::Error> = DebugCrash::new().into();
        assert_eq!(error.to_string(), "Manually triggered debug crash");
        let debug = error.downcast_ref::<DebugCrash>().unwrap();
        // The stack starts where the crash was raised (DebugCrash::new, in this file).
        let first = debug.stack.lines().next().unwrap();
        assert!(first.contains("crash.rs"), "{}", debug.stack);
    }

    #[test]
    fn worker_threads_are_the_streaming_pool() {
        assert!(is_worker_thread(Some("horizon-worker-3")));
        assert!(!is_worker_thread(Some("main")));
        assert!(!is_worker_thread(None));
    }

    #[test]
    fn a_panic_becomes_a_report() {
        // A real panic through a hook, as the game's would be.
        let caught = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = caught.clone();
        let _hook = PANIC_HOOK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if panic_message(info).contains("the test crash") {
                sink.lock().unwrap().push(panic_report(info, "client"));
            }
        }));
        let _ = std::panic::catch_unwind(|| panic!("the test crash"));
        // The same panic inside a native mod's hook, which the hook sees before the unwind.
        let _ = std::panic::catch_unwind(|| {
            let _guard = enter_native_mod(ModCall {
                id: "a",
                hook: "tick",
                part: None,
            });
            panic!("the test crash")
        });
        std::panic::set_hook(previous);
        // A caught panic leaves the thread as it found it.
        assert_eq!(attributed("boom".into()), "boom");
        let mut reports = std::mem::take(&mut *caught.lock().unwrap());
        assert_eq!(reports.len(), 2, "the hook ran for both panics");
        let in_mod = reports.pop().unwrap();
        assert!(
            in_mod.error.ends_with("\nin native mod a, hook tick"),
            "{}",
            in_mod.error
        );
        let report = reports.pop().unwrap();
        assert!(!report.error.contains("native mod"));
        assert!(report.error.contains("the test crash"));
        assert!(report.error.contains("crash.rs"));
        // The first frame is the test's own code, not the panic machinery.
        let first = report.stack.lines().next().unwrap();
        assert!(first.contains("crash.rs"), "{}", report.stack);
        let text = report.text();
        assert!(text.contains("Description: Unexpected error"));
        assert!(text.contains(&format!(
            "\tMinecraft Version: {}\n",
            horizon_core::VANILLA_VERSION
        )));
        assert!(text.contains("\tOperating System: "));
        assert!(text.contains("\tCPU: "));
        let home = std::env::temp_dir().join(format!("horizon-crash-save-{}", std::process::id()));
        let path = report.save(&home).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("-client.txt")
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    struct Quiet;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Quiet {}

    struct Broken;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Broken {
        fn component_types(
            &self,
            _context: &mod_api::RegistrationContext,
            _event: &mut mod_api::ComponentTypesEvent<'_>,
        ) {
            panic!("a broken mod");
        }
    }

    struct Line(Option<&'static str>);
    impl mod_api::CrashSection for Line {
        fn value(&self) -> Option<String> {
            self.0.map(str::to_owned)
        }
    }

    struct Panics;
    impl mod_api::CrashSection for Panics {
        fn value(&self) -> Option<String> {
            panic!("a broken crash line")
        }
    }

    struct Lines;
    impl mod_api::Hooks for Lines {
        fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.crash_section(
                "caches",
                std::sync::Arc::new(Line(Some("3 caches\n\tHorizon Version: forged"))),
            );
            r.crash_section("idle", std::sync::Arc::new(Line(None)));
            r.crash_section("worker", std::sync::Arc::new(Panics));
            r.crash_header(std::sync::Arc::new(Line(Some("modded"))));
        }
    }

    #[test]
    fn mod_lines_are_asked_when_the_report_is_written() {
        let _hook = PANIC_HOOK.lock().unwrap_or_else(|e| e.into_inner());
        let mods: mod_api::Mods =
            std::sync::Arc::from([mod_api::ModEntry::new("test", Box::new(Lines))]);
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let details = system_details("client");
            let lines: Vec<_> = details
                .iter()
                .filter(|(key, _)| key.starts_with("test"))
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect();
            assert_eq!(
                lines,
                [
                    ("test: caches", "3 caches  Horizon Version: forged"),
                    ("test: worker", "~~ERROR~~"),
                ]
            );
            let text = Report {
                headers: mod_headers(),
                ..report()
            }
            .text();
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(
                lines[..2],
                ["---- Horizon Crash Report ----", "test: modded"]
            );
            assert!(lines[2].starts_with("// "));
        });
    }

    #[test]
    fn a_panic_in_a_hook_names_the_mod_and_the_hook() {
        let mods: Vec<mod_api::ModEntry> = vec![
            mod_api::ModEntry::new("quiet", Box::new(Quiet)),
            mod_api::ModEntry::new("broken", Box::new(Broken)),
        ];
        let caught = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = caught.clone();
        let _hook = PANIC_HOOK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let error = panic_report(info, "client").error;
            if error.contains("a broken mod") {
                sink.lock().unwrap().push(error);
            }
        }));
        let unwound = mod_platform::testing::with_mods(mods.into(), || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                mod_api::dispatch_component_types(
                    &mod_api::RegistrationContext::new(),
                    &mut Vec::new(),
                )
            }))
        });
        // Once the dispatch has unwound, the thread is in no mod.
        let _ = std::panic::catch_unwind(|| panic!("a broken mod's caller"));
        std::panic::set_hook(previous);
        // The game stops as before: the panic goes on to the dispatch's caller.
        assert!(unwound.is_err());
        let errors = caught.lock().unwrap();
        assert_eq!(errors.len(), 2);
        assert!(
            errors[0].ends_with("\nin native mod broken, hook component_types"),
            "{}",
            errors[0]
        );
        assert!(!errors[1].contains("in native mod"), "{}", errors[1]);
    }
}
