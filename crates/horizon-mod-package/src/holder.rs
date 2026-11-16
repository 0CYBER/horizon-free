//! Selected-game holder bootstrap; production IO/signals are wired by the next layer.
use crate::{
    local_control::{Channel, Marker, connect},
    locks::LockMode,
    runtime_leases::{Claim, Registry, open_directory},
};
use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, OpenOptions};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::{Duration, Instant},
};
pub const ENV: &str = "HORIZON_RUNTIME_HOLDER";
pub const ENTRY: &str = "HORIZON_RUNTIME_ENTRY";
// Keep the scanner's wire bytes contiguous even when release optimization lowers
// comparisons to several immediate constants (notably on Apple Silicon).
#[used]
pub static CAPABILITY: [u8; 26] = *b"horizon-private-holder-v2;";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tree {
    #[serde(
        serialize_with = "crate::local_control::path_out",
        deserialize_with = "crate::local_control::path_in"
    )]
    path: PathBuf,
    identity: [u64; 2],
    complete: Option<[u64; 2]>,
}
/// Retained descriptions prevent inode reuse between GUI snapshot and holder acknowledgment.
struct Snapshot {
    tree: Tree,
    directory: Dir,
    _complete: Option<File>,
}
fn identity(directory: &Dir) -> io::Result<[u64; 2]> {
    let meta = directory.dir_metadata()?;
    Ok([meta.dev(), meta.ino()])
}
fn completion(directory: &Dir) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = File::from_std(crate::runtime_leases::retained(
        directory.open_with(".installed", &options)?.into_std(),
    )?);
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() != 0 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(file)
}
fn token(file: &File) -> io::Result<[u64; 2]> {
    let meta = file.metadata()?;
    Ok([meta.dev(), meta.ino()])
}
impl Snapshot {
    fn capture(path: &Path, complete: bool) -> io::Result<Self> {
        let directory = open_directory(path).map_err(|error| {
            let error = lease_error(error);
            io::Error::new(
                error.kind(),
                format!("open lease directory {}: {error}", path.display()),
            )
        })?;
        let file = complete
            .then(|| completion(&directory))
            .transpose()
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "open installation completion marker {}: {error}",
                        path.join(".installed").display()
                    ),
                )
            })?;
        let tree = Tree {
            path: path.to_owned(),
            identity: identity(&directory)?,
            complete: file.as_ref().map(token).transpose()?,
        };
        Ok(Self {
            tree,
            directory,
            _complete: file,
        })
    }
}
fn acquire_snapshots(
    registry: &std::sync::Arc<Registry>,
    snapshots: &[Snapshot],
) -> io::Result<Vec<Claim>> {
    let claims = snapshots
        .iter()
        .map(|snapshot| {
            registry
                .lock(&snapshot.directory, LockMode::Shared)
                .map_err(|error| match error {
                    crate::runtime_leases::Error::Io(error)
                        if error.kind() == io::ErrorKind::NotFound =>
                    {
                        io::Error::new(
                            io::ErrorKind::NotFound,
                            "this managed installation has unregistered cache directories; use Horizon Launcher Play from the same launcher data directory with the intended instance and installed version (horizon-launcher play --instance <instance> --version <tag>)",
                        )
                    }
                    error => lease_error(error),
                })
        })
        .collect::<io::Result<Vec<_>>>()?;
    for snapshot in snapshots {
        snapshot.tree.recheck()?;
    }
    Ok(claims)
}

