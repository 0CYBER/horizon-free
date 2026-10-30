//! Resource packs carried intact into a private import/profile. Copying is not a rendering check.
use crate::{import, pipeline};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_FILES: usize = 65_536;
const MAX_BYTES: u64 = 512 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Available,
    Missing,
    Failed,
    Conflict,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub state: State,
    pub detail: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub files: BTreeMap<String, Outcome>,
}

impl Report {
    pub fn load(work: &Path) -> Self {
        pipeline::read_json(&work.join("resourcepacks.json")).unwrap_or_default()
    }
    pub fn save(&self, work: &Path) -> Result<(), String> {
        pipeline::write_json(&work.join("resourcepacks.json"), self)
    }
    pub fn set(&mut self, path: &str, state: State, detail: impl Into<String>) {
        self.files.insert(
            path.into(),
            Outcome {
                state,
                detail: detail.into(),
            },
        );
    }
}

// Cross-platform relative names: archive paths must mean the same thing on Windows and Unix.
fn check_path(name: &str) -> Result<(), String> {
    if name.len() > 4096 || name.split('/').count() > 64 {
        return Err("resource-pack path is too deep or long".into());
    }
    for part in name.split('/') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if part.is_empty()
            || matches!(part, "." | "..")
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err(format!("unsafe resource-pack path: {name:?}"));
        }
    }
    Ok(())
}

fn regular(path: &Path) -> Result<fs::Metadata, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() && !meta.is_dir() {
        return Err(format!(
            "{}: expected a regular file or directory, not a link",
            path.display()
        ));
    }
    Ok(meta)
}

/// Creates only ordinary directories, refusing links in an existing destination's ancestry.
pub(crate) fn directory(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        directory(parent)?;
    }
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && !m.is_symlink() => Ok(()),
        Ok(_) => Err(format!("{}: not an ordinary directory", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|e| format!("{}: {e}", path.display()))
        }
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

type Files = BTreeMap<String, Vec<u8>>;

fn add(files: &mut Files, name: &str, reader: impl Read, total: &mut u64) -> Result<(), String> {
    check_path(name)?;
    if files.contains_key(name) {
        return Err(format!("duplicate resource-pack entry: {name}"));
    }
    if files.len() >= MAX_FILES {
        return Err(format!("more than {MAX_FILES} resource-pack files"));
    }
    let bytes = read_limited(reader, MAX_BYTES.saturating_sub(*total))?;
    *total += bytes.len() as u64;
    files.insert(name.into(), bytes);
    Ok(())
}

fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("resource packs exceed 512 MiB".into());
    }
    Ok(bytes)
}

fn validate(files: &Files) -> Result<(), String> {
    let mut names = BTreeMap::<String, String>::new();
    for path in files.keys() {
        let mut prefix = String::new();
        for part in path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            let key = prefix.to_lowercase();
            if let Some(original) = names.get(&key) {
                if original != &prefix {
                    return Err(format!(
                        "case-conflicting resource-pack paths: {original} and {prefix}"
                    ));
                }
            } else {
                names.insert(key, prefix.clone());
            }
            if prefix != *path && files.contains_key(&prefix) {
                return Err(format!("file is also a resource-pack directory: {prefix}"));
            }
        }
    }
    Ok(())
}

fn walk(root: &Path, relative: &str, files: &mut Files, total: &mut u64) -> Result<(), String> {
    let path = root.join(relative);
    let meta = regular(&path)?;
    if !relative.is_empty() {
        check_path(relative)?;
    }
    if meta.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(&path)
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "non-UTF-8 resource-pack path")?;
            let rel = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            walk(root, &rel, files, total)?;
        }
    } else {
        if meta.len() > MAX_BYTES.saturating_sub(*total) {
            return Err("resource packs exceed 512 MiB".into());
        }
        add(
            files,
            relative,
            fs::File::open(&path).map_err(|e| e.to_string())?,
            total,
        )?;
    }
    Ok(())
}

// Each top-level ZIP or directory with pack.mcmeta is one selectable resource pack.
fn groups(files: Files) -> Result<BTreeMap<String, Files>, String> {
    let mut groups: BTreeMap<String, Files> = BTreeMap::new();
    for (path, bytes) in files {
        let top = path.split('/').next().unwrap_or_default().to_owned();
        groups.entry(top).or_default().insert(path, bytes);
    }
    for (name, files) in &groups {
        if !(files.contains_key(&format!("{name}/pack.mcmeta"))
            || (name.ends_with(".zip") && files.contains_key(name)))
        {
            return Err(format!(
                "{name}: expected a ZIP pack or directory containing pack.mcmeta"
            ));
        }
    }
    Ok(groups)
}

