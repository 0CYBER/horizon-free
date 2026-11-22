//! Data-only inputs carried from an owning load boundary to native load hooks.
use std::{
    collections::{BTreeMap, HashMap},
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The phase that owns this data load, selected before handing work to loaders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataLoadOrigin {
    ServerDataLoad,
    ClientPreload,
}

/// Immutable metadata captured with the loaded item data. Later data loads
/// borrow the same registry snapshot instead of scanning it again.
pub struct LoadInputs {
    registries: BTreeMap<String, Arc<[String]>>,
    /// The fluid tags a recipe load reads for its ingredients' `#tags`:
    /// each tag id to its fluids, in the sources' order. Empty until a load
    /// that reads recipe resources fills them.
    fluid_tags: Arc<[(String, Arc<[String]>)]>,
    settings_root: PathBuf,
}

impl LoadInputs {
    pub fn new(registries: BTreeMap<String, Vec<String>>, settings_root: PathBuf) -> Self {
        Self {
            registries: registries
                .into_iter()
                .map(|(name, ids)| (name, ids.into()))
                .collect(),
            fluid_tags: Arc::from([]),
            settings_root,
        }
    }
    /// The inputs with the fluid `tags` a recipe load reads
    /// (`data/<ns>/tags/fluid`), each tag's fluids in the sources' order.
    pub fn with_fluid_tags(mut self, tags: HashMap<String, Vec<String>>) -> Self {
        self.fluid_tags = tags
            .into_iter()
            .map(|(tag, fluids)| (tag, fluids.into()))
            .collect();
        self
    }
}

/// A load's registry snapshot with the settings captured for its mods.
#[derive(Clone)]
struct Snapshot {
    inputs: Arc<LoadInputs>,
    settings: Arc<[(String, Arc<[(String, String)]>)]>,
}

impl Snapshot {
    /// Captures settings for `mod_ids`; the registry inputs are shared, not scanned again.
    fn capture<'a>(inputs: Arc<LoadInputs>, mod_ids: impl IntoIterator<Item = &'a str>) -> Self {
        let settings = shared_settings(capture_settings(&inputs.settings_root, mod_ids));
        Self { inputs, settings }
    }

    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.inputs.registries.get(name).cloned()
    }

    /// The fluid tags the inputs carry, each tag's fluids in the sources'
    /// order; empty until a load that reads recipe resources fills them.
    fn fluid_tags(&self) -> &Arc<[(String, Arc<[String]>)]> {
        &self.inputs.fluid_tags
    }

    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        let mod_at = self
            .settings
            .binary_search_by(|(known, _)| known.as_str().cmp(mod_id))
            .ok()?;
        let values = &self.settings[mod_at].1;
        let at = values
            .binary_search_by(|(known, _)| known.as_str().cmp(key))
            .ok()?;
        Some(values[at].1.clone())
    }
}

fn shared_settings(
    settings: BTreeMap<String, BTreeMap<String, String>>,
) -> Arc<[(String, Arc<[(String, String)]>)]> {
    settings
        .into_iter()
        .map(|(id, values)| (id, values.into_iter().collect()))
        .collect()
}

/// Origin and borrowed engine inputs for one owning load. This contains no
/// game, world handles or mutable registry access.
#[derive(Clone)]
pub struct LoadContext {
    origin: DataLoadOrigin,
    snapshot: Snapshot,
}

