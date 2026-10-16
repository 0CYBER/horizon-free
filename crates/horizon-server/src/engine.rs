//! The engine: the game itself, run headless (`horizon-client --dedicated`, crates/horizon-client
//! game/dedicated.rs). It reads the vanilla files the supervisor writes, takes console commands on stdin and
//! logs on stdout.

use crate::config::Config;
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

/// The game's program names, most specific first: a headless engine once there is one, else the game.
const PROGRAMS: [&str; 2] = ["horizon-engine", "horizon-client"];

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// The game program: `engine.path`, else one installed beside `horizon-server`, else one on the PATH.
pub fn program(config: &Config, dir: &Path) -> Result<PathBuf, String> {
    if !config.engine.path.is_empty() {
        let path = dir.join(&config.engine.path);
        return path
            .is_file()
            .then_some(path)
            .ok_or_else(|| format!("engine.path {} is not a file", config.engine.path));
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf));
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

/// Where the prepared game files may be, in the order they are tried.
pub fn resource_candidates(config: &Config, dir: &Path, program: Option<&Path>) -> Vec<PathBuf> {
    if !config.engine.resources.is_empty() {
        return vec![dir.join(&config.engine.resources)];
    }
    let version = "26.3";
    let mut out = vec![dir.join("resources")];
    // A release folder or a checkout: .local/horizon-client/<version> beside the program or above it.
    if let Some(program) = program {
        for ancestor in program.ancestors().skip(1).take(4) {
            out.push(ancestor.join(".local/horizon-client").join(version));
        }
    }
    out
}

/// The prepared game files: a folder holding the asset index `horizon-prepare` writes.
pub fn resources(config: &Config, dir: &Path, program: Option<&Path>) -> Result<PathBuf, String> {
    let candidates = resource_candidates(config, dir, program);
    candidates
        .iter()
        .find(|d| d.join("asset-index.json").is_file())
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

/// Starts the engine in the server folder, with its console and log piped to the supervisor.
pub fn spawn(program: &Path, resources: &Path, dir: &Path) -> std::io::Result<Child> {
    let mut command = Command::new(program);
    command
        .arg("--resources")
        .arg(resources)
        .args(["--dedicated", "--nogui"])
        .current_dir(dir)
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
        windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP,
    );
    command.spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
