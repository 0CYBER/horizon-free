//! Offline, copy-first relocation. The fixed bootstrap is never resolved through Settings.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Roots {
    pub launcher: PathBuf,
    pub game: PathBuf,
    pub build: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Location {
    version: u32,
    roots: Roots,
}

pub fn bootstrap_path() -> Result<PathBuf> {
    // Portable installs and tests can keep their location pointer separate from the user profile.
    if let Some(path) = std::env::var_os("HORIZON_LAUNCHER_LOCATION_FILE").filter(|p| !p.is_empty())
    {
        return pointer_path(Path::new(&path));
    }
    if cfg!(test) {
        return Ok(test_home().join("location.json"));
    }
    dirs::config_dir()
        .map(|p| p.join("horizon-launcher-location").join("location.json"))
        .ok_or_else(|| "Cannot locate the platform configuration folder for location.json".into())
}

/// Where the library's own tests keep the launcher's and the game's data (`launcher`, `game`,
/// `location.json` inside) unless `HORIZON_LAUNCHER_HOME` or `HORIZON_HOME` name another:
/// never a player's install. A fixed path, not an environment change, so it holds from the
/// first call on any thread, and a test that runs its own binary again shares it.
///
/// The temporary folder in its real spelling, as [`crate::backend::build_dir`] resolves it:
/// Windows can name it by its 8.3 short form (`C:\Users\RUNNER~1\…`), so every test path
/// compares under one spelling.
pub fn test_home() -> PathBuf {
    let temp = std::env::temp_dir();
    crate::backend::plain(temp.canonicalize().unwrap_or(temp)).join("horizon-launcher-lib-tests")
}

