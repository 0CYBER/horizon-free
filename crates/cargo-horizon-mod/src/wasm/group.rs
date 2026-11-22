//! Cargo owns the group's crate graph. Each mod unit emits both its exact rlib and PIC member.
use super::*;
use std::collections::BTreeMap;

struct Member {
    root: PathBuf,
    stage: PathBuf,
    manifest: toml_edit::DocumentMut,
    krate: String,
    /// The library's crate root, relative to the package.
    lib: PathBuf,
    package: String,
}

/// The wrapper config a member was last built with, beside it in the stage ([`track_wrapper`]).
const WRAPPER_RECORD: &str = ".horizon-wrapper.json";

/// Mirrors `source` into `destination` in place, so an unchanged file keeps its time and cargo
/// doesn't rebuild its unit. What `source` no longer has goes; `generated` names at this level are
/// the caller's to write and are neither copied nor removed.
fn copy(source: &Path, destination: &Path, generated: &[&str]) -> Result<(), String> {
    stage_dir(destination)?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if !["target", ".git", "mod.wasm"]
            .iter()
            .chain(generated)
            .any(|n| name == *n)
        {
            entries.push(entry);
        }
    }
    // By exact name and before anything is written: on a case-insensitive drive, a staged
    // `Logo.json` would take a renamed `logo.json`'s write and then be removed as obsolete.
    prune(destination, |name| {
        entries.iter().any(|e| e.file_name() == name) || generated.iter().any(|n| name == *n)
    })?;
    for entry in entries {
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let to = destination.join(entry.file_name());
        if ty.is_dir() {
            copy(&entry.path(), &to, &[])?;
        } else if ty.is_file() {
            copy_file(&entry.path(), &to)?;
        } else {
            return Err(format!(
                "{}: cannot stage a symlink",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

/// Removes what `destination` has that `keep` doesn't name.
fn prune(destination: &Path, keep: impl Fn(&std::ffi::OsStr) -> bool) -> Result<(), String> {
    for entry in std::fs::read_dir(destination).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !keep(&entry.file_name()) {
            remove(&entry.path())?;
        }
    }
    Ok(())
}

/// Removes `path` without following it: a symlink goes, not what it points at.
fn remove(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        // A Windows symlink to a directory is removed as a directory.
        std::fs::remove_file(path).or_else(|e| {
            if metadata.file_type().is_symlink() {
                std::fs::remove_dir(path)
            } else {
                Err(e)
            }
        })
    }
    .map_err(|e| e.to_string())
}

/// Makes `path` a real directory. A file or symlink there is removed first, so nothing is staged
/// through a link out of the stage.
fn stage_dir(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => {
            remove(path)?;
            std::fs::create_dir(path).map_err(|e| e.to_string())
        }
        Err(_) => std::fs::create_dir_all(path).map_err(|e| e.to_string()),
    }
}

/// Stages one file. A changed file is written anew, so it gets the current time: a copy would
/// keep the source's, which is older than the last build's output when the source was edited
/// during that build, and cargo would keep the old code. The staged file is writable even when the
/// source is read-only; on Unix it has the source's executable bits.
fn copy_file(source: &Path, destination: &Path) -> Result<(), String> {
    let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
    write_changed(destination, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let exec = std::fs::metadata(source)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o111;
        let mut permissions = std::fs::metadata(destination)
            .map_err(|e| e.to_string())?
            .permissions();
        if permissions.mode() & 0o111 != exec {
            permissions.set_mode(permissions.mode() & !0o111 | exec);
            std::fs::set_permissions(destination, permissions).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Writes `bytes` to `path` unless that file already holds them; answers whether it wrote.
/// Existing entries (a changed file, a directory, or a symlink) are removed before writing so
/// neither symlinks nor hard links are written through.
fn write_changed(path: &Path, bytes: &[u8]) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            if std::fs::read(path).map_err(|e| e.to_string())? == bytes {
                return Ok(false);
            }
            writable(path)?;
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        Ok(_) => remove(path)?,
        Err(_) => {}
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Makes a staged file writable: an earlier tool copied a read-only source's permissions.
fn writable(path: &Path) -> Result<(), String> {
    let mut permissions = std::fs::metadata(path)
        .map_err(|e| e.to_string())?
        .permissions();
    if !permissions.readonly() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(permissions.mode() | 0o200);
    }
    // Only the Windows attribute: there is no other writer bit to widen.
    #[cfg(not(unix))]
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions).map_err(|e| e.to_string())
}

/// Cargo doesn't see what the wrapper hands a member's rustc (the id and version in its
/// metadata): an edit of only the manifest's version changes none of the member's sources. The member's config is recorded beside it, and a changed one gives the crate
/// root a fresh time, so cargo rebuilds the member.
fn track_wrapper(stage: &Path, crate_root: &Path, config: &Value) -> Result<(), String> {
    let record = serde_json::to_vec(config).map_err(|e| e.to_string())?;
    let record_path = stage.join(WRAPPER_RECORD);
    let unchanged = std::fs::symlink_metadata(&record_path)
        .map(|m| m.is_file())
        .unwrap_or(false)
        && std::fs::read(&record_path)
            .map(|existing| existing == record)
            .unwrap_or(false);
    if !unchanged {
        writable(crate_root)?;
        std::fs::File::options()
            .write(true)
            .open(crate_root)
            .and_then(|file| file.set_modified(std::time::SystemTime::now()))
            .map_err(|e| format!("{}: {e}", crate_root.display()))?;
        write_changed(&record_path, &record)?;
    }
    Ok(())
}

/// What the wrapper passes `member`'s rustc ([`rustc`]); `crates` are the group's library crates.
fn wrapper_config(member: &Member, crates: &[String]) -> Value {
    serde_json::json!({
        "id": member.manifest["id"].as_str(),
        "version": member.manifest["version"].as_str(),
        "stage": member.stage,
        "provided": crates
            .iter()
            .filter(|c| **c != member.krate)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(","),
    })
}

/// The library target's crate root relative to its package directory `source`.
fn lib_src_path(package: &Value) -> Result<PathBuf, String> {
    let target = package["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| {
            t["kind"].as_array().is_some_and(|kinds| {
                kinds
                    .iter()
                    .any(|k| k == "lib" || k == "cdylib" || k == "rlib")
            })
        })
        .ok_or("mod has no library")?;
    Ok(PathBuf::from(
        target["src_path"]
            .as_str()
            .ok_or("library has no src_path")?,
    ))
}

// Preserve all Cargo dependency attributes and aliases; only relocate local paths.
fn paths(
    table: &mut dyn toml_edit::TableLike,
    root: &Path,
    staged: &BTreeMap<PathBuf, PathBuf>,
) -> Result<(), String> {
    for (_, item) in table.iter_mut() {
        if let Some(child) = item.as_table_like_mut() {
            if let Some(path) = child.get("path").and_then(|i| i.as_str()) {
                let original = dunce::canonicalize(root.join(path))
                    .map_err(|e| format!("{}: {e}", root.join(path).display()))?;
                let new = staged.get(&original).unwrap_or(&original);
                child.insert("path", toml_edit::value(new.to_string_lossy().as_ref()));
            }
            paths(child, root, staged)?;
        }
    }
    Ok(())
}

fn inherit_dependencies(
    table: &mut dyn toml_edit::TableLike,
    workspace: &toml_edit::DocumentMut,
    base: &Path,
) -> Result<(), String> {
    for (key, item) in table.iter_mut() {
        let Some(child) = item.as_table_like_mut() else {
            continue;
        };
        if ["dependencies", "build-dependencies", "dev-dependencies"].contains(&key.get()) {
            for (name, value) in child.iter_mut() {
                if value
                    .as_table_like()
                    .and_then(|v| v.get("workspace"))
                    .and_then(|v| v.as_bool())
                    != Some(true)
                {
                    continue;
                }
                let mut inherited = workspace
                    .get("workspace")
                    .and_then(|v| v.get("dependencies"))
                    .and_then(|v| v.get(name.get()))
                    .ok_or(format!("workspace dependency {name} missing"))?
                    .clone();
                if let Some(version) = inherited.as_str().map(str::to_owned) {
                    inherited = toml_edit::Item::Value(toml_edit::Value::InlineTable(
                        toml_edit::InlineTable::new(),
                    ));
                    inherited["version"] = toml_edit::value(version);
                }
                let dep = inherited
                    .as_table_like_mut()
                    .ok_or("workspace dependency is not a table")?;
                if let Some(path) = dep.get("path").and_then(|v| v.as_str()) {
                    dep.insert(
                        "path",
                        toml_edit::value(base.join(path).to_string_lossy().as_ref()),
                    );
                }
                for (key, setting) in value
                    .as_table_like()
                    .ok_or("inherited dependency is not a table")?
                    .iter()
                {
                    if key == "workspace" {
                        continue;
                    }
                    if key == "features" {
                        let mut features = dep
                            .get("features")
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default();
                        for feature in setting
                            .as_array()
                            .ok_or("dependency features are not an array")?
                            .iter()
                        {
                            features.push(feature.clone());
                        }
                        dep.insert(key, toml_edit::value(features));
                    } else {
                        dep.insert(key, setting.clone());
                    }
                }
                *value = inherited;
            }
        } else {
            inherit_dependencies(child, workspace, base)?;
        }
    }
    Ok(())
}

// Registry and git mod dependencies must point at the same staged units as path mods.
fn mod_dependencies(
    table: &mut dyn toml_edit::TableLike,
    dependencies: &BTreeMap<String, PathBuf>,
) {
    for (key, item) in table.iter_mut() {
        if ["dependencies", "build-dependencies", "dev-dependencies"].contains(&key.get()) {
            if let Some(deps) = item.as_table_like_mut() {
                for (name, dependency) in deps.iter_mut() {
                    if let Some(path) = dependencies.get(&name.get().replace('-', "_")) {
                        if let Some(version) = dependency.as_str().map(str::to_owned) {
                            *dependency = toml_edit::Item::Value(toml_edit::Value::InlineTable(
                                toml_edit::InlineTable::new(),
                            ));
                            dependency["version"] = toml_edit::value(version);
                        }
                        if let Some(dep) = dependency.as_table_like_mut() {
                            for source in
                                ["git", "branch", "rev", "tag", "registry", "registry-index"]
                            {
                                dep.remove(source);
                            }
                            dep.insert("path", toml_edit::value(path.to_string_lossy().as_ref()));
                        }
                    }
                }
            }
        } else if let Some(child) = item.as_table_like_mut() {
            mod_dependencies(child, dependencies);
        }
    }
}

pub(super) fn build(root: &Path, additional: &[PathBuf]) -> Result<(), String> {
    let root = dunce::canonicalize(root).map_err(|e| e.to_string())?;
    let toolchain = toolchain()?;
    let target = root.join("target/horizon-mod");
    let stage = target.join("group");
    let mut metadatas = Vec::new();
    for directory in std::iter::once(root.as_path()).chain(additional.iter().map(PathBuf::as_path))
    {
        let metadata: Value = serde_json::from_str(&output({
            let mut c = pinned("cargo");
            c.args(["metadata", "--format-version", "1"])
                .current_dir(directory);
            c
        })?)
        .map_err(|e| e.to_string())?;
        metadatas.push(metadata);
    }
    let metadata = &metadatas[0];
    let mut members = Vec::new();
    for package in metadatas
        .iter()
        .flat_map(|metadata| metadata["packages"].as_array().into_iter().flatten())
    {
        let source = Path::new(
            package["manifest_path"]
                .as_str()
                .ok_or("no manifest path")?,
        )
        .parent()
        .ok_or("no package directory")?;
        if members.iter().any(|m: &Member| m.root == source) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(source.join(MANIFEST)) else {
            continue;
        };
        let manifest: toml_edit::DocumentMut =
            text.parse().map_err(|e| format!("{MANIFEST}: {e}"))?;
        if manifest.contains_key("native") {
            if source == root
                || additional
                    .iter()
                    .any(|p| dunce::canonicalize(p).ok().as_deref() == Some(source))
            {
                return Err(format!("{}: native mod in wasm group", source.display()));
            }
            continue;
        }
        let id = manifest["id"].as_str().ok_or("mod has no id")?;
        members.push(Member {
            root: dunce::canonicalize(source).map_err(|e| e.to_string())?,
            stage: stage.join(id),
            krate: lib_name(package).ok_or("mod has no library")?,
            lib: lib_src_path(package)?,
            package: package["name"].as_str().ok_or("no package name")?.into(),
            manifest,
        });
    }
    let mut needed = std::collections::BTreeSet::new();
    for directory in std::iter::once(root.as_path()).chain(additional.iter().map(PathBuf::as_path))
    {
        let directory = dunce::canonicalize(directory).map_err(|e| e.to_string())?;
        let member = members
            .iter()
            .find(|m| m.root == directory)
            .ok_or("group root has no horizon-mod.toml")?;
        needed.insert(member.manifest["id"].as_str().ok_or("no id")?.to_owned());
    }
    loop {
        let before = needed.len();
        for member in &members {
            if !needed.contains(member.manifest["id"].as_str().ok_or("no id")?) {
                continue;
            }
            if let Some(deps) = member
                .manifest
                .get("dependencies")
                .and_then(|d| d.as_table_like())
            {
                for (id, dependency) in deps.iter() {
                    if members
                        .iter()
                        .any(|m| m.manifest["id"].as_str() == Some(id))
                    {
                        needed.insert(id.to_owned());
                    } else if dependency
                        .as_table_like()
                        .and_then(|d| d.get("optional"))
                        .and_then(|o| o.as_bool())
                        != Some(true)
                    {
                        return Err(format!(
                            "{}: declared mod dependency {id} is not a Cargo dependency",
                            member.package
                        ));
                    }
                }
            }
        }
        if before == needed.len() {
            break;
        }
    }
    members.retain(|m| {
        m.manifest["id"]
            .as_str()
            .is_some_and(|id| needed.contains(id))
    });
    let mut ids = std::collections::BTreeSet::new();
    for member in &mut members {
        if !ids.insert(member.manifest["id"].as_str().ok_or("no id")?) {
            return Err("duplicate mod id in Cargo group".into());
        }
        let canonical_lib = dunce::canonicalize(&member.lib).unwrap_or_else(|_| member.lib.clone());
        member.lib = canonical_lib
            .strip_prefix(&member.root)
            .map(Path::to_path_buf)
            .map_err(|_| format!("{}: library root outside its package", member.lib.display()))?;
    }
    let staged: BTreeMap<_, _> = members
        .iter()
        .map(|m| (m.root.clone(), m.stage.clone()))
        .collect();
    let crates: Vec<_> = members.iter().map(|m| m.krate.clone()).collect();
    // Kept between builds: a fresh copy would rebuild every member. What a member or the group
    // no longer has is removed below and in `copy`.
    stage_dir(&stage)?;
    prune(&stage, |name| {
        name == "Cargo.toml"
            || name == "Cargo.lock"
            || members.iter().any(|m| m.stage.file_name() == Some(name))
    })?;
    for member in &members {
        let mut manifest = member.manifest.clone();
        manifest["wasm"]["sha512"] = toml_edit::value("0".repeat(128));
        manifest["wasm"]["toolchain"] = toml_edit::value("pending");
        super::validate(&manifest)?;
    }
    let mut configs = serde_json::Map::new();
    let mut features = Vec::new();
    for member in &members {
        copy(&member.root, &member.stage, &["Cargo.toml", WRAPPER_RECORD])?;
        let path = member.stage.join("Cargo.toml");
        let mut cargo: toml_edit::DocumentMut =
            std::fs::read_to_string(member.root.join("Cargo.toml"))
                .map_err(|e| e.to_string())?
                .parse()
                .map_err(|e| format!("Cargo.toml: {e}"))?;
        // Expand workspace inheritance before moving this package into the group workspace.
        let member_metadata = metadatas
            .iter()
            .find(|metadata| {
                metadata["packages"].as_array().is_some_and(|packages| {
                    packages.iter().any(|p| {
                        p["manifest_path"]
                            .as_str()
                            .is_some_and(|p| Path::new(p).parent() == Some(member.root.as_path()))
                    })
                })
            })
            .ok_or("missing member metadata")?;
        let workspace = Path::new(
            member_metadata["workspace_root"]
                .as_str()
                .ok_or("no workspace root")?,
        );
        let ws: toml_edit::DocumentMut = std::fs::read_to_string(workspace.join("Cargo.toml"))
            .map_err(|e| e.to_string())?
            .parse()
            .map_err(|e| format!("workspace: {e}"))?;
        if let Some(package) = cargo.get_mut("package").and_then(|v| v.as_table_like_mut()) {
            for (key, value) in package.iter_mut() {
                if value
                    .as_table_like()
                    .and_then(|v| v.get("workspace"))
                    .and_then(|v| v.as_bool())
                    == Some(true)
                {
                    *value = ws["workspace"]["package"]
                        .get(key.get())
                        .ok_or(format!("workspace.package.{key} missing"))?
                        .clone();
                }
            }
            package.remove("workspace");
        }
        inherit_dependencies(cargo.as_table_mut(), &ws, workspace)?;
        if cargo
            .get("lints")
            .and_then(|v| v.get("workspace"))
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            cargo["lints"] = ws
                .get("workspace")
                .and_then(|v| v.get("lints"))
                .ok_or("workspace lints missing")?
                .clone();
        }
        let package = member_metadata["packages"]
            .as_array()
            .and_then(|packages| {
                packages.iter().find(|p| {
                    p["manifest_path"]
                        .as_str()
                        .is_some_and(|p| Path::new(p).parent() == Some(member.root.as_path()))
                })
            })
            .ok_or("missing package")?;
        let mut mod_paths = BTreeMap::new();
        if let Some(node) = member_metadata["resolve"]["nodes"]
            .as_array()
            .and_then(|nodes| nodes.iter().find(|n| n["id"] == package["id"]))
        {
            for dependency in node["deps"].as_array().into_iter().flatten() {
                if let Some(package) = member_metadata["packages"]
                    .as_array()
                    .and_then(|packages| packages.iter().find(|p| p["id"] == dependency["pkg"]))
                    && let Some(path) = package["manifest_path"]
                        .as_str()
                        .and_then(|p| Path::new(p).parent())
                        .and_then(|p| staged.get(p))
                {
                    mod_paths.insert(
                        dependency["name"]
                            .as_str()
                            .ok_or("dependency has no name")?
                            .to_owned(),
                        path.clone(),
                    );
                }
            }
        }
        mod_dependencies(cargo.as_table_mut(), &mod_paths);
        cargo.remove("workspace");
        paths(cargo.as_table_mut(), &member.root, &staged)?;
        if cargo.get("lib").and_then(|l| l.get("path")).is_some() {
            cargo["lib"]["path"] = toml_edit::value(member.lib.to_string_lossy().as_ref());
        }
        cargo["lib"]["crate-type"] =
            toml_edit::value(["rlib", "cdylib"].into_iter().collect::<toml_edit::Array>());
        write_changed(&path, cargo.to_string().as_bytes())?;
        let config = wrapper_config(member, &crates);
        track_wrapper(&member.stage, &member.stage.join(&member.lib), &config)?;
        configs.insert(member.package.clone(), config);
        if let Some(deps) = member
            .manifest
            .get("dependencies")
            .and_then(|v| v.as_table_like())
        {
            for (id, dep) in deps.iter() {
                if dep
                    .as_table_like()
                    .and_then(|d| d.get("optional"))
                    .and_then(|v| v.as_bool())
                    == Some(true)
                {
                    features.push(format!("{}/{id}", member.package));
                }
            }
        }
    }
    let lock = Path::new(
        metadata["workspace_root"]
            .as_str()
            .ok_or("no workspace root")?,
    )
    .join("Cargo.lock");
    if lock.is_file() {
        copy_file(&lock, &stage.join("Cargo.lock"))?;
    } else if std::fs::symlink_metadata(stage.join("Cargo.lock")).is_ok() {
        remove(&stage.join("Cargo.lock"))?;
    }
    let mut workspace = toml_edit::DocumentMut::new();
    for metadata in &metadatas {
        let directory = Path::new(
            metadata["workspace_root"]
                .as_str()
                .ok_or("no workspace root")?,
        );
        let original: toml_edit::DocumentMut =
            std::fs::read_to_string(directory.join("Cargo.toml"))
                .map_err(|e| e.to_string())?
                .parse()
                .map_err(|e| format!("workspace: {e}"))?;
        for key in ["patch", "replace"] {
            if let Some(item) = original.get(key) {
                let mut relocated = toml_edit::Table::new();
                relocated.insert(key, item.clone());
                paths(&mut relocated, directory, &staged)?;
                let item = relocated.remove(key).ok_or("missing relocated override")?;
                if workspace
                    .get(key)
                    .is_some_and(|previous| previous.to_string() != item.to_string())
                {
                    return Err(format!(
                        "group workspaces have incompatible [{key}] dependency overrides"
                    ));
                }
                workspace[key] = item;
            }
        }
    }
    workspace["workspace"]["members"] = toml_edit::value(
        members
            .iter()
            .map(|m| m.stage.to_string_lossy().into_owned())
            .collect::<toml_edit::Array>(),
    );
    workspace["workspace"]["resolver"] = toml_edit::value("3");
    write_changed(&stage.join("Cargo.toml"), workspace.to_string().as_bytes())?;
    let mut command = cargo_command("build", "member", &stage, &target, &toolchain)?;
    command.arg("--workspace").env(
        "HORIZON_GROUP_WRAPPER",
        serde_json::to_string(&configs).map_err(|e| e.to_string())?,
    );
    if !features.is_empty() {
        command.args(["--features", &features.join(",")]);
    }
    if !command.status().map_err(|e| e.to_string())?.success() {
        return Err("Cargo group build failed".into());
    }
    let artifacts = target.join(TARGET).join(PROFILE);
    let rlibs: BTreeMap<_, _> = members
        .iter()
        .map(|m| {
            let bytes = std::fs::read(artifacts.join(format!("lib{}.rlib", m.krate)))
                .map_err(|e| e.to_string())?;
            Ok((
                m.manifest["id"].as_str().ok_or("no mod id")?.to_owned(),
                sha512_of(&bytes),
            ))
        })
        .collect::<Result<_, String>>()?;
    let mut hashes = BTreeMap::new();
    for member in &members {
        let artifacts = target.join(TARGET).join(PROFILE);
        let mut module = std::fs::read(artifacts.join(format!("{}.wasm", member.krate)))
            .map_err(|e| e.to_string())?;
        let rlib = std::fs::read(artifacts.join(format!("lib{}.rlib", member.krate)))
            .map_err(|e| e.to_string())?;
        // Included in the module hash checked by the production loader's [wasm.linked].
        let identity = serde_json::to_vec(&serde_json::json!({"compiler":toolchain.version,"target":TARGET,"flags":FLAGS,"rlib":sha512_of(&rlib), "dependency_rlibs": member.manifest.get("dependencies").and_then(|v| v.as_table_like()).map(|deps| deps.iter().filter_map(|(id, _)| rlibs.get(id).map(|hash| (id, hash))).collect::<BTreeMap<_,_>>()).unwrap_or_default(), "profile":PROFILE, "member_visibility":"protected", "codegen_units":1, "panic":"abort"})).map_err(|e| e.to_string())?;
        let mut section = Vec::new();
        wasm_encoder::Encode::encode(
            &wasm_encoder::CustomSection {
                name: "horizon.build".into(),
                data: identity.into(),
            },
            &mut section,
        );
        module.push(0);
        module.extend(section);
        hashes.insert(
            member.manifest["id"]
                .as_str()
                .ok_or("no mod id")?
                .to_owned(),
            sha512_of(&module),
        );
        std::fs::write(member.root.join(WASM), module).map_err(|e| e.to_string())?;
    }
    for member in &mut members {
        let id = member.manifest["id"]
            .as_str()
            .ok_or("no mod id")?
            .to_owned();
        let dependencies: Vec<String> = member
            .manifest
            .get("dependencies")
            .and_then(|v| v.as_table_like())
            .map(|d| d.iter().map(|(id, _)| id.to_owned()).collect())
            .unwrap_or_default();
        let mut wasm = toml_edit::Table::new();
        wasm["sha512"] = toml_edit::value(&hashes[&id]);
        wasm["toolchain"] = toml_edit::value(&toolchain.version);
        for dep in dependencies {
            let Some(hash) = hashes.get(&dep) else {
                continue;
            };
            wasm["linked"][&dep] = toml_edit::value(hash);
        }
        member.manifest["wasm"] = toml_edit::Item::Table(wasm);
        crate::internals::atomic_write_manifest(
            &member.root.join(MANIFEST),
            member.manifest.to_string(),
        )?;
    }
    for member in &members {
        // A group member packs no uses table yet, so it counts as made for its build only.
        crate::pack_into(&member.root, &target, None)?;
    }
    Ok(())
}

