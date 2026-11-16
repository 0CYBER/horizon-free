//! Open resource layers, shared by asset packs and passive mod data.
//! Archive layers retain one reader and never materialize their entries.
//! Diverges from upstream: asset and passive jar-data layers share this source.
#[cfg(not(target_arch = "wasm32"))]
use crate::zip::Zip;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

/// File identity/change stamps for resource-cache keys, following symlinks.
pub(crate) fn metadata_fingerprint(metadata: &fs::Metadata) -> io::Result<u64> {
    let mut hash = DefaultHasher::new();
    metadata.len().hash(&mut hash);
    metadata.modified()?.hash(&mut hash);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        )
            .hash(&mut hash);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.creation_time().hash(&mut hash);
    }
    Ok(hash.finish())
}

fn copy_resource_bytes(
    bytes: &[u8],
    out: &mut impl io::Write,
    budget: &std::sync::atomic::AtomicU64,
) -> io::Result<()> {
    use std::sync::atomic::Ordering;
    let mut left = budget.load(Ordering::Relaxed);
    loop {
        let next = left.checked_sub(bytes.len() as u64).ok_or_else(|| {
            io::Error::new(io::ErrorKind::FileTooLarge, "pack exceeds unpack budget")
        })?;
        match budget.compare_exchange_weak(left, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(current) => left = current,
        }
    }
    out.write_all(bytes)
}

enum Storage {
    Directory(PathBuf),
    #[cfg(not(target_arch = "wasm32"))]
    Archive(Zip),
    Prefix {
        source: ResourceSource,
        prefix: String,
    },
    Index(BTreeMap<String, Resource>),
}

/// A winning resource keeps its source alive; merged resources keep only their
/// derived bytes. Reading an archive entry remains lazy after pack resolution.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Resource(ResourceStorage);
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ResourceStorage {
    File {
        source: ResourceSource,
        name: String,
    },
    Bytes(Arc<[u8]>),
}
impl Resource {
    pub fn file(source: ResourceSource, name: String) -> Self {
        Self(ResourceStorage::File { source, name })
    }
    pub fn bytes(bytes: Vec<u8>) -> Self {
        Self(ResourceStorage::Bytes(bytes.into()))
    }
    pub fn read(&self) -> io::Result<Vec<u8>> {
        match &self.0 {
            ResourceStorage::File { source, name } => source.read(name),
            ResourceStorage::Bytes(bytes) => Ok(bytes.to_vec()),
        }
    }
}

/// A folder or an open zip/jar, with the same relative-name lookup and listing.
/// Clones share the open archive; construct a new source for a resource reload.
/// Equality and hashing identify that generation, rather than comparing file contents.
#[derive(Clone)]
pub struct ResourceSource(Arc<Storage>);
impl PartialEq for ResourceSource {
    fn eq(&self, other: &Self) -> bool {
        self.same_generation(other)
    }
}
impl Eq for ResourceSource {}
impl Hash for ResourceSource {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}
impl std::fmt::Debug for ResourceSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.0.as_ref() {
            Storage::Directory(_) => "Directory",
            #[cfg(not(target_arch = "wasm32"))]
            Storage::Archive(_) => "Archive",
            Storage::Prefix { .. } => "Prefix",
            Storage::Index(_) => "Index",
        };
        f.debug_tuple("ResourceSource").field(&kind).finish()
    }
}