fn overridden(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

fn has_override() -> bool {
    overridden("HORIZON_LAUNCHER_HOME") || overridden("HORIZON_HOME")
}

/// Roots resolved once per process. `retry` clears it so a repaired location is read again.
static LOCATION: Mutex<Option<Result<Option<Roots>>>> = Mutex::new(None);
/// A startup failure falls back to the default folders, so a window can still open.
static FORCE_DEFAULT: AtomicBool = AtomicBool::new(false);
static STARTUP_ERROR: Mutex<Option<String>> = Mutex::new(None);
static STARTUP_LEASE: Mutex<Option<File>> = Mutex::new(None);

fn location() -> std::sync::MutexGuard<'static, Option<Result<Option<Roots>>>> {
    LOCATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn open_error(error: &str) -> String {
    format!("Cannot open the data folder: {error}")
}

fn remember_startup_error(error: Option<String>) {
    *STARTUP_ERROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = error;
}

/// Stable for the lifetime of this process, including errors. Environment overrides are
/// applied per root by the caller; fully isolated callers need not read the real bootstrap.
pub fn configured_roots() -> Result<Option<Roots>> {
    if FORCE_DEFAULT.load(Ordering::Acquire) {
        return Ok(None);
    }
    // A unit test never reaches the player's real folders: unless it points at a location file
    // of its own, its roots are this test process's, under the temporary folder.
    if cfg!(test) && !overridden("HORIZON_LAUNCHER_LOCATION_FILE") {
        let home =
            std::env::temp_dir().join(format!("horizon-launcher-test-{}", std::process::id()));
        return Ok(Some(Roots {
            launcher: home.join("launcher"),
            game: home.join("game"),
            build: home.join("launcher"),
        }));
    }
    let mut slot = location();
    if slot.is_none() {
        let resolved = if overridden("HORIZON_LAUNCHER_HOME") && overridden("HORIZON_HOME") {
            Ok(None)
        } else {
            match bootstrap_path() {
                Ok(bootstrap) => resolve_at(&bootstrap),
                Err(error) => Err(error),
            }
        };
        *slot = Some(resolved);
    }
    slot.as_ref().expect("roots are resolved above").clone()
}

pub fn validate() -> Result<()> {
    configured_roots().map(|_| ())
}

/// Run before any launcher path is used. A broken location never ends the process: it is
/// recorded for Settings, and the default folders stand in until the player retries or keeps
/// them. The returned text is also [`startup_error`].
pub fn startup() -> Option<String> {
    let mut problems = Vec::new();
    if let Err(error) = configured_roots() {
        FORCE_DEFAULT.store(true, Ordering::Release);
        problems.push(open_error(&error));
    }
    match acquire_startup_lock() {
        Ok(lease) => {
            *STARTUP_LEASE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = lease;
        }
        Err(error) => problems.push(error),
    }
    let error = (!problems.is_empty()).then(|| problems.join("\n"));
    remember_startup_error(error.clone());
    error
}

pub fn startup_error() -> Option<String> {
    STARTUP_ERROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Try the configured location again. Success requires a restart: paths already resolved to
/// the default folders are not switched under a running window.
pub fn retry() -> Result<()> {
    FORCE_DEFAULT.store(false, Ordering::Release);
    *location() = None;
    if let Err(error) = configured_roots() {
        FORCE_DEFAULT.store(true, Ordering::Release);
        remember_startup_error(Some(error.clone()));
        return Err(error);
    }
    match acquire_startup_lock() {
        Ok(lease) => {
            *STARTUP_LEASE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = lease;
        }
        Err(error) => {
            remember_startup_error(Some(error.clone()));
            return Err(error);
        }
    }
    remember_startup_error(None);
    Ok(())
}

/// The error `startup` would report for one bootstrap; the UI opens with it instead of
/// the launcher exiting.
pub fn startup_error_at(bootstrap: &Path) -> Option<String> {
    resolve_at(bootstrap).err().map(|error| open_error(&error))
}

/// Forget the pointer and keep the default folders. The old folders are untouched and the
/// pointer is kept beside it as `location.json.old`, so it can be restored by hand.
pub fn use_default_folder() -> Result<()> {
    use_default_at(&bootstrap_path()?)
}

/// If the selected launcher data root is a link, return the existing folder it points to.
/// Environment overrides are intentionally excluded: their owners selected those roots directly.
pub fn linked_data_folder_target() -> Option<PathBuf> {
    if has_override() {
        return None;
    }
    let bootstrap = bootstrap_path().ok()?;
    let launcher = stored_roots_at(&bootstrap)
        .ok()?
        .map(|roots| roots.launcher)
        .unwrap_or_else(crate::backend::data_dir);
    let metadata = fs::symlink_metadata(&launcher).ok()?;
    if !is_link(&metadata) {
        return None;
    }
    fs::canonicalize(&launcher).ok().map(plain_drive)
}

/// Record a linked launcher data folder's existing target as the configured root. No files are
/// copied or removed; the next process opens the same data through its real path.
pub fn use_real_data_folder() -> Result<()> {
    if has_override() {
        return Err("Data folders are set by HORIZON_LAUNCHER_HOME or HORIZON_HOME".into());
    }
    use_real_at(&bootstrap_path()?)
}

fn use_real_at(bootstrap: &Path) -> Result<()> {
    let _lease = MigrationLease::take(bootstrap)?;
    let mut roots = roots_for_real_folder(bootstrap)?;
    let metadata =
        fs::symlink_metadata(&roots.launcher).map_err(|e| problem(&roots.launcher, e))?;
    if !is_link(&metadata) {
        return Err(format!(
            "The launcher data folder is not a link: {}",
            roots.launcher.display()
        ));
    }
    let linked_launcher = roots.launcher.clone();
    roots.launcher = fs::canonicalize(&linked_launcher)
        .map(plain_drive)
        .map_err(|e| problem(&linked_launcher, e))?;
    if roots.build == linked_launcher {
        roots.build = roots.launcher.clone();
    }
    roots = validate_roots(&roots)?;
    tool_safe(&roots.build)?;
    for root in [&roots.launcher, &roots.game, &roots.build] {
        if !root.is_dir() {
            fs::create_dir_all(root).map_err(|e| problem(root, e))?;
        }
    }
    roots = validate_roots(&roots)?;
    commit(bootstrap, &roots)
}

fn roots_for_real_folder(bootstrap: &Path) -> Result<Roots> {
    Ok(stored_roots_at(bootstrap)?.unwrap_or_else(|| Roots {
        launcher: crate::backend::data_dir(),
        game: crate::backend::game_home(),
        build: crate::backend::build_dir(),
    }))
}

fn stored_roots_at(bootstrap: &Path) -> Result<Option<Roots>> {
    let bootstrap = pointer_path(bootstrap)?;
    let bytes = match fs::read(&bootstrap) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(problem(&bootstrap, error)),
    };
    let location: Location = serde_json::from_slice(&bytes).map_err(|e| {
        format!(
            "Invalid {}: {e}; restore this file or the previous data folder",
            bootstrap.display()
        )
    })?;
    if location.version != 1 {
        return Err(format!(
            "Unsupported data location version {} in {}",
            location.version,
            bootstrap.display()
        ));
    }
    Ok(Some(location.roots))
}

fn use_default_at(bootstrap: &Path) -> Result<()> {
    if bootstrap.exists() {
        let backup = bootstrap.with_file_name("location.json.old");
        if backup.exists() {
            fs::remove_file(&backup).map_err(|e| problem(&backup, e))?;
        }
        fs::rename(bootstrap, &backup).map_err(|e| problem(bootstrap, e))?;
    }
    FORCE_DEFAULT.store(true, Ordering::Release);
    *location() = Some(Ok(None));
    remember_startup_error(None);
    Ok(())
}

/// Explicit paths make tests independent of the real user's bootstrap and environment.
pub fn resolve_at(bootstrap: &Path) -> Result<Option<Roots>> {
    pointer_path(bootstrap)?;
    let bytes = match fs::read(bootstrap) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(problem(bootstrap, e)),
    };
    let location: Location = serde_json::from_slice(&bytes).map_err(|e| {
        format!(
            "Invalid {}: {e}; restore this file or the previous data folder",
            bootstrap.display()
        )
    })?;
    if location.version != 1 {
        return Err(format!(
            "Unsupported data location version {} in {}",
            location.version,
            bootstrap.display()
        ));
    }
    let canonical = validate_roots(&location.roots)?;
    tool_safe(&canonical.build)?;
    for root in [
        &location.roots.launcher,
        &location.roots.game,
        &location.roots.build,
    ] {
        if !root.is_dir() {
            return Err(format!(
                "Data folder {} is missing; reconnect its drive or restore location.json",
                root.display()
            ));
        }
    }
    Ok(Some(canonical))
}

