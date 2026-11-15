//! Shared tree-lock primitives. Persistent files are lock inodes, never busy markers.
use crate::native_cache::{Error, LockMode, Release};
use cap_std::fs::Dir;
use std::{fs, path::Path, process::Command, sync::Arc};

pub(crate) const SOURCE: &str = "HORIZON_RUNTIME_SOURCE";

pub(crate) fn mark(command: &mut Command, source: &Path) {
    command.env(SOURCE, source);
}

// Outer None inherits the environment; inner None explicitly removes the variable.
fn override_value<'a>(command: &'a Command, key: &str) -> Option<Option<&'a std::ffi::OsStr>> {
    command
        .get_envs()
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value)
}

// This adapter owns localized launcher errors; the kernel protocol lives below game/launcher.
pub(crate) struct Registry(Arc<horizon_mod_package::runtime_leases::Registry>);
pub(crate) use horizon_mod_package::runtime_leases::Claim;
impl Registry {
    fn at(data: &Path) -> Result<Arc<Self>, Error> {
        Self::with_release(data, Release::Unlock)
    }
    pub(crate) fn with_release(data: &Path, release: Release) -> Result<Arc<Self>, Error> {
        horizon_mod_package::runtime_leases::Registry::with_release(data, release)
            .map(|registry| Arc::new(Self(registry)))
            .map_err(Into::into)
    }
    pub(crate) fn open() -> Result<Arc<Self>, Error> {
        Self::at(&crate::backend::data_dir())
    }
    pub(crate) fn lock(self: &Arc<Self>, tree: &Dir, mode: LockMode) -> Result<Claim, Error> {
        self.0.lock(tree, mode).map_err(Into::into)
    }
}

/// Launcher-owned primitives-in-use; this is not a standalone cleanup safety guarantee.
pub(crate) fn shared(command: &Command) -> Result<Vec<Claim>, Error> {
    let logs = override_value(command, "HORIZON_HOME").map_or_else(
        || std::env::var_os("HORIZON_HOME").map(std::path::PathBuf::from),
        |value| value.map(std::path::PathBuf::from),
    );
    let source = override_value(command, SOURCE)
        .flatten()
        .map(Path::new)
        .or_else(|| {
            let program = Path::new(command.get_program());
            program.is_absolute().then(|| program.parent()).flatten()
        });
    let mut paths = Vec::new();
    if let Some(encoded) = override_value(command, GAME).flatten() {
        let words: Vec<std::ffi::OsString> = serde_json::from_str(
            encoded
                .to_str()
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidData))?,
        )
        .map_err(std::io::Error::other)?;
        let image = words
            .first()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
        let mut game = Command::new(image);
        game.args(&words[1..]);
        if let Some(cwd) = command.get_current_dir() {
            game.current_dir(cwd);
        }
        for (key, value) in command.get_envs() {
            match value {
                Some(value) => {
                    game.env(key, value);
                }
                None => {
                    game.env_remove(key);
                }
            }
        }
        let selection = horizon_mod_package::selection::LaunchArguments::for_command(&game)?;
        paths.push(selection.context.game_dir);
        paths.push(selection.context.home);
        let cwd = command.get_current_dir().unwrap_or(Path::new("."));
        for argument in selection.rest {
            if let horizon_mod_package::selection::Argument::Value(name, value) = argument {
                if name == "--resources" {
                    paths.push(cwd.join(value));
                }
            }
        }
    }
    if let Some(home) = command.get_current_dir() {
        let home = home.canonicalize()?;
        for relative in [".horizon-cache", "wasm-cache", "cache"] {
            crate::storage::mark_used(&home.join(relative))?;
        }
        use horizon_mod_package::{mod_list, Package};
        for folder in [mod_list::DIR, mod_list::NATIVE_DIR] {
            for path in horizon_mod_package::find(&home.join(folder)) {
                if let Ok(package) = Package::read(&path) {
                    if crate::browser::java::is_enabled(&home, &package.manifest.id) {
                        for jar in package.manifest.jar {
                            crate::storage::mark_used(&jar.cache_path(&home.join("jars")))?;
                        }
                    }
                }
            }
        }
        paths.push(home);
    }
    if let Some(logs) = logs.filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(&logs)?;
        paths.push(logs.canonicalize()?);
    }
    if let Some(source) = source {
        let source = source.canonicalize()?;
        if let Some(parent) = source.parent() {
            paths.push(parent.to_owned());
        }
        paths.push(source);
    }
    // Native clients read prepared data from a different installed version. Hold that
    // tree explicitly as well as the executable's tree for the child's entire lifetime.
    // prepare_command also runs non-game commands (including hooks and tools), whose
    // arguments must not be interpreted by the game's parser.
    if command.get_args().any(|argument| argument == "--resources") {
        let arguments = horizon_mod_package::selection::LaunchArguments::for_command(command)?;
        for argument in arguments.rest {
            if let horizon_mod_package::selection::Argument::Value(flag, resources) = argument {
                if flag == "--resources" {
                    paths.push(
                        command
                            .get_current_dir()
                            .unwrap_or(Path::new("."))
                            .join(resources)
                            .canonicalize()?,
                    );
                }
            }
        }
    }
    paths = paths
        .into_iter()
        .map(|path| path.canonicalize())
        .collect::<Result<_, _>>()?;
    let registry = Registry::open()?;
    let data = crate::backend::data_dir().canonicalize()?;
    for leaf in paths.clone() {
        if leaf.starts_with(&data) && leaf != data {
            for parent in leaf.ancestors().skip(1).take_while(|p| *p != data) {
                paths.push(parent.to_owned());
            }
        }
    }
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    for path in &paths {
        crate::storage::mark_used(path)?;
    }
    paths
        .into_iter()
        .map(|path| {
            let directory = Dir::open_ambient_dir(path, cap_std::ambient_authority())?;
            registry.lock(&directory, LockMode::Shared)
        })
        .collect()
}

