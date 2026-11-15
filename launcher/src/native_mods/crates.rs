//! Lock-file isolation for the launcher's flat native crates.
//! The launcher stages one crate at native-api/<mod>/.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Where the launcher stages the mods' crates.
pub(super) const DIR: &str = "native-api";

pub(super) fn check_game_graph_names(
    stage: &Path,
    text: &str,
    game_names: &BTreeSet<String>,
) -> Result<(), String> {
    let lock: toml::Table = text.parse().map_err(|e| format!("Cargo.lock: {e}"))?;
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or("Cargo.lock: missing packages")?;
    let mut staged = Vec::new();
    for (mod_id, manifest) in manifests(&stage.join(DIR))? {
        let text = fs::read_to_string(manifest).map_err(|e| e.to_string())?;
        let value: toml::Table = text.parse().map_err(|e| format!("Cargo.toml: {e}"))?;
        let name = value
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str)
            .ok_or("Cargo.toml: missing package name")?
            .to_owned();
        staged.push((mod_id, name));
    }
    // Each staged mod's generated facade, by package, and the mod crate it belongs to. Only that
    // crate passes through it: a facade with a registry or git source, or one reached from any
    // other crate, is walked like any dependency, so its game dependency is refused.
    let facades: BTreeMap<String, &str> = staged
        .iter()
        .filter(|(id, _)| {
            let facade = super::internals::package(id);
            packages.iter().all(|p| {
                p.get("name").and_then(toml::Value::as_str) != Some(&facade)
                    || p.get("source").is_none()
            })
        })
        .map(|(id, name)| (super::internals::package(id), name.as_str()))
        .collect();
    for (mod_id, name) in &staged {
        let mut todo = vec![(name.to_owned(), name.to_owned())];
        let mut seen = BTreeSet::new();
        while let Some((current, via)) = todo.pop() {
            if !seen.insert(current.clone()) {
                continue;
            }
            for package in packages
                .iter()
                .filter(|p| p.get("name").and_then(toml::Value::as_str) == Some(&current))
            {
                for dep in package
                    .get("dependencies")
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(toml::Value::as_str)
                {
                    let dep = dep.split_whitespace().next().unwrap_or(dep);
                    if dep == super::api::API_PACKAGE || facades.get(dep) == Some(&current.as_str())
                    {
                        continue;
                    }
                    let first = if current == *name {
                        dep.to_owned()
                    } else {
                        via.clone()
                    };
                    if game_names.contains(dep) {
                        return Err(super::api::ProjectError::GameDependency {
                            mod_id: mod_id.clone(),
                            key: format!("{first} -> {dep}"),
                        }
                        .to_string());
                    }
                    todo.push((dep.to_owned(), first));
                }
            }
        }
    }
    Ok(())
}

/// A crate a mod's crate asks for in its `Cargo.toml`: under its own name or `package`, in any
/// dependency table (dev and build ones too: the lock has them all).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Wanted {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub requirement: semver::VersionReq,
}