/// Hold until process exit. Workers must enter before acquiring this shared lease.
/// Override-based isolated runs do not touch the user's bootstrap lock.
pub fn acquire_startup_lock() -> Result<Option<File>> {
    if overridden("HORIZON_LAUNCHER_HOME") && overridden("HORIZON_HOME") {
        Ok(None)
    } else {
        acquire_lock_at(&bootstrap_path()?).map(Some)
    }
}

pub fn acquire_lock_at(bootstrap: &Path) -> Result<File> {
    lock_at(bootstrap, false)
}

fn lock_at(bootstrap: &Path, exclusive: bool) -> Result<File> {
    let bootstrap = &pointer_path(bootstrap)?;
    let parent = bootstrap
        .parent()
        .ok_or("Bootstrap needs a parent folder")?;
    fs::create_dir_all(parent).map_err(|e| problem(parent, e))?;
    let path = parent.join("migration.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| problem(&path, e))?;
    let result = if exclusive {
        file.try_lock()
    } else {
        file.try_lock_shared()
    };
    result.map_err(|e| {
        format!(
            "Cannot lock {}: {e}; close other launchers and stop games/builds before retrying",
            path.display()
        )
    })?;
    Ok(file)
}

pub fn migrate_current(
    source: &Roots,
    target: &Path,
    mut progress: impl FnMut(&str),
) -> Result<Roots> {
    migrate_current_with_prepare(
        source,
        target,
        |message| {
            progress(message);
            Ok(())
        },
        |_, _| Ok(()),
    )
}

/// Called only by the offline helper, after the GUI requests exit. Wait briefly for its lease.
/// An error from progress stops the move there, between files, like any other failure.
pub fn migrate_current_with_prepare(
    source: &Roots,
    target: &Path,
    progress: impl FnMut(&str) -> Result<()>,
    prepare: impl FnOnce(&Roots, &Roots) -> Result<()>,
) -> Result<Roots> {
    if has_override() {
        return Err("Unset HORIZON_LAUNCHER_HOME and HORIZON_HOME before moving data".into());
    }
    let bootstrap = bootstrap_path()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let lease = loop {
        match MigrationLease::take(&bootstrap) {
            Ok(lease) => break lease,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e),
        }
    };
    migrate_locked(source, target, &bootstrap, progress, prepare, &lease)
}

pub fn migrate(
    source: &Roots,
    target: &Path,
    bootstrap: &Path,
    progress: impl FnMut(&str),
) -> Result<Roots> {
    migrate_with_prepare(source, target, bootstrap, progress, |_, _| Ok(()))
}

/// prepare receives (staging roots, final roots), after byte verification and before commit.
/// It may rewrite managed absolute paths in structured files, and must sync those writes.
/// This explicit API deliberately ignores environment variables; production uses migrate_current.
pub fn migrate_with_prepare(
    source: &Roots,
    target: &Path,
    bootstrap: &Path,
    mut progress: impl FnMut(&str),
    prepare: impl FnOnce(&Roots, &Roots) -> Result<()>,
) -> Result<Roots> {
    let lease = MigrationLease::take(bootstrap)?;
    let progress = |message: &str| {
        progress(message);
        Ok(())
    };
    migrate_locked(source, target, bootstrap, progress, prepare, &lease)
}