impl LoadContext {
    /// Called at a load boundary, never by a reader of already-loaded data.
    /// Captures settings for the owning phase's loaded package IDs; reusing the
    /// registry inputs for another load still captures fresh settings.
    pub fn new<'a>(
        origin: DataLoadOrigin,
        inputs: Arc<LoadInputs>,
        mod_ids: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self {
            origin,
            snapshot: Snapshot::capture(inputs, mod_ids),
        }
    }

    /// The initial resource load over registration's already-captured, sorted
    /// settings. Only this load's admitted package IDs remain visible; values
    /// and (when the scope is unchanged) the outer snapshot are shared.
    pub fn from_settings_snapshot<'a>(
        origin: DataLoadOrigin,
        inputs: Arc<LoadInputs>,
        mod_ids: impl IntoIterator<Item = &'a str>,
        settings: Arc<[(String, Arc<[(String, String)]>)]>,
    ) -> Self {
        let ids: std::collections::BTreeSet<_> = mod_ids.into_iter().collect();
        let settings = if settings.iter().all(|(id, _)| ids.contains(id.as_str())) {
            settings
        } else {
            settings
                .iter()
                .filter(|(id, _)| ids.contains(id.as_str()))
                .cloned()
                .collect()
        };
        Self {
            origin,
            snapshot: Snapshot { inputs, settings },
        }
    }

    /// A new owning load of `origin` over this load's registry inputs, with fresh settings.
    pub fn for_load<'a>(
        &self,
        origin: DataLoadOrigin,
        mod_ids: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self::new(origin, self.snapshot.inputs.clone(), mod_ids)
    }

    /// This load over the same registries and settings, with the fluid `tags`
    /// its recipe pass reads (`data/<ns>/tags/fluid`): what that pass's
    /// ingredient hooks resolve fluid `#tags` with. A recipe load builds it
    /// where it knows its overlay; loads that read no recipe resources share
    /// the inputs as-is, with no tags.
    pub fn with_fluid_tags(&self, tags: HashMap<String, Vec<String>>) -> Self {
        let inputs = Arc::new(LoadInputs {
            registries: self.snapshot.inputs.registries.clone(),
            fluid_tags: tags
                .into_iter()
                .map(|(tag, fluids)| (tag, fluids.into()))
                .collect(),
            settings_root: self.snapshot.inputs.settings_root.clone(),
        });
        Self {
            origin: self.origin,
            snapshot: Snapshot {
                inputs,
                settings: self.snapshot.settings.clone(),
            },
        }
    }

    /// The fluid tags this load's recipe pass reads: each tag id to its
    /// fluids, in the sources' order. Empty where no recipe pass filled them.
    pub fn fluid_tags(&self) -> &Arc<[(String, Arc<[String]>)]> {
        self.snapshot.fluid_tags()
    }

    /// Readers of this load's resource generation, with the same captured
    /// settings. Starting a distinct generation uses `loop_context` instead.
    pub fn captured_loop_context(&self, side: LoopSide) -> LoopContext {
        LoopContext {
            side,
            snapshot: self.snapshot.clone(),
            recipe_properties: Default::default(),
        }
    }

    /// The context of a menu, recipe or player-list loop of `side`, over this
    /// load's registry inputs, with fresh settings. Built where the side's loop
    /// owner starts or reloads its resources, never by a reader.
    pub fn loop_context<'a>(
        &self,
        side: LoopSide,
        mod_ids: impl IntoIterator<Item = &'a str>,
    ) -> LoopContext {
        LoopContext {
            side,
            snapshot: Snapshot::capture(self.snapshot.inputs.clone(), mod_ids),
            recipe_properties: Default::default(),
        }
    }

    /// The context of the server's world generation, over this load's
    /// registry inputs, with fresh settings. Built where the server starts or
    /// reloads its resources, never by a reader.
    pub fn worldgen_context<'a>(
        &self,
        mod_ids: impl IntoIterator<Item = &'a str>,
    ) -> WorldgenContext {
        WorldgenContext {
            snapshot: Snapshot::capture(self.snapshot.inputs.clone(), mod_ids),
        }
    }

    pub fn origin(&self) -> DataLoadOrigin {
        self.origin
    }

    /// Names in registry ID order, from the load's registry snapshot.
    pub fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.snapshot.registry(name)
    }

    /// A loaded mod's setting captured at this load boundary, with no file I/O.
    pub fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.snapshot.setting(mod_id, key)
    }
}

/// The side a loop context serves, chosen where the context is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoopSide {
    Server,
    Client,
}

/// Registries, settings and the side for hooks fired inside a menu, recipe or
/// player-list loop, where no game is at hand. It holds no game, world handle
/// or mutable registry access, and is built once per resource generation.
#[derive(Clone)]
pub struct LoopContext {
    side: LoopSide,
    snapshot: Snapshot,
    recipe_properties: Arc<HashMap<String, std::collections::HashSet<String>>>,
}

