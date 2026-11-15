//! Cargo metadata is read without compiling dependencies or executing build-time code.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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
}
#[derive(Deserialize)]
pub(super) struct Target {
    pub kind: Vec<String>,
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
                {"id":"root", "name":"mod", "version":"1.0.0", "source":null, "links":null, "targets":[{"kind":["lib"]}]},
                {"id":"script", "name":"native-sys", "version":"2.0.0", "source":"registry+https://github.com/rust-lang/crates.io-index", "links":"example", "targets":[{"kind":["lib"]},{"kind":["custom-build"]}]},
                {"id":"macro", "name":"derive-example", "version":"1.0.0", "source":"registry+https://github.com/rust-lang/crates.io-index", "links":null, "targets":[{"kind":["proc-macro"]}]},
                {"id":"unused", "name":"dev-only", "version":"1.0.0", "source":"registry+https://github.com/rust-lang/crates.io-index", "links":"example", "targets":[{"kind":["custom-build"]}]}
            ]
        })).unwrap()
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