/// A migration's exclusive lock, released before its file closes. A child process spawned while
/// it is held, by any thread, has a copy of the descriptor until its exec, and closing ours alone
/// would leave the lock taken through that copy: the next migration would read "Cannot lock"
/// (#18138). `horizon_platform::mod_package::locks::Locked` releases its locks the same way.
struct MigrationLease(File);

impl MigrationLease {
    fn take(bootstrap: &Path) -> Result<Self> {
        lock_at(bootstrap, true).map(Self)
    }
}

impl Drop for MigrationLease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn migrate_locked(
    source: &Roots,
    target: &Path,
    bootstrap: &Path,
    mut progress: impl FnMut(&str) -> Result<()>,
    prepare: impl FnOnce(&Roots, &Roots) -> Result<()>,
    _lease: &MigrationLease,
) -> Result<Roots> {
    // How the launcher spelled its folder, which the links it made may name.
    let written = source.launcher.clone();
    let source = validate_roots(source)?;
    if let Some(current) = resolve_at(bootstrap)? {
        if current != source {
            return Err(
                "Data location changed while waiting; restart the launcher and retry the move"
                    .into(),
            );
        }
    }
    let target = checked_path(target)?;
    // Live data in a folder of such a name would look like a stopped move's to a retry.
    if target
        .components()
        .any(|part| part.as_os_str().to_string_lossy().starts_with(STAGING))
    {
        return Err(format!(
            "Destination {} has a {STAGING} folder in its path; choose another folder",
            target.display()
        ));
    }
    for root in [
        &source.launcher,
        &source.game,
        &source.build,
        &pointer_path(bootstrap)?,
    ] {
        if overlaps(root, &target) {
            return Err(format!(
                "Destination {} overlaps existing data or its bootstrap {}",
                target.display(),
                root.display()
            ));
        }
    }
    if target.exists()
        && (!target.is_dir() || !staging_only(&target).map_err(|e| problem(&target, e))?)
    {
        return Err(format!(
            "Destination {} must be an empty folder",
            target.display()
        ));
    }
    // Held through the commit: two moves into one destination, from different bootstraps, would
    // otherwise mix their folders at the final renames.
    let _destination_lease = lock_destination(&target)?;
    let mut destination = layout(&source, &target);
    // MinGW needs the *physical* path to be ASCII: aliases are canonicalized by rustc.
    // Reserve a fresh sibling, never reuse or modify another location's build data.
    let separate_build = tool_safe(&destination.build).is_err();
    if separate_build {
        destination.build = reserve_build_folder(&source, &target, bootstrap)?;
    }
    let mut ours = None;
    let result = (|| {
        fs::create_dir_all(&target).map_err(|e| problem(&target, e))?;
        let stage = claim_stage(&target)?;
        ours = Some(stage.clone());
        let mut staged = layout(&source, &stage);
        if separate_build {
            // This unique folder is not published until the location pointer commits. It need
            // not be renamed across volumes, and preparation sees its final, tool-safe path.
            staged.build = destination.build.clone();
        }
        progress(progress::LAUNCHER)?;
        // Preparing links other launchers' files into the launcher's data and links them again
        // each time; they are never followed or copied. The server runtime's links are made
        // again below, to the same release.
        let runtime = crate::servers::RuntimeLinks::find(&source.launcher, &written)?;
        let mut skip = runtime.paths(&source.launcher);
        skip.extend(crate::sources::linked_layout_links(&source.launcher));
        copy_tree(
            &source.launcher,
            &source.launcher,
            &staged.launcher,
            bootstrap,
            &skip,
            &mut progress,
        )?;
        progress(progress::GAME)?;
        // Every instance's cache holds links the game made, which copy_tree refuses; the
        // game rebuilds the cache on its next start.
        copy_tree(
            &source.game,
            &source.game,
            &staged.game,
            bootstrap,
            &crate::instances::game_caches(&source.game),
            &mut progress,
        )?;
        if separate_build || !within(&source.build, &source.launcher) {
            progress(progress::BUILD)?;
            copy_tree(
                &source.build,
                &source.build,
                &staged.build,
                bootstrap,
                &[],
                &mut progress,
            )?;
        } else if !staged.build.exists() {
            fs::create_dir_all(&staged.build).map_err(|e| problem(&staged.build, e))?;
        }
        prepare(&staged, &destination)?;
        // The last point at which progress may stop the move. From the first rename on it runs
        // to its commit or fails on its own: a stop between the renames would leave the
        // destination neither moved nor open to a retry.
        progress(progress::COMMIT)?;
        // The stage always holds launcher and game; build only when it is neither inside
        // launcher nor in a separate folder. A missing one, or one already at the destination,
        // fails the move before the commit.
        let own_build = !separate_build && !within(&source.build, &source.launcher);
        for (name, required) in [("launcher", true), ("game", true), ("build", own_build)] {
            let (from, to) = (stage.join(name), target.join(name));
            if required || from.exists() {
                if fs::symlink_metadata(&to).is_ok() {
                    return Err(format!("{} already exists", to.display()));
                }
                fs::rename(&from, &to).map_err(|e| problem(&from, e))?;
            }
        }
        // Before the commit, so a stage that is left never holds data a location names.
        fs::remove_dir(&stage).map_err(|e| problem(&stage, e))?;
        runtime.make(&destination.launcher)?;
        sync_dir(&target)?;
        commit(bootstrap, &destination)?;
        Ok(destination.clone())
    })();
    result.map_err(|e: String| {
        let args = [
            ("error", e.as_str()),
            ("target", &target.display().to_string()),
            ("build", &destination.build.display().to_string()),
        ];
        // A failure after the renames leaves data beside the stage, which a retry refuses.
        let error = if staging_only(&target).unwrap_or(false) {
            crate::i18n::trf(
                "{error}. Original data was kept. Partial copies may remain in {target} and {build}. Retry with the same folder or another empty one",
                &args,
            )
        } else {
            crate::i18n::trf(
                "{error}. Original data was kept. Partial copies may remain in {target} and {build}; choose an empty folder to retry",
                &args,
            )
        };
        match kept_note(&target, ours.as_deref()) {
            Some(note) => format!("{error}. {note}"),
            None => error,
        }
    })
}