#[path = "native_load_context/recipe_properties.rs"]
mod recipe_properties;

impl LoopContext {
    pub fn side(&self) -> LoopSide {
        self.side
    }

    /// The same resource generation's settings and registries for world generation.
    /// Sharing this snapshot performs no capture or diagnostic reporting.
    pub fn worldgen_context(&self) -> WorldgenContext {
        WorldgenContext {
            snapshot: self.snapshot.clone(),
        }
    }

    /// Names in registry ID order, from the registry snapshot it was built over.
    pub fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.snapshot.registry(name)
    }

    /// A loaded mod's setting captured when this context was built, with no file I/O.
    pub fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.snapshot.setting(mod_id, key)
    }
}

/// Capture regular UTF-8 files only, never following symlinks (even within the root).
/// Refused keys are absent. Limits are measured before whitespace trimming.
pub const MAX_SETTING_BYTES: usize = 64 * 1024;
const MAX_VISITED_NAMES: usize = 4096;
pub const MAX_SETTING_KEYS: usize = 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

pub fn capture_settings<'a>(
    settings_root: &Path,
    mod_ids: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    capture_settings_with_diagnostics(settings_root, mod_ids, |diagnostics| {
        log::warn!("Mod settings snapshot omissions at {settings_root:?}:\n{diagnostics}");
    })
}

fn capture_settings_with_diagnostics<'a>(
    settings_root: &Path,
    mod_ids: impl IntoIterator<Item = &'a str>,
    report: impl FnOnce(&str),
) -> BTreeMap<String, BTreeMap<String, String>> {
    use std::fmt::Write;
    let mut diagnostics = String::new();
    let settings = capture_settings_with_report(settings_root, mod_ids, |mod_id, key, reason| {
        match key {
            Some(key) => writeln!(diagnostics, "  {mod_id:?}/{key:?}: {reason}"),
            None => writeln!(diagnostics, "  mod {mod_id:?}: {reason}"),
        }
        .expect("writing diagnostics to a String");
    });
    if !diagnostics.is_empty() {
        report(&diagnostics);
    }
    settings
}

