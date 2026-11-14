//! The launch paths shared by the game and the launcher's mod preparation.
use std::{
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
};

/// The game's platform data policy, evaluated against the final process environment.
pub fn data_dir_with(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> PathBuf {
    let path = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = path("HOME").or_else(|| path("USERPROFILE"));
    if cfg!(windows) {
        if let Some(d) = path("APPDATA") {
            return d;
        }
    } else if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            return h.join("Library/Application Support");
        }
    } else if let Some(d) = path("XDG_DATA_HOME") {
        return d;
    } else if let Some(h) = &home {
        return h.join(".local/share");
    }
    std::env::temp_dir()
}

#[derive(Clone, Debug)]
pub struct ModSelectionContext {
    pub game_dir: PathBuf,
    pub home: PathBuf,
    pub dev_mods: Vec<PathBuf>,
}

/// Arguments the game owns after the selection options have been consumed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Argument {
    Switch(String),
    Value(String, String),
    Dedicated(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct LaunchArguments {
    pub context: ModSelectionContext,
    pub rest: Vec<Argument>,
}

#[derive(Debug)]
pub struct MissingValue(String);

impl fmt::Display for MissingValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "missing value after {}", self.0)
    }
}
impl std::error::Error for MissingValue {}

impl LaunchArguments {
    /// The installation receiving dependency declarations: an integrated server
    /// belongs to the client installation; `--dedicated` selects the server one.
    pub fn installation_side(&self) -> crate::manifest::Side {
        if self
            .rest
            .iter()
            .any(|argument| matches!(argument, Argument::Dedicated(_)))
        {
            crate::manifest::Side::Server
        } else {
            crate::manifest::Side::Client
        }
    }