/// Registry dependencies of each staged crate, validating generated local paths as well.
pub(super) fn wanted(
    stage: &Path,
    declared: &BTreeMap<&str, BTreeSet<&str>>,
) -> Result<Vec<Wanted>, String> {
    let mut out = Vec::new();
    let staged_manifests = manifests(&stage.join(DIR))?;
    let mut pending = staged_manifests.clone();
    let mut seen = BTreeSet::new();
    while let Some((mod_id, manifest)) = pending.pop() {
        if !seen.insert(manifest.clone()) {
            continue;
        }
        let text =
            fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
        let value: toml::Table = text
            .parse()
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
        for section in ["patch", "replace", "workspace"] {
            if value.contains_key(section) {
                return Err(format!("{}: [{section}] is refused", manifest.display()));
            }
        }
        let mut tables: Vec<&toml::Value> = Vec::new();
        // Cargo still reads the underscore spellings before edition 2024.
        let kinds = [
            "dependencies",
            "dev-dependencies",
            "build-dependencies",
            "dev_dependencies",
            "build_dependencies",
        ];
        tables.extend(kinds.iter().filter_map(|k| value.get(*k)));
        for target in value
            .get("target")
            .and_then(|t| t.as_table())
            .into_iter()
            .flat_map(|t| t.values())
        {
            tables.extend(kinds.iter().filter_map(|k| target.get(*k)));
        }
        for (key, dep) in tables.iter().filter_map(|t| t.as_table()).flatten() {
            if dep.get("package").is_some_and(|package| !package.is_str()) {
                return Err(format!(
                    "{}: dependency {key} has a non-string package name",
                    manifest.display()
                ));
            }
            let get = |name: &str| dep.get(name).and_then(|v| v.as_str());
            if get("package").unwrap_or(key) == "horizon-client" {
                return Err(super::api::ProjectError::GameDependency {
                    mod_id: mod_id.clone(),
                    key: key.clone(),
                }
                .to_string());
            }
            if let Some(path) = dep.get("path") {
                let path = path.as_str().ok_or_else(|| {
                    format!(
                        "{}: dependency {key} has a non-string path",
                        manifest.display()
                    )
                })?;
                if [
                    "git",
                    "registry",
                    "registry-index",
                    "branch",
                    "rev",
                    "tag",
                    "workspace",
                ]
                .iter()
                .any(|source| dep.get(*source).is_some())
                {
                    return Err(format!(
                        "{}: dependency {key} has conflicting source selectors",
                        manifest.display()
                    ));
                }
                let package = match dep.get("package") {
                    Some(value) => value.as_str().ok_or_else(|| {
                        format!(
                            "{}: dependency {key} has a non-string package name",
                            manifest.display()
                        )
                    })?,
                    None => key,
                };
                // The game's own packages, at the path `api::stage` rewrites them to.
                if package == super::internals::package(&mod_id)
                    && path == format!("../../{}", super::internals::relative(&mod_id))
                {
                    continue;
                }
                if super::api::PACKAGES.iter().any(|(game, _)| {
                    *game != super::api::INTERNALS_PACKAGE
                        && package == *game
                        && path == format!("../../crates/{game}")
                }) {
                    continue;
                }
                let root = stage.join(DIR).join(&mod_id);
                let relative = manifest
                    .parent()
                    .ok_or("invalid native manifest")?
                    .strip_prefix(&root)
                    .map_err(|e| e.to_string())?;
                if !path.starts_with("../") || !relative.as_os_str().is_empty() {
                    let inside = local_path(&root, relative, path)?;
                    let nested = root.join(inside).join("Cargo.toml");
                    if is_link_or_junction(
                        &fs::symlink_metadata(&nested).map_err(|e| e.to_string())?,
                    ) {
                        return Err("linked dependency manifest is refused".into());
                    }
                    let nested_manifest: toml::Table = fs::read_to_string(&nested)
                        .map_err(|e| e.to_string())?
                        .parse()
                        .map_err(|e| format!("Cargo.toml: {e}"))?;
                    if nested_manifest
                        .get("package")
                        .and_then(|p| p.get("name"))
                        .and_then(toml::Value::as_str)
                        != Some(package)
                    {
                        return Err(format!(
                            "{}: path dependency {key} has the wrong package name",
                            manifest.display()
                        ));
                    }
                    pending.push((mod_id.clone(), nested));
                    continue;
                }
                validate_sibling_path(
                    &mod_id,
                    &manifest,
                    path,
                    package,
                    &staged_manifests,
                    declared,
                )?;
                continue;
            }
            for selector in [
                "git",
                "registry",
                "registry-index",
                "branch",
                "rev",
                "tag",
                "workspace",
            ] {
                if dep.get(selector).is_some() {
                    return Err(format!("{}: dependency {key}: {selector} sources are not allowed; use crates.io with Cargo.lock", manifest.display()));
                }
            }
            let version = dep
                .as_str()
                .or(get("version"))
                .ok_or_else(|| {
                    format!(
                        "{}: registry dependency {key} must declare a version",
                        manifest.display()
                    )
                })?
                .to_owned();
            let requirement = semver::VersionReq::parse(&version)
                .map_err(|e| format!("{mod_id}: dependency {key}: {e}"))?;
            out.push(Wanted {
                mod_id: mod_id.to_owned(),
                name: get("package").unwrap_or(key).to_owned(),
                version,
                requirement,
            });
        }
    }
    out.sort_by(|a, b| (&a.mod_id, &a.name, &a.version).cmp(&(&b.mod_id, &b.name, &b.version)));
    Ok(out)
}

