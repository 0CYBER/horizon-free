use super::process::probe_output;
use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const CONFIG_FILE: &str = "horizon-export.txt";
// FFmpeg's libavformat/movenc.h flags that can reopen fd: for reading.
const FASTSTART: u32 = 1 << 7;
const GLOBAL_SIDX: u32 = 1 << 14;
const REOPENS_OUTPUT: u32 = FASTSTART | GLOBAL_SIDX;

#[derive(Debug)]
pub struct EncoderConfig {
    pub(super) program: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) container: &'static str,
}

#[derive(Debug)]
pub enum ConfigError {
    EncoderUnavailable,
    InvalidLine(usize),
    Read(io::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EncoderUnavailable => f.write_str("export encoder is unavailable"),
            Self::InvalidLine(line) => {
                write!(f, "invalid encoder configuration at {CONFIG_FILE}:{line}")
            }
            Self::Read(err) => write!(f, "could not read {CONFIG_FILE}: {err}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(err) => Some(err),
            _ => None,
        }
    }
}

impl EncoderConfig {
    /// Read the player's configuration. Encoder paths are absolute or relative to the
    /// game directory. Argument quotes group words, with no shell expansion.
    /// Set `allow_path_fallback` only for callers allowed to find ffmpeg in PATH.
    pub fn load(game_dir: &Path, allow_path_fallback: bool) -> Result<Self, ConfigError> {
        let path = allow_path_fallback.then(|| env::var_os("PATH")).flatten();
        Self::load_with_path(game_dir, path.as_deref())
    }

    pub(super) fn validate_output_args(&self) -> io::Result<()> {
        if !matches!(self.container, "mp4" | "mov") {
            return Ok(());
        }
        let movflags = self.args.iter().enumerate().filter_map(|(index, arg)| {
            if arg == "-movflags" {
                self.args.get(index + 1).map(String::as_str)
            } else {
                arg.strip_prefix("-movflags=")
            }
        });
        // FFmpeg replaces a previous option with an unsigned value and appends
        // values that start with + or -. Only the final assignment and its
        // following modifiers reach the muxer.
        let flags: Vec<_> = movflags.collect();
        let start = flags
            .iter()
            .rposition(|value| !value.starts_with(['+', '-']))
            .unwrap_or(0);
        let reopen_flags = flags[start..].iter().try_fold(0, |enabled, flags| {
            apply_movflags(enabled, flags).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "use named MOV flags or literal masks without +faststart or +global_sidx when exporting MP4 or MOV",
                )
            })
        })?;
        if reopen_flags != 0 {
            // Both flags reopen the output for reading, and bare fd: reads stdin.
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "disable +faststart and +global_sidx in -movflags when exporting MP4 or MOV",
            ));
        }
        Ok(())
    }

    /// Check the binary at the resolved path before creating export files.
    pub(super) fn verify_fd_output(&self, timeout: Duration) -> io::Result<()> {
        let mut command = Command::new(&self.program);
        command.arg("-protocols");
        let stdout = probe_output(command, timeout).map_err(|err| {
            io::Error::new(
                err.kind(),
                format!("could not verify encoder fd: output protocol: {err}"),
            )
        })?;
        if !supports_fd_output(&String::from_utf8_lossy(&stdout)) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "encoder does not support the fd: output protocol",
            ));
        }
        Ok(())
    }

    fn load_with_path(game_dir: &Path, search_path: Option<&OsStr>) -> Result<Self, ConfigError> {
        let config = match fs::read_to_string(game_dir.join(CONFIG_FILE)) {
            Ok(config) => config,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(ConfigError::Read(err)),
        };
        let mut program = None;
        let mut args = None;
        let mut container = None;
        for (index, line) in config.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let invalid = || ConfigError::InvalidLine(index + 1);
            let (key, value) = line.split_once('=').ok_or_else(invalid)?;
            let value = value.trim();
            if value.contains('\0') {
                return Err(invalid());
            }
            match key.trim() {
                "encoder" if program.is_none() && !value.is_empty() => program = Some(value),
                "encoder_args" if args.is_none() => {
                    args = Some(shlex::split(value).ok_or_else(invalid)?)
                }
                "container" if container.is_none() => {
                    container = Some(match value {
                        "mp4" => "mp4",
                        "mkv" => "mkv",
                        "mov" => "mov",
                        "webm" => "webm",
                        _ => return Err(invalid()),
                    });
                }
                _ => return Err(invalid()),
            }
        }
        let program = match program {
            Some(path) => resolve(&game_dir.join(path)),
            None => search_path.and_then(find_ffmpeg),
        }
        .ok_or(ConfigError::EncoderUnavailable)?;
        Ok(Self {
            program,
            args: args.unwrap_or_default(),
            container: container.unwrap_or("mp4"),
        })
    }
}

