//! The CLI and Storage page share one fixed-path inventory.
#[cfg(test)]
#[path = "inventory_regressions.rs"]
mod regressions;
use super::*;
use crate::{backend, instances, native_mods};
use serde::Serialize;
use std::sync::Arc;

const BUILD_CACHES: [BuildCache; 9] = [
    BuildCache::Source,
    BuildCache::Tools,
    BuildCache::PrebuiltOutput,
    BuildCache::InternalsSync,
    BuildCache::Prebuilt,
    BuildCache::ServerBuilds,
    BuildCache::Rust,
    BuildCache::Cargo,
    BuildCache::Scratch,
];

#[derive(Serialize)]
pub struct Row {
    pub category: Category,
    pub bytes: u64,
    pub freed_bytes: u64,
    pub remaining_bytes: u64,
    /// Refused or busy units are retained, with their allocated bytes and a visible reason.
    pub kept: Vec<Kept>,
}

#[derive(Serialize)]
pub struct Kept {
    pub bytes: u64,
    pub reason: String,
}

#[derive(Serialize)]
pub struct Report {
    pub rows: Vec<Row>,
    pub total_bytes: u64,
}

impl Category {
    pub const ALL: [Self; 5] = [
        Self::BuildFiles,
        Self::PrebuiltGames,
        Self::SpareGameBuilds,
        Self::RustCompiler,
        Self::OtherCaches,
    ];
}