/// What a move passes to its `progress` callback: each part as it starts, then each file.
pub mod progress {
    pub const LAUNCHER: &str = "Copying and verifying launcher data";
    pub const GAME: &str = "Copying and verifying game data";
    pub const BUILD: &str = "Copying and verifying build data";
    pub const FILE: &str = "Verified file";
    pub const COMMIT: &str = "Committing data location";
}

/// What the stage a move copies into inside its destination starts with: a unique name per run.
/// A move that failed or was stopped leaves it behind.
///
/// A move never touches a staging folder it didn't create in this run: an earlier one may be the
/// only copy, or another profile's live data (#19607 review).
const STAGING: &str = ".relocation-staging";
/// The destination's lock, which a move holds through its commit. It stays once released.
const LOCK: &str = ".relocation.lock";

/// Creates `target` and takes its lock, refused while another move holds it.
fn lock_destination(target: &Path) -> Result<MigrationLease> {
    fs::create_dir_all(target).map_err(|e| problem(target, e))?;
    let path = target.join(LOCK);
    // `staging_only` refused a link or folder of that name; this sees one made since.
    if let Ok(meta) = fs::symlink_metadata(&path) {
        reject_link(&path, &meta)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| problem(&path, e))?;
    file.try_lock()
        .map_err(|_| format!("Another move is using {}", target.display()))?;
    // Released before it closes, as the migration lease is.
    Ok(MigrationLease(file))
}

/// Whether `dir` holds nothing but plain folders whose names start with `STAGING`, which are
/// never read, and the plain file `LOCK`: a link or any other entry is false.
fn staging_only(dir: &Path) -> std::io::Result<bool> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let meta = fs::symlink_metadata(&path)?;
        let name = path.file_name().unwrap_or_default();
        let fits = if name == LOCK {
            meta.is_file()
        } else {
            name.to_string_lossy().starts_with(STAGING) && meta.is_dir()
        };
        if !fits || reject_link(&path, &meta).is_err() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The staging folders in `dir` other than `ours`, by name.
pub fn kept_copies(dir: &Path, ours: Option<&Path>) -> Vec<PathBuf> {
    let mut kept: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            Some(path.as_path()) != ours
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(STAGING))
        })
        .collect();
    kept.sort();
    kept
}

/// What a player is told about the staging folders in `dir` other than `ours`, if any.
pub fn kept_note(dir: &Path, ours: Option<&Path>) -> Option<String> {
    let kept = kept_copies(dir, ours);
    (!kept.is_empty()).then(|| {
        let folders = kept
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        crate::i18n::trf(
            "Horizon left these folders in {target} as they were: {folders}",
            &[
                ("target", &dir.display().to_string()),
                ("folders", &folders),
            ],
        )
    })
}