fn apply_movflags(mut enabled: u32, mut value: &str) -> Option<u32> {
    loop {
        let operation = value.as_bytes().first().copied()?;
        if matches!(operation, b'+' | b'-') {
            value = &value[1..];
        }
        let end = value.find(['+', '-']).unwrap_or(value.len());
        let mask = output_reopen_mask(value[..end].trim())?;
        enabled = match operation {
            b'+' => enabled | mask,
            b'-' => enabled & !mask,
            _ => mask,
        };
        if end == value.len() {
            return Some(enabled);
        }
        value = &value[end..];
    }
}

fn output_reopen_mask(value: &str) -> Option<u32> {
    match value {
        "faststart" => Some(FASTSTART),
        "global_sidx" => Some(GLOBAL_SIDX),
        "all" | "max" => Some(REOPENS_OUTPUT),
        "default"
        | "none"
        | "min"
        | "cmaf"
        | "dash"
        | "default_base_moof"
        | "delay_moov"
        | "disable_chpl"
        | "empty_moov"
        | "frag_custom"
        | "frag_discont"
        | "frag_every_frame"
        | "frag_keyframe"
        | "hybrid_fragmented"
        | "isml"
        | "negative_cts_offsets"
        | "omit_tfhd_offset"
        | "prefer_icc"
        | "rtphint"
        | "separate_moof"
        | "skip_sidx"
        | "skip_trailer"
        | "use_metadata_tags"
        | "write_colr"
        | "write_gama" => Some(0),
        literal => {
            let mask = match literal
                .strip_prefix("0x")
                .or_else(|| literal.strip_prefix("0X"))
            {
                Some(hex) => u32::from_str_radix(hex, 16),
                None => literal.parse::<u32>(),
            }
            .ok()?;
            Some(mask & REOPENS_OUTPUT)
        }
    }
}

fn supports_fd_output(protocols: &str) -> bool {
    protocols
        .lines()
        .map(str::trim)
        .skip_while(|line| *line != "Output:")
        .skip(1)
        .take_while(|line| !line.ends_with(':'))
        .any(|line| line == "fd")
}

fn find_ffmpeg(path: &OsStr) -> Option<PathBuf> {
    let executable = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    env::split_paths(path).find_map(|dir| resolve(&dir.join(executable)))
}

fn resolve(path: &Path) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    let metadata = path.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    // Rust can invoke cmd.exe implicitly for batch files on Windows.
    #[cfg(windows)]
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    Some(path)
}

#[cfg(test)]
#[test]
fn requires_fd_in_output_protocols() {
    assert!(supports_fd_output("Input:\n  file\nOutput:\n  fd\n"));
    assert!(!supports_fd_output("Input:\n  fd\nOutput:\n  pipe\n"));
    assert!(!supports_fd_output("Output:\n  fds\nInput:\n  fd\n"));
}

