//! Declared internals, shared by both native build routes.
use super::{api, inject};
use crate::i18n::trf;
use horizon_mod_package::internals::{self, ListedInternal};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub(super) const INTERNALS: &str = "internals.json";

#[derive(Default)]
pub(super) struct Catalog {
    pub(super) internals: Vec<ListedInternal>,
    pub(super) discovery: bool,
    pub(super) points: Vec<inject::ListedPoint>,
}

pub(super) struct Facade {
    pub(super) source: String,
    pub(super) crates: BTreeSet<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Error {
    Inject(String),
    Missing {
        mod_id: String,
        path: String,
    },
    Changed {
        mod_id: String,
        path: String,
        signature: String,
    },
    Gated {
        mod_id: String,
        path: String,
    },
    Ambiguous {
        mod_id: String,
        path: String,
        kinds: Vec<String>,
    },
    Path {
        path: String,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Inject(error) => error.clone(),
            Self::Missing { mod_id, path } => trf(
                "E_INTERNAL_MISSING: {mod} uses {path}, which this game doesn't have; run cargo horizon-mod internals sync to discover this pin's used internals.",
                &[("mod", mod_id), ("path", path)],
            ),
            Self::Changed { mod_id, path, signature } => trf(
                "E_INTERNAL_CHANGED: {mod} uses {path}, whose signature is now {signature}; run cargo horizon-mod update to see what changed.",
                &[("mod", mod_id), ("path", path), ("signature", signature)],
            ),
            Self::Gated { mod_id, path } => trf(
                "E_INTERNAL_GATED: {mod} uses {path}, which exists only under a cfg; target-specific internals aren't supported.",
                &[("mod", mod_id), ("path", path)],
            ),
            Self::Ambiguous { mod_id, path, kinds } => trf(
                "E_INTERNAL_AMBIGUOUS: {mod} uses {path}, which has multiple kinds; choose a kind from: {kinds}.",
                &[("mod", mod_id), ("path", path), ("kinds", &kinds.join(", "))],
            ),
            Self::Path { path } => trf(
                "E_INTERNAL_PATH: {path} cannot be re-exported as a declared internal; declare its types or free functions instead.",
                &[("path", path)],
            ),
        };
        f.write_str(&message)
    }
}
impl std::error::Error for Error {}

impl From<internals::Error> for Error {
    fn from(error: internals::Error) -> Self {
        match error {
            internals::Error::Missing { mod_id, path } => Self::Missing { mod_id, path },
            internals::Error::Changed {
                mod_id,
                path,
                signature,
            } => Self::Changed {
                mod_id,
                path,
                signature,
            },
            internals::Error::Gated { mod_id, path } => Self::Gated { mod_id, path },
            internals::Error::Ambiguous {
                mod_id,
                path,
                kinds,
            } => Self::Ambiguous {
                mod_id,
                path,
                kinds,
            },
        }
    }
}

impl Catalog {
    pub(super) fn target(&self, at: &str) -> Option<&ListedInternal> {
        self.internals.iter().find(|item| {
            !item.shape_only
                && plain_path(&item.path) == plain_path(at)
                && matches!(item.kind.as_str(), "fn" | "method" | "struct")
        })
    }

    /// `manifest` with each absent `hash` taken from this catalog, the pinned game's as the build
    /// checks it (`[native] horizon` fixes it exactly). The build records this form in
    /// `native-mods.json`; a declaration that doesn't resolve keeps no hash and is refused by
    /// [`Self::check`].
    pub(super) fn read(dir: &Path) -> Result<Self, String> {
        let path = dir.join(INTERNALS);
        let listed = inject::catalog(&dir.join(inject::POINTS))?;
        Ok(Self {
            points: listed.points,
            internals: internals::read(&path)?,
            discovery: false,
        })
    }

    /// `manifest` with each absent `hash` taken from this catalog, the pinned game's as the build
    /// checks it (`[native] horizon` fixes it exactly). The build records this form in
    /// `native-mods.json`; a declaration that doesn't resolve keeps no hash and is refused by
    /// [`Self::check`].
    pub(super) fn pinned(
        &self,
        manifest: &horizon_mod_package::Manifest,
    ) -> horizon_mod_package::Manifest {
        let mut pinned = manifest.clone();
        for inject in pinned
            .inject
            .iter_mut()
            .filter(|inject| inject.hash.is_none())
        {
            inject.hash = self.target(&inject.at).map(|item| item.hash.clone());
        }
        for declared in pinned
            .internals
            .iter_mut()
            .filter(|declared| declared.hash.is_none())
        {
            declared.hash = internals::resolve(
                &self.internals,
                &manifest.id,
                &declared.path,
                declared.kind.as_deref(),
            )
            .ok()
            .map(|entry| entry.hash.clone());
        }
        pinned
    }

