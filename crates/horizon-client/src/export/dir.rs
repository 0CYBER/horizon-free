use super::ExportSpec;
use crate::file_id::valid_export_name;
#[cfg(not(windows))]
use cap_fs_ext::DirExt;
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, DirBuilder, OpenOptions};
use serde::Serialize;
use std::fmt;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

const EXPORTS: &str = "exports";

#[derive(Debug)]
pub enum DirError {
    InvalidName,
    Exists,
    Folder(io::Error),
}

impl fmt::Display for DirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => f.write_str("invalid export name"),
            Self::Exists => f.write_str("export folder already exists"),
            Self::Folder(err) => write!(f, "could not create export folder: {err}"),
        }
    }
}

impl std::error::Error for DirError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Folder(err) => Some(err),
            _ => None,
        }
    }
}

pub struct ExportFolder {
    dir: Dir,
    path: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct ExportManifest<'a> {
    pub spec: &'a ExportSpec,
    pub state: &'a str,
    pub frames_written: u32,
    pub build: &'a str,
    pub gpu: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ManifestState {
    Pending = 0,
    Publishing,
    Published,
    Cancelled,
    Failed,
}

impl ManifestState {
    fn from_byte(value: u8) -> Self {
        match value {
            0 => Self::Pending,
            1 => Self::Publishing,
            2 => Self::Published,
            3 => Self::Cancelled,
            4 => Self::Failed,
            _ => unreachable!("invalid manifest publication state"),
        }
    }
}

/// Shared by the publisher and the event loop. Neither holds a lock across file I/O.
#[derive(Default)]
pub struct ManifestPublication(AtomicU8);

impl ManifestPublication {
    pub fn state(&self) -> ManifestState {
        ManifestState::from_byte(self.0.load(Ordering::Acquire))
    }

    /// Cancel before rename, or report the state already reached by the publisher.
    pub fn cancel(&self) -> ManifestState {
        match self.0.compare_exchange(
            ManifestState::Pending as u8,
            ManifestState::Cancelled as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => ManifestState::Cancelled,
            Err(state) => ManifestState::from_byte(state),
        }
    }