/// Cargo still resolves and supplies every extern. This wrapper only scopes guest generation
/// and linker ownership to the package whose rustc unit is executing.
pub fn rustc(mut args: Vec<String>) -> Result<(), String> {
    let compiler = args.remove(0);
    let configs: Value =
        serde_json::from_str(&std::env::var("HORIZON_GROUP_WRAPPER").map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let package = std::env::var("CARGO_PKG_NAME").unwrap_or_default();
    let config = &configs[&package];
    let std_unit = std_unit(&args);
    if std_unit {
        let krate = args
            .windows(2)
            .find(|a| a[0] == "--crate-name")
            .map(|a| a[1].as_str());
        let extra = args
            .windows(2)
            .find(|a| a[0] == "-C" && a[1].starts_with("extra-filename="))
            .map(|a| a[1].trim_start_matches("extra-filename="))
            .unwrap_or("");
        if let (Some(krate), Some(dir)) = (krate, std::env::var_os("HORIZON_STD_ARTIFACTS")) {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join(format!("lib{krate}{extra}.rlib")), [])
                .map_err(|e| e.to_string())?;
        }
    }
    // Cargo keeps generated source under this target directory. Its location and the
    // invocation's cwd must not alter dependency SVHs in independently staged builds.
    let build_root = std::env::var("HORIZON_BUILD_ROOT").map_err(|e| e.to_string())?;
    args.push(format!("--remap-path-prefix={build_root}=/horizon/target"));
    args.push(format!(
        "--remap-path-prefix={}=/horizon/workspace",
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .display()
    ));
    if std_unit {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
        std_identity(&mut args, &package, &version);
    }
    let mut command = Command::new(compiler);
    command.env_remove(PROVIDED_ENV);
    if config.is_object() && args.iter().any(|a| a == "cdylib") {
        command.env("HORIZON_GROUP_PACKAGE", &package).env(
            PROVIDED_ENV,
            config["provided"].as_str().ok_or("no provided crates")?,
        );
        // Cargo's default disambiguator includes the staging package path. A mod's stable
        // identity must instead agree when its author and its dependents build separately.
        strip_metadata(&mut args);
        let features = features(&args);
        args.push(format!(
            "-Cmetadata={}@{}:{}",
            config["id"].as_str().ok_or("no id")?,
            config["version"].as_str().ok_or("no version")?,
            sha512_of(features.as_bytes())
        ));
        let stage = Path::new(config["stage"].as_str().ok_or("no stage")?);
        args.push(format!(
            "--remap-path-prefix={}=/horizon/group",
            stage.parent().ok_or("no workspace directory")?.display()
        ));
        args.push(format!(
            "--remap-path-prefix={}=/horizon/mod/{}",
            config["stage"].as_str().ok_or("no stage")?,
            config["id"].as_str().ok_or("no id")?
        ));
        // The group rejects symbol overrides: export this unit without making its own
        // vtables and statics preemptible through self-imported GOT entries.
        args.push("-Zdefault-visibility=protected".into());
    }
    let status = command.args(args).status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("rustc failed: {status}"))
    }
}

