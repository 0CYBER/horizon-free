//! Runtime compilation budget, shared by client and dedicated server startup.
use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

/// The runtime setting in options.txt (client) or server.properties (dedicated server).
pub const SETTING: &str = "wasm.compile_timeout_secs";

/// A runtime compilation setting that cannot be applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingError {
    /// The exact configured value is not positive, finite, representable seconds.
    InvalidValue(String),
    /// The settings file could not be read or parsed.
    File { path: PathBuf, why: String },
}
impl fmt::Display for SettingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidValue(value) => write!(
                f,
                "invalid {SETTING}={value:?}: expected positive finite seconds"
            ),
            Self::File { path, why } => write!(f, "{}: {why}", path.display()),
        }
    }
}
impl std::error::Error for SettingError {}

/// A compile limit and its provenance. Compilation is parallel: issue arismoko/horizon#3673 measured
/// a 30 MB module at about 30 s on one thread and 3 s on eight. We allow ten times
/// the one-thread measurement plus a 5 s fixed allowance: `5 + 300 / threads`.
/// Eight threads get 42.5 s, above the old 37.743749290 s Ryzen 7 5700X limit
/// (ten times a measured 3.774374929 s compile of 67,998,240 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileBudget {
    /// Wall-clock limit applied by the parent to a child compilation.
    pub limit: Duration,
    /// Available thread count for an automatic budget; `None` means configured.
    pub threads: Option<usize>,
}
impl CompileBudget {
    /// Pure default; zero is treated as one available thread.
    pub fn for_threads(threads: usize) -> Self {
        let threads = threads.max(1);
        Self {
            limit: Duration::from_secs(5) + Duration::from_secs(300).div_f64(threads as f64),
            threads: Some(threads),
        }
    }
    /// Strict positive, finite seconds, including fractional seconds for small budgets.
    pub fn parse(value: &str) -> Result<Self, SettingError> {
        let limit = value
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v > 0.0)
            .and_then(|v| Duration::try_from_secs_f64(v).ok())
            .filter(|v| !v.is_zero())
            .ok_or_else(|| SettingError::InvalidValue(value.to_owned()))?;
        Ok(Self {
            limit,
            threads: None,
        })
    }
    /// Read the resolved player's options, or the dedicated server's game-directory properties.
    /// The shared game/cache directory is independent of the client's `--player-dir`.
    pub fn read(
        launch: &horizon_mod_package::selection::LaunchArguments,
        player_dir: &Path,
    ) -> Result<Self, SettingError> {
        use horizon_mod_package::selection::Argument;
        let dedicated = launch
            .rest
            .iter()
            .any(|arg| matches!(arg, Argument::Dedicated(_)));
        let path = if dedicated {
            launch.context.game_dir.join("server.properties")
        } else {
            player_dir.join("options.txt")
        };
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(SettingError::File {
                    path,
                    why: error.to_string(),
                });
            }
        };
        if dedicated {
            let properties =
                horizon_java_properties::Properties::load(&contents.chars().collect::<Vec<_>>())
                    .map_err(|error| SettingError::File {
                        path,
                        why: error.to_string(),
                    })?;
            return properties
                .get(SETTING)
                .map(Self::parse)
                .transpose()
                .map(|budget| budget.unwrap_or_default());
        }
        let separator = ':';
        let mut configured = None;
        for line in contents.lines() {
            if let Some((name, value)) = line.split_once(separator)
                && name.trim() == SETTING
            {
                configured = Some(Self::parse(value.trim())?);
            }
        }
        Ok(configured.unwrap_or_else(Self::default))
    }
}
impl Default for CompileBudget {
    fn default() -> Self {
        Self::for_threads(std::thread::available_parallelism().map_or(1, usize::from))
    }
}
impl fmt::Display for CompileBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} seconds (", self.limit.as_secs_f64())?;
        match self.threads {
            Some(threads) => write!(f, "default for {threads} threads")?,
            None => write!(f, "setting {SETTING}")?,
        }
        write!(f, "); increase {SETTING} to allow more compile time")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_defaults() {
        for (threads, seconds) in [(1, 305.0), (2, 155.0), (8, 42.5), (64, 9.6875)] {
            assert_eq!(
                CompileBudget::for_threads(threads).limit,
                Duration::from_secs_f64(seconds)
            );
        }
        assert!(CompileBudget::for_threads(8).limit >= Duration::from_nanos(37_743_749_290));
    }
    #[test]
    fn invalid_values_name_setting_and_value() {
        for value in ["0", "no", "NaN", "-1", "inf"] {
            let error = CompileBudget::parse(value).unwrap_err().to_string();
            assert!(error.contains(SETTING));
            assert!(error.contains(&format!("{value:?}")));
        }
        assert_eq!(
            CompileBudget::parse("0.001").unwrap().limit,
            Duration::from_millis(1)
        );
        assert_eq!(
            CompileBudget::parse("1000000000000").unwrap().limit,
            Duration::from_secs(1_000_000_000_000)
        );
    }
    #[test]
    fn startup_context_honors_player_and_dedicated_settings_locations() {
        use horizon_mod_package::selection::LaunchArguments;
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("shared");
        let player = dir.path().join("players/alex");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::create_dir_all(&player).unwrap();
        std::fs::write(game.join("options.txt"), format!("{SETTING}:0\n")).unwrap();
        std::fs::write(player.join("options.txt"), format!("{SETTING}:120\n")).unwrap();
        std::fs::write(game.join("server.properties"), format!("{SETTING}=240\n")).unwrap();
        let client = LaunchArguments::parse(
            [
                "--game-dir",
                "shared",
                "--player-dir",
                player.to_str().unwrap(),
            ]
            .into_iter()
            .map(str::to_owned),
            dir.path(),
            dir.path(),
            None,
        )
        .unwrap();
        assert_eq!(
            CompileBudget::read(&client, &player).unwrap().limit,
            Duration::from_secs(120)
        );
        let server = LaunchArguments::parse(
            ["--game-dir", "shared", "--dedicated", "nogui"]
                .into_iter()
                .map(str::to_owned),
            dir.path(),
            dir.path(),
            None,
        )
        .unwrap();
        assert_eq!(
            CompileBudget::read(&server, &player).unwrap().limit,
            Duration::from_secs(240)
        );
        for value in ["0", "not-a-number"] {
            std::fs::write(player.join("options.txt"), format!("{SETTING}:{value}\n")).unwrap();
            std::fs::write(
                game.join("server.properties"),
                format!("{SETTING}={value}\n"),
            )
            .unwrap();
            for launch in [&client, &server] {
                assert_eq!(
                    CompileBudget::read(launch, &player).unwrap_err(),
                    SettingError::InvalidValue(value.to_owned()),
                );
            }
        }
    }
}