    fn rename(&self, pending: &mut StagedFile<'_>) -> io::Result<()> {
        self.0
            .compare_exchange(
                ManifestState::Pending as u8,
                ManifestState::Publishing as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| io::Error::from(io::ErrorKind::Interrupted))?;
        let result = pending.publish();
        let state = if result.is_ok() {
            ManifestState::Published
        } else {
            ManifestState::Failed
        };
        self.0.store(state as u8, Ordering::Release);
        result
    }
}

impl ExportFolder {
    pub fn create(game_dir: &Path, name: &str) -> Result<Self, DirError> {
        if !valid_export_name(name) {
            return Err(DirError::InvalidName);
        }

        let game_path = game_dir.canonicalize().map_err(DirError::Folder)?;
        let game =
            Dir::open_ambient_dir(&game_path, ambient_authority()).map_err(DirError::Folder)?;
        match game.create_dir(EXPORTS) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(DirError::Folder(err)),
        }
        let exports = open_directory(&game, EXPORTS).map_err(DirError::Folder)?;
        exports.create_dir(name).map_err(|err| {
            if err.kind() == io::ErrorKind::AlreadyExists {
                DirError::Exists
            } else {
                DirError::Folder(err)
            }
        })?;
        let dir = open_directory(&exports, name).map_err(DirError::Folder)?;
        Ok(Self {
            dir,
            path: game_path.join(EXPORTS).join(name),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn create_new(&self, file: &str) -> io::Result<File> {
        create_new(&self.dir, file, &new_file_options())
    }

    pub(super) fn check_absent(&self, name: &str) -> io::Result<()> {
        match self.dir.symlink_metadata(name) {
            Ok(_) => Err(io::ErrorKind::AlreadyExists.into()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    pub(super) fn stage(&self) -> io::Result<Staging<'_>> {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|err| io::Error::other(err.to_string()))?;
        let name = format!(".export-{:032x}.tmp", u128::from_ne_bytes(nonce));
        let mut builder = DirBuilder::new();
        builder.recursive(false);
        #[cfg(unix)]
        cap_std::fs::DirBuilderExt::mode(&mut builder, 0o700);
        self.dir.create_dir_with(&name, &builder)?;
        Staging::open(&self.dir, &name)
    }

    /// Record the final result once, including partial output from a stopped or failed export.
    pub fn write_manifest(&self, manifest: &ExportManifest<'_>) -> io::Result<()> {
        self.write_manifest_tracked(manifest, &ManifestPublication::default())
    }

    /// Record the rename outcome before dropping the staged file and directory.
    pub fn write_manifest_tracked(
        &self,
        manifest: &ExportManifest<'_>,
        publication: &ManifestPublication,
    ) -> io::Result<()> {
        let staging = self.stage()?;
        let mut pending = staging.create_new("export.json")?;
        serde_json::to_writer_pretty(&mut pending.file, manifest)?;
        pending.file.write_all(b"\n")?;
        pending.file.sync_all()?;
        publication.rename(&mut pending)
    }
}

pub(super) struct Staging<'a> {
    dir: Option<Dir>,
    export: &'a Dir,
}

impl<'a> Staging<'a> {
    fn open(export: &'a Dir, name: &str) -> io::Result<Self> {
        #[cfg(not(windows))]
        let dir = open_directory(export, name)?;
        #[cfg(windows)]
        let dir = open_staging_dir(export, name)?;
        let staging = Self {
            dir: Some(dir),
            export,
        };
        #[cfg(unix)]
        staging.check_private()?;
        Ok(staging)
    }

    #[cfg(unix)]
    fn check_private(&self) -> io::Result<()> {
        use cap_std::fs::MetadataExt as _;

        let dir = self.dir.as_ref().expect("staging owns its directory");
        let metadata = dir.dir_metadata()?;
        // Some filesystems synthesize modes. Require ownership and exclusive
        // write access before relying on staged names for publication.
        check_private_writes(metadata.uid(), metadata.mode())?;
        #[cfg(target_os = "macos")]
        {
            use std::os::fd::AsFd as _;
            if horizon_threads::file_acl::has_extended_acl_entries(dir.as_fd())? {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "export staging directory has an extended ACL",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn create_new<'file>(
        &'file self,
        name: &'file str,
    ) -> io::Result<StagedFile<'file>> {
        let dir = self.dir.as_ref().expect("staging owns its directory");
        let options = new_file_options();
        #[cfg(unix)]
        let options = {
            use cap_std::fs::OpenOptionsExt as _;
            let mut options = options;
            options.mode(0o600);
            options
        };
        #[cfg(windows)]
        let options = {
            let mut options = options;
            use cap_std::fs::OpenOptionsExt as _;
            use windows_sys::Win32::Foundation::GENERIC_WRITE;
            use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_SHARE_READ};
            // The encoder uses a duplicate of this handle. Deny independent
            // writers and renames until the staged file is published.
            options
                .access_mode(GENERIC_WRITE | DELETE)
                .share_mode(FILE_SHARE_READ);
            options
        };
        let file = create_new(dir, name, &options)?;
        let staged = StagedFile {
            #[cfg(not(windows))]
            staging: dir,
            export: self.export,
            file,
            name,
            published: false,
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let metadata = staged.file.metadata()?;
            check_private_writes(metadata.uid(), metadata.mode())?;
        }
        Ok(staged)
    }
}

impl Drop for Staging<'_> {
    fn drop(&mut self) {
        let Some(dir) = self.dir.take() else { return };
        #[cfg(windows)]
        let _ = horizon_threads::file_publish::delete_on_close(&dir);
        #[cfg(not(windows))]
        let _ = dir.remove_open_dir();
    }
}

pub(super) struct StagedFile<'a> {
    #[cfg(not(windows))]
    staging: &'a Dir,
    export: &'a Dir,
    pub(super) file: File,
    name: &'a str,
    published: bool,
}

impl StagedFile<'_> {
    pub(super) fn publish(&mut self) -> io::Result<()> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsHandle as _;
            horizon_threads::file_publish::rename_new(
                self.file.as_handle(),
                self.export.as_handle(),
                self.name,
            )?;
        }
        #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
        {
            use rustix::fs::{RenameFlags, renameat_with};
            use rustix::io::Errno;
            match renameat_with(
                self.staging,
                self.name,
                self.export,
                self.name,
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => {}
                Err(Errno::INVAL | Errno::NOSYS | Errno::OPNOTSUPP) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "export filesystem does not support atomic no-replace rename",
                    ));
                }
                Err(err) => return Err(err.into()),
            }
        }
        #[cfg(not(any(
            windows,
            target_os = "linux",
            target_os = "android",
            target_vendor = "apple"
        )))]
        return Err(io::ErrorKind::Unsupported.into());

        self.published = true;
        Ok(())
    }
}

impl Drop for StagedFile<'_> {
    fn drop(&mut self) {
        if !self.published {
            #[cfg(windows)]
            let _ = horizon_threads::file_publish::delete_on_close(&self.file);
            #[cfg(not(windows))]
            let _ = self.staging.remove_file(self.name);
        }
    }
}

fn new_file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    options
}

fn create_new(dir: &Dir, name: &str, options: &OpenOptions) -> io::Result<File> {
    if !valid_export_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid export file name",
        ));
    }
    dir.open_with(name, options)
        .map(cap_std::fs::File::into_std)
}

#[cfg(not(windows))]
fn open_directory(parent: &Dir, name: &str) -> io::Result<Dir> {
    parent.open_dir_nofollow(name)
}