    pub(super) fn check(&self, manifest: &horizon_mod_package::Manifest) -> Result<(), Error> {
        super::slots::check(self, manifest).map_err(Error::Inject)?;
        for declared in &manifest.internals {
            internals::check(&self.internals, &manifest.id, declared)?;
        }
        Ok(())
    }

    pub(super) fn prepare<'a>(
        &self,
        manifests: impl IntoIterator<Item = &'a horizon_mod_package::Manifest>,
    ) -> Result<BTreeMap<&'a str, Facade>, Error> {
        manifests
            .into_iter()
            .map(|manifest| {
                (if self.discovery {
                    self.discovery_facade(manifest)
                } else {
                    self.facade(manifest)
                })
                .map(|facade| (manifest.id.as_str(), facade))
            })
            .collect()
    }

    pub(super) fn log_mixins(
        &self,
        manifest: &horizon_mod_package::Manifest,
    ) -> Result<(), String> {
        if manifest
            .native
            .as_ref()
            .is_some_and(|n| !n.mixin.is_empty())
        {
            super::append_log(&crate::i18n::trf(
                "{mod}: mixins compile game code",
                &[("mod", &manifest.id)],
            ))?;
        }
        Ok(())
    }

    /// Discovery opens the staged view, not the production declaration boundary.
    fn discovery_facade(&self, manifest: &horizon_mod_package::Manifest) -> Result<Facade, Error> {
        let mut manifest = manifest.clone();
        manifest.internals.clear();
        let mut facade = self.facade(&manifest)?;
        facade.source.insert_str(0, "#![allow(deprecated)]\n");
        let mut tree = Module::default();
        // A member is reached through its catalogued owner. Never open a whole engine
        // crate: unlisted public APIs have no diagnostic marker and would be missed.
        for entry in self.internals.iter().filter(|entry| {
            !entry.shape_only
                && entry.cfg.is_empty()
                && !matches!(entry.kind.as_str(), "mod" | "field" | "method")
                && !entry.signature.starts_with("impl ")
        }) {
            tree.export(&self.export_path(entry)?, &mut facade.crates);
        }
        tree.write(&mut facade.source, 0);
        Ok(facade)
    }

    /// Re-exports are the compiler's boundary: no module wildcard can widen the declaration.
    pub(super) fn facade(&self, manifest: &horizon_mod_package::Manifest) -> Result<Facade, Error> {
        self.check(manifest)?;
        let mut tree = Module::default();
        let mut crates = BTreeSet::from(["horizon_client".into()]);
        if manifest.native.is_some() {
            for declared in &manifest.internals {
                let entry = internals::resolve(
                    &self.internals,
                    &manifest.id,
                    &declared.path,
                    declared.kind.as_deref(),
                )?;
                let path = self.export_path(entry)?;
                tree.export(&path, &mut crates);
            }
        }
        let mut text = String::from(
            "//! This mod's declared game internals, generated from its pinned build.\n",
        );
        tree.write(&mut text, 0);
        Ok(Facade {
            source: text,
            crates,
        })
    }

    fn export_path(&self, entry: &ListedInternal) -> Result<String, Error> {
        let invalid = || Error::Path {
            path: entry.path.clone(),
        };
        if entry.kind == "mod" {
            return Err(invalid());
        }
        let member = matches!(entry.kind.as_str(), "field" | "method")
            || entry.signature.starts_with("impl ");
        let path = if member {
            entry.path.rsplit_once("::").ok_or_else(invalid)?.0
        } else {
            &entry.path
        };
        let path = plain_path(path);
        syn::parse_str::<syn::Path>(&path).map_err(|_| invalid())?;
        if !member {
            return Ok(path);
        }
        let name = path.rsplit("::").next().ok_or_else(invalid)?;
        // An inherent impl in a child module names a type imported from its parent. Resolve to
        // the nearest enclosing declaration rather than re-exporting that private import.
        let mut scope = path.rsplit_once("::").ok_or_else(invalid)?.0;
        loop {
            if entry.kind == "field"
                && self
                    .internals
                    .iter()
                    .any(|entry| entry.path == scope && entry.kind == "enum" && !entry.shape_only)
            {
                return Ok(scope.to_owned());
            }
            let candidate = format!("{scope}::{name}");
            if self.internals.iter().any(|entry| {
                entry.path == candidate
                    && matches!(entry.kind.as_str(), "struct" | "enum" | "type" | "trait")
                    && !entry.shape_only
            }) {
                return Ok(candidate);
            }
            let Some((parent, _)) = scope.rsplit_once("::") else {
                break;
            };
            scope = parent;
        }
        Err(invalid())
    }
}