/// Makes this run's stage under a fresh name, then makes sure no other move came in between.
fn claim_stage(target: &Path) -> Result<PathBuf> {
    let stage = tempfile::Builder::new()
        .prefix(&format!("{STAGING}-"))
        .tempdir_in(target)
        .map_err(|e| problem(target, e))?
        .keep();
    // A move from another bootstrap may have put its own data beside the stage since the check.
    if !staging_only(target).map_err(|e| problem(target, e))? {
        // Ours and still empty: remove_dir removes nothing else.
        fs::remove_dir(&stage).map_err(|e| problem(&stage, e))?;
        return Err(format!("Another move is using {}", target.display()));
    }
    Ok(stage)
}

/// Whether a move may use the folder `dir`: it holds nothing, or only staging folders of earlier
/// moves, which a move leaves alone. The picker asks; the move checks again under its lease.
pub fn reusable_destination(dir: &Path) -> Result<bool> {
    staging_only(dir).map_err(|e| problem(dir, e))
}

/// Prefer the chosen drive's nearest existing ASCII ancestor. If it is not writable
/// (or a UNC share itself is non-ASCII), try alongside the existing build data. No
/// environment lookup: explicit migration fixtures never reach the real user profile.
fn reserve_build_folder(source: &Roots, target: &Path, bootstrap: &Path) -> Result<PathBuf> {
    for parent in target
        .ancestors()
        .skip(1)
        .chain(source.build.ancestors().skip(1))
    {
        let Ok(parent) = checked_path(parent) else {
            continue;
        };
        if !parent.is_dir()
            || tool_safe(&parent).is_err()
            || [&source.launcher, &source.game, &source.build]
                .iter()
                .any(|root| within(&parent, root))
        {
            continue;
        }
        let Ok(folder) = tempfile::Builder::new()
            .prefix("horizon-build-")
            .tempdir_in(&parent)
        else {
            continue;
        };
        let real = checked_path(folder.path())?;
        tool_safe(&real)?;
        if overlaps(&real, &pointer_path(bootstrap)?) || overlaps(&real, target) {
            continue;
        }
        // Keep partial copies just as the main destination does: after a failed commit
        // no pointer names this folder, and the error tells the player where it remains.
        let _ = folder.keep();
        return Ok(real.join("build"));
    }
    Err(crate::i18n::tr("Could not create a Windows-compatible build folder. Choose a destination with a writable parent folder whose full path contains only ASCII characters and no spaces.").into())
}

fn layout(source: &Roots, target: &Path) -> Roots {
    let launcher = target.join("launcher");
    let build = if within(&source.build, &source.launcher) {
        // Windows containment is case-insensitive, while strip_prefix is not.
        launcher.join(
            source
                .build
                .components()
                .skip(source.launcher.components().count())
                .collect::<PathBuf>(),
        )
    } else {
        target.join("build")
    };
    Roots {
        launcher,
        game: target.join("game"),
        build,
    }
}

fn validate_roots(roots: &Roots) -> Result<Roots> {
    let roots = Roots {
        launcher: checked_path(&roots.launcher)?,
        game: checked_path(&roots.game)?,
        build: checked_path(&roots.build)?,
    };
    for root in [&roots.launcher, &roots.game, &roots.build] {
        match fs::metadata(root) {
            Ok(meta) if !meta.is_dir() => {
                return Err(format!("Data root must be a folder: {}", root.display()))
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(problem(root, e)),
            _ => {}
        }
    }
    if overlaps(&roots.launcher, &roots.game)
        || overlaps(&roots.build, &roots.game)
        || (overlaps(&roots.launcher, &roots.build) && !within(&roots.build, &roots.launcher))
    {
        return Err(
            "Source data roots overlap; only a build folder inside/equal to launcher is supported"
                .into(),
        );
    }
    Ok(roots)
}

fn within(path: &Path, parent: &Path) -> bool {
    #[cfg(windows)]
    {
        Path::new(&path.to_string_lossy().to_lowercase())
            .starts_with(parent.to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        path.starts_with(parent)
    }
}
fn overlaps(a: &Path, b: &Path) -> bool {
    within(a, b) || within(b, a)
}

/// Canonicalize existing ancestors, reject links/junctions and ambiguous relative components.
fn checked_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!(
            "Use an absolute path without '..': {}",
            path.display()
        ));
    }
    let mut current = PathBuf::new();
    let last = path.components().count().saturating_sub(1);
    for (i, part) in path.components().enumerate() {
        current.push(part);
        // A Windows drive/UNC prefix alone is not a filesystem root. Inspect it
        // only after RootDir completes it, including canonical verbatim paths.
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                // Only the root itself may not be a link: the folders above it are never copied
                // (a ~/.local/share linked to another disk, Fedora Atomic's /home).
                if i == last {
                    reject_link(&current, &meta)?;
                }
                current = fs::canonicalize(&current).map_err(|e| problem(&current, e))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(problem(&current, e)),
        }
    }
    Ok(plain_drive(current))
}