#[cfg(unix)]
fn check_private_writes(uid: u32, mode: u32) -> io::Result<()> {
    if uid != rustix::process::geteuid().as_raw() || mode & 0o022 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "export staging must be writable only by the player",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn open_directory(parent: &Dir, name: &str) -> io::Result<Dir> {
    open_windows_dir(parent, name, 0)
}

#[cfg(windows)]
fn open_staging_dir(parent: &Dir, name: &str) -> io::Result<Dir> {
    use windows_sys::Win32::Storage::FileSystem::DELETE;
    open_windows_dir(parent, name, DELETE)
}

#[cfg(windows)]
fn open_windows_dir(parent: &Dir, name: &str, extra_access: u32) -> io::Result<Dir> {
    use cap_std::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Foundation::GENERIC_READ;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE,
    };

    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(FollowSymlinks::No)
        .access_mode(GENERIC_READ | FILE_TRAVERSE | extra_access)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    let file = parent.open_with(name, &options)?.into_std();
    let metadata = file.metadata()?;
    if !metadata.is_dir() || crate::file_id::reparse_point(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "export directory is not a plain directory",
        ));
    }
    Ok(Dir::from_std_file(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn invalid_names_do_not_touch_disk() {
        let game = tempfile::tempdir().unwrap();
        for invalid in [
            "..", "a/b", r"a\b", "CoN.mp4", "name<", "name>", "name:", "name\"", "name|", "name?",
            "name*",
        ] {
            assert!(matches!(
                ExportFolder::create(game.path(), invalid),
                Err(DirError::InvalidName)
            ));
        }
        assert!(!game.path().join("exports").exists());
    }

    #[test]
    fn export_folders_and_files_are_created_only_once() {
        let game = tempfile::tempdir().unwrap();
        let folder = ExportFolder::create(game.path(), "take").unwrap();
        assert!(matches!(
            ExportFolder::create(game.path(), "take"),
            Err(DirError::Exists)
        ));

        folder
            .create_new("frame_000000.png")
            .unwrap()
            .write_all(b"original")
            .unwrap();
        let err = folder.create_new("frame_000000.png").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read(folder.path().join("frame_000000.png")).unwrap(),
            b"original"
        );
        assert_eq!(
            folder.create_new("../escape").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_exports_cannot_redirect_writes() {
        let game = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let exports = game.path().join("exports");
        std::os::unix::fs::symlink(outside.path(), &exports).unwrap();
        assert!(matches!(
            ExportFolder::create(game.path(), "take"),
            Err(DirError::Folder(_))
        ));

        fs::remove_file(&exports).unwrap();
        let folder = ExportFolder::create(game.path(), "take").unwrap();
        fs::rename(folder.path(), exports.join("moved")).unwrap();
        std::os::unix::fs::symlink(outside.path(), folder.path()).unwrap();
        folder.create_new("frame_000000.png").unwrap();
        assert!(exports.join("moved/frame_000000.png").is_file());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn rejected_staging_directory_is_removed() {
        use std::os::unix::fs::PermissionsExt as _;

        let game = tempfile::tempdir().unwrap();
        let folder = ExportFolder::create(game.path(), "take").unwrap();
        let name = ".export-rejected.tmp";
        let path = folder.path().join(name);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o770)).unwrap();

        assert!(matches!(
            Staging::open(&folder.dir, name),
            Err(err) if err.kind() == io::ErrorKind::PermissionDenied
        ));
        assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn staged_publication_follows_the_open_export_directory() {
        let game = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let folder = ExportFolder::create(game.path(), "take").unwrap();
        let staging = folder.stage().unwrap();
        let mut output = staging.create_new("video.mp4").unwrap();
        output.file.write_all(b"video").unwrap();

        let moved = folder.path().with_file_name("moved");
        fs::rename(folder.path(), &moved).unwrap();
        std::os::unix::fs::symlink(outside.path(), folder.path()).unwrap();
        output.publish().unwrap();
        drop(output);
        drop(staging);

        assert_eq!(fs::read(moved.join("video.mp4")).unwrap(), b"video");
        assert_eq!(fs::read_dir(moved).unwrap().count(), 1);
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn manifest_records_the_export_result() {
        use crate::export::Format;

        let game = tempfile::tempdir().unwrap();
        let folder = ExportFolder::create(game.path(), "take").unwrap();
        let spec = ExportSpec {
            name: "take".into(),
            size: [1080, 1920],
            fps: (30_000, 1001),
            frames: 0,
            format: Format::PngSequence,
            hud: true,
        };
        let stopped = ExportManifest {
            spec: &spec,
            state: "stopped",
            frames_written: 9,
            build: "build-id",
            gpu: "test adapter",
        };
        let publication = ManifestPublication::default();
        assert_eq!(publication.cancel(), ManifestState::Cancelled);
        let cancelled = folder
            .write_manifest_tracked(&stopped, &publication)
            .unwrap_err();
        assert_eq!(cancelled.kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 0);

        folder.write_manifest(&stopped).unwrap();
        let error = folder
            .write_manifest(&ExportManifest {
                spec: &spec,
                state: "failed",
                frames_written: 0,
                build: "retry",
                gpu: "test adapter",
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 1);

        let json: serde_json::Value =
            serde_json::from_reader(File::open(folder.path().join("export.json")).unwrap())
                .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "spec": {"name": "take", "size": [1080, 1920], "fps": [30_000, 1001], "frames": 0, "format": "png_sequence", "hud": true},
                "state": "stopped", "frames_written": 9, "build": "build-id", "gpu": "test adapter"
            })
        );
    }
}