    /// Snapshot a managed game command before shell/overlay wrappers are applied.
    /// These commands inherit the parent environment, with explicit overrides and removals.
    pub fn for_command(command: &std::process::Command) -> std::io::Result<Self> {
        let cwd = std::path::absolute(command.get_current_dir().unwrap_or(Path::new(".")))?;
        let env = |key: &str| {
            command
                .get_envs()
                .find(|(name, _)| *name == key)
                .map_or_else(
                    || std::env::var_os(key),
                    |(_, value)| value.map(OsStr::to_owned),
                )
        };
        let default_home = data_dir_with(env).join("horizon");
        let args: Result<Vec<_>, _> = command
            .get_args()
            .map(|arg| {
                arg.to_str().map(str::to_owned).ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "game arguments must be valid UTF-8",
                    )
                })
            })
            .collect();
        Self::parse(
            args?.into_iter(),
            &cwd,
            &default_home,
            env("HORIZON_HOME").as_deref(),
        )
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
    }

    /// Help and capability replies do not read mod packages or prepare jar data.
    pub fn is_information_request(&self) -> bool {
        self.rest.iter().any(|arg| {
            matches!(arg, Argument::Switch(name) if matches!(name.as_str(), "--help" | "-h" | "--launch-api"))
        })
    }

    /// Resolve paths against the absolute starting directory, before the game changes it.
    /// Only selection options are interpreted; other values retain their spelling and order.
    pub fn parse(
        mut args: impl Iterator<Item = String>,
        cwd: &Path,
        default_home: &Path,
        env_home: Option<&OsStr>,
    ) -> Result<Self, MissingValue> {
        let mut context = ModSelectionContext {
            game_dir: cwd.to_path_buf(),
            home: cwd.join(
                env_home
                    .filter(|v| !v.is_empty())
                    .unwrap_or(default_home.as_os_str()),
            ),
            dev_mods: Vec::new(),
        };
        let mut rest = Vec::new();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--dedicated" => {
                    rest.push(Argument::Dedicated(args.collect()));
                    break;
                }
                "--help" | "-h" | "--launch-api" => {
                    rest.push(Argument::Switch(arg));
                    break;
                }
                "--developer-single-player"
                | "--game-tests"
                | "--fullscreen"
                | "--profile"
                | "--session-stdin"
                | "--creative"
                | "--test-terrain"
                | "--no-pause"
                | "--offscreen"
                | "--headless"
                | "--control"
                | "--server-thread"
                | "--in-frame"
                | "--no-server-thread"
                | "--pregen" => {
                    rest.push(Argument::Switch(arg));
                }
                _ => {
                    let value = args.next().ok_or_else(|| MissingValue(arg.clone()))?;
                    match arg.as_str() {
                        "--game-dir" | "--gameDir" => context.game_dir = cwd.join(value),
                        "--home" => context.home = cwd.join(value),
                        "--dev-mod" => context.dev_mods.push(cwd.join(value)),
                        _ => rest.push(Argument::Value(arg, value)),
                    }
                }
            }
        }
        Ok(Self { context, rest })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_command_owns_paths_values_and_environment() {
        let cwd = std::env::current_dir().unwrap();
        let mut command = std::process::Command::new("game");
        command
            .current_dir(&cwd)
            .env("HORIZON_HOME", "env-cache")
            .args([
                "--gameDir",
                "instance",
                "--home",
                "final-cache",
                "--dev-mod",
                "workspace/source",
                "--resources",
                "--home",
                "--creative",
            ]);
        let launch = LaunchArguments::for_command(&command).unwrap();
        assert_eq!(launch.context.game_dir, cwd.join("instance"));
        assert_eq!(launch.context.home, cwd.join("final-cache"));
        assert_eq!(launch.context.dev_mods, [cwd.join("workspace/source")]);
        assert_eq!(
            launch.rest,
            [
                Argument::Value("--resources".into(), "--home".into()),
                Argument::Switch("--creative".into())
            ]
        );
        assert!(!launch.is_information_request());
    }

    #[test]
    fn final_command_home_removal_uses_its_platform_environment() {
        let cwd = std::env::current_dir().unwrap();
        let mut command = std::process::Command::new("game");
        command
            .current_dir(&cwd)
            .env_remove("HORIZON_HOME")
            .env("HOME", cwd.join("user"))
            .env("USERPROFILE", cwd.join("user"))
            .env("APPDATA", cwd.join("roaming"))
            .env("XDG_DATA_HOME", cwd.join("data"));
        let launch = LaunchArguments::for_command(&command).unwrap();
        let expected = if cfg!(windows) {
            cwd.join("roaming/horizon")
        } else if cfg!(target_os = "macos") {
            cwd.join("user/Library/Application Support/horizon")
        } else {
            cwd.join("data/horizon")
        };
        assert_eq!(launch.context.home, expected);
        command.args(["--help", "--missing-value"]);
        assert!(
            LaunchArguments::for_command(&command)
                .unwrap()
                .is_information_request()
        );
    }

    fn parse(args: &[&str], home: Option<&str>) -> LaunchArguments {
        LaunchArguments::parse(
            args.iter().map(|s| s.to_string()),
            Path::new("/start"),
            Path::new("/default"),
            home.map(OsStr::new),
        )
        .unwrap()
    }

    #[test]
    fn paths_follow_overrides_before_chdir_and_the_dedicated_boundary() {
        let parsed = parse(
            &[
                "--game-dir",
                "first",
                "--gameDir",
                "game",
                "--home",
                "cache",
                "--dev-mod",
                "one.hmod",
                "--dev-mod",
                "/two.hmod",
                "--dedicated",
                "--home",
                "server-only",
            ],
            Some("/environment"),
        );
        assert_eq!(parsed.context.game_dir, Path::new("/start/game"));
        assert_eq!(parsed.context.home, Path::new("/start/cache"));
        assert_eq!(
            parsed.context.dev_mods,
            [PathBuf::from("/start/one.hmod"), PathBuf::from("/two.hmod")]
        );
        assert_eq!(
            parsed.rest,
            [Argument::Dedicated(vec![
                "--home".into(),
                "server-only".into()
            ])]
        );
        assert_eq!(
            parse(&[], Some("relative")).context.home,
            Path::new("/start/relative")
        );
        assert_eq!(parse(&[], None).context.home, Path::new("/default"));
    }

    #[test]
    fn values_that_look_like_options_are_not_selection_options() {
        let parsed = parse(
            &[
                "--resources",
                "--home",
                "--creative",
                "--game-dir",
                "actual",
                "--name",
                "--dev-mod",
            ],
            Some("/environment"),
        );
        assert_eq!(parsed.context.game_dir, Path::new("/start/actual"));
        assert_eq!(parsed.context.home, Path::new("/environment"));
        assert!(parsed.context.dev_mods.is_empty());
        assert_eq!(
            parsed.rest,
            [
                Argument::Value("--resources".into(), "--home".into()),
                Argument::Switch("--creative".into()),
                Argument::Value("--name".into(), "--dev-mod".into())
            ]
        );
        let error = LaunchArguments::parse(
            ["--dev-mod".to_string()].into_iter(),
            Path::new("/start"),
            Path::new("/default"),
            None,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "missing value after --dev-mod");
    }
}