struct Root {
    path: PathBuf,
    directory: Option<Arc<Dir>>,
}
impl Root {
    fn open(root: &Path) -> Result<Self, Error> {
        let directory = match Dir::open_ambient_dir(root, cap_std::ambient_authority()) {
            Ok(directory) => Some(Arc::new(directory)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        checked(root, true)?;
        if root.exists() {
            // Windows canonical paths gain a `\\?\` prefix and long names, so compare ancestors' reparse points instead.
            #[cfg(windows)]
            for ancestor in root.ancestors().filter(|path| path.parent().is_some()) {
                checked(ancestor, true)?;
            }
            #[cfg(not(windows))]
            if root.canonicalize()? != root {
                return Err(Error::Unsafe(root.to_owned()));
            }
        }
        Ok(Self {
            path: root.to_owned(),
            directory,
        })
    }
}

struct Entry {
    category: Category,
    directory: Option<Arc<Dir>>,
    relative: PathBuf,
    path: PathBuf,
}

impl Entry {
    fn fixed(root: &Root, relative: &Path, category: Category) -> Result<Self, Error> {
        let mut path = root.path.clone();
        for component in relative.components() {
            let std::path::Component::Normal(component) = component else {
                return Err(Error::Unsafe(relative.to_owned()));
            };
            path.push(component);
            match fs::symlink_metadata(&path) {
                Ok(meta) if linked(&meta) => return Err(Error::Unsafe(path)),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Self {
            category,
            directory: root.directory.clone(),
            relative: relative.to_owned(),
            path,
        })
    }

    /// Parse the exact revision-bearing names written by the cache constructors.
    fn revision(&self) -> Option<&str> {
        let name = self.relative.file_name()?.to_str()?;
        let parent = self.relative.parent()?;
        let revision = if parent == Path::new("native/tools") {
            (name.len() == 12).then_some(name)?
        } else if parent == Path::new("native/prebuilt") {
            let (revision, rest) = name.split_once('-')?;
            // A build's SDK slices (`<build id>-<target>[-server]`, a developer install's with
            // `.developer`), and the older single-archive caches (`<commit>-horizon-prebuilt-v<n>-
            // <target>`), which native builds no longer read but eviction must still recognize.
            let rest = rest.strip_suffix(".developer").unwrap_or(rest);
            let current = horizon_mod_package::index::native::prebuilt_prefix();
            let target = [
                "horizon-prebuilt-v3-",
                "horizon-prebuilt-v4-",
                "horizon-prebuilt-v5-",
                current.as_str(),
            ]
            .into_iter()
            .find_map(|prefix| rest.strip_prefix(prefix))
            .unwrap_or(rest);
            let target = target.strip_suffix("-server").unwrap_or(target);
            crate::rust_toolchain::hosts()
                .iter()
                .any(|host| host.target == target)
                .then_some(revision)?
        } else if parent == Path::new("native/internals-sync") {
            if let Some(catalog) = name.strip_prefix("catalog-") {
                let (revision, host) = catalog.split_once('-')?;
                crate::rust_toolchain::hosts()
                    .iter()
                    .any(|candidate| candidate.target == host)
                    .then_some(revision)?
            } else {
                name
            }
        } else if parent == Path::new("native/source") {
            name
        } else {
            return None;
        };
        let expected = if parent == Path::new("native/tools") {
            12
        } else {
            40
        };
        (revision.len() == expected
            && revision
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .then_some(revision)
    }

    /// What an interrupted unpack or download left beside the units of `native/prebuilt`: a
    /// `.partial` folder, a `.lock` file, a `.download` archive. Nothing uses one while the build
    /// lease is held, and eviction holds it, so a lock file is removed only under that lease.
    fn leftover(&self) -> bool {
        self.relative.parent() == Some(Path::new("native/prebuilt"))
            && self
                .relative
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    [".partial", ".lock", ".download"]
                        .iter()
                        .any(|suffix| name.ends_with(suffix))
                })
    }

    fn last_used(&self) -> Result<Option<std::time::SystemTime>, Error> {
        let Some(root) = &self.directory else {
            return Ok(None);
        };
        let meta = match fs::symlink_metadata(&self.path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if linked(&meta) {
            return Err(Error::Unsafe(self.path.clone()));
        }
        let (relative, stamp) = if meta.is_dir() {
            (
                self.relative.as_path(),
                std::ffi::OsString::from(".last-used"),
            )
        } else {
            let Some(name) = self.relative.file_name() else {
                return Ok(None);
            };
            let mut stamp = std::ffi::OsString::from(".");
            stamp.push(name);
            stamp.push(".last-used");
            (self.relative.parent().unwrap_or(Path::new("")), stamp)
        };
        let mut directory = root.try_clone()?;
        for part in relative.components() {
            directory = directory.open_dir_nofollow(part.as_os_str())?;
        }
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        let mut options = cap_std::fs::OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        match directory.open_with(&stamp, &options) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let meta = directory.symlink_metadata(&stamp)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(Error::Unsafe(self.path.join(stamp)));
        }
        Ok(Some(meta.modified()?.into_std()))
    }

    /// The lease integration changes this classification, not policy or deletion code.
    fn access(&self) -> Result<Access, Error> {
        match self.category {
            Category::RustCompiler => Ok(Access::BuildTools),
            _ => Access::for_unit(&self.path),
        }
    }

    fn clear(&self) -> Result<(), Error> {
        if !self.path.try_exists()? {
            return Ok(());
        }
        let Some(root) = &self.directory else {
            return Ok(());
        };
        let access = self.access()?;
        if matches!(access, Access::LeasedGame) {
            return crate::cache_cleanup::delete_game_unit(&self.path);
        }
        access.check()?;
        let mut parent = root.try_clone()?;
        for part in self.relative.parent().unwrap_or(Path::new("")).components() {
            parent = parent.open_dir_nofollow(part.as_os_str())?;
        }
        Storage::clear_entry(root, &parent, Path::new(self.relative.file_name().unwrap()))
    }
}

fn usage_stamp(name: &Path) -> bool {
    let name = name.as_os_str().to_string_lossy();
    name == ".last-used" || (name.starts_with('.') && name.ends_with(".last-used"))
}

fn add(
    entries: &mut Vec<Entry>,
    root: &Root,
    relative: &str,
    category: Category,
) -> Result<(), Error> {
    entries.push(Entry::fixed(root, Path::new(relative), category)?);
    Ok(())
}

impl Storage {
    fn entries(&self) -> Result<Vec<Entry>, Error> {
        let mut entries = Vec::new();
        let build_root = Root::open(&self.root)?;
        for cache in BUILD_CACHES {
            if matches!(
                cache,
                BuildCache::Rust
                    | BuildCache::Source
                    | BuildCache::Tools
                    | BuildCache::PrebuiltOutput
                    | BuildCache::Prebuilt
                    | BuildCache::InternalsSync
            ) {
                let parent = self.root.join(cache.relative());
                if checked(&parent, true)?.is_some() {
                    for child in fs::read_dir(parent)? {
                        let child = child?;
                        if usage_stamp(Path::new(&child.file_name())) {
                            continue;
                        }
                        entries.push(Entry::fixed(
                            &build_root,
                            &Path::new(cache.relative()).join(child.file_name()),
                            cache.category(),
                        )?);
                    }
                }
            } else {
                add(
                    &mut entries,
                    &build_root,
                    cache.relative(),
                    cache.category(),
                )?;
            }
        }
        let build_files = Category::BuildFiles;
        for name in [
            "built.json",
            "internals.json",
            "build.log",
            "catalog.partial",
        ] {
            add(
                &mut entries,
                &build_root,
                &format!("native/{name}"),
                build_files,
            )?;
        }
        // The target lock files must remain in place. Their contents are inventoried here but
        // deletion goes through Cache, which also acquires Cargo's locks.
        let target = self.root.join("native/target");
        if checked(&target, true)?.is_some() {
            for child in fs::read_dir(&target)? {
                let child = child?;
                if crate::cache_cleanup::reserved(Path::new(&child.file_name()))
                    || usage_stamp(Path::new(&child.file_name()))
                {
                    continue;
                }
                let relative = Path::new("native/target").join(child.file_name());
                checked(&child.path(), child.file_type()?.is_dir())?;
                if child.file_type()?.is_dir() && child.file_name().to_string_lossy().contains('-')
                {
                    // Cross-compilation target triples own independent profile units too.
                    for profile in fs::read_dir(child.path())? {
                        let profile = profile?;
                        if crate::cache_cleanup::reserved(Path::new(&profile.file_name()))
                            || usage_stamp(Path::new(&profile.file_name()))
                        {
                            continue;
                        }
                        entries.push(Entry::fixed(
                            &build_root,
                            &relative.join(profile.file_name()),
                            build_files,
                        )?);
                    }
                } else {
                    entries.push(Entry::fixed(&build_root, &relative, build_files)?);
                }
            }
        }
        let data = Root::open(&backend::data_dir())?;
        let played: HashSet<_> = instances::list()
            .into_iter()
            .filter_map(|instance| instance.resolve_version(None).ok())
            .collect();
        for version in backend::installed() {
            if native_mods::is_native(&version)
                && !played.contains(&version)
                && !native_mods::resource_dependents(&version)
                && !backend::in_use(&version)
            {
                add(
                    &mut entries,
                    &data,
                    &format!("versions/{version}"),
                    Category::SpareGameBuilds,
                )?;
            }
        }
        // Interrupted removal deliberately loses completion. Keep the remaining unit
        // visible for residual reporting and a safe retry, even though installed() omits it.
        match fs::read_dir(data.path.join("versions")) {
            Ok(children) => {
                for child in children {
                    let child = child?;
                    if child.file_type()?.is_dir()
                        && !child.path().join(".installed").is_file()
                        && !child.file_name().to_string_lossy().starts_with(".trash-")
                    {
                        entries.push(Entry::fixed(
                            &data,
                            &Path::new("versions").join(child.file_name()),
                            Category::SpareGameBuilds,
                        )?);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        for relative in ["cache/github", "mod-browser", "profile-heads"] {
            add(&mut entries, &data, relative, Category::OtherCaches)?;
        }
        for instance in instances::list() {
            // A missing version cannot prove that the instance is idle. Keep its caches.
            let Ok(version) = instance.resolve_version(None) else {
                continue;
            };
            if backend::in_use(&version) {
                continue;
            }
            let root = Root::open(&instance.home)?;
            old_logs(&mut entries, &root)?;
            unused_jars(&mut entries, &root)?;
            for relative in [".horizon-cache", "wasm-cache", "cache"] {
                add(&mut entries, &root, relative, Category::OtherCaches)?;
            }
        }
        // Detached trees can remain beside any managed cache after an interrupted removal.
        let parents: HashSet<_> = entries
            .iter()
            .filter_map(|entry| entry.path.parent().map(Path::to_owned))
            .chain([
                self.root.clone(),
                data.path.clone(),
                data.path.join("versions"),
            ])
            .collect();
        for parent in parents {
            let root = Root::open(&parent)?;
            if root.directory.is_none() {
                continue;
            }
            for entry in fs::read_dir(&parent)? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with(".trash-") {
                    entries.push(Entry::fixed(
                        &root,
                        Path::new(&entry.file_name()),
                        Category::OtherCaches,
                    )?);
                }
            }
        }
        Ok(entries)
    }

    pub(super) fn stamp_build(&self) -> Result<(), Error> {
        for entry in self.entries()? {
            if !entry.path.starts_with(&self.root) || !entry.path.is_dir() {
                continue;
            }
            let relative = entry.path.strip_prefix(&self.root).unwrap();
            if !(relative == Path::new("cargo")
                || relative == Path::new("tmp")
                || (relative.starts_with("rust")
                    && entry.path.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .starts_with(crate::rust_toolchain::VERSION)
                    })))
            {
                continue;
            }
            let Some(root) = &entry.directory else {
                continue;
            };
            let mut directory = root.try_clone()?;
            for part in entry.relative.components() {
                directory = directory.open_dir_nofollow(part.as_os_str())?;
            }
            use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
            let mut options = cap_std::fs::OpenOptions::new();
            options
                .write(true)
                .create(true)
                .truncate(true)
                .follow(FollowSymlinks::No);
            directory.open_with(".last-used", &options)?;
        }
        Ok(())
    }

    pub(super) fn evict_inventory(&self, now: std::time::SystemTime) -> Result<Report, Error> {
        use super::policy::{self, Tier, Unit};
        let entries = self.entries()?;
        for entry in &entries {
            entry.last_used()?;
        }
        let settings = backend::Settings::load();
        let mut revisions = HashSet::new();
        let mut unknown_revision = false;
        let mut native_enabled = false;
        for instance in instances::list() {
            for bundle in crate::native_status::bundles_on(&instance.home) {
                if let Some(native) = bundle.manifest.native {
                    native_enabled = true;
                    revisions.insert(native.horizon);
                }
            }
            match instance.resolve_version(None) {
                Ok(version) => {
                    if let Some(about) = native_mods::about(&version) {
                        revisions.insert(about.horizon);
                    } else if let Some(nightly) = version.strip_prefix("nightly-") {
                        revisions.insert(nightly.rsplit('-').next().unwrap_or(nightly).to_owned());
                    } else {
                        unknown_revision = true;
                    }
                }
                Err(_) => unknown_revision = true,
            }
        }
        let mut seen = HashSet::new();
        let units: Vec<_> = entries
            .iter()
            .map(|entry| {
                let last_used = entry.last_used()?;
                let tier = if entry.relative.starts_with("rust")
                    && entry.path.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .starts_with(crate::rust_toolchain::VERSION)
                    })
                    && settings.native_mods_allowed_at.is_some()
                    && native_enabled
                {
                    // Conservatively retain the whole toolchain set while consent is enabled.
                    Tier::Protected
                } else if entry.leftover() {
                    Tier::Expiring
                } else if [
                    "native/source",
                    "native/tools",
                    "native/internals-sync",
                    "native/prebuilt",
                ]
                .iter()
                .any(|root| entry.relative.starts_with(root))
                {
                    let Some(unit_revision) = entry.revision() else {
                        return Ok(Unit {
                            eligible: false,
                            bytes: walk(&entry.path, &mut seen)?,
                            last_used,
                            tier: Tier::Protected,
                        });
                    };
                    let tool = entry.relative.starts_with("native/tools");
                    let used = unknown_revision
                        || (tool && native_mods::MIXINS_REV.get(..12) == Some(unit_revision))
                        || revisions.iter().any(|revision| {
                            let length = revision.len().min(unit_revision.len());
                            revision.get(..length) == unit_revision.get(..length)
                        });
                    if used && entry.category == Category::PrebuiltGames {
                        Tier::Lru
                    } else if used {
                        Tier::Protected
                    } else {
                        Tier::Expiring
                    }
                } else if entry.relative.starts_with("rust")
                    || entry
                        .path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with(".trash-"))
                {
                    Tier::Expiring
                } else {
                    Tier::Lru
                };
                Ok(Unit {
                    eligible: entry.access()?.check().is_ok(),
                    bytes: walk(&entry.path, &mut seen)?,
                    last_used,
                    tier,
                })
            })
            .collect::<Result<_, Error>>()?;
        let limit = settings
            .cache_limit_gib
            .unwrap_or(policy::DEFAULT_LIMIT_GIB);
        let selected = policy::select(
            &units,
            (limit != 0).then(|| limit.saturating_mul(1 << 30)),
            now,
        );
        let mut report = self.apply_retaining(Vec::new(), None, false, true)?;
        for row in &mut report.rows {
            row.bytes = allocated(&entries, row.category)?;
        }
        for (index, entry) in entries.iter().enumerate() {
            let row = report
                .rows
                .iter_mut()
                .find(|row| row.category == entry.category)
                .unwrap();
            if selected.contains(&index) {
                let result = if entry.path == self.root.join("native/target") {
                    self.lease.clear().map(|_| ())
                } else {
                    entry.clear()
                };
                match result {
                    Ok(()) => {}
                    Err(
                        error @ (Error::Busy | Error::UnprovenExclusion | Error::IncompleteUnit),
                    ) => row.kept.push(Kept {
                        bytes: allocated_bytes(&entry.path)?,
                        reason: native_mods::cache_error(error),
                    }),
                    Err(error) => return Err(error),
                }
            }
            let remaining = allocated_bytes(&entry.path)?;
            if !selected.contains(&index) && remaining > 0 {
                row.kept.push(Kept {
                    bytes: remaining,
                    reason: if !units[index].eligible {
                        native_mods::cache_error(
                            entry
                                .access()?
                                .check()
                                .err()
                                .unwrap_or(Error::UnprovenExclusion),
                        )
                    } else {
                        crate::i18n::tr("Protected, recently used, or last use unknown.").into()
                    },
                });
            }
        }
        for row in &mut report.rows {
            row.remaining_bytes = allocated(&entries, row.category)?;
            row.freed_bytes = row.bytes.saturating_sub(row.remaining_bytes);
        }
        report.total_bytes = report.rows.iter().map(|row| row.bytes).sum();
        Ok(report)
    }