/// A unit of the standard library built from source for the wasm target: a toolchain `library/`
/// crate or one of its crates.io dependencies. Cargo passes `-Z force-unstable-if-unmarked` only
/// to the units `-Zbuild-std` adds, so an ordinary dependency that enables a `rustc-dep-of-std`
/// feature stays a mod's own crate. Host units (std's build scripts) don't count: their symbols
/// never reach a member.
fn std_unit(args: &[String]) -> bool {
    let marked = args
        .windows(2)
        .any(|a| a[0] == "-Z" && a[1] == "force-unstable-if-unmarked")
        || args.iter().any(|a| a == "-Zforce-unstable-if-unmarked");
    let wasm = args
        .windows(2)
        .any(|a| a[0] == "--target" && a[1] == super::TARGET)
        || args
            .iter()
            .any(|a| *a == format!("--target={}", super::TARGET));
    marked && wasm
}

/// The unit's Cargo features, sorted and comma-joined.
fn features(args: &[String]) -> String {
    let mut features: Vec<_> = args
        .windows(2)
        .filter(|a| a[0] == "--cfg" && a[1].starts_with("feature="))
        .map(|a| a[1].as_str())
        .collect();
    features.sort_unstable();
    features.join(",")
}

fn strip_metadata(args: &mut Vec<String>) {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "-C"
            && args
                .get(index + 1)
                .is_some_and(|a| a.starts_with("metadata="))
        {
            args.drain(index..index + 2);
        } else if args[index].starts_with("-Cmetadata=") {
            args.remove(index);
        } else {
            index += 1;
        }
    }
}