fn publish(root: &Path, name: &str, files: &Files) -> Result<Outcome, String> {
    directory(root)?;
    let target = root.join(name);
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            let mut old = Files::new();
            walk(root, name, &mut old, &mut 0)?;
            return Ok(Outcome {
                state: if &old == files {
                    State::Available
                } else {
                    State::Conflict
                },
                detail: if &old == files {
                    "already staged"
                } else {
                    "existing pack preserved; different incoming content"
                }
                .into(),
            });
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let is_directory = !files.contains_key(name);
    if is_directory {
        fs::create_dir(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    for (relative, bytes) in files {
        let dest = root.join(relative);
        directory(dest.parent().ok_or("no resource-pack parent")?)?;
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
        {
            Ok(file) => file,
            Err(e) => {
                if is_directory {
                    let _ = fs::remove_dir_all(&target);
                }
                return Err(format!("{}: {e}", dest.display()));
            }
        };
        let result = file.write_all(bytes);
        if let Err(e) = result {
            // Only this newly created pack is ours; existing packs never enter this branch.
            if target.is_dir() {
                let _ = fs::remove_dir_all(&target);
            } else {
                let _ = fs::remove_file(&target);
            }
            return Err(format!("{}: {e}", dest.display()));
        }
    }
    Ok(Outcome {
        state: State::Available,
        detail: "staged; enable in Resource Packs (rendering not checked)".into(),
    })
}

/// Stage embedded packs while the archive/instance is still available. Client overrides win.
pub fn stage(source: &Path, work: &Path, pack: &import::Pack) -> Result<Report, String> {
    let mut files = Files::new();
    let mut total = 0;
    let mut report = Report::load(work);
    if source.is_dir() {
        regular(source)?;
        let game = import::game_dir(source)?;
        regular(&game)?;
        let root = game.join("resourcepacks");
        if fs::symlink_metadata(&root).is_ok() {
            walk(&root, "", &mut files, &mut total)?;
        }
        if game.join("shaderpacks").exists() {
            report.set(
                "shaderpacks/",
                State::Unsupported,
                "shader-pack conversion is pending",
            );
        }
    } else if pack.format == "mrpack" || pack.format == "curseforge" {
        regular(source)?;
        let mut zip = zip::ZipArchive::new(fs::File::open(source).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let roots = if pack.format == "mrpack" {
            vec!["overrides".to_owned(), "client-overrides".to_owned()]
        } else {
            let manifest: serde_json::Value =
                serde_json::from_reader(zip.by_name("manifest.json").map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let root = manifest["overrides"]
                .as_str()
                .unwrap_or("overrides")
                .to_owned();
            check_path(&root)?;
            vec![root]
        };
        for root in roots {
            let prefix = format!("{root}/resourcepacks/");
            let mut layer = Files::new();
            for i in 0..zip.len() {
                let entry = zip.by_index(i).map_err(|e| e.to_string())?;
                let raw = entry.name().to_owned();
                if raw.starts_with(&format!("{root}/shaderpacks/")) {
                    report.set(
                        "shaderpacks/",
                        State::Unsupported,
                        "shader-pack conversion is pending",
                    );
                }
                let normalized = raw.replace('\\', "/");
                let Some(name) = normalized.strip_prefix(&prefix) else {
                    continue;
                };
                check_path(raw.trim_end_matches('/'))?;
                if entry
                    .unix_mode()
                    .is_some_and(|m| !matches!(m & 0o170000, 0 | 0o100000 | 0o040000))
                {
                    return Err(format!("{raw}: resource-pack symlink refused"));
                }
                if entry.is_dir() {
                    continue;
                }
                if entry.size() > MAX_BYTES.saturating_sub(total) {
                    return Err("resource packs exceed 512 MiB".into());
                }
                add(&mut layer, name, entry, &mut total)?;
            }
            // Only an exact same path from a higher-priority root may replace an entry.
            for (name, bytes) in layer {
                files.insert(name, bytes);
            }
        }
    }
    // Catch file/directory and case collisions across override layers before publishing anything.
    validate(&files)?;
    for (name, files) in groups(files)? {
        report.files.insert(
            format!("resourcepacks/{name}"),
            publish(&work.join("resourcepacks"), &name, &files)?,
        );
    }
    for file in &pack.files {
        match file.kind() {
            "resourcepacks" if file.client == "unsupported" => report.set(
                &file.path,
                State::Unsupported,
                "not supported on the client",
            ),
            "resourcepacks" => {
                report.files.entry(file.path.clone()).or_insert(Outcome {
                    state: State::Missing,
                    detail: "not obtained yet; run port".into(),
                });
            }
            "shaderpacks" => report.set(
                &file.path,
                State::Unsupported,
                "shader-pack conversion is pending",
            ),
            _ => {}
        }
    }
    Ok(report)
}

/// Ordinary staged packs, also supporting work directories created before resourcepacks.json existed.
pub fn available(work: &Path) -> Vec<PathBuf> {
    let root = work.join("resourcepacks");
    if regular(&root).is_err() {
        return Vec::new();
    }
    let mut paths: Vec<_> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            regular(p).is_ok_and(|m| {
                (m.is_file() && p.extension().is_some_and(|e| e == "zip"))
                    || (m.is_dir() && regular(&p.join("pack.mcmeta")).is_ok_and(|m| m.is_file()))
            })
        })
        .collect();
    paths.sort();
    paths
}

/// Copy missing whole packs. Never merge into or overwrite an existing profile pack.
pub fn install(work: &Path, home: &Path) -> Result<Report, String> {
    let mut report = Report::default();
    let mut files = Files::new();
    let root = work.join("resourcepacks");
    if !available(work).is_empty() {
        walk(&root, "", &mut files, &mut 0)?;
        validate(&files)?;
        for (name, files) in groups(files)? {
            report.files.insert(
                format!("resourcepacks/{name}"),
                publish(&home.join("resourcepacks"), &name, &files)?,
            );
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
