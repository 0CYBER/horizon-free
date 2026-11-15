//! Tool-written internal declarations and the old/new pin's signature diff.
use horizon_mod_package::{
    Manifest,
    internals::{self, ListedInternal},
    manifest::MANIFEST,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use toml_edit::{DocumentMut, Item, Value};

const USAGE: &str = "usage: cargo horizon-mod internals sync | add <path>… [--kind <kind>]";
const CATALOG: &str = "internals.json";

pub fn run(args: &[String]) -> Result<(), String> {
    let Some((command, args)) = args.split_first() else {
        return Err(USAGE.into());
    };
    if command == "sync" {
        if !args.is_empty() {
            return Err(USAGE.into());
        }
        return sync(Path::new("."));
    }
    let mut paths = Vec::new();
    let mut kind = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--kind" && kind.is_none() {
            kind = Some(
                args.next()
                    .filter(|k| !k.starts_with('-'))
                    .ok_or(USAGE)?
                    .as_str(),
            );
        } else if arg.starts_with('-') {
            return Err(USAGE.into());
        } else {
            paths.push(arg.as_str());
        }
    }
    if command != "add" || paths.is_empty() {
        return Err(USAGE.into());
    }
    add(Path::new("."), &paths, kind)
}

fn add(dir: &Path, paths: &[&str], kind: Option<&str>) -> Result<(), String> {
    let path = dir.join(MANIFEST);
    let text = crate::bounded::read(&path, crate::bounded::MAX_MANIFEST)?;
    let manifest = Manifest::parse(&text)?;
    let native = manifest
        .native
        .as_ref()
        .ok_or("internals add needs a [native] mod")?;
    let catalog = if native.module.is_empty() {
        load(dir, &native.horizon)?
    } else {
        internals::read(&catalog_directory(dir, &manifest)?.join(CATALOG))?
    };
    // Resolve every request before editing: any unavailable or ambiguous path leaves the file alone.
    let entries = paths
        .iter()
        .map(|path| {
            internals::resolve(&catalog, &manifest.id, path, kind).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    upsert(&mut doc, &manifest.internals, &entries)?;
    for entry in entries {
        println!("Added internal {}", entry.path);
    }
    fs::write(&path, doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))
}

/// Adds or refreshes `entries` in `internals`.
fn upsert(
    doc: &mut DocumentMut,
    declared: &[horizon_mod_package::manifest::Internal],
    entries: &[&ListedInternal],
) -> Result<(), String> {
    let mut listed: BTreeMap<_, _> = declared
        .iter()
        .enumerate()
        .map(|(index, entry)| ((entry.path.as_str(), entry.kind.as_deref()), index))
        .collect();
    for entry in entries {
        let key = (entry.path.as_str(), Some(entry.kind.as_str()));
        let existing = listed
            .get(&key)
            .or_else(|| listed.get(&(entry.path.as_str(), None)))
            .copied();
        if let Some(index) = existing {
            set_fingerprint(doc, index, &entry.hash, Some(&entry.kind))?;
            if listed.get(&(entry.path.as_str(), None)) == Some(&index) {
                listed.remove(&(entry.path.as_str(), None));
            }
            listed.insert(key, index);
        } else {
            let item = &mut doc["internals"];
            if item.is_none() {
                *item = toml_edit::value(toml_edit::Array::new());
            }
            // The manifest accepts both inline arrays and [[internals]] tables.
            if let Some(array) = item.as_array_mut() {
                let mut internal = toml_edit::InlineTable::new();
                internal.insert("path", entry.path.as_str().into());
                internal.insert("hash", entry.hash.as_str().into());
                internal.insert("kind", entry.kind.as_str().into());
                internal.fmt();
                array.push(internal);
            } else if let Some(array) = item.as_array_of_tables_mut() {
                let mut internal = toml_edit::Table::new();
                internal["path"] = toml_edit::value(&entry.path);
                internal["hash"] = toml_edit::value(&entry.hash);
                internal["kind"] = toml_edit::value(&entry.kind);
                array.push(internal);
            } else {
                return Err(format!("{MANIFEST}: internals must be an array"));
            }
            listed.insert(key, listed.len());
        }
    }
    Ok(())
}

/// Module discovery uses the launcher's source and toolchain management.
pub(crate) fn catalog_directory(dir: &Path, manifest: &Manifest) -> Result<PathBuf, String> {
    let native = manifest
        .native
        .as_ref()
        .ok_or("catalog needs a native mod")?;
    if native.module.is_empty() {
        return crate::sdk_fetch::build_data(dir, &native.horizon);
    }
    let launcher =
        std::env::var_os("HORIZON_LAUNCHER").unwrap_or_else(|| "horizon-launcher".into());
    let snapshots = dir.join("target/horizon-mod/catalog-snapshots");
    fs::create_dir_all(&snapshots).map_err(|e| e.to_string())?;
    let destination = snapshots
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
        ));
    let output = std::process::Command::new(launcher)
        .args(["native-build", "--catalog", "--trust-native-code", "--catalog-output"])
        .arg(&destination)
        .arg(dir.canonicalize().map_err(|e| e.to_string())?)
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| format!("Could not run horizon-launcher: {e}. Install it on PATH or set HORIZON_LAUNCHER to its executable."))?;
    if !output.status.success() {
        return Err("Module catalog discovery failed".into());
    }
    Ok(PathBuf::from(
        String::from_utf8(output.stdout)
            .map_err(|e| e.to_string())?
            .trim(),
    ))
}

