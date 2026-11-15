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
mod tests {
    use super::*;

    fn root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "snapshot-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("reader")).unwrap();
        root
    }

    #[test]
    fn missing_and_empty_settings_do_not_report_omissions() {
        let root = root("empty");
        let captured = capture_settings_with_diagnostics(&root, ["reader", "absent"], |reason| {
            panic!("empty settings reported an omission: {reason}");
        });
        assert!(captured["reader"].is_empty());
        assert!(!captured.contains_key("absent"));
        fs::remove_dir_all(&root).unwrap();
        assert!(
            capture_settings_with_diagnostics(&root, ["reader"], |reason| {
                panic!("missing settings root reported an omission: {reason}");
            })
            .is_empty()
        );
        assert!(!root.exists());
    }

    #[test]
    fn failed_enumeration_retains_known_omitted_keys() {
        let names = [
            Ok(OsString::from("known")),
            Err(std::io::ErrorKind::PermissionDenied.into()),
        ];
        let error = sorted_names(names.into_iter()).unwrap_err();
        let SnapshotError::Enumeration { names, error } = error else {
            panic!("expected an enumeration failure");
        };
        assert_eq!(names, ["known"]);
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        let error = sorted_names((0..=MAX_VISITED_NAMES).map(|i| Ok(format!("key{i:04}").into())))
            .unwrap_err();
        let SnapshotError::EnumerationLimit(names) = error else {
            panic!("expected an enumeration limit");
        };
        assert_eq!(names.len(), MAX_VISITED_NAMES + 1);
        assert_eq!(names.first().unwrap(), "key0000");
        assert_eq!(names.last().unwrap(), "key4096");
    }

    #[test]
    #[cfg(unix)]
    fn unsafe_unreadable_and_non_utf8_entries_keep_their_names() {
        use std::os::unix::ffi::OsStringExt;
        let root = root("named-refusals");
        fs::write(root.join("reader/value"), "inside").unwrap();
        fs::create_dir(root.join("reader/directory")).unwrap();
        std::os::unix::fs::symlink("missing", root.join("reader/dangling")).unwrap();
        let mut refused = BTreeMap::new();
        let captured =
            capture_settings_with_report(&root, ["reader", "reader"], |mod_id, key, reason| {
                assert_eq!(mod_id, "reader");
                assert!(
                    refused
                        .insert(key.unwrap().to_owned(), reason.to_string())
                        .is_none()
                );
            });
        assert_eq!(captured["reader"].len(), 1);
        assert_eq!(refused.len(), 2);
        assert!(refused[OsStr::new("directory")].contains("not a regular file"));
        assert!(refused[OsStr::new("dangling")].starts_with("unsafe or unreadable entry:"));
        let invalid_key = OsString::from_vec(vec![0xff]);
        fs::write(root.join("reader").join(&invalid_key), "invalid key").unwrap();
        refused.clear();
        let captured = capture_settings_with_report(&root, ["reader"], |_, key, reason| {
            refused.insert(key.unwrap().to_owned(), reason.to_string());
        });
        assert!(!captured.contains_key("reader"));
        assert_eq!(refused.len(), 4);
        assert!(refused[&invalid_key].starts_with("package contains a non-UTF-8 key:"));
        assert!(refused[OsStr::new("value")].starts_with("package contains a non-UTF-8 key:"));
        std::os::unix::fs::symlink(root.join("reader"), root.join("alias")).unwrap();
        let mut scopes = Vec::new();
        let captured = capture_settings_with_report(&root, ["alias"], |mod_id, key, reason| {
            assert_eq!(mod_id, "alias");
            assert!(key.is_none());
            scopes.push(reason.to_string());
        });
        assert!(captured.is_empty());
        assert_eq!(scopes.len(), 1);
        assert!(scopes[0].starts_with("cannot inspect settings directory:"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn published_loop_readers_share_one_immutable_settings_capture() {
        let root = root("published");
        fs::write(root.join("reader/value"), "before").unwrap();
        fs::write(root.join("reader/invalid"), [0xff]).unwrap();
        fs::write(root.join("reader/large"), vec![b'x'; MAX_SETTING_BYTES + 1]).unwrap();
        let load = LoadContext::new(
            DataLoadOrigin::ServerDataLoad,
            Arc::new(LoadInputs::new(BTreeMap::new(), root.clone())),
            [],
        );
        let mut diagnostics = Vec::new();
        let settings = capture_settings_with_diagnostics(&root, ["reader"], |report| {
            diagnostics.push(report.to_owned());
        });
        let game = LoopContext {
            side: LoopSide::Server,
            snapshot: Snapshot {
                inputs: load.snapshot.inputs.clone(),
                settings: shared_settings(settings),
            },
            recipe_properties: Default::default(),
        };
        fs::write(root.join("reader/value"), "after").unwrap();
        fs::write(root.join("reader/later-invalid"), [0xff]).unwrap();
        let audio = game.clone();
        let worldgen = game.worldgen_context();
        assert!(Arc::ptr_eq(
            &game.snapshot.settings,
            &audio.snapshot.settings
        ));
        assert!(Arc::ptr_eq(
            &game.snapshot.settings,
            &worldgen.snapshot.settings
        ));
        std::thread::scope(|scope| {
            scope.spawn(|| {
                assert_eq!(
                    worldgen.setting("reader", "value").as_deref(),
                    Some("before")
                );
                assert_eq!(worldgen.setting("reader", "invalid"), None);
            });
            scope.spawn(|| {
                assert_eq!(audio.setting("reader", "value").as_deref(), Some("before"));
                assert_eq!(audio.setting("reader", "later-invalid"), None);
            });
        });
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("\"reader\"/\"invalid\": invalid UTF-8:"));
        assert!(diagnostics[0].contains("\"reader\"/\"large\": file byte limit"));
        capture_settings_with_diagnostics(&root, ["reader"], |report| {
            diagnostics.push(report.to_owned());
        });
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics[1].contains("\"reader\"/\"later-invalid\": invalid UTF-8:"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn initial_snapshot_is_shared_scoped_and_renewed_only_by_a_new_load() {
        let root = root("initial-publication");
        fs::create_dir(root.join("inactive")).unwrap();
        fs::write(root.join("reader/mode"), "before").unwrap();
        fs::write(root.join("inactive/mode"), "not admitted").unwrap();
        let settings = shared_settings(capture_settings_with_diagnostics(
            &root,
            ["reader", "inactive"],
            |reason| panic!("valid initial settings reported a refusal: {reason}"),
        ));
        let inputs = Arc::new(LoadInputs::new(BTreeMap::new(), root.clone()));
        let initial = LoadContext::from_settings_snapshot(
            DataLoadOrigin::ClientPreload,
            inputs.clone(),
            ["reader", "inactive"],
            settings.clone(),
        );
        assert!(Arc::ptr_eq(&settings, &initial.snapshot.settings));
        let admitted = LoadContext::from_settings_snapshot(
            DataLoadOrigin::ClientPreload,
            inputs,
            ["reader"],
            settings.clone(),
        );
        assert_eq!(admitted.setting("inactive", "mode"), None);
        assert!(Arc::ptr_eq(
            &settings[1].1,
            &admitted.snapshot.settings[0].1
        ));
        fs::write(root.join("reader/mode"), "after").unwrap();
        fs::write(root.join("reader/late"), "later key").unwrap();
        let loop_context = admitted.captured_loop_context(LoopSide::Client);
        let worldgen = loop_context.worldgen_context();
        assert!(Arc::ptr_eq(
            &admitted.snapshot.settings,
            &loop_context.snapshot.settings
        ));
        assert_eq!(
            loop_context.setting("reader", "mode").as_deref(),
            Some("before")
        );
        assert_eq!(worldgen.setting("reader", "late"), None);
        let next = admitted.for_load(DataLoadOrigin::ClientPreload, ["reader"]);
        assert_eq!(next.setting("reader", "mode").as_deref(), Some("after"));
        assert_eq!(next.setting("reader", "late").as_deref(), Some("later key"));
        assert_eq!(
            loop_context.setting("reader", "mode").as_deref(),
            Some("before")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn reviewer_capture_follows_external_symlink() {
        let root = root("symlinks");
        let data = root.join("data");
        fs::create_dir_all(data.join("reader")).unwrap();
        fs::write(root.join("secret"), "outside-token").unwrap();
        std::os::unix::fs::symlink(root.join("secret"), data.join("reader/token")).unwrap();
        fs::write(data.join("reader/normal"), "inside").unwrap();
        std::os::unix::fs::symlink("normal", data.join("reader/internal")).unwrap();
        std::os::unix::fs::symlink("missing", data.join("reader/dangling")).unwrap();
        std::os::unix::fs::symlink(root.join("reader"), data.join("alias")).unwrap();
        fs::write(root.join("reader/token"), "escaped").unwrap();
        let captured = capture_settings(&data, ["reader", "alias", "../reader"]);
        assert_eq!(captured["reader"].get("token"), None);
        assert_eq!(captured["reader"].get("internal"), None);
        assert_eq!(captured["reader"].get("dangling"), None);
        assert_eq!(captured["reader"]["normal"], "inside");
        assert!(!captured.contains_key("alias"));
        assert!(!captured.contains_key("../reader"));
        std::os::unix::fs::symlink(&data, root.join("linked-root")).unwrap();
        assert!(capture_settings(&root.join("linked-root"), ["reader"]).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(unix)]
    fn fifo_and_directory_do_not_block_or_become_settings() {
        let root = root("fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(root.join("reader/pipe"))
                .status()
                .unwrap()
                .success()
        );
        fs::create_dir(root.join("reader/directory")).unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let input = root.clone();
        let worker =
            std::thread::spawn(move || send.send(capture_settings(&input, ["reader"])).unwrap());
        let captured = receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("snapshot blocked on FIFO");
        assert!(captured["reader"].is_empty());
        worker.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn admission_is_sorted_and_enumeration_is_bounded() {
        let root = root("ordered");
        fs::create_dir(root.join("second")).unwrap();
        for i in 0..1025 {
            fs::write(root.join(format!("reader/key{i:04}")), "x").unwrap();
            fs::write(root.join(format!("second/key{:04}", 1024 - i)), "x").unwrap();
        }
        let captured = capture_settings(&root, ["second", "reader"]);
        assert_eq!(captured["reader"], captured["second"]);
        assert!(!captured["reader"].contains_key("key1024"));
        assert_eq!(captured["reader"]["key0000"], "x");
        for i in 1025..4097 {
            fs::write(root.join(format!("reader/key{i:04}")), "x").unwrap();
        }
        assert!(!capture_settings(&root, ["reader"]).contains_key("reader"));
        assert!(sorted_names((0..4097).map(|i| Ok(i.to_string().into()))).is_err());
        assert_eq!(
            sorted_names((0..4096).map(|i| Ok(i.to_string().into())))
                .unwrap()
                .len(),
            4096
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(unix)]
    fn validates_open_handle_after_entry_replacement() {
        use std::io::Read;
        let root = root("replaced");
        let path = root.join("reader/value");
        fs::write(&path, "opened value").unwrap();
        let entries = snapshot_files_with_hook(&root, "reader", |_| {
            fs::remove_file(&path).unwrap();
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
        })
        .unwrap();
        let (_, file) = entries.into_iter().next().unwrap();
        let mut file = file.unwrap();
        let mut value = String::new();
        file.read_to_string(&mut value).unwrap();
        assert_eq!(value, "opened value");

        // Now open the FIFO, then replace its path with a regular file. Path
        // metadata would wrongly accept it, but descriptor metadata must refuse.
        let entries = snapshot_files_with_hook(&root, "reader", |_| {
            fs::remove_file(&path).unwrap();
            fs::write(&path, "replacement").unwrap();
        })
        .unwrap();
        assert!(entries.into_iter().next().unwrap().1.is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn byte_limits_and_invalid_utf8() {
        let root = root("bytes");
        fs::write(root.join("reader/exact"), vec![b'a'; 65536]).unwrap();
        fs::write(root.join("reader/large"), vec![b'b'; 65537]).unwrap();
        fs::write(root.join("reader/invalid"), [0xff]).unwrap();
        let mut refused = Vec::new();
        let captured = capture_settings_with_report(&root, ["reader"], |mod_id, key, reason| {
            refused.push((
                mod_id.to_owned(),
                key.unwrap().to_owned(),
                reason.to_string(),
            ));
        });
        assert_eq!(captured["reader"]["exact"].len(), 65536);
        assert_eq!(captured["reader"].get("large"), None);
        assert_eq!(captured["reader"].get("invalid"), None);
        assert_eq!(refused.len(), 2);
        assert_eq!(refused[0].0, "reader");
        assert_eq!(refused[0].1, "invalid");
        assert!(refused[0].2.starts_with("invalid UTF-8:"));
        assert_eq!(refused[1].1, "large");
        assert!(refused[1].2.starts_with("file byte limit"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn key_limit() {
        let root = root("count");
        for i in 0..1027 {
            fs::write(root.join(format!("reader/key{i:04}")), "x").unwrap();
        }
        let mut refused = Vec::new();
        let captured = capture_settings_with_report(&root, ["reader"], |mod_id, key, reason| {
            assert_eq!(mod_id, "reader");
            assert!(reason.to_string().starts_with("key limit"));
            refused.push(key.unwrap().to_owned());
        });
        assert_eq!(captured["reader"].len(), 1024);
        assert_eq!(refused, ["key1024", "key1025", "key1026"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reviewer_whitespace_spends_raw_aggregate_budget() {
        let root = root("raw-budget");
        for i in 0..256 {
            fs::write(root.join(format!("reader/key{i:04}")), vec![b' '; 65536]).unwrap();
        }
        fs::write(root.join("reader/z-extra"), "x").unwrap();
        let mut refused = Vec::new();
        let captured = capture_settings_with_report(&root, ["reader"], |mod_id, key, reason| {
            assert_eq!(mod_id, "reader");
            assert!(reason.to_string().starts_with("snapshot byte limit"));
            refused.push(key.unwrap().to_owned());
        });
        assert_eq!(captured["reader"].len(), 256);
        assert!(!captured["reader"].contains_key("z-extra"));
        assert_eq!(refused, ["z-extra"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn reviewer_symlink_replacement_keeps_opened_value() {
        use std::io::Read;
        let root = root("symlink-replaced");
        let path = root.join("reader/value");
        fs::write(&path, "opened").unwrap();
        fs::write(root.join("target"), "replacement").unwrap();
        let entries = snapshot_files_with_hook(&root, "reader", |_| {
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(root.join("target"), &path).unwrap();
        })
        .unwrap();
        let (_, file) = entries.into_iter().next().unwrap();
        let mut file = file.unwrap();
        let mut value = String::new();
        file.read_to_string(&mut value).unwrap();
        assert_eq!(value, "opened");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn aggregate_limit() {
        let root = root("aggregate");
        fs::create_dir(root.join("second")).unwrap();
        for i in 0..129 {
            for package in ["reader", "second"] {
                fs::write(root.join(format!("{package}/key{i}")), vec![b'x'; 65536]).unwrap();
            }
        }
        let captured = capture_settings(&root, ["second", "reader"]);
        assert_eq!(captured["reader"].len(), 129);
        assert_eq!(captured["second"].len(), 127);
        assert_eq!(
            captured
                .values()
                .flat_map(|values| values.values())
                .map(String::len)
                .sum::<usize>(),
            16 * 1024 * 1024
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn settings_are_scoped_and_frozen_until_a_new_load() {
        let root = std::env::temp_dir().join(format!(
            "horizon-load-settings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let active = root.join("active");
        fs::create_dir_all(&active).unwrap();
        fs::create_dir_all(root.join("inactive")).unwrap();
        fs::write(active.join("mode"), "  before\n").unwrap();
        fs::write(active.join("empty"), " \n").unwrap();
        fs::write(active.join("invalid"), [0xff]).unwrap();
        fs::write(root.join("inactive/mode"), "not loaded").unwrap();
        let inputs = Arc::new(LoadInputs::new(BTreeMap::new(), root.clone()));
        let before = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs.clone(), ["active"]);

        // Change the files before the first lookup, not just between reads.
        fs::write(active.join("mode"), "  after\n").unwrap();
        fs::write(active.join("late"), "new key").unwrap();
        assert_eq!(before.setting("active", "mode").as_deref(), Some("before"));
        assert_eq!(before.setting("active", "empty").as_deref(), Some(""));
        assert_eq!(before.setting("active", "invalid"), None);
        assert_eq!(before.setting("active", "late"), None);
        assert_eq!(before.setting("inactive", "mode"), None);
        assert_eq!(read_setting(&active, "mode").as_deref(), Some("after"));

        let after = LoadContext::new(DataLoadOrigin::ClientPreload, inputs.clone(), ["active"]);
        assert_eq!(after.setting("active", "mode").as_deref(), Some("after"));
        assert_eq!(after.setting("active", "late").as_deref(), Some("new key"));
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(before.setting("active", "mode").as_deref(), Some("before"));
        assert_eq!(after.setting("active", "mode").as_deref(), Some("after"));
        let missing = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, ["active"]);
        assert_eq!(missing.setting("active", "mode"), None);
        assert!(!root.exists());
    }

    #[test]
    fn a_loop_context_shares_the_registries_and_captures_its_own_settings() {
        let root = std::env::temp_dir().join(format!(
            "horizon-loop-settings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let mod_dir = root.join("active");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("mode"), "one").unwrap();
        let registries = BTreeMap::from([("item".to_owned(), vec!["a:b".to_owned()])]);
        let inputs = Arc::new(LoadInputs::new(registries, root.clone()));
        let load = LoadContext::new(DataLoadOrigin::ClientPreload, inputs, ["active"]);

        let client = load.loop_context(LoopSide::Client, ["active"]);
        fs::write(mod_dir.join("mode"), "two").unwrap();
        let server = load.loop_context(LoopSide::Server, ["active"]);

        assert_eq!(client.side(), LoopSide::Client);
        assert_eq!(server.side(), LoopSide::Server);
        assert_eq!(
            client.registry("item").as_deref(),
            Some(&["a:b".to_owned()][..])
        );
        assert_eq!(server.registry("item"), client.registry("item"));
        assert_eq!(client.setting("active", "mode").as_deref(), Some("one"));
        assert_eq!(server.setting("active", "mode").as_deref(), Some("two"));
        assert_eq!(load.setting("active", "mode").as_deref(), Some("one"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_worldgen_context_shares_the_registries_and_captures_its_own_settings() {
        let root = std::env::temp_dir().join(format!(
            "horizon-worldgen-settings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let mod_dir = root.join("active");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("ground"), "podzol").unwrap();
        let registries = BTreeMap::from([("block".to_owned(), vec!["a:b".to_owned()])]);
        let inputs = Arc::new(LoadInputs::new(registries, root.clone()));
        let load = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, ["active"]);
        fs::write(mod_dir.join("ground"), "mud").unwrap();

        let worldgen = load.worldgen_context(["active"]);
        fs::write(mod_dir.join("ground"), "moss").unwrap();

        assert_eq!(
            worldgen.registry("block").as_deref(),
            Some(&["a:b".to_owned()][..])
        );
        assert_eq!(worldgen.setting("active", "ground").as_deref(), Some("mud"));
        assert_eq!(worldgen.setting("inactive", "ground"), None);
        assert_eq!(load.setting("active", "ground").as_deref(), Some("podzol"));
        fs::remove_dir_all(&root).unwrap();
    }
}
