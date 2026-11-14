//! Persistent, account-trusted Wasmtime artifacts. Only the game supplies this root.
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use wasmtime::{Engine, Module};

/// The budget for current-engine entries: 1 GiB holds 30 copies of the largest measured
/// `.cwasm` (34,039,320 bytes, from a 12.5 MB wasm module) plus headroom. 30 modules is
/// an allowance, not a measured pack; see #2896.
const LIMIT: u64 = 1024 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Compiled,
    Cached,
}

type ChildCommand = fn(&Path, &str) -> std::process::Command;
type ChildCompile = Option<(ChildCommand, std::time::Duration)>;

pub(crate) const CACHE_DIR: &str = ".horizon-cache/wasm";

pub(crate) struct ModuleCache {
    pub(super) root: PathBuf,
    pub(super) disabled: Option<String>,
    warned: AtomicBool,
    pub(crate) budget: crate::compile_budget::CompileBudget,
    #[cfg(test)]
    compiles: AtomicU64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompileTimeout(pub(crate) crate::compile_budget::CompileBudget);
impl std::fmt::Display for CompileTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "compilation time limit exceeded: {}", self.0)
    }
}
impl std::error::Error for CompileTimeout {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompileUnavailable {
    pub(crate) budget: crate::compile_budget::CompileBudget,
    pub(crate) why: String,
}
impl std::fmt::Display for CompileUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot enforce compilation budget {}: {}; restore a private writable .horizon-cache/wasm directory and a working compiler executable",
            self.budget, self.why
        )
    }
}
impl std::error::Error for CompileUnavailable {}

impl ModuleCache {
    /// Hits are mapped by `Module::deserialize_open_file`, as in `deserialize_file`.
    /// Unix checks folder ownership and write permissions without following a symlink.
    /// Each opened entry must be regular, owned by this user, not group/other writable,
    /// and have exactly one link. Windows caching is disabled until owner, DACL and
    /// inheritance trust can be established for folders and opened entries (#2901).
    pub(crate) fn new(root: PathBuf) -> Self {
        Self::new_for_platform(root, cfg!(windows))
    }

    fn new_for_platform(root: PathBuf, windows: bool) -> Self {
        Self::new_with_limit(root, windows, LIMIT)
    }

    pub(crate) fn new_with_limit(root: PathBuf, windows: bool, limit: u64) -> Self {
        Self::prepare(root, windows, Some(limit))
    }

    /// Child preparation must preserve live parent inputs and cache entries.
    pub(super) fn for_child(root: PathBuf) -> Self {
        Self::prepare(root, cfg!(windows), None)
    }

    fn prepare(root: PathBuf, windows: bool, limit: Option<u64>) -> Self {
        let disabled = if windows {
            Err(io::Error::other(
                "Windows owner/DACL/inheritance trust is not verified; compiling wasm",
            ))
        } else {
            prepare_folder(&root)
        }
        .err()
        .map(|why| format!("folder {}: {why}", root.display()));
        if let Some(limit) = limit.filter(|_| disabled.is_none()) {
            remove_stale_temporaries(&root);
            match crate::runtime::engine() {
                Ok(engine) => {
                    let current = Self::engine_part(&engine);
                    remove_obsolete_entries(&root, &current);
                    enforce_size_limit(&root, &current, limit);
                }
                Err(why) => log::debug!("wasm cache cleanup skipped: {why}"),
            }
        }
        if let Some(why) = &disabled {
            log::warn!("wasm module cache disabled: {why}");
        }
        Self {
            root,
            budget: crate::compile_budget::CompileBudget::default(),
            #[cfg(test)]
            compiles: AtomicU64::new(0),
            warned: AtomicBool::new(disabled.is_some()),
            disabled,
        }
    }

    /// Independent sha256 digests identify the engine and module inputs.
    pub(crate) fn key(engine: &Engine, bytes: &[u8]) -> String {
        format!("{}-{:x}", Self::engine_part(engine), Sha256::digest(bytes))
    }

    fn engine_part(engine: &Engine) -> String {
        let (setting, enabled) = crate::runtime::ENGINE_CONFIG;
        let mut digest = KeyDigest(Sha256::new());
        digest.0.update((setting.len() as u64).to_le_bytes());
        digest.0.update(setting.as_bytes());
        digest.0.update([u8::from(enabled)]);
        for (setting, enabled) in crate::runtime::THREADS {
            digest.0.update((setting.len() as u64).to_le_bytes());
            digest.0.update(setting.as_bytes());
            digest.0.update([u8::from(enabled)]);
        }
        // Runtime stack bounds need not affect Wasmtime's compiled-code hash,
        // but a policy change still starts a new Horizon cache generation.
        digest
            .0
            .update((horizon_guest_stack::GUEST_STACK_BYTES as u64).to_le_bytes());
        digest
            .0
            .update((horizon_guest_stack::HOST_HEADROOM as u64).to_le_bytes());
        engine.precompile_compatibility_hash().hash(&mut digest);
        format!("{:x}", digest.0.finalize())
    }