/// The launcher owns source staging and compiler diagnostics; this tool owns manifest edits.
fn sync(dir: &Path) -> Result<(), String> {
    let path = dir.join(MANIFEST);
    let original = crate::bounded::read(&path, crate::bounded::MAX_MANIFEST)?;
    let manifest = Manifest::parse(&original)?;
    manifest
        .native
        .as_ref()
        .ok_or("internals sync needs a [native] mod")?;
    let launcher =
        std::env::var_os("HORIZON_LAUNCHER").unwrap_or_else(|| "horizon-launcher".into());
    let output = std::process::Command::new(launcher)
        .args(["native-build", "--sync-internals", "--trust-native-code"])
        .arg(dir.canonicalize().map_err(|e| e.to_string())?)
        .stderr(std::process::Stdio::inherit())
        .output().map_err(|e| format!("Could not run horizon-launcher: {e}. Install it on PATH or set HORIZON_LAUNCHER to its executable."))?;
    if !output.status.success() {
        return Err("Internals discovery failed; the manifest was not changed.".into());
    }
    let entries: Vec<ListedInternal> = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid internals discovery result: {e}"))?;
    let (candidate, report) = replace(&original, &entries)?;
    // The explicit check may take time. Do not overwrite edits made while it was running.
    if crate::bounded::read(&path, crate::bounded::MAX_MANIFEST)? != original {
        return Err("horizon-mod.toml changed during discovery; run cargo horizon-mod internals sync again.".into());
    }
    atomic_write(&path, candidate)?;
    print!("{report}");
    Ok(())
}

/// Manifest edits follow a user-managed symlink; slot publishing replaces its output link.
pub(crate) fn atomic_write_manifest(path: &Path, contents: String) -> Result<(), String> {
    let referent = fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    atomic_write(&referent, contents)
}

/// Attempts at an unused temporary name before `atomic_write` gives up.
const TEMPORARY_ATTEMPTS: u64 = 16;

/// Replaces `path` with `contents` through a temporary file in the same directory.
///
/// The temporary file is created with `create_new`, so a file or symlink planted at its name
/// makes that name be skipped instead of followed. The rename replaces `path` itself, even
/// when `path` is a symlink, and the existing file's permissions carry over.
pub(crate) fn atomic_write(path: &Path, contents: String) -> Result<(), String> {
    static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);
    let permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let first = NEXT_TEMPORARY.fetch_add(TEMPORARY_ATTEMPTS, Ordering::Relaxed);
    let (temporary, mut file) =
        create_temporary(path, first).map_err(|e| format!("{}: {e}", path.display()))?;
    let written = file
        .write_all(contents.as_bytes())
        .and_then(|()| match permissions {
            Some(permissions) => file.set_permissions(permissions),
            None => Ok(()),
        });
    drop(file);
    written
        .and_then(|()| fs::rename(&temporary, path))
        .map_err(|e| {
            let _ = fs::remove_file(&temporary);
            format!("{}: {e}", path.display())
        })
}

/// The `number`th candidate temporary name for `path`, in the same directory.
fn temporary_path(path: &Path, number: u64) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{}.{number}.tmp", std::process::id()))
}