/// Prepared registry tables, resource reads and disposable loader caches have
/// separate locations. Archive resources never acquire a filesystem pathname.
#[derive(Clone, Debug)]
pub struct ResourceRoot(Arc<Root>);
#[derive(Clone, Debug)]
struct Root {
    prepared: PathBuf,
    cache: PathBuf,
    source: ResourceSource,
    data: ResourceSource,
    data_changes: ResourceSource,
    runtime_block_sprites: Vec<String>,
}
impl ResourceRoot {
    pub fn directory(path: PathBuf) -> Self {
        Self::new(path.clone(), path.clone(), ResourceSource::directory(path))
    }
    pub fn new(prepared: PathBuf, cache: PathBuf, source: ResourceSource) -> Self {
        let data = ResourceSource::directory(prepared.clone());
        Self(Arc::new(Root {
            prepared,
            cache,
            source,
            data,
            data_changes: ResourceSource::default(),
            runtime_block_sprites: Vec::new(),
        }))
    }
    /// Passive server data is independent of the client's selected asset packs.
    pub fn with_data(self, data: ResourceSource, data_changes: ResourceSource) -> Self {
        Self(Arc::new(Root {
            prepared: self.0.prepared.clone(),
            cache: self.0.cache.clone(),
            source: self.0.source.clone(),
            data,
            data_changes,
            runtime_block_sprites: self.0.runtime_block_sprites.clone(),
        }))
    }
    /// Add generated client textures as the highest asset layer, keeping the
    /// prepared tables and passive server data on their original sources.
    pub fn with_runtime_textures(
        mut self,
        textures: BTreeMap<String, Resource>,
        block_sprites: Vec<String>,
    ) -> Self {
        if textures.is_empty() {
            return self;
        }
        let root = Arc::make_mut(&mut self.0);
        let source = &root.source;
        let mut files: BTreeMap<_, _> = source
            .list("")
            .into_iter()
            .map(|name| (name.clone(), Resource::file(source.clone(), name)))
            .collect();
        files.extend(textures);
        root.source = ResourceSource::indexed(files);
        root.runtime_block_sprites = block_sprites;
        self
    }
    /// Runtime block sprites to stitch even when no baked model names them.
    pub fn runtime_block_sprites(&self) -> &[String] {
        &self.0.runtime_block_sprites
    }
    pub fn data_changes(&self) -> &ResourceSource {
        &self.0.data_changes
    }
    pub fn data(&self) -> &ResourceSource {
        &self.0.data
    }
    pub fn prepared(&self) -> &Path {
        &self.0.prepared
    }
    pub fn cache(&self) -> &Path {
        &self.0.cache
    }
    pub fn source(&self) -> &ResourceSource {
        &self.0.source
    }
}
impl Default for ResourceSource {
    fn default() -> Self {
        Self::indexed(BTreeMap::new())
    }
}

/// `name` from the highest-priority source that supplies it, read when the
/// whole resource fits `max` bytes. Sources are ordered lowest priority first,
/// as vanilla's `MultiPackResourceManager` lists a pack stack: a later source
/// overrides an earlier one. None when no source has the file, the winning
/// source cannot read it, or it is larger than `max` (a lower source's copy is
/// not substituted for an oversized winner).
pub fn read_winning_resource(sources: &[&ResourceSource], name: &str, max: u64) -> Option<Vec<u8>> {
    let winner = sources.iter().rev().find(|source| source.contains(name))?;
    winner.read_limited(name, max).ok()
}

