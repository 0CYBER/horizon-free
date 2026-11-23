//! Cargo metadata is read without compiling dependencies or executing build-time code.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeLink {
    pub name: String,
    pub version: String,
    pub library: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    pub added_crates: usize,
    pub build_scripts: Vec<String>,
    pub proc_macros: Vec<String>,
}

impl Inventory {
    pub fn label(&self) -> String {
        crate::i18n::trf(
            "Native mod adds {count} registry crates",
            &[("count", &self.added_crates.to_string())],
        )
    }
    pub fn detail(&self) -> String {
        let none = crate::i18n::tr("None");
        let scripts = self.build_scripts.join(", ");
        let macros = self.proc_macros.join(", ");
        [
            crate::i18n::tr("Registry dependencies are native code. Their build scripts and proc macros run during compilation, before the game starts.").to_owned(),
            crate::i18n::trf("Build scripts: {crates}", &[("crates", if self.build_scripts.is_empty() { none } else { &scripts })]),
            crate::i18n::trf("Proc macros: {crates}", &[("crates", if self.proc_macros.is_empty() { none } else { &macros })]),
        ].join("\n\n")
    }
}

#[derive(Deserialize)]
pub(super) struct Metadata {
    pub packages: Vec<Package>,
    workspace_members: Vec<String>,
    resolve: Resolve,
}
#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}
#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<Dependency>,
}
#[derive(Deserialize)]
struct Dependency {
    pkg: String,
    dep_kinds: Vec<Kind>,
}
#[derive(Deserialize)]
struct Kind {
    kind: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Package {
    id: String,
    pub name: String,
    pub version: String,
    pub source: Option<String>,
    pub links: Option<String>,
    pub targets: Vec<Target>,
    pub manifest_path: PathBuf,
}
#[derive(Deserialize)]
pub(super) struct Target {
    pub kind: Vec<String>,
}

type ReleaseDependencies<'a> = BTreeMap<&'a str, BTreeSet<&'a str>>;

/// `cargo tree --edges normal,no-proc-macro`: omit dev features as well as dev edges.
/// Each package prints its children once; `(*)` still identifies a real incoming edge.
fn release_tree(mut cargo: std::process::Command) -> Result<String, String> {
    let output = cargo
        .args([
            "--workspace",
            "--offline",
            "--locked",
            "--edges",
            "normal,no-proc-macro",
            "--prefix",
            "depth",
            "--format",
            "|{p}",
            "--color",
            "never",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    String::from_utf8(output.stdout).map_err(|error| format!("Cargo release tree: {error}"))
}

fn tree_package(label: &str) -> Option<(&str, &str, Option<&Path>)> {
    let label = label.strip_suffix(" (*)").unwrap_or(label);
    let (name, version) = label.split_once(" v")?;
    let (version, directory) = match version.split_once(" (") {
        Some((version, directory)) => (version, Some(Path::new(directory.strip_suffix(')')?))),
        None => (version, None),
    };
    Some((name, version, directory))
}

impl Metadata {
    fn reachable(&self, runtime: bool) -> Result<BTreeSet<&str>, String> {
        let nodes: std::collections::BTreeMap<_, _> = self
            .resolve
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();
        let packages: std::collections::BTreeMap<_, _> = self
            .packages
            .iter()
            .map(|package| (package.id.as_str(), package))
            .collect();
        let mut pending: Vec<_> = self.workspace_members.iter().map(String::as_str).collect();
        let mut reachable = BTreeSet::new();
        while let Some(id) = pending.pop() {
            let package = packages.get(id).ok_or("Cargo metadata: missing package")?;
            if runtime
                && package
                    .targets
                    .iter()
                    .any(|target| target.kind.iter().any(|kind| kind == "proc-macro"))
            {
                continue;
            }
            if !reachable.insert(id) {
                continue;
            }
            let node = nodes
                .get(id)
                .ok_or("Cargo metadata: missing resolved node")?;
            pending.extend(
                node.deps
                    .iter()
                    .filter(|dep| {
                        dep.dep_kinds.iter().any(|kind| {
                            if runtime {
                                kind.kind.is_none()
                            } else {
                                kind.kind.as_deref() != Some("dev")
                            }
                        })
                    })
                    .map(|dep| dep.pkg.as_str()),
            );
        }
        Ok(reachable)
    }

    pub fn runtime_packages(&self) -> Result<BTreeSet<String>, String> {
        self.reachable(true)
            .map(|ids| ids.into_iter().map(str::to_owned).collect())
    }

    /// Use metadata for exact identities, but Cargo's release tree for active target edges.
    fn release_dependencies<'a>(&'a self, tree: &str) -> Result<ReleaseDependencies<'a>, String> {
        let mut by_label = BTreeMap::new();
        for package in &self.packages {
            // The native plan admits crates.io and local paths only. Cargo displays a
            // directory for local packages, which can share a name/version with crates.io.
            let key = (
                package.name.as_str(),
                package.version.as_str(),
                if package.source.is_none() {
                    package.manifest_path.parent()
                } else {
                    None
                },
            );
            if by_label.insert(key, package.id.as_str()).is_some() {
                return Err(format!(
                    "Cargo release tree: ambiguous package {}",
                    package.name
                ));
            }
        }
        let mut dependencies: ReleaseDependencies<'_> = BTreeMap::new();
        let mut parents = Vec::new();
        for line in tree.lines().filter(|line| !line.is_empty()) {
            let invalid = || format!("Cargo release tree: unrecognized entry {line:?}");
            let (depth, label) = line.split_once('|').ok_or_else(invalid)?;
            let depth: usize = depth.parse().map_err(|_| invalid())?;
            let key = tree_package(label).ok_or_else(invalid)?;
            let id = by_label
                .get(&key)
                .copied()
                .or_else(|| {
                    // Cargo may display the same local path with different Windows prefixes.
                    let (name, version, path) = key;
                    let path = path?.canonicalize().ok()?;
                    self.packages
                        .iter()
                        .find(|package| {
                            package.name == name
                                && package.version == version
                                && package.source.is_none()
                                && package.manifest_path.parent().is_some_and(|dir| {
                                    dir.canonicalize().is_ok_and(|dir| dir == path)
                                })
                        })
                        .map(|package| package.id.as_str())
                })
                .ok_or_else(invalid)?;
            if depth > parents.len() {
                return Err(invalid());
            }
            parents.truncate(depth);
            if let Some(&parent) = parents.last() {
                dependencies.entry(parent).or_default().insert(id);
            }
            dependencies.entry(id).or_default();
            parents.push(id);
        }
        Ok(dependencies)
    }

    /// Target Rust libraries that more than one staged mod would otherwise link statically.
    /// `sdk` is the canonical root used to validate the SDK patch paths.
    pub fn shared_libraries(
        &self,
        roots: &[PathBuf],
        sdk: &Path,
        cargo_tree: std::process::Command,
    ) -> Result<(Vec<&Package>, BTreeSet<usize>), String> {
        self.shared_in_graph(
            roots,
            sdk,
            &self.release_dependencies(&release_tree(cargo_tree)?)?,
        )
    }

    fn shared_in_graph(
        &self,
        roots: &[PathBuf],
        sdk: &Path,
        dependencies: &ReleaseDependencies<'_>,
    ) -> Result<(Vec<&Package>, BTreeSet<usize>), String> {
        let packages: BTreeMap<_, _> = self
            .packages
            .iter()
            .map(|package| (package.id.as_str(), package))
            .collect();
        let roots: Vec<_> = roots
            .iter()
            .map(|root| {
                self.packages
                    .iter()
                    .find(|package| package.manifest_path == *root)
                    .map(|package| package.id.as_str())
                    .ok_or("Cargo metadata: missing staged mod")
            })
            .collect::<Result<_, _>>()?;
        let mod_ids: BTreeSet<_> = roots.iter().copied().collect();
        let mut first = BTreeMap::new();
        let mut shared = BTreeSet::new();
        let mut users = BTreeSet::new();
        for (index, root) in roots.into_iter().enumerate() {
            let mut pending = vec![root];
            let mut closure = BTreeSet::new();
            while let Some(id) = pending.pop() {
                // Another mod dylib already owns its closure; only count crates reached
                // independently of it through this mod's own target dependencies.
                if id != root && mod_ids.contains(id) {
                    continue;
                }
                let package = packages.get(id).ok_or("Cargo metadata: missing package")?;
                if !package
                    .targets
                    .iter()
                    .flat_map(|target| &target.kind)
                    .any(|kind| matches!(kind.as_str(), "lib" | "rlib" | "dylib"))
                    || !closure.insert(id)
                {
                    continue;
                }
                // Cargo's file URLs can spell a patched manifest as a drive path
                // on Windows even though the validated SDK root is verbatim.
                if package.source.is_some()
                    || package
                        .manifest_path
                        .canonicalize()
                        .is_ok_and(|path| path.starts_with(sdk))
                {
                    let owner = *first.entry(id).or_insert(index);
                    if owner != index {
                        shared.insert(id);
                        users.extend([owner, index]);
                    }
                }
                pending.extend(
                    dependencies
                        .get(id)
                        .ok_or("Cargo release tree: missing target package")?
                        .iter()
                        .copied(),
                );
            }
        }
        Ok((shared.into_iter().map(|id| packages[id]).collect(), users))
    }

    pub fn inventory(
        &self,
        game: &super::registry_lock::RegistryLock,
        links: &[NativeLink],
    ) -> Result<Inventory, String> {
        let reachable = self.reachable(false)?;
        let runtime = self.reachable(true)?;
        let mut scripts = BTreeSet::new();
        let mut macros = BTreeSet::new();
        let mut added = BTreeSet::new();
        for package in &self.packages {
            if package.source.is_none() || !reachable.contains(package.id.as_str()) {
                continue;
            }
            let label = format!("{} {}", package.name, package.version);
            if let Some(library) = package
                .links
                .as_ref()
                .filter(|_| runtime.contains(package.id.as_str()))
            {
                if let Some(existing) = links.iter().find(|claim| &claim.library == library) {
                    return Err(format!("Native library {library}: mod package {} {} would compile a second provider of SDK package {} {}; the prebuilt game cannot be relinked with a different provider", package.name, package.version, existing.name, existing.version));
                }
            }
            if game
                .packages
                .keys()
                .any(|id| id.name == package.name && id.version.to_string() == package.version)
            {
                continue;
            }
            added.insert(label.clone());
            if package
                .targets
                .iter()
                .any(|target| target.kind.iter().any(|kind| kind == "custom-build"))
            {
                scripts.insert(label.clone());
            }
            if package
                .targets
                .iter()
                .any(|target| target.kind.iter().any(|kind| kind == "proc-macro"))
            {
                macros.insert(label);
            }
        }
        Ok(Inventory {
            added_crates: added.len(),
            build_scripts: scripts.into_iter().collect(),
            proc_macros: macros.into_iter().collect(),
        })
    }
}

pub(super) fn read(
    rust: &crate::rust_toolchain::Toolchain,
    workspace: &std::path::Path,
) -> Result<Metadata, String> {
    let output = super::cargo(rust, workspace, "metadata")
        .args([
            "--offline",
            "--format-version",
            "1",
            "--filter-platform",
            rust.host.target,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("Cargo metadata: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata() -> Metadata {
        serde_json::from_value(serde_json::json!({
            "workspace_members": ["root"],
            "resolve": {"nodes": [
                {"id":"root", "deps":[{"pkg":"script", "dep_kinds":[{"kind":null}]}, {"pkg":"macro", "dep_kinds":[{"kind":null}]}, {"pkg":"unused", "dep_kinds":[{"kind":"dev"}]}]},
                {"id":"script", "deps":[]}, {"id":"macro", "deps":[]}, {"id":"unused", "deps":[]}
            ]},
            "packages": [
                {"id":"root", "name":"mod", "version":"1.0.0", "source":null, "links":null, "manifest_path":"mod/Cargo.toml", "targets":[{"kind":["lib"]}]},
                {"id":"script", "name":"native-sys", "version":"2.0.0", "source":"registry+https://github.com/rust-lang/crates.io-index", "links":"example", "manifest_path":"native-sys/Cargo.toml", "targets":[{"kind":["lib"]},{"kind":["custom-build"]}]},
                {"id":"macro", "name":"derive-example", "version":"1.0.0", "source":"registry+https://github.com/rust-lang/crates.io-index", "links":null, "manifest_path":"derive-example/Cargo.toml", "targets":[{"kind":["proc-macro"]}]},
                {"id":"unused", "name":"dev-only", "version":"1.0.0", "source":"registry+https://github.com/rust-lang/crates.io-index", "links":"example", "manifest_path":"dev-only/Cargo.toml", "targets":[{"kind":["custom-build"]}]}
            ]
        })).unwrap()
    }
    #[test]
    fn shares_normal_target_dependencies_through_local_crates_only_between_consumers() {
        let mut metadata = metadata();
        for name in ["local", "second", "dependent"] {
            metadata.packages.push(Package {
                id: name.into(),
                name: name.into(),
                version: "1.0.0".into(),
                source: None,
                links: None,
                targets: vec![Target {
                    kind: vec!["lib".into()],
                }],
                manifest_path: Path::new(name).join("Cargo.toml"),
            });
        }
        let roots = ["mod", "second", "dependent"].map(|name| Path::new(name).join("Cargo.toml"));
        let release = metadata
            .release_dependencies(
                "0|dependent v1.0.0 (dependent)\n1|mod v1.0.0 (mod)\n2|native-sys v2.0.0\n\
                 0|mod v1.0.0 (mod) (*)\n\
                 0|second v1.0.0 (second)\n1|local v1.0.0 (local)\n2|native-sys v2.0.0 (*)\n",
            )
            .unwrap();
        let (packages, users) = metadata
            .shared_in_graph(&roots, Path::new("sdk"), &release)
            .unwrap();
        assert_eq!(users, BTreeSet::from([0, 1]));
        assert_eq!(
            packages
                .iter()
                .map(|package| package.name.as_str())
                .collect::<Vec<_>>(),
            ["native-sys"],
        );
    }

    #[test]
    fn two_mods_share_the_sdk_patch_but_not_its_dev_only_feature_dependency() {
        use std::{fs, process::Command};

        let scratch = tempfile::tempdir().unwrap();
        let sdk = scratch.path().join("sdk");
        let patch = sdk.join("patched library");
        let dev_only = sdk.join("dev-only");
        let stage = scratch.path().join("stage");
        let mods = ["first", "second"];
        for name in mods {
            let dir = stage.join(name);
            fs::create_dir_all(dir.join("src")).unwrap();
            fs::write(
                dir.join("src/lib.rs"),
                "pub use horizon_test_sdk_patch::Value;",
            )
            .unwrap();
            fs::write(
                dir.join("Cargo.toml"),
                format!(
                    "[package]\nname = '{name}'\nversion = '0.1.0'\nedition = '2024'\n\
                     [dependencies]\nhorizon-test-sdk-patch = '=0.1.0'\n\
                     [dev-dependencies]\nhorizon-test-sdk-patch = {{ version = '=0.1.0', features = ['test-only'] }}\n"
                ),
            )
            .unwrap();
        }
        fs::create_dir_all(patch.join("src")).unwrap();
        fs::write(patch.join("src/lib.rs"), "pub struct Value;").unwrap();
        fs::write(
            patch.join("Cargo.toml"),
            "[package]\nname = 'horizon-test-sdk-patch'\nversion = '0.1.0'\nedition = '2024'\n\
             [dependencies]\nhorizon-test-dev-only = { path = '../dev-only', optional = true }\n\
             [features]\ntest-only = ['dep:horizon-test-dev-only']\n",
        )
        .unwrap();
        fs::create_dir_all(dev_only.join("src")).unwrap();
        fs::write(
            dev_only.join("src/lib.rs"),
            "compile_error!(\"the dev-only crate must not enter a release build\");",
        )
        .unwrap();
        fs::write(
            dev_only.join("Cargo.toml"),
            "[package]\nname = 'horizon-test-dev-only'\nversion = '0.1.0'\nedition = '2024'\n",
        )
        .unwrap();
        let patch = patch.canonicalize().unwrap();
        let workspace = toml::toml! {
            [workspace]
            resolver = "2"
            members = ["first", "second"]
            [patch.crates-io]
            horizon-test-sdk-patch.path = (patch.to_str().unwrap())
        };
        fs::write(stage.join("Cargo.toml"), workspace.to_string()).unwrap();
        let cargo = Command::new(env!("CARGO"))
            .args(["metadata", "--offline", "--format-version", "1"])
            .current_dir(&stage)
            .output()
            .unwrap();
        assert!(
            cargo.status.success(),
            "{}",
            String::from_utf8_lossy(&cargo.stderr)
        );
        let metadata: Metadata = serde_json::from_slice(&cargo.stdout).unwrap();
        let roots = mods.map(|name| stage.join(name).join("Cargo.toml"));
        let mut cargo_tree = Command::new(env!("CARGO"));
        cargo_tree.arg("tree").current_dir(&stage);
        let (packages, users) = metadata
            .shared_libraries(&roots, &sdk.canonicalize().unwrap(), cargo_tree)
            .unwrap();
        assert_eq!(users, BTreeSet::from([0, 1]));
        assert_eq!(
            packages
                .iter()
                .map(|package| (package.name.as_str(), package.source.as_deref()))
                .collect::<Vec<_>>(),
            [("horizon-test-sdk-patch", None)]
        );
    }

    #[test]
    fn consent_counts_only_reachable_build_packages_and_names_executable_code() {
        let inventory = metadata()
            .inventory(&super::super::registry_lock::RegistryLock::default(), &[])
            .unwrap();
        assert_eq!(inventory.added_crates, 2);
        assert_eq!(inventory.build_scripts, ["native-sys 2.0.0"]);
        assert_eq!(inventory.proc_macros, ["derive-example 1.0.0"]);
    }
    #[test]
    fn host_build_dependencies_do_not_claim_a_game_runtime_native_library() {
        let mut metadata = metadata();
        metadata.resolve.nodes[0].deps[0].dep_kinds[0].kind = Some("build".into());
        let inventory = metadata
            .inventory(
                &super::super::registry_lock::RegistryLock::default(),
                &[NativeLink {
                    name: "sdk-sys".into(),
                    version: "1.0.0".into(),
                    library: "example".into(),
                }],
            )
            .unwrap();
        assert_eq!(inventory.build_scripts, ["native-sys 2.0.0"]);
        assert_eq!(inventory.added_crates, 2);
    }

    #[test]
    fn a_second_native_provider_is_refused_with_both_packages_versions_and_library() {
        let error = metadata()
            .inventory(
                &super::super::registry_lock::RegistryLock::default(),
                &[NativeLink {
                    name: "sdk-sys".into(),
                    version: "1.0.0".into(),
                    library: "example".into(),
                }],
            )
            .unwrap_err();
        for detail in ["native-sys", "2.0.0", "sdk-sys", "1.0.0", "example"] {
            assert!(error.contains(detail), "{error}");
        }
    }
}