/// Creates the first unused temporary file among `TEMPORARY_ATTEMPTS` names counted from `first`.
fn create_temporary(path: &Path, first: u64) -> std::io::Result<(PathBuf, fs::File)> {
    for number in first..first + TEMPORARY_ATTEMPTS {
        let temporary = temporary_path(path, number);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            created => return created.map(|file| (temporary, file)),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no unused temporary file name",
    ))
}

fn replace(text: &str, entries: &[ListedInternal]) -> Result<(String, String), String> {
    let manifest = Manifest::parse(text)?;
    manifest
        .native
        .as_ref()
        .ok_or("internals sync needs a [native] mod")?;
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{MANIFEST}: {e}"))?;
    // Each used access keeps at most one old row: an exact typed row first, then an untyped
    // row for its path. Every other row is dropped, so no stale duplicate survives the sync.
    let mut unclaimed: BTreeSet<_> = entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry.kind.as_str()))
        .collect();
    let mut keep = vec![false; manifest.internals.len()];
    for typed in [true, false] {
        for (index, old) in manifest.internals.iter().enumerate() {
            if old.kind.is_some() != typed {
                continue;
            }
            let claim = unclaimed
                .iter()
                .find(|(path, kind)| {
                    *path == old.path && old.kind.as_deref().is_none_or(|k| k == *kind)
                })
                .copied();
            if let Some(claim) = claim {
                unclaimed.remove(&claim);
                keep[index] = true;
            }
        }
    }
    let mut report = String::new();
    for (index, old) in manifest.internals.iter().enumerate().rev() {
        if !keep[index] {
            let item = &mut doc["internals"];
            if let Some(array) = item.as_array_mut() {
                // TOML stores a trailing comment after the previous comma as this
                // value's prefix. Keep it when dropping the declaration that follows it.
                let removed = array.remove(index);
                let prefix = removed
                    .decor()
                    .prefix()
                    .and_then(|raw| raw.as_str())
                    .unwrap_or("");
                if let Some(next) = array.get_mut(index) {
                    let next_prefix = next
                        .decor()
                        .prefix()
                        .and_then(|raw| raw.as_str())
                        .unwrap_or("");
                    let combined = format!("{prefix}{next_prefix}");
                    next.decor_mut().set_prefix(combined);
                } else {
                    let trailing = array.trailing().as_str().unwrap_or("");
                    array.set_trailing(format!("{prefix}{trailing}"));
                }
            } else if let Some(array) = item.as_array_of_tables_mut() {
                array.remove(index);
            }
            report.push_str(&format!("removed: {}\n", old.path));
        }
    }
    let retained = Manifest::parse(&doc.to_string())?;
    for (path, kind) in &unclaimed {
        report.push_str(&format!("added: {path} ({kind})\n"));
    }
    let entries: Vec<_> = entries.iter().collect();
    upsert(&mut doc, &retained.internals, &entries)?;
    Ok((doc.to_string(), report))
}

/// Report all declarations, leaving unavailable ones declared so a build still refuses them.
/// Only resolvable entries receive the new game's fingerprint.
pub fn refresh(
    manifest: &Manifest,
    doc: &mut DocumentMut,
    old: &[ListedInternal],
    new: &[ListedInternal],
) -> Result<String, String> {
    manifest
        .native
        .as_ref()
        .ok_or("update needs a [native] mod")?;
    let mut report = String::new();
    for (index, declared) in manifest.internals.iter().enumerate() {
        let path = &declared.path;
        let entry = match internals::resolve(new, &manifest.id, path, declared.kind.as_deref()) {
            Ok(entry) => entry,
            Err(error) => {
                report.push_str(&format!("missing: {path} ({error})\n"));
                continue;
            }
        };
        match internals::resolve(old, &manifest.id, path, declared.kind.as_deref()) {
            Ok(previous) if previous.hash == entry.hash => {
                report.push_str(&format!("unchanged: {path}\n"));
            }
            previous => {
                let signature =
                    previous.map_or("<unavailable in old game>", |e| e.signature.as_str());
                report.push_str(&format!(
                    "changed: {path}\n  {signature} → {}\n",
                    entry.signature
                ));
            }
        }
        // A declaration without a hash follows the pin, so it stays without one.
        if declared.hash.is_some() {
            set_fingerprint(doc, index, &entry.hash, declared.kind.as_deref())?;
        }
    }
    Ok(report)
}