#[cfg(test)]
mod tests;

const GAME: &str = "HORIZON_LAUNCHER_GAME_COMMAND";

/// Preserve the engine-owned argument grammar before wrappers obscure it.
pub(crate) fn snapshot(command: &mut Command) {
    let words: Vec<_> = std::iter::once(command.get_program().to_owned())
        .chain(command.get_args().map(std::ffi::OsStr::to_owned))
        .collect();
    // serde serializes OsString as a primitive platform enum and integer/string sequence;
    // Vec<OsString> contains no fallible custom serializers or map keys.
    command.env(
        GAME,
        serde_json::to_string(&words).expect("OS string serialization"),
    );
}

/// Unix holder owns the wrapped program; Windows retains its existing launch path.
pub(crate) fn spawn(command: &mut Command, piped: bool) -> std::io::Result<std::process::Child> {
    spawn_inner(command, piped).map_err(|error| {
        match write_launch_failure(&crate::backend::data_dir().join("logs"), command, &error) {
            Ok(path) => std::io::Error::new(
                error.kind(),
                format!("{error}; diagnostic log: {}", path.display()),
            ),
            Err(log_error) => std::io::Error::new(
                error.kind(),
                format!("{error}; could not write launcher diagnostic log: {log_error}"),
            ),
        }
    })
}

