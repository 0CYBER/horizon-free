//! Cargo owns the group's crate graph. Each mod unit emits both its exact rlib and PIC member.
use super::*;
use std::collections::BTreeMap;

struct Member {
    root: PathBuf,
    stage: PathBuf,
    manifest: toml_edit::DocumentMut,
    krate: String,
    package: String,
    registrations: String,
    points: String,
}

fn copy(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if ["target", ".git", "mod.wasm"].iter().any(|n| name == *n) {
            continue;
        }
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        if ty.is_dir() {
            copy(&entry.path(), &destination.join(name))?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), destination.join(name)).map_err(|e| e.to_string())?;
        } else {
            return Err(format!(
                "{}: cannot stage a symlink",
                entry.path().display()
            ));
        }
    }
    Ok(())
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

pub(super) fn build(
    root: &Path,
    catalog: Option<&Path>,
    additional: &[PathBuf],
) -> Result<(), String> {
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
            package: package["name"].as_str().ok_or("no package name")?.into(),
            manifest,
            registrations: String::new(),
            points: String::new(),
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
    }
    let staged: BTreeMap<_, _> = members
        .iter()
        .map(|m| (m.root.clone(), m.stage.clone()))
        .collect();
    let crates: Vec<_> = members.iter().map(|m| m.krate.clone()).collect();
    if stage.exists() {
        std::fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    }
    for member in &members {
        std::fs::create_dir_all(&member.stage).map_err(|e| e.to_string())?;
    }
    for member in &mut members {
        let mut manifest = member.manifest.clone();
        manifest["wasm"]["sha512"] = toml_edit::value("0".repeat(128));
        manifest["wasm"]["toolchain"] = toml_edit::value("pending");
        let (registrations, points) = super::registrations(&manifest, catalog)?;
        member.registrations = registrations;
        member.points = points;
    }
    let mut configs = serde_json::Map::new();
    let mut features = Vec::new();
    let mut point_names = std::collections::BTreeSet::<String>::new();
    for member in &members {
        copy(&member.root, &member.stage)?;
        let path = member.stage.join("Cargo.toml");
        let mut cargo: toml_edit::DocumentMut = std::fs::read_to_string(&path)
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
        cargo["lib"]["crate-type"] =
            toml_edit::value(["rlib", "cdylib"].into_iter().collect::<toml_edit::Array>());
        std::fs::write(path, cargo.to_string()).map_err(|e| e.to_string())?;
        let (registrations, points) = (&member.registrations, &member.points);
        point_names.extend(serde_json::from_str::<Vec<String>>(points).map_err(|e| e.to_string())?);
        configs.insert(member.package.clone(), serde_json::json!({"id":member.manifest["id"].as_str(), "version":member.manifest["version"].as_str(), "stage":member.stage, "registrations":registrations,"points":points,"provided":crates.iter().filter(|c| **c != member.krate).map(String::as_str).collect::<Vec<_>>().join(",")}));
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
        std::fs::copy(lock, stage.join("Cargo.lock")).map_err(|e| e.to_string())?;
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
    std::fs::write(stage.join("Cargo.toml"), workspace.to_string()).map_err(|e| e.to_string())?;
    let mut command = cargo_command("build", "member", &stage, &target, &toolchain)?;
    command.env(
        "HORIZON_POINT_NAMES",
        serde_json::to_string(&point_names).map_err(|e| e.to_string())?,
    );
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
        std::fs::write(member.root.join(MANIFEST), member.manifest.to_string())
            .map_err(|e| e.to_string())?;
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
    if args.iter().any(|a| a == "feature=\"rustc-dep-of-std\"")
        || std::env::var("CARGO_MANIFEST_DIR").is_ok_and(|p| {
            p.replace('\\', "/")
                .contains("/lib/rustlib/src/rust/library/")
        })
    {
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
    let mut command = Command::new(compiler);
    command
        .env_remove("HORIZON_GUEST_REGISTRATIONS")
        .env_remove("HORIZON_POINT_NAMES")
        .env_remove(PROVIDED_ENV);
    if config.is_object() && args.iter().any(|a| a == "cdylib") {
        command
            .env("HORIZON_GROUP_PACKAGE", &package)
            .env(
                "HORIZON_GUEST_REGISTRATIONS",
                config["registrations"].as_str().ok_or("no registrations")?,
            )
            .env(
                "HORIZON_POINT_NAMES",
                config["points"].as_str().ok_or("no points")?,
            )
            .env(
                PROVIDED_ENV,
                config["provided"].as_str().ok_or("no provided crates")?,
            );
        // Cargo's default disambiguator includes the staging package path. A mod's stable
        // identity must instead agree when its author and its dependents build separately.
        let mut index = 0;
        while index + 1 < args.len() {
            if args[index] == "-C" && args[index + 1].starts_with("metadata=") {
                args.drain(index..index + 2);
            } else {
                index += 1;
            }
        }
        let mut features: Vec<_> = args
            .windows(2)
            .filter(|a| a[0] == "--cfg" && a[1].starts_with("feature="))
            .map(|a| a[1].clone())
            .collect();
        features.sort();
        args.push(format!(
            "-Cmetadata={}@{}:{}",
            config["id"].as_str().ok_or("no id")?,
            config["version"].as_str().ok_or("no version")?,
            sha512_of(features.join(",").as_bytes())
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
        args.push(format!(
            "--cfg=horizon_guest_registration=\"{}\"",
            sha512_of(format!("{}{}", config["registrations"], config["points"]).as_bytes())
        ));
    }
    let status = command.args(args).status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("rustc failed: {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