/// `(mod id, its crate's Cargo.toml)` under `native-api/`, sorted.
fn manifests(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut out = Vec::new();
    let read = |d: &Path| -> Result<Vec<PathBuf>, String> {
        let metadata = fs::symlink_metadata(d).map_err(|e| format!("{}: {e}", d.display()))?;
        if is_link_or_junction(&metadata) {
            return Err(format!("{}: linked native crate directory", d.display()));
        }
        if !metadata.is_dir() {
            return Err(format!("{}: not a native crate directory", d.display()));
        }
        let entries = fs::read_dir(d).map_err(|e| format!("{}: {e}", d.display()))?;
        let mut paths = Vec::new();
        for entry in entries {
            let path = entry.map_err(|e| format!("{}: {e}", d.display()))?.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if is_link_or_junction(&metadata) {
                return Err(format!("{}: linked native crate entry", path.display()));
            }
            if metadata.is_dir() {
                paths.push(path);
            }
        }
        paths.sort();
        Ok(paths)
    };
    for mod_dir in read(dir)? {
        let id = mod_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let manifest = mod_dir.join("Cargo.toml");
        match fs::symlink_metadata(&manifest) {
            Ok(metadata) if is_link_or_junction(&metadata) => {
                return Err(format!("{}: linked crate manifest", manifest.display()));
            }
            Ok(metadata) if metadata.is_file() => out.push((id, manifest)),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("{}: {error}", manifest.display())),
        }
    }
    Ok(out)
}

/// Resolve a path within one mod without following links or traversing outside its root.
pub(super) fn local_path(root: &Path, relative: &Path, path: &str) -> Result<PathBuf, String> {
    if path.contains('\\') {
        return Err("dependency path must use portable separators".into());
    }
    let mut inside = relative.to_path_buf();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => inside.push(part),
            Component::CurDir => {}
            Component::ParentDir if inside.pop() => {}
            _ => return Err("path outside the mod is refused".into()),
        }
    }
    let mut checked = root.to_path_buf();
    for part in inside.components() {
        checked.push(part);
        let meta = fs::symlink_metadata(&checked).map_err(|e| e.to_string())?;
        if is_link_or_junction(&meta) {
            return Err("linked dependency path is refused".into());
        }
    }
    Ok(inside)
}

/// Only a declared `../<mod-id>` may name another staged mod's crate. Its directory and
/// manifest must be ordinary entries; never canonicalize a source path or follow a link.
fn validate_sibling_path(
    mod_id: &str,
    manifest: &Path,
    path: &str,
    package: &str,
    staged_manifests: &[(String, PathBuf)],
    declared: &BTreeMap<&str, BTreeSet<&str>>,
) -> Result<(), String> {
    let invalid = || {
        format!(
            "{}: dependency path {path:?} is not a declared sibling crate",
            manifest.display()
        )
    };
    let Some(crate_id) = path.strip_prefix("../") else {
        return Err(invalid());
    };
    if crate_id.is_empty()
        || crate_id.contains('/')
        || crate_id.contains('\\')
        || !declared
            .get(mod_id)
            .is_some_and(|ids| ids.contains(crate_id))
    {
        return Err(invalid());
    }
    let mut components = Path::new(path).components();
    if !matches!(components.next(), Some(Component::ParentDir))
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(invalid());
    }

    let crate_dir = manifest.parent().ok_or_else(invalid)?;
    let mod_dir = crate_dir.parent().ok_or_else(invalid)?;
    let sibling = mod_dir.join(crate_id);
    let sibling_metadata = fs::symlink_metadata(&sibling).map_err(|_| invalid())?;
    if is_link_or_junction(&sibling_metadata) || !sibling_metadata.is_dir() {
        return Err(invalid());
    }

    let sibling_manifest = sibling.join("Cargo.toml");
    let manifest_metadata = fs::symlink_metadata(&sibling_manifest).map_err(|_| invalid())?;
    if is_link_or_junction(&manifest_metadata) || !manifest_metadata.is_file() {
        return Err(invalid());
    }
    if !staged_manifests
        .iter()
        .any(|(_, staged)| staged == &sibling_manifest)
    {
        return Err(invalid());
    }

    let text = fs::read_to_string(&sibling_manifest).map_err(|_| invalid())?;
    let value: toml::Table = text.parse().map_err(|_| invalid())?;
    let staged_package = value
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(invalid)?;
    if staged_package != package {
        return Err(invalid());
    }
    Ok(())
}