/// Generic arguments belong to a fingerprint, not to a use path or a site target's identity.
pub(super) fn plain_path(path: &str) -> String {
    let mut depth = 0;
    path.chars()
        .filter(|&c| match c {
            '<' => {
                depth += 1;
                false
            }
            '>' => {
                depth -= 1;
                false
            }
            _ => depth == 0 && !c.is_whitespace(),
        })
        .collect()
}

#[derive(Default)]
struct Module {
    items: BTreeSet<String>,
    children: BTreeMap<String, Module>,
}
impl Module {
    fn export(&mut self, path: &str, crates: &mut BTreeSet<String>) {
        let parts: Vec<_> = path.split("::").collect();
        crates.insert(parts[0].to_owned());
        // The game is the facade's root; lower crates retain their own namespace.
        let parts = if parts[0] == "horizon_client" {
            &parts[1..]
        } else {
            &parts[..]
        };
        let mut module = self;
        for part in &parts[..parts.len() - 1] {
            module = module.children.entry((*part).to_owned()).or_default();
        }
        module.items.insert(format!("pub use {path};"));
    }

    fn write(&self, text: &mut String, depth: usize) {
        let indent = "    ".repeat(depth);
        for item in &self.items {
            text.push_str(&format!("{indent}{item}\n"));
        }
        for (name, child) in &self.children {
            text.push_str(&format!("{indent}pub mod {name} {{\n"));
            child.write(text, depth + 1);
            text.push_str(&format!("{indent}}}\n"));
        }
    }
}

pub(super) fn needed(manifest: &horizon_mod_package::Manifest) -> bool {
    !manifest.internals.is_empty()
}

pub(super) fn package(id: &str) -> String {
    format!("horizon-internals-{id}")
}
pub(super) fn relative(id: &str) -> String {
    format!("native-internals/{id}")
}

