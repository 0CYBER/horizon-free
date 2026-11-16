//! Shared runtime lease protocol; kernel exclusion protects participating cleanup, not arbitrary deletion.
use crate::locks::{LockMode, Locked, Release, locked};
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use sha2::{Digest, Sha256};
use std::{fs, fs::Metadata, io, path::Path, sync::Arc};

#[derive(Debug)]
pub enum Error {
    Busy,
    UnsupportedFilesystem,
    Unsafe(std::path::PathBuf),
    Io(std::io::Error),
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        #[cfg(windows)]
        if matches!(error.raw_os_error(), Some(32 | 33)) {
            return Self::Busy;
        }
        if error.kind() == io::ErrorKind::WouldBlock {
            return Self::Busy;
        }
        Self::Io(error)
    }
}
impl From<fs::TryLockError> for Error {
    fn from(error: fs::TryLockError) -> Self {
        match error {
            fs::TryLockError::WouldBlock => Self::Busy,
            fs::TryLockError::Error(error) => Self::Io(error),
        }
    }
}

pub fn linked(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes junctions and other reparse points, not just symbolic links.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub fn checked(path: &Path, directory: bool) -> Result<Option<Metadata>, Error> {
    match fs::symlink_metadata(path) {
        Ok(meta)
            if linked(&meta)
                || (if directory {
                    !meta.is_dir()
                } else {
                    !meta.is_file()
                }) =>
        {
            Err(Error::Unsafe(path.to_owned()))
        }
        Ok(meta) => Ok(Some(meta)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub struct Registry {
    directory: Dir,
    gate: Locked,
    release: Release,
    create: bool,
}

// Field order releases the tree before the last reference can release the gate.
pub struct Claim {
    tree: Locked,
    registry: Arc<Registry>,
    #[cfg(not(windows))]
    key: String,
    exclusive: bool,
}

/// Serializes executable publication against shared-cache capability snapshots.
pub struct Publication {
    _lock: Locked,
    _registry: Arc<Registry>,
}

impl Claim {
    /// Remove this deleted unit's key while its exclusive tree and namespace claims remain held.
    pub fn retire(self) -> Result<(), Error> {
        if !self.exclusive {
            return Err(io::Error::from(io::ErrorKind::InvalidInput).into());
        }
        #[cfg(not(windows))]
        self.registry.directory.remove_file(&self.key)?;
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_DISPOSITION_INFO, FileDispositionInfo, SetFileInformationByHandle,
            };
            // Opening another DELETE handle conflicts with our own exclusive sharing mode.
            // Mark this owned handle delete-pending without dropping the exclusive claim;
            // its close completes unlinking before retire returns.
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            // SAFETY: the claim owns this live DELETE-capable handle; the disposition
            // buffer and exact size remain valid for the duration of the synchronous call.
            let result = unsafe {
                SetFileInformationByHandle(
                    self.tree.file().as_raw_handle().cast(),
                    FileDispositionInfo,
                    (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                    std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            };
            if result == 0 {
                return Err(io::Error::last_os_error().into());
            }
        }
        Ok(())
    }

    pub fn files(&self) -> [&fs::File; 2] {
        [self.tree.file(), self.registry.gate.file()]
    }
}

impl Registry {
    pub fn with_release(data: &Path, release: Release) -> Result<Arc<Self>, Error> {
        checked(data, true)?;
        fs::create_dir_all(data)?;
        let data = Dir::open_ambient_dir(data.canonicalize()?, cap_std::ambient_authority())?;
        Self::from_directory(data, release, true)
    }

    /// Entry never creates registry, gate, tree or record entries.
    pub fn existing(data: &Path) -> Result<Arc<Self>, Error> {
        Self::from_directory(open_directory(data)?, Release::Close, false)
    }

    fn from_directory(data: Dir, release: Release, create: bool) -> Result<Arc<Self>, Error> {
        filesystem::local(&data)?;
        if create {
            match data.create_dir("run-leases") {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        let directory = Dir::from_std_file(retained(
            data.open_dir_nofollow("run-leases")?.into_std_file(),
        )?);
        let gate = open_lock(&directory, "gate", LockMode::Shared, release, create)?;
        Ok(Arc::new(Self {
            directory,
            gate,
            release,
            create,
        }))
    }

    pub fn publication(self: &Arc<Self>, mode: LockMode) -> Result<Publication, Error> {
        let lock = open_lock(
            &self.directory,
            "publication",
            mode,
            self.release,
            self.create,
        )?;
        Ok(Publication {
            _lock: lock,
            _registry: self.clone(),
        })
    }

    /// Delete an authorized unit while excluding both runtime users and new executable
    /// publication. Authorization runs after identity recheck and before invalidation.
    /// A markerless tree is detached atomically; partial failures never restore completion.
    pub fn remove_unit<E: From<Error> + From<io::Error>>(
        self: &Arc<Self>,
        path: &Path,
        authorize: impl FnOnce(&Dir, bool) -> Result<(), E>,
    ) -> Result<(), E> {
        let parent_path = path
            .parent()
            .ok_or_else(|| Error::Unsafe(path.to_owned()))?;
        let parent = open_directory(parent_path)?;
        let meta = fs::symlink_metadata(path)?;
        checked(path, meta.is_dir())?;
        let path = path.canonicalize()?;
        let path = path.as_path();
        let is_directory = meta.is_dir();
        let tree_path = if is_directory {
            path
        } else {
            path.parent()
                .ok_or_else(|| Error::Unsafe(path.to_owned()))?
        };
        let _publication = self.publication(LockMode::Exclusive)?;
        let directory = open_directory(tree_path)?;
        let claim = self.lock(&directory, LockMode::Exclusive)?;
        let current = open_directory(tree_path)?;
        let before = directory.dir_metadata()?;
        let after = current.dir_metadata()?;
        if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
            return Err(Error::Unsafe(path.to_owned()).into());
        }
        drop(current);
        authorize(&directory, is_directory)?;
        let name = path
            .file_name()
            .ok_or_else(|| Error::Unsafe(path.to_owned()))?;
        if is_directory {
            match directory.remove_file(".installed") {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(removal_error(error).into()),
            }
        }
        // Windows retained directory handles prohibit rename; kernel claims remain held.
        drop(directory);
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let trash = format!(
            ".trash-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        parent
            .rename(name, &parent, &trash)
            .map_err(removal_error)?;
        let result = if is_directory {
            parent.remove_dir_all(&trash)
        } else {
            parent.remove_file(&trash)
        };
        if result.is_ok() && is_directory {
            claim.retire()?;
        }
        result.map_err(|error| removal_error(error).into())
    }

    /// Gate first; identity comes from a held directory, never a mutable path.
    pub fn lock(self: &Arc<Self>, tree: &Dir, mode: LockMode) -> Result<Claim, Error> {
        filesystem::local(tree)?;
        let metadata = tree.dir_metadata()?;
        let identity = format!("{}:{}", metadata.dev(), metadata.ino());
        let key = format!("{:x}", Sha256::digest(identity.as_bytes()));
        let exclusive = matches!(mode, LockMode::Exclusive);
        let tree = open_lock(&self.directory, &key, mode, self.release, self.create)?;
        Ok(Claim {
            tree,
            registry: self.clone(),
            #[cfg(not(windows))]
            key,
            exclusive,
        })
    }
}

// Windows directory rename/delete reports open sharing/mapping refusals as access
// denied too. Treat them as non-destructive Busy at this removal boundary, not a
// reason to continue deleting in place or restore a completion marker.
fn removal_error(error: io::Error) -> Error {
    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(5 | 32 | 33 | 1224)) {
        return Error::Busy;
    }
    error.into()
}

// A creator must be compatible with shared readers even before its handle closes.
fn initialize_lock(directory: &Dir, key: &str) -> std::io::Result<cap_std::fs::File> {
    let mut initial = OpenOptions::new();
    initial
        .read(true)
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        initial
            .access_mode(windows_sys::Win32::Foundation::GENERIC_READ)
            .share_mode(1);
    }
    directory.open_with(key, &initial)
}

fn open_lock(
    directory: &Dir,
    key: &str,
    mode: LockMode,
    release: Release,
    create: bool,
) -> Result<Locked, Error> {
    if create {
        match initialize_lock(directory, key) {
            Ok(file) => drop(file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(not(windows))]
    options.write(true);
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        if matches!(mode, LockMode::Exclusive) {
            use windows_sys::Win32::Storage::FileSystem::{
                DELETE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
            };
            options.access_mode(FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE);
        }
        options
            .write(matches!(mode, LockMode::Exclusive))
            .share_mode(if matches!(mode, LockMode::Shared) {
                1
            } else {
                0
            });
    }
    let file = directory
        .open_with(key, &options)
        .map_err(Error::from)?
        .into_std();
    if !file.metadata()?.is_file() {
        return Err(Error::Unsafe(key.into()));
    }
    let lock = locked(retained(file)?, mode)?;
    Ok(match release {
        Release::Unlock => lock,
        Release::Close => lock.close_only(),
    })
}

/// Resolve an authoritative absolute directory with no symlink/reparse traversal.
pub fn open_directory(path: &Path) -> Result<Dir, Error> {
    use std::path::Component;
    if !path.is_absolute() {
        return Err(Error::Unsafe(path.into()));
    }
    let root = path
        .ancestors()
        .last()
        .ok_or_else(|| Error::Unsafe(path.into()))?;
    let mut directory = Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
    for component in path
        .strip_prefix(root)
        .map_err(|_| Error::Unsafe(path.into()))?
        .components()
    {
        let Component::Normal(name) = component else {
            return Err(Error::Unsafe(path.into()));
        };
        directory = directory.open_dir_nofollow(name)?;
    }
    Ok(Dir::from_std_file(retained(directory.into_std_file())?))
}

/// Fresh retained metadata and leases must never become later stdio replacement targets.
pub(crate) fn retained(file: fs::File) -> io::Result<fs::File> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        if file.as_raw_fd() <= 2 {
            // SAFETY: file owns the source; fcntl returns a new exclusively owned CLOEXEC fd.
            let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: successful F_DUPFD_CLOEXEC returned a fresh fd; transfer sole ownership.
            return Ok(unsafe { fs::File::from_raw_fd(fd) });
        }
    }
    Ok(file)
}

pub mod cleanup;
mod filesystem;
#[cfg(test)]
mod tests;

// Unrelated libtest threads can fork while a close-owned claim is live. Its inherited
// descriptor can outlive a test's local Drop, although production retains claims to exit.
// Lifetime/drop assertions therefore need a process with no unrelated fork owners.
#[cfg(test)]
pub(crate) fn isolated_test() -> bool {
    if std::env::var_os("RUNTIME_LEASE_TEST_ISOLATED").is_some() {
        return false;
    }
    let test = std::thread::current().name().unwrap().to_owned();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--include-ignored", "--exact", &test, "--nocapture"])
        .env("RUNTIME_LEASE_TEST_ISOLATED", "1")
        .output()
        .unwrap();
    // The child must run this one test: a success that ran nothing (an ignored or filtered name) checks nothing.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed;"),
        "the isolated rerun of {test} did not pass exactly one test:\n{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    true
}