    pub fn report(&self, clear: Option<Category>, all: bool) -> Result<Report, Error> {
        let registry_entries = match fs::read_dir(backend::data_dir().join("run-leases")) {
            Ok(mut entries) => entries.try_fold(0usize, |count, entry| entry.map(|_| count + 1))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        eprintln!(
            "{}",
            crate::i18n::trf(
                "Runtime lease registry entries: {count}",
                &[("count", &registry_entries.to_string())]
            )
        );
        // Also refuse an external Cargo holding a target lock before starting a cleanup.
        self.lease.bytes()?;
        if clear.is_some() || all {
            use horizon_mod_package::runtime_leases::{cleanup::Cleanup, Error as LeaseError};
            match Cleanup::existing(&backend::data_dir()) {
                Ok(cleanup) => {
                    cleanup.collect_records()?;
                }
                Err(LeaseError::Busy) => {} // Other cache classes can still be reported/retained.
                Err(LeaseError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let entries = self.entries()?;
        let mut report = self.apply_retaining(entries, clear, all, true)?;
        if clear.is_none() && !all {
            for entry in self.entries()? {
                if let Err(error) = entry.access().and_then(|access| access.check()) {
                    let bytes = allocated_bytes(&entry.path)?;
                    if bytes > 0 {
                        report
                            .rows
                            .iter_mut()
                            .find(|row| row.category == entry.category)
                            .unwrap()
                            .kept
                            .push(Kept {
                                bytes,
                                reason: native_mods::cache_error(error),
                            });
                    }
                }
            }
        }
        Ok(report)
    }

    #[cfg(test)]
    fn apply(
        &self,
        entries: Vec<Entry>,
        clear: Option<Category>,
        all: bool,
    ) -> Result<Report, Error> {
        self.apply_retaining(entries, clear, all, false)
    }

    fn apply_retaining(
        &self,
        entries: Vec<Entry>,
        clear: Option<Category>,
        all: bool,
        retain: bool,
    ) -> Result<Report, Error> {
        // Validate every descendant before any deletion and deduplicate allocated blocks per row.
        let mut rows = Vec::new();
        for category in Category::ALL {
            let bytes = allocated(&entries, category)?;
            rows.push(Row {
                category,
                bytes,
                freed_bytes: 0,
                remaining_bytes: bytes,
                kept: Vec::new(),
            });
        }
        for entry in &entries {
            if all || clear == Some(entry.category) {
                let result = if entry.path == self.root.join("native/target") {
                    self.lease.clear().map(|_| ())
                } else {
                    entry.clear()
                };
                match result {
                    Ok(()) => {}
                    Err(
                        error @ (Error::Busy | Error::UnprovenExclusion | Error::IncompleteUnit),
                    ) if retain => {
                        let row = rows
                            .iter_mut()
                            .find(|row| row.category == entry.category)
                            .unwrap();
                        row.kept.push(Kept {
                            bytes: allocated_bytes(&entry.path)?,
                            reason: native_mods::cache_error(error),
                        });
                        // A busy build may be using subsequent build-tool units too. Stop
                        // this clear attempt rather than deleting more after contention.
                        if matches!(entry.access()?, Access::BuildTools) {
                            break;
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        for row in &mut rows {
            if all || clear == Some(row.category) {
                let remaining = allocated(&entries, row.category)?;
                row.freed_bytes = row.bytes.saturating_sub(remaining);
                row.remaining_bytes = remaining;
            }
        }
        Ok(Report {
            total_bytes: rows
                .iter()
                .fold(0u64, |total, row| total.saturating_add(row.bytes)),
            rows,
        })
    }
}

fn allocated(entries: &[Entry], category: Category) -> Result<u64, Error> {
    let mut seen = HashSet::new();
    entries
        .iter()
        .filter(|entry| entry.category == category)
        .try_fold(0u64, |bytes, entry| {
            Ok(bytes.saturating_add(walk(&entry.path, &mut seen)?))
        })
}

fn old_logs(entries: &mut Vec<Entry>, home: &Root) -> Result<(), Error> {
    for directory in ["logs", "crash-reports"] {
        let folder = Entry::fixed(home, Path::new(directory), Category::OtherCaches)?.path;
        if checked(&folder, true)?.is_none() {
            continue;
        }
        let mut files = Vec::new();
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            if crate::cache_cleanup::reserved(Path::new(&entry.file_name()))
                || usage_stamp(Path::new(&entry.file_name()))
            {
                continue;
            }
            let Some(meta) = checked(&entry.path(), false)? else {
                continue;
            };
            files.push((meta.modified()?, entry.file_name()));
        }
        files.sort();
        files.pop(); // Always retain the latest report, even when it is older than fourteen days.
        for (modified, name) in files {
            if modified
                .elapsed()
                .is_ok_and(|age| age.as_secs() > 14 * 86400)
            {
                entries.push(Entry::fixed(
                    home,
                    &Path::new(directory).join(name),
                    Category::OtherCaches,
                )?);
            }
        }
    }
    Ok(())
}

fn unused_jars(entries: &mut Vec<Entry>, home: &Root) -> Result<(), Error> {
    use horizon_mod_package::{
        mod_list::{DIR, NATIVE_DIR},
        Package,
    };
    let folder = Entry::fixed(home, Path::new("jars"), Category::OtherCaches)?.path;
    if checked(&folder, true)?.is_none() {
        return Ok(());
    }
    let mut referenced = HashSet::new();
    for directory in [DIR, NATIVE_DIR] {
        let path = Entry::fixed(home, Path::new(directory), Category::OtherCaches)?.path;
        for package in horizon_mod_package::find(&path) {
            // A malformed package cannot prove a jar is unused. Retain all jars in that case.
            let Ok(package) = Package::read(&package) else {
                return Ok(());
            };
            referenced.extend(
                package
                    .manifest
                    .jar
                    .iter()
                    .map(|jar| jar.cache_path(&folder)),
            );
        }
    }
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        if !crate::cache_cleanup::reserved(Path::new(&entry.file_name()))
            && !usage_stamp(Path::new(&entry.file_name()))
            && !referenced.contains(&entry.path())
        {
            entries.push(Entry::fixed(
                home,
                &Path::new("jars").join(entry.file_name()),
                Category::OtherCaches,
            )?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod eviction_tests {
    use super::*;
    #[test]
    fn first_seen_stamp_ages_without_being_refreshed_by_inventory() {
        let home = crate::storage::tests::Home::new();
        home.seed("rust/old/payload");
        let root = Root::open(&home.0).unwrap();
        let entry = Entry::fixed(&root, Path::new("rust/old"), Category::RustCompiler).unwrap();
        let used = entry.last_used().unwrap().unwrap();
        assert_eq!(entry.last_used().unwrap(), Some(used));
        let units = [policy::Unit {
            bytes: 100,
            eligible: true,
            last_used: Some(used),
            tier: policy::Tier::Expiring,
        }];
        assert!(policy::select(&units, None, used).is_empty());
        assert_eq!(
            policy::select(
                &units,
                None,
                used + std::time::Duration::from_secs(7 * 86400)
            ),
            [0]
        );
    }

    #[test]
    fn revision_names_are_parsed_not_searched() {
        let home = crate::storage::tests::Home::new();
        let root = Root::open(&home.0).unwrap();
        let sha = "0123456789abcdef0123456789abcdef01234567";
        for (path, expected) in [
            (format!("native/source/{sha}"), Some(sha)),
            (format!("native/source/not-{sha}"), None),
            (format!("native/tools/{}", &sha[..12]), Some(&sha[..12])),
            (
                format!("native/prebuilt/{sha}-horizon-prebuilt-v3-x86_64-unknown-linux-gnu"),
                Some(sha),
            ),
            (
                format!("native/prebuilt/{sha}-horizon-prebuilt-v3-x86_64-unknown-linux-gnu-server"),
                Some(sha),
            ),
            (
                format!("native/prebuilt/{sha}-horizon-prebuilt-v4-x86_64-unknown-linux-gnu"),
                Some(sha),
            ),
            (
                format!("native/prebuilt/{sha}-horizon-prebuilt-v4-x86_64-unknown-linux-gnu-server"),
                Some(sha),
            ),
            (format!("native/prebuilt/{sha}-horizon-prebuilt-v3-unknown-target"), None),
            (format!("native/prebuilt/{sha}-horizon-prebuilt-v4-unknown-target"), None),
            (format!("native/prebuilt/{sha}-horizon-prebuilt-v30-x86_64-unknown-linux-gnu"), None),
            (format!("native/prebuilt/{sha}-horizon-prebuilt-v4-x86_64-unknown-linux-gnu-server-server"), None),
            (format!("native/prebuilt/{sha}-horizon-prebuilt-v4-x86_64-unknown-linux-gnu.tar.gz"), None),
            (format!("native/prebuilt/not-{sha}-horizon-prebuilt-v4-x86_64-unknown-linux-gnu"), None),
            // A build's SDK slices, and a developer install's laid-out game.
            (format!("native/prebuilt/{sha}-x86_64-unknown-linux-gnu"), Some(sha)),
            (format!("native/prebuilt/{sha}-x86_64-unknown-linux-gnu-server"), Some(sha)),
            (format!("native/prebuilt/{sha}-x86_64-unknown-linux-gnu.developer"), Some(sha)),
            (format!("native/prebuilt/{sha}-unknown-target"), None),
            (format!("native/prebuilt/{sha}-x86_64-unknown-linux-gnu.partial"), None),
            (
                format!("native/internals-sync/catalog-{sha}-x86_64-unknown-linux-gnu"),
                Some(sha),
            ),
            (format!("native/internals-sync/garbage-{sha}"), None),
        ] {
            assert_eq!(
                Entry::fixed(&root, Path::new(&path), Category::BuildFiles)
                    .unwrap()
                    .revision(),
                expected
            );
        }
        // What an interrupted unpack or download left is known as such, and nothing else is.
        for (path, leftover) in [
            (format!("native/prebuilt/.{sha}-horizon-prebuilt-v6-x86_64-unknown-linux-gnu.tar.gz.partial"), true),
            (format!("native/prebuilt/{sha}-x86_64-unknown-linux-gnu.lock"), true),
            ("native/prebuilt/runtime-x86_64-unknown-linux-gnu.tar.xz.download".to_owned(), true),
            (format!("native/prebuilt/{sha}-x86_64-unknown-linux-gnu"), false),
            (format!("native/source/{sha}.partial"), false),
        ] {
            let entry = Entry::fixed(&root, Path::new(&path), Category::BuildFiles).unwrap();
            assert_eq!(entry.leftover(), leftover, "{path}");
        }
    }
    #[test]
    fn refused_units_are_residual_with_bytes_and_reason() {
        let home = crate::storage::tests::Home::new();
        let file = home.seed("game/output");
        let storage = Storage::at(&home.0).unwrap();
        let root = Root::open(&home.0).unwrap();
        let entries = vec![Entry::fixed(&root, Path::new("game"), Category::BuildFiles).unwrap()];
        let directory = file.parent().unwrap();
        // Give inline-directory filesystems enough names to allocate directory blocks too.
        for index in 0..32 {
            fs::write(
                directory.join(format!("padding-{index:02}-{}", "x".repeat(180))),
                [],
            )
            .unwrap();
        }
        // Independent filesystem allocations, not the recursive production sum.
        let own_bytes = |path: &Path| {
            super::super::allocation(path, &fs::symlink_metadata(path).unwrap())
                .unwrap()
                .1
        };
        let bytes = own_bytes(directory)
            + fs::read_dir(directory)
                .unwrap()
                .map(|entry| own_bytes(&entry.unwrap().path()))
                .sum::<u64>();
        let report = storage.apply_retaining(entries, None, true, true).unwrap();
        let row = report
            .rows
            .iter()
            .find(|row| row.category == Category::BuildFiles)
            .unwrap();
        assert_eq!(row.bytes, bytes);
        assert_eq!(row.freed_bytes, 0);
        assert_eq!(row.remaining_bytes, bytes);
        assert_eq!(row.kept.len(), 1);
        assert_eq!(row.kept[0].bytes, bytes);
        assert!(!row.kept[0].reason.is_empty());
        assert!(file.exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "manual allocated 40 GiB proof; requires HORIZON_EVICTION_PROOF_ROOT in checkout scratch"]
    fn allocated_forty_gib_cleanup_proof() {
        let path = PathBuf::from(
            std::env::var_os("HORIZON_EVICTION_PROOF_ROOT").expect("isolated proof root"),
        );
        fs::create_dir_all(&path).unwrap();
        let home = crate::storage::tests::Home(path.canonicalize().unwrap());
        for (folder, gib) in [
            ("rust/old", 12),
            ("cargo", 12),
            ("tmp", 4),
            ("native/patched/proof", 4),
            ("versions/native-leased", 4),
            ("versions/native-legacy", 4),
        ] {
            let file = home.seed(&format!("{folder}/payload"));
            assert!(std::process::Command::new("fallocate")
                .args(["-l", &format!("{gib}G")])
                .arg(file)
                .status()
                .unwrap()
                .success());
        }
        assert_eq!(backend::data_dir().canonicalize().unwrap(), home.0);
        let leased = home.0.join("versions/native-leased");
        let legacy = home.0.join("versions/native-legacy");
        for directory in [&leased, &legacy] {
            fs::write(directory.join(".installed"), b"").unwrap();
        }
        fs::write(
            leased.join(backend::CLIENT),
            horizon_mod_package::holder::CAPABILITY,
        )
        .unwrap();
        fs::write(legacy.join(backend::CLIENT), b"pre-holder game").unwrap();
        crate::run_leases::publish(&leased).unwrap();
        let registry = horizon_mod_package::runtime_leases::Registry::existing(&home.0).unwrap();
        let _game = registry
            .lock(
                &horizon_mod_package::runtime_leases::open_directory(&leased).unwrap(),
                horizon_mod_package::locks::LockMode::Shared,
            )
            .unwrap();
        let storage = Storage::at(&home.0).unwrap();
        assert!(matches!(Cache::acquire(&home.0), Err(Error::Busy)));
        let before = allocated_bytes(&home.0).unwrap();
        let now = std::time::SystemTime::now();
        let first = storage.evict_inventory(now).unwrap();
        let later = now + std::time::Duration::from_secs(8 * 86400);
        let second = storage.evict_inventory(later).unwrap();
        let freed: u64 = first
            .rows
            .iter()
            .chain(&second.rows)
            .map(|row| row.freed_bytes)
            .sum();
        let residual = allocated_bytes(&home.0).unwrap();
        println!("allocated_before={before} inventoried_before={} freed={freed} allocated_residual={residual}", first.total_bytes);
        assert!(before >= 40 * (1 << 30));
        assert!(freed >= 28 * (1 << 30));
        assert!(residual >= 12 * (1 << 30));
        assert!(residual < 25 * (1 << 30));
        assert!(leased.join("payload").exists());
        assert!(legacy.join("payload").exists());
        assert!(home.0.join("native/patched/proof/payload").exists());
        assert!(second.rows.iter().any(|row| row
            .kept
            .iter()
            .any(|kept| kept.bytes >= 4 * (1 << 30) && !kept.reason.is_empty())));
        assert!(matches!(Cache::acquire(&home.0), Err(Error::Busy)));
    }
}