/// A resource name cannot escape its layer. ZIP names already have separators
/// normalized by `Zip`; reject drive/stream names and NULs on every platform.
/// Whether a relative resource name cannot escape its source root.
pub fn stays_inside(name: &str) -> bool {
    !name.contains([':', '\0', '\\'])
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn prefix_inside(prefix: &str) -> bool {
    prefix.is_empty() || stays_inside(prefix.trim_end_matches('/'))
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>, active: &mut BTreeSet<PathBuf>) {
    let Ok(canonical) = fs::canonicalize(dir) else {
        return;
    };
    if !active.insert(canonical.clone()) {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        active.remove(&canonical);
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let directory = match entry.file_type() {
            Ok(kind) if !kind.is_symlink() => kind.is_dir(),
            _ => path.is_dir(),
        };
        if directory {
            walk(root, &path, out, active);
        } else if let Ok(relative) = path.strip_prefix(root) {
            let name = relative.to_string_lossy().replace('\\', "/");
            if stays_inside(&name) {
                out.push(name);
            }
        }
    }
    active.remove(&canonical);
}

impl ResourceSource {
    /// Observe whether any reader still owns this resource generation, without retaining it.
    pub fn liveness(&self) -> impl Fn() -> bool + Send + Sync + use<> {
        let weak = Arc::downgrade(&self.0);
        move || weak.strong_count() != 0
    }

    /// Whether both readers belong to the same opened resource generation.
    pub fn same_generation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Namespace folders beneath a resource top level, in name order.
    pub fn namespaces(&self, top: &str) -> Vec<String> {
        if !stays_inside(top) {
            return Vec::new();
        }
        if let Storage::Directory(root) = self.0.as_ref() {
            let mut names: Vec<_> = fs::read_dir(root.join(top))
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            return names;
        }
        fn collect<'a>(names: impl Iterator<Item = &'a str>, top: &str) -> Vec<String> {
            names
                .filter_map(|name| {
                    name.strip_prefix(top)?
                        .strip_prefix('/')?
                        .split_once('/')
                        .map(|(namespace, _)| namespace)
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(str::to_owned)
                .collect()
        }
        match self.0.as_ref() {
            #[cfg(not(target_arch = "wasm32"))]
            Storage::Archive(archive) => {
                collect(archive.names().filter(|name| stays_inside(name)), top)
            }
            Storage::Index(files) => collect(files.keys().map(String::as_str), top),
            _ => collect(
                self.list(&format!("{top}/")).iter().map(String::as_str),
                top,
            ),
        }
    }
    pub fn indexed(files: BTreeMap<String, Resource>) -> Self {
        Self(Arc::new(Storage::Index(files)))
    }
    pub fn directory(root: PathBuf) -> Self {
        Self(Arc::new(Storage::Directory(root)))
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn archive(path: &Path) -> io::Result<Self> {
        Ok(Self::opened_archive(Zip::open(path)?))
    }
    /// Share an archive that was already opened while discovering its metadata.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn opened_archive(archive: Zip) -> Self {
        Self(Arc::new(Storage::Archive(archive)))
    }
    /// Select an overlay root. Folder paths are trusted repository inputs;
    /// archive prefixes must remain inside that archive.
    pub fn subdirectory(&self, directory: &str) -> io::Result<Self> {
        if directory.is_empty() {
            return Ok(self.clone());
        }
        if let Storage::Directory(root) = self.0.as_ref() {
            return Ok(Self::directory(root.join(directory)));
        }
        if !stays_inside(directory) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe archive prefix",
            ));
        }
        Ok(Self(Arc::new(Storage::Prefix {
            source: self.clone(),
            prefix: format!("{directory}/"),
        })))
    }
    pub fn read(&self, name: &str) -> io::Result<Vec<u8>> {
        if !stays_inside(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe resource name",
            ));
        }
        match self.0.as_ref() {
            Storage::Directory(root) => horizon_core::prepared_files::read(&root.join(name)),
            #[cfg(not(target_arch = "wasm32"))]
            Storage::Archive(archive) => archive.read(name),
            Storage::Prefix { source, prefix } => source.read(&format!("{prefix}{name}")),
            Storage::Index(files) => files
                .get(name)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, name.to_owned()))?
                .read(),
        }
    }
    /// The folder that supplies `name`, following an index to the layer its
    /// winning file comes from. None for an archive, merged bytes, or no file.
    pub fn folder_of(&self, name: &str) -> Option<PathBuf> {
        if !stays_inside(name) {
            return None;
        }
        match self.0.as_ref() {
            Storage::Directory(root) => root.join(name).is_file().then(|| root.clone()),
            #[cfg(not(target_arch = "wasm32"))]
            Storage::Archive(_) => None,
            Storage::Prefix { source, prefix } => source.folder_of(&format!("{prefix}{name}")),
            Storage::Index(files) => match &files.get(name)?.0 {
                ResourceStorage::File { source, name } => source.folder_of(name),
                ResourceStorage::Bytes(_) => None,
            },
        }
    }
    /// Copy a resource under a shared output budget, streaming archive entries.
    pub fn copy_to(
        &self,
        name: &str,
        out: &mut impl io::Write,
        budget: &std::sync::atomic::AtomicU64,
    ) -> io::Result<()> {
        if !stays_inside(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe resource name",
            ));
        }
        match self.0.as_ref() {
            #[cfg(not(target_arch = "wasm32"))]
            Storage::Archive(archive) => archive.copy_to(name, out, budget).map(|_| ()),
            Storage::Prefix { source, prefix } => {
                source.copy_to(&format!("{prefix}{name}"), out, budget)
            }
            Storage::Index(files) => match &files
                .get(name)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, name.to_owned()))?
                .0
            {
                ResourceStorage::File { source, name } => source.copy_to(name, out, budget),
                ResourceStorage::Bytes(bytes) => copy_resource_bytes(bytes, out, budget),
            },
            Storage::Directory(root) => copy_resource_bytes(
                &horizon_core::prepared_files::read(&root.join(name))?,
                out,
                budget,
            ),
        }
    }
    pub fn read_to_string(&self, name: &str) -> io::Result<String> {
        String::from_utf8(self.read(name)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
    /// At most the first `max` bytes, for image headers and other bounded probes.
    pub fn read_head(&self, name: &str, max: u64) -> io::Result<Vec<u8>> {
        if !stays_inside(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe resource name",
            ));
        }
        match self.0.as_ref() {
            Storage::Directory(root) => {
                horizon_core::prepared_files::read_head(&root.join(name), max)
            }
            #[cfg(not(target_arch = "wasm32"))]
            Storage::Archive(archive) => archive.read_head(name, max),
            Storage::Prefix { source, prefix } => source.read_head(&format!("{prefix}{name}"), max),
            Storage::Index(files) => match &files
                .get(name)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, name.to_owned()))?
                .0
            {
                ResourceStorage::File { source, name } => source.read_head(name, max),
                ResourceStorage::Bytes(bytes) => Ok(bytes
                    [..bytes.len().min(usize::try_from(max).unwrap_or(usize::MAX))]
                    .to_vec()),
            },
        }
    }
    /// Whether a resource exists, without inflating an archive entry.
    pub fn contains(&self, name: &str) -> bool {
        if !stays_inside(name) {
            return false;
        }
        match self.0.as_ref() {
            Storage::Directory(root) => {
                let path = root.join(name);
                if horizon_core::prepared_files::find(&path).is_some() {
                    horizon_core::prepared_files::read_head(&path, 0).is_ok()
                } else {
                    path.is_file()
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Storage::Archive(archive) => archive.contains(name),
            Storage::Prefix { source, prefix } => source.contains(&format!("{prefix}{name}")),
            Storage::Index(files) => files.contains_key(name),
        }
    }
    /// Read only when the complete resource fits the limit. Bounds actual bytes
    /// read, including deflate output and folder files changed during the read.
    pub fn read_limited(&self, name: &str, max: u64) -> io::Result<Vec<u8>> {
        if !stays_inside(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe resource name",
            ));
        }
        let bytes = match self.0.as_ref() {
            Storage::Directory(root) => {
                horizon_core::prepared_files::read_head(&root.join(name), max.saturating_add(1))?
            }
            #[cfg(not(target_arch = "wasm32"))]
            Storage::Archive(archive) => return archive.read_limited(name, max),
            Storage::Prefix { source, prefix } => {
                return source.read_limited(&format!("{prefix}{name}"), max);
            }
            Storage::Index(files) => match &files
                .get(name)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, name.to_owned()))?
                .0
            {
                ResourceStorage::File { source, name } => return source.read_limited(name, max),
                ResourceStorage::Bytes(bytes) if bytes.len() as u64 <= max => bytes.to_vec(),
                ResourceStorage::Bytes(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "resource exceeds the size limit",
                    ));
                }
            },
        };
        if bytes.len() as u64 > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "resource exceeds the size limit",
            ));
        }
        Ok(bytes)
    }
    /// File names from the layer root, in name order. A directory prefix ends
    /// with `/`; partial file-name prefixes are supported for archives too.
    pub fn list(&self, prefix: &str) -> Vec<String> {
        if !prefix_inside(prefix) {
            return Vec::new();
        }
        match self.0.as_ref() {
            Storage::Directory(root) => {
                let mut out = Vec::new();
                let directory = if prefix.ends_with('/') || prefix.is_empty() {
                    prefix
                } else {
                    prefix.rsplit_once('/').map_or("", |(parent, _)| parent)
                };
                walk(root, &root.join(directory), &mut out, &mut BTreeSet::new());
                out.retain(|name| name.starts_with(prefix));
                out.sort();
                out
            }
            #[cfg(not(target_arch = "wasm32"))]
            Storage::Archive(archive) => archive
                .names()
                .filter(|name| name.starts_with(prefix) && stays_inside(name))
                .map(str::to_owned)
                .collect(),
            Storage::Prefix {
                source,
                prefix: base,
            } => source
                .list(&format!("{base}{prefix}"))
                .into_iter()
                .filter_map(|name| name.strip_prefix(base).map(str::to_owned))
                .collect(),
            Storage::Index(files) => files
                .range(prefix.to_owned()..)
                .map_while(|(name, _)| name.starts_with(prefix).then(|| name.clone()))
                .collect(),
        }
    }
    /// Namespace/kind listing. Folder layers visit only the requested kinds;
    /// archive layers filter their already indexed names without inflating files.
    pub fn list_under(&self, top: &str, kinds: &[&str]) -> Vec<String> {
        debug_assert!(kinds.is_sorted(), "kinds in walk order");
        if !stays_inside(top) || kinds.iter().any(|kind| !stays_inside(kind)) {
            return Vec::new();
        }
        match self.0.as_ref() {
            Storage::Directory(root) => {
                let Ok(entries) = fs::read_dir(root.join(top)) else {
                    return Vec::new();
                };
                let mut namespaces: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
                namespaces.sort();
                let mut out = Vec::new();
                for namespace in namespaces {
                    for kind in kinds {
                        walk(root, &namespace.join(kind), &mut out, &mut BTreeSet::new());
                    }
                }
                out.sort();
                out
            }
            _ => self
                .list(&format!("{top}/"))
                .into_iter()
                .filter(|name| {
                    name.strip_prefix(top)
                        .and_then(|rest| rest.strip_prefix('/'))
                        .and_then(|rest| rest.split_once('/'))
                        .is_some_and(|(_, rest)| {
                            kinds.iter().any(|kind| {
                                rest.strip_prefix(kind)
                                    .is_some_and(|tail| tail.starts_with('/'))
                            })
                        })
                })
                .collect(),
        }
    }

    /// Fingerprint selected resource kinds for a derived cache. Memoization is
    /// local to one operation: a later reload observes folder edits even when
    /// a source is shared, while archive identities follow their open readers.
    pub fn fingerprint_under(&self, top: &str, kinds: &[&str]) -> Option<u64> {
        if let Storage::Directory(root) = self.0.as_ref()
            && !root.join(top).is_dir()
        {
            return None;
        }
        let mut archives = HashMap::new();
        let mut hash = DefaultHasher::new();
        for name in self.list_under(top, kinds) {
            name.hash(&mut hash);
            self.file_fingerprint(&name, &mut archives)?.hash(&mut hash);
        }
        Some(hash.finish())
    }
    fn file_fingerprint(&self, name: &str, archives: &mut HashMap<usize, u64>) -> Option<u64> {
        match self.0.as_ref() {
            Storage::Directory(root) => {
                metadata_fingerprint(&fs::metadata(root.join(name)).ok()?).ok()
            }
            #[cfg(not(target_arch = "wasm32"))]
            Storage::Archive(archive) => {
                let key = Arc::as_ptr(&self.0) as usize;
                let fingerprint = match archives.get(&key) {
                    Some(fingerprint) => *fingerprint,
                    None => {
                        let fingerprint = archive.fingerprint().ok()?;
                        archives.insert(key, fingerprint);
                        fingerprint
                    }
                };
                let mut hash = DefaultHasher::new();
                fingerprint.hash(&mut hash);
                name.hash(&mut hash);
                Some(hash.finish())
            }
            Storage::Prefix { source, prefix } => {
                source.file_fingerprint(&format!("{prefix}{name}"), archives)
            }
            Storage::Index(files) => match &files.get(name)?.0 {
                ResourceStorage::File { source, name } => source.file_fingerprint(name, archives),
                ResourceStorage::Bytes(bytes) => {
                    let mut hash = DefaultHasher::new();
                    bytes.hash(&mut hash);
                    Some(hash.finish())
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Fixture(PathBuf);

    #[test]
    fn runtime_textures_join_the_asset_generation_without_changing_data() {
        let source = ResourceSource::indexed(BTreeMap::from([(
            "assets/demo/textures/block/original.png".into(),
            Resource::bytes(vec![1]),
        )]));
        let data = ResourceSource::default();
        let root = ResourceRoot::new("prepared".into(), "cache".into(), source.clone())
            .with_data(data.clone(), ResourceSource::default());
        let name = "assets/demo/textures/block/generated.png";
        let texture = |byte| BTreeMap::from([(name.into(), Resource::bytes(vec![byte]))]);
        let first = root
            .clone()
            .with_runtime_textures(texture(2), vec!["demo:block/generated".into()]);
        let second = root.with_runtime_textures(texture(3), vec!["demo:block/generated".into()]);
        assert_eq!(first.source().list("assets/").len(), 2);
        assert_eq!(first.source().read(name).unwrap(), [2]);
        assert!(!source.contains(name));
        assert!(first.data().same_generation(&data));
        assert_eq!(first.runtime_block_sprites(), ["demo:block/generated"]);
        assert_ne!(
            first.source().fingerprint_under("assets", &["textures"]),
            second.source().fingerprint_under("assets", &["textures"]),
        );
    }
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "horizon-resource-source-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn archive(&self, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
            let path = self.0.join(name);
            let mut writer = zip::ZipWriter::new(fs::File::create(&path).unwrap());
            for (name, bytes) in files {
                writer
                    .start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[cfg(unix)]
    #[test]
    fn generation_identity_is_shared_only_by_clones() {
        let first = ResourceSource::directory(PathBuf::from("same-prepared-root"));
        assert!(first.same_generation(&first.clone()));
        let reload = ResourceSource::directory(PathBuf::from("same-prepared-root"));
        assert!(!first.same_generation(&reload));
    }

    #[cfg(unix)]
    #[test]
    fn directory_aliases_are_listed_but_cycles_are_not_followed() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.0.join("assets/demo/textures")).unwrap();
        fs::write(fixture.0.join("assets/demo/textures/one.png"), b"one").unwrap();
        std::os::unix::fs::symlink(
            fixture.0.join("assets"),
            fixture.0.join("assets/demo/textures/loop"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            fixture.0.join("assets/demo"),
            fixture.0.join("assets/alias"),
        )
        .unwrap();
        let source = ResourceSource::directory(fixture.0.clone());
        assert_eq!(
            source.list("assets/"),
            [
                "assets/alias/textures/one.png",
                "assets/demo/textures/one.png"
            ]
        );
        let first = source.fingerprint_under("assets", &["textures"]).unwrap();
        fs::write(fixture.0.join("assets/demo/textures/one.png"), b"changed").unwrap();
        assert_ne!(
            source.fingerprint_under("assets", &["textures"]),
            Some(first)
        );
    }

    #[test]
    fn folder_and_archive_have_identical_lookup_and_listing() {
        let fixture = Fixture::new();
        let files: &[(&str, &[u8])] = &[
            ("assets/demo/a/file.json", b"nested"),
            ("assets/demo/a.json", b"flat"),
            ("data/demo/tags/item/test.json", b"tag"),
            ("data/demo/recipe/test.json", b"recipe"),
        ];
        let directory = fixture.0.join("folder");
        for (name, bytes) in files {
            let path = directory.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let folder = ResourceSource::directory(directory);
        let archive = ResourceSource::archive(&fixture.archive("pack.jar", files)).unwrap();
        for (name, bytes) in files {
            assert_eq!(folder.read(name).unwrap(), *bytes);
            assert_eq!(archive.read(name).unwrap(), *bytes);
            for source in [&folder, &archive] {
                assert!(source.contains(name));
                assert_eq!(
                    source.read_limited(name, bytes.len() as u64).unwrap(),
                    *bytes
                );
                assert_eq!(
                    source
                        .read_limited(name, bytes.len() as u64 - 1)
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::InvalidData
                );
            }
        }
        assert_eq!(folder.namespaces("assets"), ["demo"]);
        assert_eq!(archive.namespaces("assets"), ["demo"]);
        for source in [&folder, &archive] {
            assert!(!source.contains("missing"));
            assert!(!source.contains("../escape"));
            assert!(source.namespaces("../assets").is_empty());
        }
        for prefix in ["", "assets/", "assets/demo/a", "missing/"] {
            assert_eq!(folder.list(prefix), archive.list(prefix));
        }
        assert_eq!(
            folder.list_under("data", &["tags"]),
            archive.list_under("data", &["tags"])
        );
        assert_eq!(
            archive.list_under("data", &["tags"]),
            ["data/demo/tags/item/test.json"]
        );
        assert_eq!(
            archive.read("missing").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn archive_safety_is_enforced_without_extracting_files() {
        let fixture = Fixture::new();
        let path = fixture.archive(
            "pack.zip",
            &[
                ("assets/demo/good.txt", b"good"),
                ("assets\\demo\\normalized.txt", b"normalized"),
                ("../escape", b"bad"),
                ("assets/demo/../escape", b"bad"),
                ("assets//bad", b"bad"),
                ("C:/escape", b"bad"),
            ],
        );
        let source = ResourceSource::archive(&path).unwrap();
        assert_eq!(
            source.list(""),
            ["assets/demo/good.txt", "assets/demo/normalized.txt"]
        );
        assert_eq!(source.read("assets/demo/good.txt").unwrap(), b"good");
        assert_eq!(
            source.read("assets/demo/normalized.txt").unwrap(),
            b"normalized"
        );
        for name in [
            "../escape",
            "assets/demo/../escape",
            "/escape",
            "C:/escape",
            "assets//bad",
        ] {
            assert_eq!(
                source.read(name).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert!(!fixture.0.join("assets").exists());
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
    }

    #[test]
    fn bounded_indexed_and_scoped_reads_keep_source_limits() {
        let fixture = Fixture::new();
        let archive =
            ResourceSource::archive(&fixture.archive("pack.jar", &[("layer/file", b"abcd")]))
                .unwrap();
        let scoped = archive.subdirectory("layer").unwrap();
        let indexed = ResourceSource::indexed(BTreeMap::from([
            ("file".into(), Resource::file(scoped.clone(), "file".into())),
            ("derived".into(), Resource::bytes(b"abcd".to_vec())),
        ]));
        for (source, name) in [(&scoped, "file"), (&indexed, "file"), (&indexed, "derived")] {
            assert!(source.contains(name));
            assert_eq!(source.read_head(name, 2).unwrap(), b"ab");
            assert_eq!(source.read_head(name, 0).unwrap(), b"");
            assert_eq!(source.read_head(name, 8).unwrap(), b"abcd");
            assert_eq!(source.read_limited(name, 4).unwrap(), b"abcd");
            assert_eq!(
                source.read_limited(name, 3).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            assert_eq!(
                source.read_limited("missing", 4).unwrap_err().kind(),
                io::ErrorKind::NotFound
            );
        }
    }

    #[test]
    fn prepared_resource_presence_uses_the_same_bytes_as_reads_and_keeps_disk_sounds() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("horizon-resources.json"), b"{}").unwrap();
        let image = "assets/demo/textures/image.png";
        let sound = "assets/demo/sounds/sound.ogg";
        for (name, bytes) in [(image, b"image".as_slice()), (sound, b"sound")] {
            let path = fixture.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        horizon_core::prepared_files::write(&fixture.0).unwrap();
        fs::write(
            fixture.0.join("assets/demo/textures/added.png"),
            b"not prepared",
        )
        .unwrap();
        let source = ResourceSource::directory(fixture.0.clone());
        assert!(source.contains(image));
        assert!(source.contains(sound));
        assert_eq!(source.read_limited(image, 5).unwrap(), b"image");
        assert_eq!(source.read_limited(sound, 5).unwrap(), b"sound");
        assert!(!source.contains("assets/demo/textures/added.png"));
        assert!(!source.contains("assets/demo/textures"));
        assert!(
            source
                .list("assets/demo/sounds/")
                .contains(&sound.to_owned())
        );
    }

    #[test]
    fn winning_resource_takes_the_last_layer_with_a_bounded_read() {
        let base = ResourceSource::indexed(BTreeMap::from([
            (
                "data/demo/config.json".to_owned(),
                Resource::bytes(b"base".to_vec()),
            ),
            (
                "data/demo/only-base.json".to_owned(),
                Resource::bytes(b"base-only".to_vec()),
            ),
        ]));
        let pack = ResourceSource::indexed(BTreeMap::from([(
            "data/demo/config.json".to_owned(),
            Resource::bytes(b"pack".to_vec()),
        )]));
        let empty = ResourceSource::default();
        let sources = [&base, &empty, &pack];
        assert_eq!(
            read_winning_resource(&sources, "data/demo/config.json", 64),
            Some(b"pack".to_vec())
        );
        assert_eq!(
            read_winning_resource(&sources, "data/demo/only-base.json", 64),
            Some(b"base-only".to_vec())
        );
        assert_eq!(
            read_winning_resource(&sources, "data/demo/missing.json", 64),
            None
        );
        // The winning layer is too large: the lower layer's smaller copy is not used.
        assert_eq!(
            read_winning_resource(&sources, "data/demo/config.json", 3),
            None
        );
        assert_eq!(
            read_winning_resource(&sources, "../data/demo/config.json", 64),
            None
        );
    }

    #[test]
    fn reload_opens_new_bytes_and_existing_readers_keep_their_snapshot() {
        let fixture = Fixture::new();
        let path = fixture.archive("pack.jar", &[("assets/demo/value.txt", b"before")]);
        let before = ResourceSource::archive(&path).unwrap();
        let shared = before.clone();
        let replacement = fixture.archive(
            "new.jar",
            &[
                ("assets/demo/value.txt", b"after"),
                ("data/demo/new.json", b"new"),
            ],
        );
        fs::rename(&path, fixture.0.join("old.jar")).unwrap();
        fs::rename(replacement, &path).unwrap();
        let after = ResourceSource::archive(&path).unwrap();
        assert_eq!(before.read("assets/demo/value.txt").unwrap(), b"before");
        assert_eq!(shared.read("assets/demo/value.txt").unwrap(), b"before");
        assert_eq!(after.read("assets/demo/value.txt").unwrap(), b"after");
        assert!(before.list("data/").is_empty());
        assert_eq!(after.list("data/"), ["data/demo/new.json"]);
        assert!(!fixture.0.join("assets").exists());
    }
}