fn set_fingerprint(
    doc: &mut DocumentMut,
    index: usize,
    hash: &str,
    kind: Option<&str>,
) -> Result<(), String> {
    let item = &mut doc["internals"];
    let value = if let Some(array) = item.as_array_mut() {
        array
            .get_mut(index)
            .and_then(Value::as_inline_table_mut)
            .and_then(|i| {
                if let Some(kind) = kind {
                    set_string(i.entry("kind").or_insert(kind.into()), kind);
                }
                i.get_mut("hash")
            })
    } else {
        item.as_array_of_tables_mut()
            .and_then(|array| array.get_mut(index))
            .and_then(|i| {
                if let Some(kind) = kind {
                    let value = i.entry("kind").or_insert(toml_edit::value(kind));
                    set_string(
                        value.as_value_mut().expect("manifest kind is a string"),
                        kind,
                    );
                }
                i.get_mut("hash")
            })
            .and_then(Item::as_value_mut)
    }
    .ok_or_else(|| format!("{MANIFEST}: internal {index} has no hash"))?;
    set_string(value, hash);
    Ok(())
}

/// Replacing only the value keeps whitespace and comments around tool-written strings.
pub fn set_string(value: &mut Value, text: &str) {
    if value.as_str() == Some(text) {
        return;
    }
    let decor = value.decor().clone();
    *value = Value::from(text);
    *value.decor_mut() = decor;
}