fn write_launch_failure(
    directory: &Path,
    command: &Command,
    error: &std::io::Error,
) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write;
    fs::create_dir_all(directory)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_nanos();
    let path = directory.join(format!("launch-failure-{}-{nonce}.log", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    // Values can contain tokens (including custom wrapper arguments). Keep argument positions
    // and switch names, but never persist credentials or the private holder bootstrap env.
    let args: Vec<_> = command
        .get_args()
        .map(|arg| {
            let value = arg.to_string_lossy();
            if value.starts_with("--") && !value.contains('=') {
                value.into_owned()
            } else {
                "<redacted>".to_owned()
            }
        })
        .collect();
    writeln!(
        file,
        "program: {:?}\narguments (values redacted): {args:?}\ncwd: {:?}\nerror: {error}",
        command.get_program(),
        command.get_current_dir()
    )?;
    record_launch_path(&mut file, "program", Path::new(command.get_program()))?;
    let cwd = std::path::absolute(command.get_current_dir().unwrap_or(Path::new(".")))?;
    record_launch_path(&mut file, "working directory", &cwd)?;
    // Snapshotting may fail before exec: report completion-marker candidates, not only
    // the executable whose name used to hide all of these distinct failures.
    let image = cwd.join(command.get_program());
    let data = crate::backend::data_dir();
    for parent in image
        .ancestors()
        .skip(1)
        .take_while(|path| path.starts_with(&data))
    {
        record_launch_path(
            &mut file,
            "completion marker candidate",
            &parent.join(".installed"),
        )?;
    }
    #[cfg(target_os = "macos")]
    record_launch_path(
        &mut file,
        "control socket fallback parent",
        Path::new("/private/tmp"),
    )?;
    #[cfg(all(unix, not(target_os = "macos")))]
    record_launch_path(
        &mut file,
        "control socket fallback parent",
        Path::new("/tmp"),
    )?;
    for key in ["XDG_RUNTIME_DIR", "TMPDIR"] {
        let value = override_value(command, key).map_or_else(
            || std::env::var_os(key),
            |value| value.map(std::ffi::OsStr::to_owned),
        );
        writeln!(file, "{key}: {value:?}")?;
        if let Some(value) = value {
            record_launch_path(&mut file, key, Path::new(&value))?;
        }
    }
    Ok(path)
}

fn record_launch_path(file: &mut fs::File, role: &str, path: &Path) -> std::io::Result<()> {
    use std::io::Write;
    match fs::metadata(path) {
        Ok(metadata) => writeln!(
            file,
            "{role}: {}: exists=true is-dir={} permissions={:?}",
            path.display(),
            metadata.is_dir(),
            metadata.permissions()
        ),
        Err(error) => writeln!(file, "{role}: {}: stat failed: {error}", path.display()),
    }
}

fn spawn_inner(command: &mut Command, piped: bool) -> std::io::Result<std::process::Child> {
    #[cfg(not(unix))]
    let _ = piped;
    #[cfg(unix)]
    {
        use horizon_mod_package::{holder, selection::LaunchArguments};
        let encoded = override_value(command, GAME)
            .flatten()
            .map(std::ffi::OsStr::to_owned);
        if let Some(encoded) = encoded {
            let words: Vec<std::ffi::OsString> =
                serde_json::from_str(encoded.to_str().ok_or(std::io::ErrorKind::InvalidData)?)?;
            let image = words.first().ok_or(std::io::ErrorKind::InvalidData)?;
            let cwd = std::path::absolute(command.get_current_dir().unwrap_or(Path::new(".")))?;
            let image = cwd.join(image);
            let executable = image.canonicalize().map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("resolve game executable {}: {error}", image.display()),
                )
            })?;
            if holder::capable(&executable)? {
                let mut game = Command::new(&executable);
                game.args(&words[1..]).current_dir(&cwd);
                let mut environment: std::collections::BTreeMap<_, _> =
                    std::env::vars_os().collect();
                for (key, value) in command.get_envs() {
                    if let Some(value) = value {
                        environment.insert(key.to_owned(), value.to_owned());
                    } else {
                        environment.remove(key);
                    }
                    match value {
                        Some(value) => {
                            game.env(key, value);
                        }
                        None => {
                            game.env_remove(key);
                        }
                    }
                }
                environment.remove(std::ffi::OsStr::new(GAME));
                let selection = LaunchArguments::for_command(&game)?;
                let data = crate::backend::data_dir().canonicalize()?;
                let trees = holder::managed_lease_paths(&executable, &selection, &data, &cwd)?;
                let program = holder::Program {
                    executable: command.get_program().to_owned(),
                    arguments: command.get_args().map(std::ffi::OsStr::to_owned).collect(),
                    environment: environment.into_iter().collect(),
                    directory: cwd,
                    raw_windows: None,
                };
                let prepared =
                    holder::Prepared::new(data.clone(), trees, program, &selection.context)?;
                let marker = horizon_mod_package::local_control::Marker::runtime()?;
                let mut holder_command = Command::new(executable);
                // Private bootstrap ignores arguments; retaining the final command's arguments
                // also permits receiver harnesses whose process entry parses test filters first.
                holder_command.args(command.get_args());
                // Spawn inherits the caller's chosen pipe/terminal configuration.
                // Transfer stdio through the original Command is not possible; callers set
                // the same policy on the holder via the explicit mode below.
                if piped {
                    holder_command
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped());
                }
                if piped {
                    use std::os::unix::process::CommandExt;
                    holder_command.process_group(0);
                }
                return match holder::launch(
                    holder_command,
                    &marker,
                    &prepared,
                    std::time::Instant::now() + std::time::Duration::from_secs(10),
                )? {
                    holder::Started::Holder { child } => Ok(child),
                    holder::Started::Legacy(mut legacy) => {
                        if piped {
                            legacy
                                .stdin(std::process::Stdio::piped())
                                .stdout(std::process::Stdio::piped())
                                .stderr(std::process::Stdio::piped());
                            use std::os::unix::process::CommandExt;
                            legacy.process_group(0);
                        }
                        spawn_program(&mut legacy)
                    }
                };
            }
        }
    }
    command.env_remove(GAME);
    spawn_program(command)
}