impl Tree {
    fn snapshot(&self) -> io::Result<Snapshot> {
        let snapshot = Snapshot::capture(&self.path, self.complete.is_some())?;
        if snapshot.tree.identity != self.identity || snapshot.tree.complete != self.complete {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(snapshot)
    }
    fn recheck(&self) -> io::Result<Dir> {
        self.snapshot().map(|snapshot| snapshot.directory)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub executable: OsString,
    pub arguments: Vec<OsString>,
    pub environment: Vec<(OsString, OsString)>,
    #[serde(
        serialize_with = "crate::local_control::path_out",
        deserialize_with = "crate::local_control::path_in"
    )]
    pub directory: PathBuf,
    /// Lossless cmd.exe syntax from its owning wrapper planner, never recovered from args.
    pub raw_windows: Option<OsString>,
}
impl Program {
    fn command(&self) -> io::Result<Command> {
        let mut command = Command::new(&self.executable);
        if self.raw_windows.is_some() {
            return Err(io::ErrorKind::Unsupported.into());
        }
        command.args(&self.arguments);
        command
            .current_dir(&self.directory)
            .env_clear()
            .envs(self.environment.iter().cloned());
        command.env_remove(ENV).env_remove(ENTRY);
        Ok(command)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    #[serde(deserialize_with = "crate::local_control::version")]
    version: u8,
    #[serde(
        serialize_with = "crate::local_control::path_out",
        deserialize_with = "crate::local_control::path_in"
    )]
    data: PathBuf,
    trees: Vec<Tree>,
}
/// Independently opened tree and completion objects prevent identity reuse for this process.
pub struct Acquired {
    _record: Option<File>,
    _capture: Option<Dir>,
    _claims: Vec<Claim>,
    _snapshots: Vec<Snapshot>,
}
impl Entry {
    fn validate(&self, maximum: usize) -> io::Result<()> {
        if self.trees.is_empty()
            || self.trees.len() > maximum
            || !self.trees.iter().any(|tree| tree.complete.is_some())
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(())
    }
    /// Strictly noncreating, gate before trees; used by holder and independent game entry.
    pub fn acquire(&self) -> io::Result<Acquired> {
        self.validate(16)?;
        let registry = Registry::existing(&self.data).map_err(lease_error)?;
        let snapshots = self
            .trees
            .iter()
            .map(Tree::snapshot)
            .collect::<io::Result<Vec<_>>>()?;
        let claims = acquire_snapshots(&registry, &snapshots)?;
        self.check_image(&std::env::current_exe()?)?;
        Ok(Acquired {
            _record: None,
            _capture: None,
            _claims: claims,
            _snapshots: snapshots,
        })
    }
    fn check_image(&self, executable: &Path) -> io::Result<()> {
        let directory = open_directory(executable.parent().ok_or(io::ErrorKind::InvalidInput)?)
            .map_err(lease_error)?;
        let expected = identity(&directory)?;
        if !self.trees.iter().any(|tree| tree.identity == expected) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(())
    }
    fn recheck(&self) -> io::Result<()> {
        self.trees
            .iter()
            .try_for_each(|tree| tree.recheck().map(drop))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    entry: Entry,
    program: Program,
    record: record::Reference,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Reply {
    Acquired,
    Commit,
}
fn run(encoded: OsString, claims: &mut Option<Acquired>) -> io::Result<ExitStatus> {
    let marker: Marker = serde_json::from_str(encoded.to_str().ok_or(io::ErrorKind::InvalidData)?)
        .map_err(|_| io::ErrorKind::InvalidData)?;
    // SAFETY: earliest-main private mode precedes threads and never resumes normal startup.
    unsafe {
        std::env::remove_var(ENV);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let stream = connect(&marker, deadline)?;
    let channel = Channel::authenticate(stream, &marker, false, deadline)?;
    let (channel, request) = channel.receive::<Request>()?;
    *claims = Some(request.entry.acquire()?);
    request.entry.recheck()?;
    let channel = channel.send(&Reply::Acquired)?;
    let (channel, reply) = channel.receive::<Reply>()?;
    if !matches!(reply, Reply::Commit) {
        return Err(io::ErrorKind::InvalidData.into());
    }
    request.entry.recheck()?;
    drop(channel);
    let mut command = request.program.command()?;
    command.env(ENTRY, request.record.encode()?);
    command.status()
}
/// Retain the contiguous capability marker without entering private holder mode.
#[inline(never)]
pub fn retain_capability() {
    // An address escaping through an opaque operation retains the actual static,
    // including under linker dead stripping.
    std::hint::black_box(&CAPABILITY);
}

/// Does nothing for ordinary startup. Private startup can never reach the normal game path.
pub fn enter() {
    retain_capability();
    let Some(encoded) = std::env::var_os(ENV) else {
        return;
    };
    let mut claims = None;
    let result = run(encoded, &mut claims);
    // These claims are deliberately still owned when ExitProcess/exit closes the process.
    std::process::exit(result.map_or(78, |status| status.code().unwrap_or(1)));
}
fn lease_error(error: crate::runtime_leases::Error) -> io::Error {
    match error {
        crate::runtime_leases::Error::Busy => io::ErrorKind::WouldBlock.into(),
        crate::runtime_leases::Error::Io(error) => error,
        _ => io::ErrorKind::InvalidInput.into(),
    }
}
mod controller;
#[cfg(test)]
mod entry_tests;
mod record;
pub use controller::{Prepared, Started, capable, launch};
mod game_entry;
pub use game_entry::{acquire, acquire_entry, lease_paths, managed_lease_paths};
#[cfg(test)]
mod tests;
