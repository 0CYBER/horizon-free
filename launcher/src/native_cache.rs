//! Disposable native compilation outputs only. Installed games and player data are never targets.
//! Keep Cargo's profile directories and lock files in place: unlinking a locked inode would let
//! another Cargo acquire a replacement lock while cleanup was still deleting its outputs.
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use std::fs::{self, File, Metadata};
use std::io;
use std::path::{Path, PathBuf};

pub(crate) const PROFILES: [&str; 2] = ["fast", "release"];

#[derive(Debug)]
pub enum Error {
    Busy,
    UnprovenExclusion,
    IncompleteUnit,
    /// The folder whose file system was refused, once a caller knows it.
    UnsupportedFilesystem(Option<PathBuf>),
    Unsafe(PathBuf),
    /// An installed native build takes its resources from this unit; name it.
    Dependent(String),
    Io(io::Error),
}

impl Error {
    /// The same error, naming `path` as the refused folder when it names none yet: the shared
    /// check only sees a directory handle.
    pub(crate) fn on(self, path: &Path) -> Self {
        match self {
            Self::UnsupportedFilesystem(None) => Self::UnsupportedFilesystem(Some(path.to_owned())),
            error => error,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(crate) struct Cache {
    root: PathBuf,
    directory: Dir,
    _lease: std::sync::Arc<Locked>,
}

pub(crate) use horizon_platform::mod_package::locks::{LockMode, Locked, Release};

pub(crate) use horizon_platform::mod_package::runtime_leases::linked;
pub(crate) fn checked(path: &Path, directory: bool) -> Result<Option<Metadata>, Error> {
    horizon_platform::mod_package::runtime_leases::checked(path, directory).map_err(Into::into)
}
impl From<horizon_platform::mod_package::runtime_leases::Error> for Error {
    fn from(error: horizon_platform::mod_package::runtime_leases::Error) -> Self {
        use horizon_platform::mod_package::runtime_leases::Error as Shared;
        match error {
            Shared::Busy => Self::Busy,
            Shared::UnsupportedFilesystem => Self::UnsupportedFilesystem(None),
            Shared::Unsafe(path) => Self::Unsafe(path),
            Shared::Io(error) => Self::Io(error),
        }
    }
}

fn lock(path: &Path) -> Result<Locked, Error> {
    checked(path, false)?;
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    locked(file, LockMode::Exclusive)
}

/// The exclusive lock on the file `path`, checked as the build lease is (no link, nothing but a
/// file), taken once it is free. It is polled, so `stop` can end the wait: `Ok(None)` then,
/// also when `stop` came while the holder let go.
pub(crate) fn waited(path: &Path, stop: &dyn Fn() -> bool) -> Result<Option<Locked>, Error> {
    loop {
        match lock(path) {
            Ok(locked) => return Ok((!stop()).then_some(locked)),
            Err(Error::Busy) => {}
            Err(error) => return Err(error),
        }
        if stop() {
            return Ok(None);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// The lock `name` beside the build lease, in the native root validated as [`Cache::acquire`]
/// does, waited for. It never excludes the build lease itself.
pub(crate) fn side_lock(build_dir: &Path, name: &str) -> Result<Locked, Error> {
    let (root, _) = native_root(build_dir)?;
    Ok(waited(&root.join(name), &|| false)?.expect("a wait that never stops ends locked"))
}

/// The folder `dir`, made if missing: when it lies in `<build_dir>/native`, every folder from the
/// native root down to it is opened (or made) without following a link or junction, as
/// [`Cache::acquire`] opens the root. Elsewhere (a test's scratch folder) only `dir` itself is
/// checked.
pub(crate) fn checked_tree(build_dir: &Path, dir: &Path) -> Result<(), Error> {
    let Ok(below) = dir.strip_prefix(build_dir.join("native")) else {
        checked(dir, true)?;
        fs::create_dir_all(dir)?;
        return Ok(());
    };
    let (_, mut directory) = native_root(build_dir)?;
    for component in below.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(Error::Unsafe(dir.to_owned()));
        };
        match directory.create_dir(name) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        directory = directory.open_dir_nofollow(name)?;
    }
    Ok(())
}

/// `<build_dir>/native`, made if missing and opened from the filesystem root without following
/// a link.
fn native_root(build_dir: &Path) -> Result<(PathBuf, Dir), Error> {
    checked(build_dir, true)?;
    fs::create_dir_all(build_dir)?;
    let root = build_dir.canonicalize()?.join("native");
    checked(&root, true)?;
    fs::create_dir_all(&root)?;
    let anchor = root.ancestors().last().expect("canonical root is absolute");
    let mut directory = Dir::open_ambient_dir(anchor, cap_std::ambient_authority())?;
    for component in root.strip_prefix(anchor).expect("root anchor").components() {
        directory = directory.open_dir_nofollow(component.as_os_str())?;
    }
    Ok((root, directory))
}

pub(crate) fn locked(file: File, mode: LockMode) -> Result<Locked, Error> {
    horizon_platform::mod_package::locks::locked(file, mode).map_err(|error| match error {
        fs::TryLockError::WouldBlock => Error::Busy,
        fs::TryLockError::Error(error) => Error::Io(error),
    })
}

fn size(path: &Path) -> Result<u64, Error> {
    let meta = fs::symlink_metadata(path)?;
    if linked(&meta) {
        return Err(Error::Unsafe(path.to_owned()));
    }
    if meta.is_file() {
        return Ok(meta.len());
    }
    if !meta.is_dir() {
        return Err(Error::Unsafe(path.to_owned()));
    }
    let mut bytes = 0u64;
    for entry in fs::read_dir(path)? {
        bytes = bytes.saturating_add(size(&entry?.path())?);
    }
    Ok(bytes)
}

impl Cache {
    pub(crate) fn share(&self) -> Result<Self, Error> {
        Ok(Self {
            root: self.root.clone(),
            directory: self.directory.try_clone()?,
            _lease: self._lease.clone(),
        })
    }

    /// One cross-process lease covers staging, Cargo, installation/export and cleanup.
    /// OS locks release on exit, including a crash; a leftover lock file is not a busy marker.
    pub(crate) fn acquire(build_dir: &Path) -> Result<Self, Error> {
        let (root, directory) = native_root(build_dir)?;
        let lease = lock(&root.join(".build.lock"))?;
        Ok(Self {
            root,
            directory,
            _lease: std::sync::Arc::new(lease),
        })
    }

    pub(crate) fn bytes(&self) -> Result<u64, Error> {
        self.process(None, false)
    }
    pub(crate) fn clear(&self) -> Result<u64, Error> {
        crate::cache_cleanup::Access::IncompleteGame.check(crate::cache_cleanup::Proof::Lease)?;
        self.process(None, true)
    }
    #[cfg(test)]
    pub(crate) fn trim(&self, limit: u64) -> Result<u64, Error> {
        if limit == 0 {
            return Ok(0);
        }
        if self.bytes()? <= limit {
            return Ok(0);
        }
        crate::cache_cleanup::Access::IncompleteGame.check(crate::cache_cleanup::Proof::Lease)?;
        self.process(Some(limit), true)
    }

    // Detect a managed directory redirected through an ancestor link after taking the lease.
    fn unchanged_path(&self, path: &Path) -> Result<(), Error> {
        if path.canonicalize()? != path {
            return Err(Error::Unsafe(path.to_owned()));
        }
        Ok(())
    }

    fn process(&self, limit: Option<u64>, clear: bool) -> Result<u64, Error> {
        checked(&self.root, true)?;
        self.unchanged_path(&self.root)?;
        let target = self.root.join("target");
        if checked(&target, true)?.is_none() {
            return Ok(0);
        }
        self.unchanged_path(&target)?;
        let mut locks = Vec::new();
        let mut folders = Vec::new();
        for profile in PROFILES {
            let folder = target.join(profile);
            if checked(&folder, true)?.is_some() {
                // Also protects an older launcher's Cargo or one started outside our lease.
                self.unchanged_path(&folder)?;
                locks.push(lock(&folder.join(".cargo-lock"))?);
                folders.push(folder);
            }
        }
        // Validate ALL profiles and descendants before deleting ANY output. Never traverse links.
        let mut entries = Vec::new();
        let mut bytes = 0u64;
        for folder in folders {
            for entry in fs::read_dir(folder)? {
                let entry = entry?;
                if entry.file_name() == ".cargo-lock" {
                    continue;
                }
                bytes = bytes.saturating_add(size(&entry.path())?);
                entries.push(entry.path());
            }
        }
        if !clear {
            return Ok(bytes);
        }
        if limit.is_some_and(|limit| bytes <= limit) {
            return Ok(0);
        }
        let records = self.root.join("built.json");
        let has_records = checked(&records, false)?.is_some();
        for path in entries {
            self.unchanged_path(&path)?;
            // The launcher lease and all Cargo locks remain held through removal. These are
            // fixed children of the two validated cache profiles, never caller-supplied paths.
            if fs::symlink_metadata(&path)?.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
        if has_records {
            fs::remove_file(records)?;
        }
        drop(locks);
        Ok(bytes)
    }
}

#[cfg(test)]
#[path = "native_cache/tests.rs"]
mod tests;