fn spawn_program(command: &mut Command) -> std::io::Result<std::process::Child> {
    let cwd = std::path::absolute(command.get_current_dir().unwrap_or(Path::new(".")))?;
    command.spawn().map_err(|error| {
        if let Err(cwd_error) = fs::metadata(&cwd) {
            return std::io::Error::new(
                cwd_error.kind(),
                format!("open working directory {}: {cwd_error}", cwd.display()),
            );
        }
        std::io::Error::new(
            error.kind(),
            format!(
                "execute {} (working directory {}): {error}",
                Path::new(command.get_program()).display(),
                cwd.display()
            ),
        )
    })
}

/// Publish stable tree lock inodes before completing a newly installed unit.
pub(crate) fn publish(path: &Path) -> Result<(), Error> {
    let path = path.canonicalize()?;
    let registry = Registry::open()?;
    let data = crate::backend::data_dir().canonicalize()?;
    if !path.starts_with(&data) {
        return Ok(());
    }
    let mut units: Vec<_> = path
        .ancestors()
        .take_while(|unit| *unit != data)
        .map(Path::to_owned)
        .collect();
    for relative in [
        ".local/horizon-client/26.3",
        "resources",
        ".horizon-cache",
        "wasm-cache",
        "cache",
        "logs",
        "crash-reports",
    ] {
        let child = path.join(relative);
        if child.try_exists()? {
            units.extend(
                child
                    .ancestors()
                    .take_while(|unit| *unit != path)
                    .map(Path::to_owned),
            );
        }
    }
    for unit in units {
        let directory = Dir::open_ambient_dir(&unit, cap_std::ambient_authority())?;
        drop(registry.lock(&directory, LockMode::Shared)?);
    }
    Ok(())
}

/// Register older installations before startup can offer cleanup. Never scan Cargo artifacts.
pub(crate) fn publish_existing() -> Result<(), Error> {
    let data = crate::backend::data_dir();
    if !data.try_exists()? {
        return Ok(());
    }
    let mut units = vec![crate::backend::build_dir().join("native/target")];
    for collection in [
        data.join("versions"),
        data.join("instances"),
        crate::backend::build_dir().join("native/server-builds"),
    ] {
        match fs::read_dir(collection) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    if entry.file_type()?.is_dir() {
                        let path = entry.path();
                        units.push(path.clone());
                        for relative in [
                            ".local/horizon-client/26.3",
                            "resources",
                            ".horizon-cache",
                            "wasm-cache",
                            "cache",
                            "logs",
                            "crash-reports",
                        ] {
                            units.push(path.join(relative));
                        }
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    for profile in ["debug", "release", "fast", "developer"] {
        units.push(
            crate::backend::build_dir()
                .join("native/target")
                .join(profile),
        );
    }
    for unit in units {
        if unit.try_exists()? {
            publish(&unit)?;
        }
    }
    Ok(())
}

/// Held from before an executable becomes visible through its completion publication.
pub(crate) fn installation() -> Result<horizon_mod_package::runtime_leases::Publication, Error> {
    use horizon_mod_package::{
        locks::{LockMode, Release},
        runtime_leases::Registry,
    };
    Registry::with_release(&crate::backend::data_dir(), Release::Close)?
        .publication(LockMode::Shared)
        .map_err(Into::into)
}

pub(crate) mod control;