/// The internals of the game build `revision`, from its signed SDK's build data.
pub fn load(dir: &Path, revision: &str) -> Result<Vec<ListedInternal>, String> {
    let path = crate::sdk_fetch::build_data(dir, revision)?.join(CATALOG);
    let text = crate::bounded::read(&path, crate::bounded::MAX_CATALOG)?;
    internals::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the system temp dir, removed on drop (also when a test panics).
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let unique = format!(
                "horizon-mod-atomic-{name}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let dir = std::env::temp_dir().join(unique);
            // `create_dir` fails on an existing path, a symlink included.
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }
    }

    impl std::ops::Deref for Scratch {
        type Target = Path;

        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn atomic_write_replaces_contents_and_leaves_no_temporary_file() {
        let dir = Scratch::new("replace");
        let path = dir.join("horizon-mod.toml");
        atomic_write(&path, "first".into()).unwrap();
        atomic_write(&path, "second".into()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(fs::read_dir(&*dir).unwrap().count(), 1);
    }

    #[test]
    fn atomic_write_ignores_files_planted_at_the_old_temporary_name() {
        let dir = Scratch::new("planted");
        let path = dir.join("horizon-mod.toml");
        let planted = dir.join("horizon-mod.toml.sync");
        fs::write(&planted, "planted").unwrap();
        atomic_write(&path, "written".into()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "written");
        assert_eq!(fs::read_to_string(&planted).unwrap(), "planted");
    }

    #[test]
    fn create_temporary_skips_names_that_already_exist() {
        let dir = Scratch::new("skip");
        let path = dir.join("horizon-mod.toml");
        fs::write(temporary_path(&path, 7), "taken").unwrap();
        let (temporary, file) = create_temporary(&path, 7).unwrap();
        drop(file);
        assert_eq!(temporary, temporary_path(&path, 8));
        assert_eq!(
            fs::read_to_string(temporary_path(&path, 7)).unwrap(),
            "taken"
        );
    }

    #[test]
    fn create_temporary_gives_up_when_every_name_is_taken() {
        let dir = Scratch::new("exhausted");
        let path = dir.join("horizon-mod.toml");
        for number in 0..TEMPORARY_ATTEMPTS {
            fs::write(temporary_path(&path, number), "taken").unwrap();
        }
        let error = create_temporary(&path, 0).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn atomic_write_removes_its_temporary_file_when_the_rename_fails() {
        let dir = Scratch::new("rename-fails");
        let path = dir.join("horizon-mod.toml");
        // A non-empty directory at `path` cannot be replaced by a file.
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), "keep").unwrap();
        assert!(atomic_write(&path, "x".into()).is_err());
        assert!(fs::read_dir(&*dir).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
        assert_eq!(fs::read_to_string(path.join("keep")).unwrap(), "keep");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_does_not_follow_symlinks_at_temporary_names() {
        let dir = Scratch::new("symlink");
        let path = dir.join("horizon-mod.toml");
        let referent = dir.join("referent");
        let dangling = dir.join("dangling");
        fs::write(&referent, "keep").unwrap();
        // The old fixed name, and a name `create_temporary` tries first.
        std::os::unix::fs::symlink(&referent, dir.join("horizon-mod.toml.sync")).unwrap();
        std::os::unix::fs::symlink(&referent, temporary_path(&path, 3)).unwrap();
        std::os::unix::fs::symlink(&dangling, temporary_path(&path, 4)).unwrap();
        let (temporary, file) = create_temporary(&path, 3).unwrap();
        drop(file);
        assert_eq!(temporary, temporary_path(&path, 5));
        atomic_write(&path, "written".into()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "written");
        assert_eq!(fs::read_to_string(&referent).unwrap(), "keep");
        assert!(!dangling.exists());
    }

    #[test]
    fn sync_replaces_declarations_and_preserves_retained_formatting_for_both_array_forms() {
        let header = format!(
            "# Mod\nid='example'\nname='Example'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[native] # settings\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let (metadata, pins) = header.split_once("[native]").unwrap();
        let entries: Vec<ListedInternal> = serde_json::from_str(r#"[{"path":"Used","kind":"struct","signature":"struct Used","hash":"new"},{"path":"New","kind":"fn","signature":"fn new()","hash":"added"}]"#).unwrap();
        for body in [
            "internals = [\n  { path = 'Used', hash = 'old' }, # keep\n  { path = 'Unused', hash = 'gone' },\n]\n",
            "[[internals]] # keep\npath = 'Used'\nhash = 'old' # hash\n[[internals]]\npath = 'Unused'\nhash = 'gone'\n",
        ] {
            let (candidate, report) =
                replace(&format!("{metadata}{body}[native]{pins}"), &entries).unwrap();
            let manifest = Manifest::parse(&candidate).unwrap();
            assert_eq!(manifest.internals.len(), 2);
            assert_eq!(manifest.internals[0].hash.as_deref(), Some("new"));
            assert_eq!(manifest.internals[0].kind.as_deref(), Some("struct"));
            assert!(candidate.contains("# keep"));
            assert!(candidate.starts_with(metadata));
            assert!(candidate.contains(&format!("[native]{pins}")));
            assert!(report.contains("removed: Unused"));
            assert!(report.contains("added: New (fn)"));
            assert!(!report.contains("added: Used"));
            let (unchanged, report) = replace(&candidate, &entries).unwrap();
            assert_eq!(unchanged, candidate);
            assert!(report.is_empty());
            let (empty, report) = replace(&candidate, &[]).unwrap();
            assert!(Manifest::parse(&empty).unwrap().internals.is_empty());
            assert!(report.contains("removed: Used"));
        }
        // An untyped and a typed row for one access leave one current declaration.
        let body = "internals = [{ path = 'Used', hash = 'stale' }, { path = 'Used', kind = 'struct', hash = 'stale' }]\n";
        let (candidate, _) =
            replace(&format!("{metadata}{body}[native]{pins}"), &entries[..1]).unwrap();
        let manifest = Manifest::parse(&candidate).unwrap();
        assert_eq!(manifest.internals.len(), 1);
        assert_eq!(manifest.internals[0].hash.as_deref(), Some("new"));
        assert_eq!(manifest.internals[0].kind.as_deref(), Some("struct"));
    }

    #[test]
    fn tools_and_launcher_share_the_freshness_check() {
        let entries: Vec<ListedInternal> = horizon_mod_package::internals::parse(include_str!(
            "../tests/fixtures/internals-new.json"
        ))
        .unwrap();
        let mut declared = horizon_mod_package::manifest::Internal {
            path: "horizon_client::game::Game<'_>::reach".into(),
            kind: None,
            hash: Some("2222".into()),
        };
        assert_eq!(
            internals::check(&entries, "example", &declared)
                .unwrap_err()
                .to_string(),
            "E_INTERNAL_CHANGED: example uses horizon_client::game::Game<'_>::reach, whose signature is now impl Game<'_> { fn reach(&self, hand: Hand) -> f64 }; run cargo horizon-mod update to see what changed."
        );
        declared.hash = Some("abcd".into());
        internals::check(&entries, "example", &declared).unwrap();
    }
}