fn capture_settings_with_report<'a>(
    settings_root: &Path,
    mod_ids: impl IntoIterator<Item = &'a str>,
    mut report: impl FnMut(&str, Option<&OsStr>, &dyn std::fmt::Display),
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut settings = BTreeMap::new();
    let mut total = 0;
    for mod_id in mod_ids
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
    {
        // Package ids are normally validated at load time; do not let this public
        // filesystem boundary interpret a path supplied by a caller.
        if Path::new(mod_id).components().count() != 1
            || !matches!(
                Path::new(mod_id).components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            report(mod_id, None, &"unsafe package path");
            continue;
        }
        let files = match snapshot_files(settings_root, mod_id) {
            Ok(files) => files,
            // A mod need not declare or save any settings. Absence of its
            // directory (or the settings root) does not omit a key.
            Err(SnapshotError::Missing) => continue,
            Err(SnapshotError::Directory(error)) => {
                report(
                    mod_id,
                    None,
                    &format_args!("cannot inspect settings directory: {error}"),
                );
                continue;
            }
            Err(SnapshotError::Enumeration { names, error }) => {
                for key in &names {
                    report(
                        mod_id,
                        Some(key),
                        &format_args!("enumeration failed: {error}"),
                    );
                }
                report(
                    mod_id,
                    None,
                    &format_args!("cannot finish enumerating settings: {error}"),
                );
                continue;
            }
            Err(SnapshotError::EnumerationLimit(names)) => {
                for key in &names {
                    report(
                        mod_id,
                        Some(key),
                        &format_args!(
                            "package exceeds enumeration limit ({MAX_VISITED_NAMES} names)"
                        ),
                    );
                }
                continue;
            }
            Err(SnapshotError::InvalidKey { names, index }) => {
                for key in &names {
                    report(
                        mod_id,
                        Some(key),
                        &format_args!("package contains a non-UTF-8 key: {:?}", names[index]),
                    );
                }
                continue;
            }
        };
        let mut values = BTreeMap::new();
        for (key, file) in files {
            let key = key.into_string().expect("validated UTF-8 setting name");
            let mut file = match file {
                Ok(file) => file,
                Err(error) => {
                    report(
                        mod_id,
                        Some(key.as_ref()),
                        &format_args!("unsafe or unreadable entry: {error}"),
                    );
                    continue;
                }
            };
            if values.len() >= MAX_SETTING_KEYS {
                report(
                    mod_id,
                    Some(key.as_ref()),
                    &format_args!("key limit ({MAX_SETTING_KEYS})"),
                );
                continue;
            }
            use std::io::Read;
            let mut bytes = Vec::new();
            if let Err(error) = file
                .by_ref()
                .take((MAX_SETTING_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
            {
                report(
                    mod_id,
                    Some(key.as_ref()),
                    &format_args!("read failed: {error}"),
                );
                continue;
            }
            if bytes.len() > MAX_SETTING_BYTES {
                report(
                    mod_id,
                    Some(key.as_ref()),
                    &format_args!("file byte limit ({MAX_SETTING_BYTES})"),
                );
                continue;
            }
            if bytes.len() > MAX_SNAPSHOT_BYTES - total {
                report(
                    mod_id,
                    Some(key.as_ref()),
                    &format_args!("snapshot byte limit ({MAX_SNAPSHOT_BYTES})"),
                );
                continue;
            }
            let value = match String::from_utf8(bytes) {
                Ok(value) => value,
                Err(error) => {
                    report(
                        mod_id,
                        Some(key.as_ref()),
                        &format_args!("invalid UTF-8: {}", error.utf8_error()),
                    );
                    continue;
                }
            };
            total += value.len();
            values.insert(key, value.trim().to_owned());
        }
        settings.insert(mod_id.to_owned(), values);
    }
    settings
}

#[derive(Debug)]
enum SnapshotError {
    Missing,
    Directory(std::io::Error),
    Enumeration {
        names: Vec<OsString>,
        error: std::io::Error,
    },
    EnumerationLimit(Vec<OsString>),
    InvalidKey {
        names: Vec<OsString>,
        index: usize,
    },
}

impl From<std::io::Error> for SnapshotError {
    fn from(error: std::io::Error) -> Self {
        Self::Directory(error)
    }
}

// Only a missing root/package directory means no settings. A missing individual
// entry or a failed enumeration still reports the unreadable scope or key.
fn settings_directory<T>(result: std::io::Result<T>) -> Result<T, SnapshotError> {
    result.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SnapshotError::Missing
        } else {
            SnapshotError::Directory(error)
        }
    })
}

// Reject the whole package beyond the enumeration budget: retaining an arbitrary
// prefix would make the result depend on filesystem enumeration order again.
fn sorted_names(
    names: impl Iterator<Item = std::io::Result<OsString>>,
) -> Result<Vec<OsString>, SnapshotError> {
    let mut result = Vec::new();
    for name in names {
        let name = match name {
            Ok(name) => name,
            Err(error) => {
                return Err(SnapshotError::Enumeration {
                    names: result,
                    error,
                });
            }
        };
        result.push(name);
        if result.len() > MAX_VISITED_NAMES {
            result.sort_unstable();
            return Err(SnapshotError::EnumerationLimit(result));
        }
    }
    // The native encoding is a superset of UTF-8: valid keys retain their
    // existing byte order without revalidating UTF-8 on every comparison.
    result.sort_unstable_by(|a, b| a.as_encoded_bytes().cmp(b.as_encoded_bytes()));
    // Preserve whole-package refusal for an invalid filename, but retain the
    // actual names so the diagnostic identifies what was discarded and why.
    if let Some(index) = result.iter().position(|key| key.to_str().is_none()) {
        return Err(SnapshotError::InvalidKey {
            names: result,
            index,
        });
    }
    Ok(result)
}

#[cfg(unix)]
fn snapshot_files(
    root: &Path,
    mod_id: &str,
) -> Result<impl Iterator<Item = (OsString, std::io::Result<fs::File>)>, SnapshotError> {
    snapshot_files_with_hook(root, mod_id, |_| {})
}