/// `canonicalize`'s verbatim `\\?\C:\…` as the plain drive path the other roots use.
fn plain_drive(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    if let Some(rest) = path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        if rest.as_bytes().get(1) == Some(&b':') {
            return PathBuf::from(rest);
        }
    }
    path
}

/// The pointer and its lock sit in the platform config folder, which may be a link (a
/// dotfiles-managed `~/.config`, Silverblue's `/home`): unlike the data roots, they are never
/// copied, so a link on the way is followed, not refused. Refusing it would report a broken
/// data folder at every start to players who never moved their data.
fn pointer_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!(
            "Use an absolute path without '..': {}",
            path.display()
        ));
    }
    let mut existing = path;
    let mut rest = Vec::new();
    loop {
        match fs::canonicalize(existing) {
            Ok(real) => {
                let real = plain_drive(real);
                return Ok(rest.iter().rev().fold(real, |path, part| path.join(part)));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(problem(existing, e)),
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return Ok(path.to_path_buf()),
        }
    }
}

/// The target of the symlink at `path` when the same link at the destination points into the
/// copy: relative, only `..` then plain names, and lexically inside `root` from the link's folder.
/// The link is read, never followed.
#[cfg(unix)]
fn contained_link(
    root: &Path,
    path: &Path,
    meta: &fs::Metadata,
) -> std::io::Result<Option<PathBuf>> {
    if !meta.file_type().is_symlink() {
        return Ok(None);
    }
    use std::os::unix::ffi::OsStrExt;
    let target = fs::read_link(path)?;
    let (mut up, mut down) = (0, 0);
    // Split by hand: `Path::components` would drop an interior `.` and repeated slashes.
    for part in target.as_os_str().as_bytes().split(|&byte| byte == b'/') {
        match part {
            b".." if down == 0 => up += 1,
            b"" | b"." | b".." => return Ok(None),
            _ => down += 1,
        }
    }
    let depth = path
        .parent()
        .and_then(|folder| folder.strip_prefix(root).ok())
        .map(|folder| folder.components().count());
    Ok((down > 0 && depth.is_some_and(|depth| up <= depth)).then_some(target))
}

pub(crate) fn reject_link(path: &Path, meta: &fs::Metadata) -> Result<()> {
    if is_link(meta) {
        return Err(format!(
            "Symbolic links and reparse points cannot be relocated: {}",
            path.display()
        ));
    }
    Ok(())
}

fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    meta.file_type().is_symlink() || reparse
}

fn tool_safe(path: &Path) -> Result<()> {
    if cfg!(windows)
        && !path
            .to_str()
            .is_some_and(|s| s.is_ascii() && !s.bytes().any(|b| b.is_ascii_whitespace()))
    {
        return Err(crate::i18n::trf(
            "Windows build tools cannot use the build folder \"{path}\" because its full path contains spaces, other whitespace, or non-ASCII characters. Choose an empty destination whose full path uses English letters, numbers, underscores or hyphens, for example D:\\Horizon_Data. Check every parent folder too.",
            &[("path", &path.display().to_string())],
        ));
    }
    Ok(())
}

fn private_dir(path: &Path) -> Result<()> {
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    builder.create(path).map_err(|e| problem(path, e))
}

fn new_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|e| problem(path, e))
}