    pub(crate) fn load_or_compile(
        &self,
        engine: &Engine,
        bytes: &[u8],
    ) -> wasmtime::Result<(Module, Source)> {
        #[cfg(unix)]
        let child = if cfg!(test) {
            None
        } else {
            Some((
                production_command as fn(&Path, &str) -> std::process::Command,
                self.budget.limit,
            ))
        };
        #[cfg(not(unix))]
        let child = None;
        self.load_with_child(engine, bytes, child)
    }

    pub(crate) fn load_with_child(
        &self,
        engine: &Engine,
        bytes: &[u8],
        child: ChildCompile,
    ) -> wasmtime::Result<(Module, Source)> {
        self.load_with_child_command(engine, bytes, child)
    }

    pub(crate) fn load_with_child_command(
        &self,
        engine: &Engine,
        bytes: &[u8],
        child: Option<(
            impl FnOnce(&Path, &str) -> std::process::Command,
            std::time::Duration,
        )>,
    ) -> wasmtime::Result<(Module, Source)> {
        if let Some(why) = &self.disabled {
            #[cfg(unix)]
            if let Some((_, timeout)) = child.as_ref() {
                return Err(CompileUnavailable {
                    budget: crate::compile_budget::CompileBudget {
                        limit: *timeout,
                        ..self.budget
                    },
                    why: why.clone(),
                }
                .into());
            }
            // Only a bounded child compile reports why caching is off; Windows has none (#2901).
            #[cfg(not(unix))]
            let _ = why;
            return Module::new(engine, bytes).map(|module| (module, Source::Compiled));
        }
        let path = self.entry_path(&Self::key(engine, bytes));
        if let Some(module) = self.load_cached(engine, &path) {
            return Ok((module, Source::Cached));
        }
        // Windows caching is disabled before this point (#2901).
        #[cfg(not(unix))]
        let _ = child;
        #[cfg(unix)]
        if let Some((command, timeout)) = child {
            let result =
                crate::compile_child::run(self, bytes, &Self::key(engine, bytes), command, timeout);
            let why = match result {
                Ok(()) => {
                    if let Some(module) = self.load_cached(engine, &path) {
                        return Ok((module, Source::Cached));
                    }
                    "no valid cache entry".to_owned()
                }
                Err(why)
                    if why
                        .downcast_ref::<io::Error>()
                        .is_some_and(|e| e.kind() == io::ErrorKind::TimedOut) =>
                {
                    return Err(CompileTimeout(crate::compile_budget::CompileBudget {
                        limit: timeout,
                        ..self.budget
                    })
                    .into());
                }
                Err(why) => why.to_string(),
            };
            return Err(CompileUnavailable {
                budget: crate::compile_budget::CompileBudget {
                    limit: timeout,
                    ..self.budget
                },
                why,
            }
            .into());
        }
        #[cfg(test)]
        self.compiles.fetch_add(1, Ordering::Relaxed);
        let module = Module::new(engine, bytes)?;
        if let Err(why) = self.write(&path, &module) {
            log::debug!("wasm module cache write {}: {why}", path.display());
        }
        Ok((module, Source::Compiled))
    }

    pub(crate) fn entry_path(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.cwasm"))
    }

    fn load_cached(&self, engine: &Engine, path: &Path) -> Option<Module> {
        match trusted_entry(path) {
            Ok(file) => {
                // SAFETY: the account is the trust boundary. Unix folder and entry are owned
                // by this user and not group/other writable. We verify the opened handle,
                // not the path: a directory swap can only present another user's file
                // (refused), or this account's trusted file. Artifacts are produced by
                // Module::serialize for this engine; Wasmtime checks header/compatibility.
                // A mapped entry must remain unchanged for the Module lifetime. Writers
                // only temp-and-rename (including corrupt-entry recovery), giving a new
                // inode and preserving every live mapping; they never rewrite in place.
                // Unix also rejects hard links; Windows never reaches this path.
                match unsafe { Module::deserialize_open_file(engine, file) } {
                    Ok(module) => return Some(module),
                    Err(why) => self.warn_once(path, &why),
                }
            }
            Err(why) if why.kind() == io::ErrorKind::NotFound => {}
            Err(why) => self.warn_once(path, &why),
        }
        None
    }

    fn warn_once(&self, path: &Path, why: &dyn std::fmt::Display) {
        if !self.warned.swap(true, Ordering::Relaxed) {
            log::warn!(
                "wasm module cache refused {}: {why}; compiling wasm",
                path.display()
            );
        }
    }