#[cfg(test)]
#[test]
fn rejects_movflags_that_reopen_mp4_and_mov_outputs() {
    let mut config = EncoderConfig {
        program: PathBuf::new(),
        args: Vec::new(),
        container: "mp4",
    };
    for container in ["mp4", "mov"] {
        config.container = container;
        for (args, valid) in [
            (&["-movflags", "faststart"][..], false),
            (&["-movflags", "+faststart"][..], false),
            (&["-movflags", "+faststart-frag_keyframe"][..], false),
            (&["-movflags", "frag_keyframe+faststart"][..], false),
            (&["-movflags", "128"][..], false),
            (&["-movflags", "0x80"][..], false),
            (&["-movflags", "+frag_keyframe+global_sidx"][..], false),
            (&["-movflags", "-global_sidx"][..], true),
            (&["-movflags", "0x4000"][..], false),
            (&["-movflags", "16384"][..], false),
            // FFmpeg assigns bit 21 to skip_sidx, which does not reopen output.
            (&["-movflags", "0x200000"][..], true),
            (&["-movflags", "2097152"][..], true),
            (
                &["-movflags", "+faststart+global_sidx-faststart"][..],
                false,
            ),
            (
                &[
                    "-movflags",
                    "+faststart+global_sidx",
                    "-movflags",
                    "-global_sidx",
                ][..],
                false,
            ),
            (
                &[
                    "-movflags",
                    "+faststart+global_sidx",
                    "-movflags",
                    "-faststart-global_sidx",
                ][..],
                true,
            ),
            (&["-movflags", "all"][..], false),
            (&["-movflags", "64*2"][..], false),
            (&["-movflags", "(faststart)"][..], false),
            (&["-movflags", "faststart*1"][..], false),
            (&["-movflags=frag_keyframe+faststart"][..], false),
            (&["-movflags", "-faststart"][..], true),
            (&["-movflags", "0x84-128"][..], true),
            (&["-movflags", "+frag_keyframe+empty_moov"][..], true),
            (
                &["-movflags", "+faststart", "-movflags", "-faststart"][..],
                true,
            ),
            (&["-movflags", "+faststart", "-movflags", "0"][..], true),
            (
                &["-movflags", "+faststart", "-movflags", "+use_metadata_tags"][..],
                false,
            ),
            (&["-movflags", "0", "-movflags", "128"][..], false),
        ] {
            config.args = args.iter().map(|&arg| arg.to_owned()).collect();
            let result = config.validate_output_args();
            assert_eq!(result.is_ok(), valid, "{args:?}: {result:?}");
            if let Err(error) = result {
                assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
                assert!(error.to_string().contains("+faststart"));
                assert!(error.to_string().contains("+global_sidx"));
            }
        }
    }
    config.container = "mkv";
    config.args = vec!["-movflags".into(), "+faststart+global_sidx".into()];
    assert!(config.validate_output_args().is_ok());
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn resolves_only_configured_or_explicitly_allowed_encoders() {
        let game = tempfile::tempdir().unwrap();
        let bin = game.path().join("encoder program");
        std::os::unix::fs::symlink(env::current_exe().unwrap(), &bin).unwrap();
        std::os::unix::fs::symlink(&bin, game.path().join("ffmpeg")).unwrap();
        let config = game.path().join(CONFIG_FILE);
        assert!(matches!(
            EncoderConfig::load(game.path(), false),
            Err(ConfigError::EncoderUnavailable)
        ));
        let without_ffmpeg = tempfile::tempdir().unwrap();
        assert!(matches!(
            EncoderConfig::load_with_path(game.path(), Some(without_ffmpeg.path().as_os_str())),
            Err(ConfigError::EncoderUnavailable)
        ));
        let from_path =
            EncoderConfig::load_with_path(game.path(), Some(game.path().as_os_str())).unwrap();
        assert_eq!(from_path.program, bin.canonicalize().unwrap());

        fs::write(
            &config,
            "encoder=encoder program\nencoder_args=-metadata 'title=one take'\ncontainer=webm\n",
        )
        .unwrap();
        let options = EncoderConfig::load(game.path(), false).unwrap();
        assert_eq!(options.program, bin.canonicalize().unwrap());
        assert_eq!(options.args, ["-metadata", "title=one take"]);
        assert_eq!(options.container, "webm");

        fs::write(&config, "container=../mp4\n").unwrap();
        assert!(matches!(
            EncoderConfig::load(game.path(), false),
            Err(ConfigError::InvalidLine(1))
        ));
    }
}