#[cfg(unix)]
fn snapshot_files_with_hook(
    root: &Path,
    mod_id: &str,
    after_open: impl Fn(&str),
) -> Result<impl Iterator<Item = (OsString, std::io::Result<fs::File>)>, SnapshotError> {
    use rustix::fs::{Dir, Mode, OFlags, open, openat};
    use std::os::unix::ffi::OsStrExt;
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let root = settings_directory(
        open(root, flags | OFlags::DIRECTORY, Mode::empty()).map_err(Into::into),
    )?;
    let directory = settings_directory(
        openat(&root, mod_id, flags | OFlags::DIRECTORY, Mode::empty()).map_err(Into::into),
    )?;
    let entries = Dir::read_from(&directory).map_err(std::io::Error::from)?;
    let names = sorted_names(entries.filter_map(|entry| {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return Some(Err(error.into())),
        };
        let name = entry.file_name();
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            return None;
        }
        Some(Ok(OsStr::from_bytes(name.to_bytes()).to_owned()))
    }))?;
    Ok(names.into_iter().map(move |key| {
        let file = (|| {
            let file = fs::File::from(openat(&directory, key.as_os_str(), flags, Mode::empty())?);
            if let Some(key) = key.to_str() {
                after_open(key);
            }
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "entry is not a regular file",
                ));
            }
            Ok(file)
        })();
        (key, file)
    }))
}

#[cfg(windows)]
fn snapshot_files(
    root: &Path,
    mod_id: &str,
) -> Result<impl Iterator<Item = (OsString, std::io::Result<fs::File>)>, SnapshotError> {
    use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{Dir, OpenOptions};
    // Open the parent as a capability, then refuse a reparse point at the root
    // and package boundary as well as at each individual file boundary.
    let parent = root
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let name = root
        .file_name()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let parent = settings_directory(Dir::open_ambient_dir(parent, cap_std::ambient_authority()))?;
    let root = settings_directory(parent.open_dir_nofollow(name))?;
    let directory = settings_directory(root.open_dir_nofollow(mod_id))?;
    let names = sorted_names(directory.entries()?.map(|entry| Ok(entry?.file_name())))?;
    Ok(names.into_iter().map(move |key| {
        let file = (|| {
            let metadata = directory.symlink_metadata(&key)?;
            use cap_std::fs::MetadataExt;
            if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "entry is not a regular file or is a reparse point",
                ));
            }
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            let file = directory.open_with(&key, &options)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "entry is not a regular file or is a reparse point",
                ));
            }
            Ok(file.into_std())
        })();
        (key, file)
    }))
}

#[cfg(not(any(unix, windows)))]
fn snapshot_files(
    root: &Path,
    mod_id: &str,
) -> Result<impl Iterator<Item = (OsString, std::io::Result<fs::File>)>, SnapshotError> {
    // Fail closed on targets without descriptor-relative no-follow traversal.
    let _ = (root, mod_id);
    Err::<std::iter::Empty<(OsString, std::io::Result<fs::File>)>, _>(
        std::io::Error::from(std::io::ErrorKind::Unsupported).into(),
    )
}

/// Registries and settings for hooks fired while a feature generates, on a
/// worldgen worker or the server thread, where no game is at hand. It holds no
/// game, world handle or mutable registry access, and is built once per
/// resource generation of the server.
pub struct WorldgenContext {
    snapshot: Snapshot,
}

impl WorldgenContext {
    /// Names in registry ID order, from the registry snapshot it was built over.
    pub fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.snapshot.registry(name)
    }

    /// A loaded mod's setting captured when this context was built, with no file I/O.
    pub fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.snapshot.setting(mod_id, key)
    }
}

/// Read the same per-key storage as the runtime settings service. No folder is
/// created, and unrelated mod data files are not scanned or loaded eagerly.
pub fn read_setting(directory: &Path, key: &str) -> Option<String> {
    fs::read_to_string(directory.join(key))
        .ok()
        .map(|value| value.trim().to_owned())
}

#[cfg(test)]
mod tests;
