//! Offline data relocation: the window exits before the worker takes the migration lock.
use crate::{
    backend,
    data_location::{self, Roots},
    process,
};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

fn remap(value: &mut Value, source: &Roots, target: &Roots) {
    let Some(text) = value.as_str() else { return };
    let path = Path::new(text);
    // The build directory can be nested in the launcher directory: the most specific wins.
    let mut pairs = [
        (&source.launcher, &target.launcher),
        (&source.game, &target.game),
        (&source.build, &target.build),
    ];
    pairs.sort_by_key(|(old, _)| std::cmp::Reverse(old.components().count()));
    for (old, new) in pairs {
        #[cfg(windows)]
        let matches = Path::new(&path.to_string_lossy().to_lowercase())
            .starts_with(old.to_string_lossy().to_lowercase());
        #[cfg(not(windows))]
        let matches = path.starts_with(old);
        if matches {
            let rest: PathBuf = path.components().skip(old.components().count()).collect();
            *value = Value::String(new.join(rest).to_string_lossy().into_owned());
            return;
        }
    }
}

/// Only fields whose schema denotes a filesystem path; never commands, URLs or arbitrary text.
pub fn prepare_paths(stage: &Roots, source: &Roots, target: &Roots) -> Result<(), String> {
    for name in ["settings.json", "servers.json", "instances.json"] {
        let path = stage.launcher.join(name);
        // A copied relative link must not be written through to another staged file.
        if let Ok(meta) = fs::symlink_metadata(&path) {
            data_location::reject_link(&path, &meta)?;
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut value: Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        match name {
            "settings.json" => {
                for key in ["minecraft_dir", "server_dir", "background"] {
                    if let Some(v) = value.get_mut(key) {
                        remap(v, source, target);
                    }
                }
            }
            "servers.json" => {
                if let Some(entries) = value.as_array_mut() {
                    for entry in entries {
                        if let Some(v) = entry.get_mut("dir") {
                            remap(v, source, target);
                        }
                    }
                }
            }
            "instances.json" => {
                if let Some(entries) = value.get_mut("instances").and_then(Value::as_object_mut) {
                    for entry in entries.values_mut() {
                        if let Some(v) = entry.get_mut("java").and_then(|j| j.get_mut("home")) {
                            remap(v, source, target);
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
        let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// Refuse live games/build tools even if the window that started them has already exited.
fn ensure_idle(source: &Roots) -> Result<(), String> {
    // A background service may run the server runtime's program from the launcher's data, and
    // which one it runs can't be told without running it, so any registered one stops the move.
    let services = crate::servers::registered_services()
        .map_err(|e| format!("Couldn't check for background servers: {e}"))?;
    if !services.is_empty() {
        return Err(format!(
            "Turn off running in the background on the Servers page, or run `horizon-server --dir <server folder> service uninstall` (add --system for a system service), before moving data: {}",
            services.join(", ")
        ));
    }
    #[cfg(windows)]
    let output = process::command("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "Get-CimInstance Win32_Process | Select-Object Name,ExecutablePath,CommandLine | ConvertTo-Json -Compress"])
        .output();
    #[cfg(not(windows))]
    let output = process::command("ps")
        .args(["-A", "-o", "comm=,args="])
        .output();
    let output = output.map_err(|e| format!("Could not check running games and builds: {e}"))?;
    if !output.status.success() {
        return Err("Could not check running games and builds; no files were moved.".into());
    }
    let roots: Vec<_> = [&source.launcher, &source.game, &source.build]
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/").to_lowercase())
        .collect();
    #[cfg(windows)]
    let processes: Vec<(String, String)> = {
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("Could not read running processes: {e}"))?;
        let entries = value.as_array().cloned().unwrap_or_else(|| vec![value]);
        entries
            .into_iter()
            .map(|p| {
                (
                    p["Name"].as_str().unwrap_or("").to_owned(),
                    format!(
                        "{} {}",
                        p["ExecutablePath"].as_str().unwrap_or(""),
                        p["CommandLine"].as_str().unwrap_or("")
                    ),
                )
            })
            .collect()
    };
    #[cfg(not(windows))]
    let processes: Vec<(String, String)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().split_once(char::is_whitespace))
        .map(|(name, command)| (name.to_owned(), command.to_owned()))
        .collect();
    for (name, command) in processes {
        let name = Path::new(&name)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name);
        let command = command.replace('\\', "/").to_lowercase();
        let managed = roots.iter().any(|root| command.contains(root));
        if matches!(
            name,
            "horizon-client" | "horizon-server" | "horizon-prepare"
        ) || (managed
            && matches!(
                name,
                "java" | "javaw" | "cargo" | "rustc" | "horizon-mixins"
            ))
        {
            return Err(format!(
                "Close games, servers and native builds before moving data ({name} is running)."
            ));
        }
    }
    Ok(())
}

/// What the worker is doing, in the order it does it. Checking comes again just before the
/// commit, when the copies are verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Checking,
    /// The lease waits for the window that asked for the move to close.
    Waiting,
    Launcher,
    Game,
    Build,
    Committing,
}

impl Step {
    /// The English words, which `i18n::tr` looks up.
    pub fn text(self) -> &'static str {
        match self {
            Step::Checking => "Checking for running games and builds",
            Step::Waiting => "Waiting for the launcher to close",
            Step::Launcher => "Copying launcher data",
            Step::Game => "Copying game data",
            Step::Build => "Copying build data",
            Step::Committing => "Saving the new location",
        }
    }
}

/// What the worker reports while it moves the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Update {
    Step(Step),
    /// One more file copied and verified.
    File,
}

impl Update {
    /// The update a message of `data_location::progress` stands for.
    pub fn from_progress(message: &str) -> Option<Self> {
        use data_location::progress;
        Some(match message {
            progress::FILE => Update::File,
            progress::LAUNCHER => Update::Step(Step::Launcher),
            progress::GAME => Update::Step(Step::Game),
            progress::BUILD => Update::Step(Step::Build),
            progress::COMMIT => Update::Step(Step::Committing),
            _ => return None,
        })
    }
}

/// What the move has done so far, for a window to show.
#[derive(Debug, Default)]
pub struct Progress {
    pub step: Option<Step>,
    /// Files copied and verified.
    pub files: u64,
}

impl Progress {
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Step(step) => self.step = Some(step),
            Update::File => self.files += 1,
        }
    }
}

/// A line on stderr. A closed or broken stream must not stop a move, as `eprintln!` would.
fn say(line: impl std::fmt::Display) {
    let _ = writeln!(std::io::stderr(), "{line}");
}

/// `horizon-launcher --relocate-data <target> [--restart]`, which the launcher's Move runs after
/// its window has closed.
pub struct Worker {
    target: Option<PathBuf>,
    restart: bool,
}

impl Worker {
    /// This process is a worker. Asked before normal startup, so it owns no shared data-location
    /// lease.
    pub fn from_args() -> Option<Self> {
        let mut args = std::env::args_os().skip(1);
        if args.next().as_deref() != Some(std::ffi::OsStr::new("--relocate-data")) {
            return None;
        }
        let target = args.next().map(PathBuf::from);
        let restart = args.next().is_some_and(|a| a == "--restart");
        Some(Self { target, restart })
    }

    /// The launcher asked for the move and opens again after it: the player waits for it.
    pub fn restarts(&self) -> bool {
        self.restart
    }

    pub fn target(&self) -> Option<&Path> {
        self.target.as_deref()
    }

    /// The whole of a worker's job: the move, then, for the launcher's move, its reopening with
    /// the result. A panic in the move is a failed move, still reported. stop set ends the move
    /// before its next file. The exit code.
    pub fn run_and_finish(&self, mut report: impl FnMut(Update), stop: &AtomicBool) -> i32 {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run(&mut report, stop)))
                .unwrap_or_else(|_| {
                    Err(crate::i18n::tr("The move stopped unexpectedly").to_string())
                });
        self.finish(&result)
    }

    /// Moves the data, telling `report` what it does. Ok holds the old roots, which the
    /// reopened launcher offers to remove.
    fn run(&self, report: &mut impl FnMut(Update), stop: &AtomicBool) -> Result<String, String> {
        let target = self
            .target
            .as_ref()
            .ok_or("--relocate-data needs an empty destination folder")?;
        data_location::validate()?;
        let source = Roots {
            launcher: backend::data_dir(),
            game: backend::game_home(),
            build: backend::build_dir(),
        };
        report(Update::Step(Step::Checking));
        ensure_idle(&source)?;
        let backup = format!(
            "{}\n{}\n{}",
            source.launcher.display(),
            source.game.display(),
            source.build.display()
        );
        report(Update::Step(Step::Waiting));
        // Both callbacks report; the move calls them one at a time.
        let report = std::cell::RefCell::new(report);
        let destination = data_location::migrate_current_with_prepare(
            &source,
            target,
            |message| {
                say(message);
                if let Some(update) = Update::from_progress(message) {
                    (report.borrow_mut())(update);
                }
                if stop.load(Ordering::Relaxed) {
                    // Between files: the originals are untouched and this run's stage stays.
                    return Err(crate::i18n::tr(
                        "The move stopped because its window closed. Your data is where it was.",
                    )
                    .to_string());
                }
                Ok(())
            },
            |stage, target| {
                (report.borrow_mut())(Update::Step(Step::Checking));
                ensure_idle(&source)?;
                prepare_paths(stage, &source, target)
            },
        )?;
        let _ = writeln!(std::io::stdout(), "{}", destination.launcher.display());
        Ok(backup)
    }

    /// Reports the result and, for the launcher's move, opens it again to show it. The exit code.
    fn finish(&self, result: &Result<String, String>) -> i32 {
        if let Err(error) = result {
            say(format_args!("Data was not relocated: {error}"));
        }
        // Staging folders of earlier moves, which the move left as they were.
        let kept = self
            .target()
            .and_then(|target| data_location::kept_note(target, None));
        if let Some(kept) = &kept {
            say(kept);
        }
        let code = i32::from(result.is_err());
        if self.restart {
            let mut command = process::command(backend::exe_path());
            command
                .env_remove("HORIZON_RELOCATION_ERROR")
                .env_remove("HORIZON_RELOCATION_BACKUP")
                .env_remove("HORIZON_RELOCATION_KEPT");
            match result {
                Ok(backup) => {
                    command.env("HORIZON_RELOCATION_BACKUP", backup);
                    if let Some(kept) = &kept {
                        command.env("HORIZON_RELOCATION_KEPT", kept);
                    }
                }
                Err(error) => {
                    command.env("HORIZON_RELOCATION_ERROR", error);
                }
            }
            if let Err(e) = command.spawn() {
                say(format_args!("Could not reopen the launcher: {e}"));
                return 1;
            }
        }
        code
    }
}

#[cfg(test)]
#[path = "relocation/tests.rs"]
mod tests;