/// `symlink_metadata` reports links without following them. Windows junctions are reparse points
/// as well, so reject the reparse attribute even if the standard file type is not a symlink.
pub(super) fn is_link_or_junction(metadata: &fs::Metadata) -> bool {
    let link = metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        link || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    link
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_STAGE: AtomicUsize = AtomicUsize::new(0);

    struct StageFixture {
        root: PathBuf,
    }

    impl StageFixture {
        fn new() -> Self {
            let root = loop {
                let root = std::env::temp_dir().join(format!(
                    "horizon-native-sibling-{}-{}",
                    std::process::id(),
                    NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&root) {
                    Ok(()) => break root,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("{}: {error}", root.display()),
                }
            };
            Self { root }
        }

        fn root(&self) -> &Path {
            &self.root
        }

        fn add_crate(&self, mod_id: &str, crate_id: &str, package: &str, body: &str) -> PathBuf {
            let id = if crate_id == "engine" {
                mod_id
            } else {
                crate_id
            };
            let dir = self.root.join(DIR).join(id);
            write_manifest(&dir, package, body);
            dir
        }
    }

    impl Drop for StageFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn write_manifest(dir: &Path, package: &str, body: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname = {package:?}\nversion = \"0.1.0\"\n{body}"),
        )
        .unwrap();
    }

    fn wanted(stage: &Path) -> Result<Vec<Wanted>, String> {
        let targets = [
            "same_name",
            "industrial_sim",
            "test_support",
            "build_helper",
            "unix_helper",
            "windows_test",
            "target_build",
            "actual",
            "absolute",
            "linked",
            "local",
            "not_a_crate",
        ];
        let declared = BTreeMap::from([("bake", targets.into_iter().collect())]);
        super::wanted(stage, &declared)
    }

    fn assert_rejected_path(label: &str, dependency: impl FnOnce(&StageFixture) -> String) {
        let stage = StageFixture::new();
        let entry = dependency(&stage);
        stage.add_crate(
            "bake",
            "engine",
            "bake-engine",
            &format!("[dependencies]\n{entry}\n"),
        );
        assert!(wanted(stage.root()).is_err(), "{label}");
    }

    fn wants(mod_id: &str, name: &str, version: &str) -> Wanted {
        Wanted {
            mod_id: mod_id.into(),
            name: name.into(),
            version: version.into(),
            requirement: semver::VersionReq::parse(version).unwrap(),
        }
    }

    #[test]
    fn wanted_rejects_unknown_malformed_or_out_of_mod_sibling_paths() {
        assert_rejected_path("unknown sibling", |_| {
            "unknown = { path = \"../missing\" }".into()
        });
        assert_rejected_path("package alias does not match sibling manifest", |stage| {
            stage.add_crate("bake", "actual", "actual-package", "");
            "local = { path = \"../actual\", package = \"different-package\" }".into()
        });
        assert_rejected_path("absolute path", |stage| {
            let sibling = stage.add_crate("bake", "absolute", "absolute-package", "");
            format!(
                "local = {{ path = {:?}, package = \"absolute-package\" }}",
                sibling.to_string_lossy().to_string()
            )
        });
        assert_rejected_path("path escapes the mod directory", |stage| {
            write_manifest(&stage.root.join("outside"), "outside-package", "");
            "local = { path = \"../../../outside\", package = \"outside-package\" }".into()
        });
        assert_rejected_path("path crosses into another mod", |stage| {
            stage.add_crate("zoom", "foreign", "foreign-package", "");
            "local = { path = \"../../zoom/foreign\", package = \"foreign-package\" }".into()
        });
        assert_rejected_path("path is not a string", |_| "local = { path = 7 }".into());
        assert_rejected_path("sibling Cargo.toml is not a regular file", |stage| {
            let sibling = stage.root.join(DIR).join("not_a_crate");
            fs::create_dir_all(sibling.join("Cargo.toml")).unwrap();
            "local = { path = \"../not_a_crate\" }".into()
        });
    }

    #[test]
    fn sibling_crates_require_a_manifest_dependency_and_the_game_path_is_exact() {
        let stage = StageFixture::new();
        stage.add_crate(
            "bake",
            "engine",
            "bake-engine",
            "[dependencies]\nother={path='../other'}\n",
        );
        stage.add_crate("other", "other", "other", "");
        assert!(super::wanted(stage.root(), &BTreeMap::new()).is_err());
        let declared = BTreeMap::from([("bake", BTreeSet::from(["other"]))]);
        assert!(super::wanted(stage.root(), &declared).is_ok());
        let root = stage.root().join(DIR).join("bake/Cargo.toml");
        for (package, path, valid) in [
            ("horizon-mod-api", "../../crates/horizon-mod-api", true),
            ("horizon-mod-api", "../../other/horizon-mod-api", false),
        ] {
            fs::write(&root, format!("[package]\nname='bake'\nversion='1'\n[dependencies]\napi={{package='{package}',path='{path}'}}\n")).unwrap();
            assert_eq!(super::wanted(stage.root(), &BTreeMap::new()).is_ok(), valid);
        }
    }

    #[test]
    fn registry_sources_are_refused_before_any_cargo_resolution() {
        for (selector, value) in [
            ("git", "'https://example.invalid/repo'"),
            ("registry", "'other'"),
            ("registry-index", "'https://example.invalid/index'"),
            ("workspace", "true"),
        ] {
            let stage = StageFixture::new();
            stage.add_crate(
                "bake",
                "engine",
                "bake-engine",
                &format!("[dependencies]\nremote={{version='1', {selector}={value}}}\n"),
            );
            let error = wanted(stage.root()).unwrap_err();
            assert!(error.contains(selector), "{error}");
        }
    }

    #[test]
    fn local_crate_registry_requests_are_included_and_nested_patches_are_refused() {
        let stage = StageFixture::new();
        let root = stage.add_crate(
            "bake",
            "engine",
            "bake-engine",
            "[dependencies]\nhelper={path='src/helper'}\n",
        );
        write_manifest(
            &root.join("src/helper"),
            "helper",
            "[dependencies]\nitoa='=0.4.8'\n",
        );
        assert_eq!(
            wanted(stage.root()).unwrap(),
            [wants("bake", "itoa", "=0.4.8")]
        );
        write_manifest(
            &root.join("src/helper"),
            "helper",
            "[patch.crates-io]\nitoa={path='../outside'}\n",
        );
        assert!(wanted(stage.root()).unwrap_err().contains("[patch]"));
    }

    #[test]
    fn wanted_rejects_each_conflicting_path_source_selector() {
        let selectors = [
            ("git", "\"https://example.invalid/repo\""),
            ("registry", "\"crates-io\""),
            ("registry-index", "\"https://example.invalid/index\""),
            ("branch", "\"main\""),
            ("rev", "\"0123456789abcdef\""),
            ("tag", "\"v1.0.0\""),
            ("workspace", "true"),
        ];
        let accepted: Vec<&str> = selectors
            .iter()
            .filter_map(|(selector, value)| {
                let stage = StageFixture::new();
                stage.add_crate("bake", "local", "local-package", "");
                let dependency = format!(
                    "local = {{ path = \"../local\", package = \"local-package\", {selector} = {value} }}"
                );
                stage.add_crate(
                    "bake",
                    "engine",
                    "bake-engine",
                    &format!("[dependencies]\n{dependency}\n"),
                );
                wanted(stage.root()).is_ok().then_some(*selector)
            })
            .collect();
        assert!(
            accepted.is_empty(),
            "path dependencies accepted conflicting source selectors: {accepted:?}"
        );
    }
}