pub(super) fn stage(
    stage: &Path,
    project: &api::Project<'_>,
    facade: &Facade,
) -> Result<(), String> {
    let manifest = &project.bundle.manifest;
    let dest = stage.join(relative(&manifest.id));
    fs::create_dir_all(dest.join("src")).map_err(|e| e.to_string())?;
    let mut cargo = format!("[package]\nname = {:?}\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\nname = \"internals\"\n[dependencies]\nhorizon-client = {{ path = \"../../crates/horizon-client\", default-features = false }}\n", package(&manifest.id));
    for krate in &facade.crates {
        if krate != "horizon_client" {
            let package = krate.replace('_', "-");
            cargo.push_str(&format!(
                "{krate} = {{ package = {package:?}, path = \"../../crates/{package}\" }}\n"
            ));
        }
    }
    fs::write(dest.join("Cargo.toml"), cargo).map_err(|e| e.to_string())?;
    fs::write(dest.join("src/lib.rs"), &facade.source).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> (std::path::PathBuf, super::super::Bundle, Catalog) {
        let dir = super::super::tests::dir(&format!("internals-{name}"));
        super::super::tests::write_bundle(&dir, "example", "");
        fs::write(dir.join("Cargo.toml"), "[package]\nname='example'\nversion='1.0.0'\nedition='2024'\n[dependencies]\nhorizon-mod-api={path='.horizon/sdk/api/crates/horizon-mod-api'}\n").unwrap();
        let path = dir.join(horizon_mod_package::manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap().replace("[native]", "internals = [{ path = 'horizon_client::game::Game', hash = '1234' }, { path = 'horizon_client::game::maps::dimension_id', hash = '5678' }]\n[native]\n");
        fs::write(path, text).unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        let catalog = Catalog {
            points: Vec::new(),
            discovery: false,
            internals: serde_json::from_value(serde_json::json!([
                { "path": "horizon_client::game::Game", "kind": "struct", "signature": "struct Game<'a> { world: World }", "hash": "1234" },
                { "path": "horizon_client::game::maps::dimension_id", "kind": "fn", "signature": "fn dimension_id() -> u8", "hash": "5678" },
                { "path": "horizon_client::game::maps::Game<'_>::dimension_id", "kind": "method", "signature": "impl Game<'_> { fn dimension_id(&self) -> u8 }", "hash": "abcd" },
                { "path": "horizon_client::game::Game<'_>::tick", "kind": "method", "signature": "fn tick(&mut self)", "hash": "tick-hash" }
            ])).unwrap(),
        };
        (dir, bundle, catalog)
    }

    #[test]
    fn reads_and_pins_only_declared_internals() {
        let (dir, bundle, catalog) = fixture("pins");
        let build = serde_json::json!({"commit": "c".repeat(40)});
        fs::write(
            dir.join(INTERNALS),
            serde_json::json!({"schema": 1, "build": build, "internals": catalog.internals})
                .to_string(),
        )
        .unwrap();
        fs::write(
            dir.join(inject::POINTS),
            serde_json::json!({"schema": 1, "build": build, "points": [], "wire": []}).to_string(),
        )
        .unwrap();
        let read = Catalog::read(&dir).unwrap();
        let mut manifest = bundle.manifest.clone();
        manifest.internals[0].hash = None;
        manifest.internals[1].hash = Some("explicit".into());
        let pinned = read.pinned(&manifest);
        assert_eq!(pinned.internals[0].hash.as_deref(), Some("1234"));
        assert_eq!(pinned.internals[1].hash.as_deref(), Some("explicit"));
        let mut valid = pinned;
        valid.internals[1].hash = Some("5678".into());
        read.check(&valid).unwrap();
        let source = read.facade(&valid).unwrap().source;
        assert!(source.contains("pub use horizon_client::game::Game;"));
        assert!(source.contains("pub use horizon_client::game::maps::dimension_id;"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn freshness_refuses_missing_changed_gated_and_shape_only_entries() {
        let (dir, bundle, mut catalog) = fixture("freshness");
        let manifest = &bundle.manifest;
        catalog.check(manifest).unwrap();
        catalog.internals[0].hash = "new".into();
        let error = catalog.check(manifest).unwrap_err();
        assert!(matches!(error, Error::Changed { .. }));
        assert!(error.to_string().contains("E_INTERNAL_CHANGED"));
        assert!(error
            .to_string()
            .contains("struct Game<'a> { world: World }"));
        catalog.internals[0].cfg = "feature = \"controllers\"".into();
        assert!(matches!(catalog.check(manifest), Err(Error::Gated { .. })));
        assert!(catalog
            .check(manifest)
            .unwrap_err()
            .to_string()
            .contains("E_INTERNAL_GATED"));
        catalog.internals[0].shape_only = true;
        assert!(matches!(
            catalog.check(manifest),
            Err(Error::Missing { .. })
        ));
        catalog.internals.remove(0);
        assert!(matches!(
            catalog.check(manifest),
            Err(Error::Missing { .. })
        ));
        assert!(catalog
            .check(manifest)
            .unwrap_err()
            .to_string()
            .contains("E_INTERNAL_MISSING"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn member_kind_reaches_the_facade_and_ambiguity_is_translated() {
        let (dir, mut bundle, mut catalog) = fixture("member-kind");
        let method = catalog.internals[2].clone();
        let mut field = method.clone();
        field.kind = "field".into();
        field.hash = "field".into();
        field.signature = "u8".into();
        catalog.internals.insert(2, field);
        bundle.manifest.internals[1] = horizon_mod_package::manifest::Internal {
            path: method.path.clone(),
            kind: Some("method".into()),
            hash: Some(method.hash),
        };
        let facade = catalog.facade(&bundle.manifest).unwrap();
        assert!(facade
            .source
            .contains("pub use horizon_client::game::Game;"));
        bundle.manifest.internals[1].hash = Some("field".into());
        assert!(
            matches!(catalog.check(&bundle.manifest), Err(Error::Changed { signature, .. }) if signature == method.signature)
        );
        bundle.manifest.internals[1].kind = None;
        let error = catalog.check(&bundle.manifest).unwrap_err();
        assert!(matches!(error, Error::Ambiguous { .. }));
        let text = error.to_string();
        assert!(text.contains("E_INTERNAL_AMBIGUOUS"), "{text}");
        assert!(text.contains(&method.path), "{text}");
        assert!(text.contains("field, method"), "{text}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn direct_game_dependencies_are_refused_in_every_cargo_dependency_table() {
        let (dir, bundle, _) = fixture("game-dependency");
        let path = dir.join("Cargo.toml");
        let original = fs::read_to_string(&path).unwrap();
        for table in [
            "dependencies",
            "dev-dependencies",
            "build-dependencies",
            "target.'cfg(unix)'.dependencies",
            "target.'cfg(unix)'.build-dependencies",
        ] {
            let extra = "game = { package = 'horizon-client', version = '0.1' }\n";
            let text = if table == "dependencies" {
                format!("{original}{extra}")
            } else {
                format!("{original}[{table}]\n{extra}")
            };
            fs::write(&path, text).unwrap();
            let Err(error) = api::Project::read(&bundle) else {
                panic!("accepted {table}");
            };
            assert!(matches!(error, api::ProjectError::GameDependency { .. }));
            assert!(error.to_string().contains("E_GAME_DEPENDENCY"));
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