/// Copies `source`, inside `root`, to `dest`, leaving out the paths in `skip`.
#[cfg_attr(not(unix), allow(clippy::only_used_in_recursion))]
fn copy_tree(
    root: &Path,
    source: &Path,
    dest: &Path,
    bootstrap: &Path,
    skip: &[PathBuf],
    progress: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let meta = match fs::symlink_metadata(source) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return private_dir(dest),
        Err(e) => return Err(problem(source, e)),
    };
    #[cfg(unix)]
    if let Some(target) = contained_link(root, source, &meta).map_err(|e| problem(source, e))? {
        return std::os::unix::fs::symlink(&target, dest).map_err(|e| problem(dest, e));
    }
    reject_link(source, &meta)?;
    if meta.is_dir() {
        private_dir(dest)?;
        copy_acl(source, dest)?;
        for entry in fs::read_dir(source).map_err(|e| problem(source, e))? {
            let entry = entry.map_err(|e| problem(source, e))?;
            let path = entry.path();
            if skip.contains(&path) {
                continue;
            }
            let controls = [
                bootstrap.to_path_buf(),
                bootstrap.with_file_name("migration.lock"),
                bootstrap.with_file_name("location.json.new"),
            ];
            if controls
                .iter()
                .any(|p| pointer_path(p).is_ok_and(|p| within(&path, &p) && within(&p, &path)))
            {
                continue;
            }
            copy_tree(
                root,
                &path,
                &dest.join(entry.file_name()),
                bootstrap,
                skip,
                progress,
            )?;
        }
        fs::set_permissions(dest, meta.permissions()).map_err(|e| problem(dest, e))?;
        sync_dir(dest)?;
    } else if meta.is_file() {
        let mut input = File::open(source).map_err(|e| problem(source, e))?;
        let mut output = new_file(dest)?;
        copy_acl(source, dest)?; // Apply the source DACL before writing credentials or other bytes.
        std::io::copy(&mut input, &mut output).map_err(|e| problem(dest, e))?;
        output.sync_all().map_err(|e| problem(dest, e))?;
        if hash(source)? != hash(dest)? {
            return Err(format!("Copy verification failed for {}", source.display()));
        }
        let after = fs::symlink_metadata(source).map_err(|e| problem(source, e))?;
        reject_link(source, &after)?;
        if meta.len() != after.len() || meta.modified().ok() != after.modified().ok() {
            return Err(format!(
                "Source changed during copying: {}; stop all writers and retry",
                source.display()
            ));
        }
        fs::set_permissions(dest, meta.permissions()).map_err(|e| problem(dest, e))?;
        progress(progress::FILE)?;
    } else {
        return Err(format!("Unsupported special file: {}", source.display()));
    }
    Ok(())
}

fn hash(path: &Path) -> Result<[u8; 32]> {
    let mut file = File::open(path).map_err(|e| problem(path, e))?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| problem(path, e))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().into())
}

fn commit(bootstrap: &Path, roots: &Roots) -> Result<()> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let temp = bootstrap.with_file_name(format!("location.{}.{nonce}.new", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&Location {
        version: 1,
        roots: roots.clone(),
    })
    .map_err(|e| e.to_string())?;
    let mut file = new_file(&temp)?;
    let written = file
        .write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| problem(&temp, e));
    drop(file);
    let replaced = written.and_then(|_| atomic_replace(&temp, bootstrap));
    if replaced.is_err() {
        let _ = fs::remove_file(&temp);
    }
    replaced?;
    // The pointer is now committed: failure to sync the directory must not report rollback.
    let _ = sync_dir(
        bootstrap
            .parent()
            .ok_or("Bootstrap needs a parent folder")?,
    );
    Ok(())
}

pub(crate) fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|f| f.sync_all())
            .map_err(|e| problem(path, e))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(from: &Path, to: &Path) -> Result<()> {
    fs::rename(from, to).map_err(|e| problem(to, e))
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain([0]).collect()
}

#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> Result<()> {
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    if unsafe { MoveFileExW(wide(from).as_ptr(), wide(to).as_ptr(), 1 | 8) } == 0 {
        return Err(problem(to, std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(not(windows))]
fn copy_acl(_source: &Path, _dest: &Path) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn copy_acl(source: &Path, dest: &Path) -> Result<()> {
    use std::ffi::c_void;
    #[link(name = "advapi32")]
    extern "system" {
        fn GetFileSecurityW(
            path: *const u16,
            info: u32,
            descriptor: *mut c_void,
            length: u32,
            needed: *mut u32,
        ) -> i32;
        fn SetFileSecurityW(path: *const u16, info: u32, descriptor: *const c_void) -> i32;
    }
    let mut needed = 0;
    let source_w = wide(source);
    unsafe {
        GetFileSecurityW(source_w.as_ptr(), 4, std::ptr::null_mut(), 0, &mut needed);
    }
    if needed == 0 {
        return Err(problem(source, std::io::Error::last_os_error()));
    }
    // Aligned storage for the self-relative SECURITY_DESCRIPTOR; protect against target inheritance.
    let mut descriptor = vec![0u32; (needed as usize).div_ceil(4)];
    if unsafe {
        GetFileSecurityW(
            source_w.as_ptr(),
            4,
            descriptor.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(problem(source, std::io::Error::last_os_error()));
    }
    if unsafe {
        SetFileSecurityW(
            wide(dest).as_ptr(),
            4 | 0x8000_0000,
            descriptor.as_ptr().cast(),
        )
    } == 0
    {
        return Err(format!(
            "Cannot preserve access permissions on {}: {}",
            dest.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn problem(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

#[cfg(test)]
#[path = "data_location/tests.rs"]
mod tests;