    pub(super) fn write(&self, path: &Path, module: &Module) -> wasmtime::Result<()> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let bytes = module.serialize()?;
        let (temporary, mut file) = loop {
            let temporary = self.root.join(format!(
                ".{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&temporary) {
                Ok(file) => break (temporary, file),
                Err(why) if why.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(why) => return Err(why.into()),
            }
        };
        let result = (|| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(&bytes)?;
            #[cfg(test)]
            if std::env::var("HORIZON_COMPILER_MODE").as_deref() == Ok("midwrite") {
                assert!(!path.exists());
                // Let the parent kill a writer with an uncommitted artifact on disk.
                fs::write(self.root.parent().unwrap().join("write-blocked"), b"ready")?;
                loop {
                    std::thread::park();
                }
            }
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        Ok(())
    }
}

/// A `Hasher` that feeds every write into one sha256, so `Hash` impls can join the key.
struct KeyDigest(Sha256);
impl Hasher for KeyDigest {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
    /// Unused: the key reads the full digest. `Hash` impls only call `write`.
    fn finish(&self) -> u64 {
        0
    }
}

/// Temporary files a crashed write left behind. Only ones older than an hour are removed, so a
/// second game writing into the same folder right now keeps its file.
fn remove_stale_temporaries(root: &Path) {
    const STALE: std::time::Duration = std::time::Duration::from_secs(3600);
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with('.') && name.ends_with(".tmp")) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Names carry both digests so obsolete artifacts need never be opened.
fn remove_obsolete_entries(root: &Path, current_engine: &str) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|extension| extension != "cwasm")
        {
            continue;
        }
        let current = current_entry(&path, current_engine);
        if !current && fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
            let _ = fs::remove_file(path);
        }
    }
}

fn current_entry(path: &Path, current_engine: &str) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "cwasm")
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| {
                let Some((engine, module)) = stem.split_once('-') else {
                    return false;
                };
                let hex_digest = |part: &str| {
                    part.len() == 64 && part.bytes().all(|byte| byte.is_ascii_hexdigit())
                };
                hex_digest(engine) && hex_digest(module) && engine == current_engine
            })
}

/// Only runs in the constructor, before this launch can write any entries.
fn enforce_size_limit(root: &Path, current_engine: &str, limit: u64) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut total = 0u128;
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !current_entry(&path, current_engine) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        total += u128::from(metadata.len());
        if let Ok(modified) = metadata.modified() {
            candidates.push((modified, path, metadata.len()));
        }
    }
    if total <= u128::from(limit) {
        return;
    }
    candidates.sort_unstable();
    for (_, path, size) in candidates {
        if total <= u128::from(limit) {
            break;
        }
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file())
            && fs::remove_file(path).is_ok()
        {
            total -= u128::from(size);
        }
    }
}

fn prepare_folder(root: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    if let Some(parent) = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        builder.create(parent)?;
    }
    builder.recursive(false);
    let created = match builder.create(root) {
        Ok(()) => true,
        Err(why) if why.kind() == io::ErrorKind::AlreadyExists => false,
        Err(why) => return Err(why),
    };
    #[cfg(unix)]
    if created {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
        let folder = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::NOFOLLOW).bits() as i32,
            )
            .open(root)?;
        if folder.metadata()?.uid() != rustix::process::getuid().as_raw() {
            return Err(io::Error::other("new folder is not owned by this user"));
        }
        folder.set_permissions(fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = created;
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() {
        return Err(io::Error::other("not a directory"));
    }
    check_permissions(&metadata)?;
    #[cfg(windows)]
    {
        let canonical = root.canonicalize()?;
        let under_profile = ["LOCALAPPDATA", "USERPROFILE"]
            .iter()
            .filter_map(std::env::var_os)
            .filter_map(|path| PathBuf::from(path).canonicalize().ok())
            .any(|profile| canonical.starts_with(profile));
        if !under_profile {
            return Err(io::Error::other("outside the user's profile"));
        }
    }
    Ok(())
}

fn trusted_entry(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(
            (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT; FILE_GENERIC_READ | FILE_GENERIC_EXECUTE.
        options
            .custom_flags(0x00200000)
            .access_mode(0x00120089 | 0x001200a0);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("entry is not a regular file"));
    }
    check_permissions(&metadata)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(io::Error::other("entry has multiple hard links"));
        }
    }
    Ok(file)
}

fn check_permissions(metadata: &fs::Metadata) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != rustix::process::getuid().as_raw() || metadata.mode() & 0o022 != 0 {
            return Err(io::Error::other(
                "not owned by this user or writable by group/others",
            ));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("reparse point"));
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = metadata;
        return Err(io::Error::other(
            "ownership checks unavailable on this platform",
        ));
    }
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(all(test, not(windows)))]
#[path = "module_cache_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "module_cache_gate_tests.rs"]
mod gate_tests;

#[cfg(unix)]
pub(crate) fn production_command(input: &Path, key: &str) -> std::process::Command {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap_or_default());
    command
        .env_clear()
        .arg("--compile-wasm-module")
        .arg(input)
        .arg(key);
    command
}
