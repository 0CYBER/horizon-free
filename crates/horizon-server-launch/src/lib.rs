//! The dedicated engine's configuration, install layout and command description.
//! Shared by the supervisor and launcher without starting either or loading supervisor services.

pub mod config;
mod install;
pub mod prune;

use config::Config;
pub use install::Install;
use std::{
    fmt, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// The game's program names, most specific first: the server build (the game without sound or
/// controllers, so it needs neither ALSA nor udev on Linux; `horizon-server-linux-x86_64.tar.gz` holds it),
/// else the full game.
const PROGRAMS: [&str; 2] = ["horizon-engine", "horizon-client"];

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// The game program: `engine.path`, else one installed beside `horizon-server` (in the release in use when
/// it was installed by the installers), else one on the PATH.
pub fn program(config: &Config, dir: &Path) -> Result<PathBuf, String> {
    program_in(config, dir, Install::find().as_ref())
}

/// [`program`], for the install `install`.
pub fn program_in(
    config: &Config,
    dir: &Path,
    install: Option<&Install>,
) -> Result<PathBuf, String> {
    let executable = std::env::current_exe().ok();
    program_near(config, dir, install, executable.as_deref())
}

fn program_near(
    config: &Config,
    dir: &Path,
    install: Option<&Install>,
    executable: Option<&Path>,
) -> Result<PathBuf, String> {
    if !config.engine.path.is_empty() {
        let path = dir.join(&config.engine.path);
        return path
            .is_file()
            .then_some(path)
            .ok_or_else(|| format!("engine.path {} is not a file", config.engine.path));
    }
    // Installed: the release `current` points at now, so a switch applies at the next start.
    let beside = install
        .map(Install::programs)
        .or_else(|| executable.and_then(|e| e.parent().map(Path::to_path_buf)));
    let path_dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    PROGRAMS
        .iter()
        .flat_map(|name| {
            beside
                .iter()
                .chain(&path_dirs)
                .map(move |d| d.join(exe(name)))
        })
        .find(|p| p.is_file())
        .ok_or_else(|| {
            "the game program (horizon-client) is neither beside horizon-server nor on the PATH: \
             install it there, or set engine.path in server.toml"
                .into()
        })
}

/// A failure while describing a configured dedicated-engine command, without starting it.
#[derive(Debug)]
#[non_exhaustive]
pub enum EngineCommandError {
    Configuration(String),
    Program(String),
    Resources(String),
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for EngineCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(error) | Self::Program(error) | Self::Resources(error) => {
                f.write_str(error)
            }
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}
impl std::error::Error for EngineCommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Describe the engine the supervisor at `supervisor` would run in `dir`.
/// Uses the same config, install/current, PATH, resources and command builder as the supervisor.
/// Does not launch the engine, run a preflight query, write properties, or download anything.
pub fn engine_command(supervisor: &Path, dir: &Path) -> Result<Command, EngineCommandError> {
    // Not `fs::canonicalize`: its `\\?\` prefix on Windows would reach the game's program and resources
    // paths, where a later `/` in a string no longer means a separator.
    let supervisor = dunce::canonicalize(supervisor).map_err(|source| EngineCommandError::Io {
        path: supervisor.to_owned(),
        source,
    })?;
    let config = Config::load(dir).map_err(EngineCommandError::Configuration)?;
    let install = Install::around(&supervisor);
    let program = program_near(&config, dir, install.as_ref(), Some(&supervisor))
        .map_err(EngineCommandError::Program)?;
    let resources =
        resources(&config, dir, Some(&program)).map_err(EngineCommandError::Resources)?;
    command(&program, &resources, dir, config.engine.low_memory).map_err(|source| {
        EngineCommandError::Io {
            path: dir.to_owned(),
            source,
        }
    })
}

/// The Minecraft version this server is pinned to; prepared game files must be stamped with it.
pub const GAME_VERSION: &str = horizon_core::version::VERSION_ID;

/// Where the prepared game files may be, in the order they are tried.
pub fn resource_candidates(config: &Config, dir: &Path, program: Option<&Path>) -> Vec<PathBuf> {
    if !config.engine.resources.is_empty() {
        return vec![dir.join(&config.engine.resources)];
    }
    let version = GAME_VERSION;
    let mut out = vec![dir.join("resources")];
    // A release folder or a checkout: .local/horizon-client/<version> beside the program or above it.
    if let Some(program) = program {
        for ancestor in program.ancestors().skip(1).take(4) {
            out.push(ancestor.join(".local/horizon-client").join(version));
        }
    }
    out
}

/// The prepared game files: a folder holding the manifest every `horizon-prepare` run writes
/// (`asset-index.json` comes only with the optional sounds, which a server doesn't need).
pub fn resources(config: &Config, dir: &Path, program: Option<&Path>) -> Result<PathBuf, String> {
    let candidates = resource_candidates(config, dir, program);
    candidates
        .iter()
        .find(|d| d.join("horizon-resources.json").is_file())
        .cloned()
        .ok_or_else(|| {
            let tried: Vec<String> = candidates
                .iter()
                .map(|d| format!("  {}", d.display()))
                .collect();
            format!(
                "no prepared game files. Horizon reads them from your own copy of Minecraft 26.3:\n\
                 prepare them with `horizon-prepare --prepare --server-jar PATH` (or the launcher) and copy \
                 the folder to resources/ here, or set engine.resources in server.toml. Looked in:\n{}",
                tried.join("\n")
            )
        })
}

/// The command the supervisor runs, without starting it.
pub fn command(
    program: &Path,
    resources: &Path,
    dir: &Path,
    low_memory: bool,
) -> std::io::Result<Command> {
    command_with_transfers(program, resources, dir, low_memory, None)
}

/// The supervisor command with optional transfer settings, placed before vanilla's server arguments.
pub fn command_with_transfers(
    program: &Path,
    resources: &Path,
    dir: &Path,
    low_memory: bool,
    transfers: Option<&str>,
) -> std::io::Result<Command> {
    // The game runs in the server folder, so a folder given relative to ours (`--dir srv`) would put its
    // control endpoint in `srv/srv/.horizon`, which doesn't exist.
    let dir = std::path::absolute(dir)?;
    let resources = std::path::absolute(resources)?;
    let mut command = Command::new(program);
    command
        .arg("--resources")
        .arg(resources)
        .arg("--control-socket")
        .arg(control_path(&dir));
    if let Some(transfers) = transfers {
        command.arg("--transfers").arg(transfers);
    }
    command
        .args(["--dedicated", "--nogui"])
        .current_dir(&dir)
        .env("HORIZON_NO_AUDIO", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The terminal's Ctrl-C reaches the supervisor alone, which then stops the game cleanly.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        &mut command,
        windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP
            | windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
    );
    // `[engine] low_memory`: mimalloc then keeps transparent huge pages off. Mostly empty huge pages cost
    // 60 MB idle and 190 MB at 20 players, and save ~1 % of tick time (docs/hosting.md).
    if low_memory {
        command.env("MIMALLOC_ALLOW_THP", "0");
    }
    Ok(command)
}

/// The game's control endpoint in a server folder (`--control-socket`): TCP on 127.0.0.1 with a token, on every OS.
pub fn control_path(dir: &Path) -> PathBuf {
    dir.join(".horizon/engine.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_command_uses_the_given_supervisor_and_config_without_starting() {
        // Canonical like the supervisor's own path: a temp folder can be a link or a short name.
        let root = dunce::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("horizon-server-command-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("horizon-resources.json"), "{}").unwrap();
        let bin = root.join("bin");
        let server = root.join("server");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&server).unwrap();
        let supervisor = bin.join(format!("horizon-server{}", std::env::consts::EXE_SUFFIX));
        let game = bin.join(format!("horizon-engine{}", std::env::consts::EXE_SUFFIX));
        let client = bin.join(format!("horizon-client{}", std::env::consts::EXE_SUFFIX));
        // Invalid executables prove description does not spawn or preflight any program.
        for path in [&supervisor, &game, &client] {
            std::fs::write(path, "not executable").unwrap();
        }
        let mut config = Config::default();
        config.engine.resources = root.to_str().unwrap().into();
        config.engine.low_memory = true;
        std::fs::write(server.join("server.toml"), crate::config::render(&config)).unwrap();
        let command = crate::engine_command(&supervisor, &server).unwrap();
        assert_eq!(command.get_program(), game);
        assert_eq!(command.get_current_dir(), Some(server.as_path()));
        let args: Vec<_> = command.get_args().map(std::ffi::OsStr::to_owned).collect();
        assert_eq!(
            args,
            vec![
                "--resources".into(),
                root.clone().into_os_string(),
                "--control-socket".into(),
                server.join(".horizon/engine.json").into_os_string(),
                "--dedicated".into(),
                "--nogui".into(),
            ]
        );
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "MIMALLOC_ALLOW_THP"
                    && value == Some(std::ffi::OsStr::new("0")))
        );
        assert!(!server.join("server.properties").exists());
        assert!(!server.join(".horizon").exists());

        let external = root.join("external-engine");
        std::fs::write(&external, "not executable either").unwrap();
        config.engine.path = "../external-engine".into();
        std::fs::write(server.join("server.toml"), crate::config::render(&config)).unwrap();
        assert_eq!(
            crate::engine_command(&supervisor, &server)
                .unwrap()
                .get_program(),
            server.join("../external-engine")
        );
        config.backups.every = "6 hours".into();
        std::fs::write(server.join("server.toml"), crate::config::render(&config)).unwrap();
        assert!(matches!(
            crate::engine_command(&supervisor, &server),
            Err(EngineCommandError::Configuration(_))
        ));
        std::fs::write(server.join("server.toml"), "not TOML").unwrap();
        assert!(matches!(
            crate::engine_command(&supervisor, &server),
            Err(EngineCommandError::Configuration(_))
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_installed_supervisor_describes_the_current_release_not_its_own() {
        let root = std::env::temp_dir().join(format!(
            "horizon-server-command-current-{}",
            std::process::id()
        ));
        let old = root.join("releases/old");
        let current = root.join("releases/new");
        let server = root.join("server");
        for dir in [&old, &current, &server] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let supervisor = old.join("horizon-server");
        for path in [
            supervisor.clone(),
            old.join("horizon-engine"),
            current.join("horizon-engine"),
        ] {
            std::fs::write(path, "not executable").unwrap();
        }
        std::os::unix::fs::symlink(&current, root.join("current")).unwrap();
        std::fs::write(server.join("server.toml"), "").unwrap();
        std::fs::create_dir_all(server.join("resources")).unwrap();
        std::fs::write(server.join("resources/horizon-resources.json"), "{}").unwrap();
        let command = engine_command(&supervisor, &server).unwrap();
        assert_eq!(command.get_program(), root.join("current/horizon-engine"));
        assert_eq!(command.get_current_dir(), Some(server.as_path()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn low_memory_turns_huge_pages_off_for_the_game() {
        let thp = |low_memory| {
            let dir = std::env::temp_dir();
            let command = command(Path::new("horizon-client"), &dir, &dir, low_memory).unwrap();
            command
                .get_envs()
                .find(|(k, _)| *k == "MIMALLOC_ALLOW_THP")
                .and_then(|(_, v)| v.map(|v| v.to_owned()))
        };
        assert_eq!(thp(true).as_deref(), Some(std::ffi::OsStr::new("0")));
        assert_eq!(thp(false), None);
    }

    #[test]
    fn resources_are_searched_in_order() {
        let mut config = Config::default();
        let dir = Path::new("/srv/mc");
        let program = Path::new("/opt/horizon/target/release/horizon-client");
        let found = resource_candidates(&config, dir, Some(program));
        assert_eq!(found[0], Path::new("/srv/mc/resources"));
        assert!(found.contains(&PathBuf::from("/opt/horizon/.local/horizon-client/26.3")));
        config.engine.resources = "/data/res".into();
        assert_eq!(
            resource_candidates(&config, dir, Some(program)),
            [PathBuf::from("/data/res")]
        );
    }

    #[test]
    fn resources_prepared_without_sounds_are_found() {
        let root = std::env::temp_dir().join(format!("horizon-server-res-{}", std::process::id()));
        let prepared = root.join(".local/horizon-client/26.3");
        std::fs::create_dir_all(&prepared).unwrap();
        let program = root.join("horizon-client");
        let config = Config::default();
        let server = root.join("server");
        assert!(resources(&config, &server, Some(&program)).is_err());
        // What `horizon-prepare` writes on every run; no asset index without `horizon-prepare sounds`.
        std::fs::write(prepared.join("horizon-resources.json"), "{}").unwrap();
        assert_eq!(resources(&config, &server, Some(&program)), Ok(prepared));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