/// Cargo's disambiguator for a std crate hashes the host's `rustc -vV` (its `host:` line), so
/// std built on Windows and on Linux had different symbol names
/// and members built on one host could not link against the other's std (#18261). A std unit
/// gets one from the pinned toolchain, the package and its features instead.
fn std_identity(args: &mut Vec<String>, package: &str, version: &str) {
    let krate = args
        .windows(2)
        .find(|a| a[0] == "--crate-name")
        .map(|a| a[1].clone())
        .unwrap_or_default();
    let features = features(args);
    strip_metadata(args);
    args.push(format!(
        "-Cmetadata=horizon-std:{}:{package}@{version}:{krate}:{}",
        super::TOOLCHAIN,
        sha512_of(features.as_bytes())
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    fn set_time(path: &Path, time: std::time::SystemTime) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    fn modified(path: &Path) -> std::time::SystemTime {
        std::fs::metadata(path).unwrap().modified().unwrap()
    }

    fn set_readonly(path: &Path, readonly: bool) {
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(readonly);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    /// The second build of a member whose `src/lib.rs` was edited while the first ran: older
    /// than the first build's output, newer content.
    #[test]
    fn a_staged_member_is_synced_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let (source, stage) = (dir.path().join("mod"), dir.path().join("stage"));
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1 << 30);
        let built = old + std::time::Duration::from_secs(60);
        for (root, files) in [
            (
                &source,
                &[
                    ("Cargo.toml", "source"),
                    ("src/lib.rs", "new"),
                    ("src/kept.rs", "same"),
                    ("assets/a.json", "{}"),
                ][..],
            ),
            (
                &stage,
                &[
                    ("Cargo.toml", "generated"),
                    ("src/lib.rs", "old"),
                    ("src/kept.rs", "same"),
                    ("src/gone.rs", ""),
                    ("assets", ""),
                ][..],
            ),
        ] {
            for (path, text) in files {
                let path = root.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, text).unwrap();
                set_time(&path, if root == &source { old } else { built });
            }
        }

        copy(&source, &stage, &["Cargo.toml"]).unwrap();
        assert_eq!(
            std::fs::read_to_string(stage.join("src/lib.rs")).unwrap(),
            "new"
        );
        assert!(modified(&stage.join("src/lib.rs")) > built);
        assert_eq!(modified(&stage.join("src/kept.rs")), built);
        assert!(!stage.join("src/gone.rs").exists());
        assert_eq!(
            std::fs::read_to_string(stage.join("assets/a.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            std::fs::read_to_string(stage.join("Cargo.toml")).unwrap(),
            "generated"
        );
        assert_eq!(modified(&stage.join("Cargo.toml")), built);
    }

    #[test]
    fn a_read_only_source_is_staged_writable() {
        let dir = tempfile::tempdir().unwrap();
        let (source, staged) = (dir.path().join("lib.rs"), dir.path().join("staged.rs"));
        std::fs::write(&source, "first").unwrap();
        set_readonly(&source, true);
        copy_file(&source, &staged).unwrap();
        assert_eq!(std::fs::read_to_string(&staged).unwrap(), "first");
        assert!(!std::fs::metadata(&staged).unwrap().permissions().readonly());

        // A read-only staged file an earlier tool copied with the source's permissions.
        set_readonly(&staged, true);
        set_readonly(&source, false);
        std::fs::write(&source, "second").unwrap();
        copy_file(&source, &staged).unwrap();
        assert_eq!(std::fs::read_to_string(&staged).unwrap(), "second");
    }

    #[test]
    fn a_case_only_rename_is_staged_by_its_new_name() {
        let dir = tempfile::tempdir().unwrap();
        let (source, stage) = (dir.path().join("mod"), dir.path().join("stage"));
        for (root, name) in [(&source, "logo.json"), (&stage, "Logo.json")] {
            std::fs::create_dir_all(root).unwrap();
            std::fs::write(root.join(name), "{}").unwrap();
        }
        copy(&source, &stage, &[]).unwrap();
        let names: Vec<_> = std::fs::read_dir(&stage)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["logo.json"]);
        assert_eq!(
            std::fs::read_to_string(stage.join("logo.json")).unwrap(),
            "{}"
        );
    }

    /// Answers false where this account may not make symlinks (Windows without Developer Mode).
    fn symlink(target: &Path, link: &Path, dir: bool) -> bool {
        #[cfg(unix)]
        {
            let _ = dir;
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            if dir {
                std::os::windows::fs::symlink_dir(target, link)
            } else {
                std::os::windows::fs::symlink_file(target, link)
            }
            .is_ok()
        }
    }

    #[test]
    fn a_staged_symlink_is_replaced_not_written_through() {
        let dir = tempfile::tempdir().unwrap();
        let (source, stage, outside) = (
            dir.path().join("mod"),
            dir.path().join("stage"),
            dir.path().join("outside"),
        );
        std::fs::create_dir_all(source.join("assets")).unwrap();
        std::fs::write(source.join("lib.rs"), "mod").unwrap();
        std::fs::write(source.join("assets/a.json"), "{}").unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("victim"), "kept").unwrap();
        std::fs::create_dir_all(&stage).unwrap();
        for (name, target, is_dir) in [
            ("lib.rs", outside.join("victim"), false),
            ("assets", outside.clone(), true),
            ("Cargo.toml", outside.join("victim"), false),
        ] {
            if !symlink(&target, &stage.join(name), is_dir) {
                return;
            }
        }

        copy(&source, &stage, &["Cargo.toml"]).unwrap();
        write_changed(&stage.join("Cargo.toml"), b"generated").unwrap();
        assert_eq!(
            std::fs::read_to_string(outside.join("victim")).unwrap(),
            "kept"
        );
        assert!(!outside.join("a.json").exists());
        for name in ["lib.rs", "assets", "Cargo.toml"] {
            let metadata = std::fs::symlink_metadata(stage.join(name)).unwrap();
            assert!(!metadata.file_type().is_symlink(), "{name}");
        }
        assert_eq!(
            std::fs::read_to_string(stage.join("lib.rs")).unwrap(),
            "mod"
        );
        assert_eq!(
            std::fs::read_to_string(stage.join("assets/a.json")).unwrap(),
            "{}"
        );
    }

    /// A version edit changes only the metadata the wrapper hands rustc.
    #[test]
    fn a_changed_wrapper_config_rebuilds_the_member() {
        let dir = tempfile::tempdir().unwrap();
        let stage = dir.path();
        let crate_root = stage.join("lib.rs");
        std::fs::write(&crate_root, "mod").unwrap();
        let mut member = Member {
            root: stage.to_path_buf(),
            stage: stage.to_path_buf(),
            manifest: "id = 'm'\nversion = '1.0.0'\n".parse().unwrap(),
            krate: "m".into(),
            lib: "lib.rs".into(),
            package: "mod-m".into(),
        };
        let built = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1 << 30);
        let track = |member: &Member| {
            track_wrapper(stage, &crate_root, &wrapper_config(member, &["m".into()])).unwrap();
        };

        track(&member);
        set_time(&crate_root, built);
        track(&member);
        assert_eq!(modified(&crate_root), built);
        member.manifest["version"] = toml_edit::value("1.0.1");
        track(&member);
        assert!(modified(&crate_root) > built);
    }

    #[cfg(unix)]
    #[test]
    fn an_executable_bit_change_is_staged_with_unchanged_bytes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (source, staged) = (dir.path().join("run.sh"), dir.path().join("staged.sh"));
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o111;
        let set = |path: &Path, mode: u32| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        for path in [&source, &staged] {
            std::fs::write(path, "true").unwrap();
        }
        set(&source, 0o755);
        set(&staged, 0o644);
        copy_file(&source, &staged).unwrap();
        assert_eq!(mode(&staged), 0o111);
        set(&source, 0o644);
        copy_file(&source, &staged).unwrap();
        assert_eq!(mode(&staged), 0);
    }

    #[test]
    fn std_identity_ignores_the_host() {
        let unit = |metadata: &str, target: &str| {
            let mut args = strings(&[
                "--crate-name",
                "alloc",
                "--edition=2024",
                "--cfg",
                "feature=\"compiler-builtins-mem\"",
                "-C",
                metadata,
                "-C",
                "extra-filename=-0b1c",
                "--target",
                target,
            ]);
            std_identity(&mut args, "alloc", "0.0.0");
            args
        };
        let windows = unit("metadata=7d3e9a0c1b2f4e56", crate::wasm::TARGET);
        let linux = unit("metadata=13f0a9be65c2d871", crate::wasm::TARGET);
        assert_eq!(windows, linux);
        assert!(!windows.iter().any(|a| a.starts_with("metadata=")));
        let ours: Vec<_> = windows
            .iter()
            .filter(|a| a.starts_with("-Cmetadata=horizon-std:"))
            .collect();
        assert_eq!(ours.len(), 1);
        assert!(ours[0].contains(&format!("{}:alloc@0.0.0:alloc:", crate::wasm::TOOLCHAIN)));
        // Cargo's file names stay: the std-artifact markers and the link step key on them.
        assert!(windows.contains(&"extra-filename=-0b1c".to_owned()));

        let mut joined = strings(&["-Cmetadata=abc", "--target", crate::wasm::TARGET]);
        std_identity(&mut joined, "core", "0.0.0");
        assert_eq!(joined.len(), 3);
        assert!(joined[2].starts_with("-Cmetadata=horizon-std:"));
        let mut other = strings(&["-Cmetadata=abc", "--target", crate::wasm::TARGET]);
        other.extend(strings(&["--cfg", "feature=\"x\""]));
        std_identity(&mut other, "core", "0.0.0");
        assert_ne!(joined.last(), other.last(), "features keep units apart");
    }

    #[test]
    fn std_units_are_the_wasm_units_build_std_marks() {
        let wasm = crate::wasm::TARGET;
        let feature = "feature=\"rustc-dep-of-std\"";
        // What cargo passes a `-Zbuild-std` unit, a crates.io dependency of std included.
        assert!(std_unit(&strings(&[
            "--crate-name",
            "hashbrown",
            "--cfg",
            feature,
            "-Z",
            "force-unstable-if-unmarked",
            "--target",
            wasm,
        ])));
        assert!(std_unit(&strings(&[
            "-Zforce-unstable-if-unmarked",
            &format!("--target={wasm}"),
        ])));
        // A mod's ordinary dependency that enables `rustc-dep-of-std` keeps its own identity and
        // stays in the mod's link.
        assert!(!std_unit(&strings(&[
            "--crate-name",
            "hashbrown",
            "--cfg",
            feature,
            "--target",
            wasm,
        ])));
        // std's build scripts run on the host.
        assert!(!std_unit(&strings(&[
            "--crate-name",
            "build_script_build",
            "-Z",
            "force-unstable-if-unmarked",
        ])));
    }

    #[test]
    fn staged_paths_keep_aliases_features_and_target_dependencies() {
        let root = std::env::temp_dir().join(format!("horizon-group-paths-{}", std::process::id()));
        let library = root.join("library");
        let ordinary = root.join("ordinary");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::create_dir_all(&ordinary).unwrap();
        let source = r#"
[dependencies]
renamed = { package = "library-package", path = "library", features = ["special"], optional = true }
registry_mod = "^1.2"
git_mod = { package = "another-library", git = "https://invalid.test/library", rev = "abc", features = ["extra"] }
[target.'cfg(target_arch = "wasm32")'.build-dependencies]
ordinary = { path = "ordinary", default-features = false }
"#;
        let mut manifest: toml_edit::DocumentMut = source.parse().unwrap();
        let relocated = root.join("staged/library");
        let staged = BTreeMap::from([(dunce::canonicalize(&library).unwrap(), relocated.clone())]);
        paths(manifest.as_table_mut(), &root, &staged).unwrap();
        let dependency = &manifest["dependencies"]["renamed"];
        assert_eq!(dependency["path"].as_str(), relocated.to_str());
        assert_eq!(dependency["package"].as_str(), Some("library-package"));
        assert_eq!(
            dependency["features"]
                .as_array()
                .unwrap()
                .get(0)
                .unwrap()
                .as_str(),
            Some("special")
        );
        assert_eq!(dependency["optional"].as_bool(), Some(true));
        let ordinary =
            &manifest["target"]["cfg(target_arch = \"wasm32\")"]["build-dependencies"]["ordinary"];
        assert_eq!(ordinary["path"].as_str(), root.join("ordinary").to_str());
        assert_eq!(ordinary["default-features"].as_bool(), Some(false));
        mod_dependencies(
            manifest.as_table_mut(),
            &BTreeMap::from([
                ("registry_mod".into(), relocated.clone()),
                ("git_mod".into(), relocated.clone()),
            ]),
        );
        assert_eq!(
            manifest["dependencies"]["registry_mod"]["path"].as_str(),
            relocated.to_str()
        );
        assert_eq!(
            manifest["dependencies"]["registry_mod"]["version"].as_str(),
            Some("^1.2")
        );
        let git = &manifest["dependencies"]["git_mod"];
        assert!(git.get("git").is_none());
        assert!(git.get("rev").is_none());
        assert_eq!(git["package"].as_str(), Some("another-library"));
        assert_eq!(
            git["features"].as_array().unwrap().get(0).unwrap().as_str(),
            Some("extra")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
