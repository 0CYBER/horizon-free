//! One dependency plan shared by source staging and immutable prebuilt SDK builds.
use super::{
    api::Project,
    crates::Wanted,
    registry_lock::{ModEdges, RegistryLock},
};
use std::path::Path;

pub(super) struct Plan {
    pub game: RegistryLock,
    pub mods: RegistryLock,
    pub allowed: RegistryLock,
    owned: Vec<(String, String, RegistryLock)>,
    roots: std::collections::BTreeSet<String>,
}

impl Plan {
    pub fn read(game: &str, projects: &[Project<'_>], wanted: &[Wanted]) -> Result<Self, String> {
        let game = RegistryLock::parse(game)?;
        let mut mods = RegistryLock::default();
        let mut owned = Vec::new();
        let roots = projects
            .iter()
            .map(|project| project.package.clone())
            .collect();
        for project in projects {
            let bundle = project.bundle;
            let requested: Vec<_> = wanted
                .iter()
                .filter(|w| w.mod_id == bundle.manifest.id)
                .collect();
            if requested.is_empty() {
                continue;
            }
            let pins = RegistryLock::parse(&super::read_lock(&bundle.dir)?)?;
            for request in requested {
                if !pins
                    .packages
                    .keys()
                    .any(|id| id.name == request.name && request.requirement.matches(&id.version))
                {
                    return Err(format!(
                        "{}: Cargo.lock does not pin {} {}",
                        request.mod_id, request.name, request.version
                    ));
                }
            }
            mods.extend(&pins)?;
            owned.push((bundle.manifest.id.clone(), project.package.clone(), pins));
        }
        let mut allowed = game.clone();
        allowed.extend(&mods)?;
        Ok(Self {
            game,
            mods,
            allowed,
            owned,
            roots,
        })
    }

    /// Patches come only from the validated SDK identity, never from a mod manifest.
    pub fn authorize_game_patches(&mut self, patches: &toml::Table) {
        let names = patches
            .values()
            .flat_map(|registry| {
                registry
                    .as_table()
                    .expect("validated SDK patch registry")
                    .keys()
                    .cloned()
            })
            .collect();
        self.allowed.allow_game_patch_edges(&self.game, &names);
    }

    /// Each mod's closure is pinned by the game or its own lock; the edges each adds to
    /// game-locked packages come only from its own lock, for packages it reaches.
    pub fn check(&self, resolved: &RegistryLock) -> Result<(), String> {
        let mut extra = ModEdges::new();
        for (id, root, pins) in &self.owned {
            let mut skip_local: std::collections::BTreeSet<String> = self
                .roots
                .iter()
                .filter(|other| *other != root)
                .cloned()
                .collect();
            skip_local.extend(
                super::api::PACKAGES
                    .iter()
                    .map(|(package, _)| (*package).to_owned()),
            );
            skip_local.insert(super::internals::package(id));
            let mut owned_pins = self.game.clone();
            owned_pins.extend(pins)?;
            resolved.check_mod(id, root, &skip_local, &owned_pins)?;
            resolved.mod_edges(id, root, &skip_local, pins, &self.game, &mut extra)?;
        }
        self.allowed
            .check_partial_resolution_with(resolved, &self.game, &extra)
    }

    pub fn explain_resolution(&self, error: String) -> String {
        let mut details = Vec::new();
        for id in self
            .mods
            .packages
            .keys()
            .filter(|id| error.contains(&id.name))
        {
            let versions = self
                .game
                .packages
                .keys()
                .filter(|game| game.name == id.name && game.version != id.version)
                .map(|game| game.version.to_string())
                .collect::<Vec<_>>();
            if !versions.is_empty() {
                details.push(format!(
                    "{}: game locks [{}], mod locks {}; the game graph must not move",
                    id.name,
                    versions.join(", "),
                    id.version
                ));
            }
        }
        if details.is_empty() {
            error
        } else {
            format!("{error}\n\n{}", details.join("\n"))
        }
    }

    pub fn fetch_with_patches(
        &self,
        rust: &crate::rust_toolchain::Toolchain,
        directory: &Path,
        patches: &toml::Table,
    ) -> Result<(), String> {
        // Resolve the validated mod workspace with its actual aliases, features
        // and defaults, rather than reconstructing package-only fetch roots.
        apply_patches(directory, patches)?;
        self.resolve_and_fetch(directory, |subcommand| {
            super::cargo(rust, directory, subcommand)
        })
    }

    fn resolve_and_fetch(
        &self,
        directory: &Path,
        mut cargo: impl FnMut(&str) -> std::process::Command,
    ) -> Result<(), String> {
        let mut index = cargo("update");
        index.arg("--workspace");
        super::run(index, "Checking the pinned registry index", |_| {})
            .map_err(|error| self.explain_resolution(error))?;
        let pins = RegistryLock::parse(&super::read_lock(directory)?)?;
        self.check(&pins)?;
        let mut fetch = cargo("fetch");
        fetch.arg("--locked");
        super::run(fetch, "Downloading the mods' pinned crates", |_| {})
    }
}

/// Only the SDK identity supplies patches; mod manifests cannot inject overrides.
pub(super) fn apply_patches(directory: &Path, patches: &toml::Table) -> Result<(), String> {
    let path = directory.join("Cargo.toml");
    let mut manifest: toml::Table = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())?
        .parse()
        .map_err(|e| format!("Cargo.toml: {e}"))?;
    manifest.insert("patch".into(), patches.clone().into());
    std::fs::write(path, manifest.to_string()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lock records `(name, version, checksum digit, edges)`; `' '` makes a local package.
    fn lock(packages: &[(&str, &str, char, &str)]) -> RegistryLock {
        let mut text = String::from("version=4\n");
        for (name, version, checksum, edges) in packages {
            text.push_str(&format!(
                "[[package]]\nname='{name}'\nversion='{version}'\ndependencies=[{edges}]\n"
            ));
            if *checksum != ' ' {
                text.push_str(&format!(
                    "source='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='{}'\n",
                    checksum.to_string().repeat(64)
                ));
            }
        }
        RegistryLock::parse(&text).unwrap()
    }

    /// A plan for staged mods whose root crate is named after the mod.
    fn plan(game: &RegistryLock, mods: &[(&str, RegistryLock)]) -> Plan {
        let mut all = RegistryLock::default();
        for (_, pins) in mods {
            all.extend(pins).unwrap();
        }
        let mut allowed = game.clone();
        allowed.extend(&all).unwrap();
        Plan {
            game: game.clone(),
            mods: all,
            allowed,
            owned: mods
                .iter()
                .map(|(id, pins)| (id.to_string(), id.to_string(), pins.clone()))
                .collect(),
            roots: mods.iter().map(|(id, _)| id.to_string()).collect(),
        }
    }

    #[test]
    fn a_mod_may_enable_features_of_a_crate_the_game_also_locks() {
        // The game locks `parent` with one optional dependency; the mod's own lock records
        // `parent` with another, which its features turn on.
        let game = lock(&[
            ("parent", "1.0.0", 'a', "'game-only'"),
            ("game-only", "1.0.0", 'b', ""),
            ("elsewhere", "1.0.0", 'c', ""),
        ]);
        let pins = lock(&[
            ("mod_a", "0.0.0", ' ', "'parent'"),
            ("parent", "1.0.0", 'a', "'mod-only'"),
            ("mod-only", "1.0.0", 'f', ""),
        ]);
        let with_lock = plan(&game, &[("mod_a", pins)]);
        let resolved = |parent: &str, extra: &[(&str, &str, char, &str)]| {
            let mut packages = vec![
                ("mod_a", "0.0.0", ' ', "'parent'"),
                ("parent", "1.0.0", 'a', parent),
                ("mod-only", "1.0.0", 'f', ""),
            ];
            packages.extend_from_slice(extra);
            lock(&packages)
        };
        with_lock.check(&resolved("'mod-only'", &[])).unwrap();
        let both = resolved(
            "'game-only', 'mod-only'",
            &[("game-only", "1.0.0", 'b', "")],
        );
        with_lock.check(&both).unwrap();
        // Without the mod's lock recording the edge, the same graph is refused.
        let without = plan(
            &game,
            &[(
                "mod_a",
                lock(&[
                    ("mod_a", "0.0.0", ' ', "'parent'"),
                    ("parent", "1.0.0", 'a', ""),
                    ("mod-only", "1.0.0", 'f', ""),
                ]),
            )],
        );
        let error = without.check(&resolved("'mod-only'", &[])).unwrap_err();
        assert!(error.contains("parent"), "{error}");
        // An edge no lock records, even to a package the game pins.
        let error = with_lock
            .check(&resolved(
                "'mod-only', 'elsewhere'",
                &[("elsewhere", "1.0.0", 'c', "")],
            ))
            .unwrap_err();
        assert!(error.contains("parent"), "{error}");
        // The game's package at another version, or with another checksum.
        for (version, checksum) in [("1.0.1", 'a'), ("1.0.0", '9')] {
            let moved = lock(&[
                ("mod_a", "0.0.0", ' ', "'parent'"),
                ("parent", version, checksum, ""),
            ]);
            let error = with_lock.check(&moved).unwrap_err();
            assert!(error.contains("parent"), "{error}");
        }
    }

    #[test]
    fn a_mod_lock_cannot_move_a_dependency_the_game_resolved() {
        let game = lock(&[
            ("parent", "1.0.0", 'a', "'child'"),
            ("child", "1.0.0", 'b', ""),
        ]);
        let pins = lock(&[
            ("mod_a", "0.0.0", ' ', "'parent'"),
            ("parent", "1.0.0", 'a', "'child'"),
            ("child", "1.1.0", 'c', ""),
        ]);
        let resolved = lock(&[
            ("mod_a", "0.0.0", ' ', "'parent'"),
            ("parent", "1.0.0", 'a', "'child'"),
            ("child", "1.1.0", 'c', ""),
        ]);
        let error = plan(&game, &[("mod_a", pins)])
            .check(&resolved)
            .unwrap_err();
        assert!(error.contains("parent"), "{error}");
    }

    #[test]
    fn a_mod_lock_cannot_add_an_edge_onto_an_sdk_patched_crate() {
        let game = lock(&[("parent", "1.0.0", 'a', ""), ("naga", "30.0.1", ' ', "")]);
        let pins = lock(&[
            ("mod_a", "0.0.0", ' ', "'parent'"),
            ("parent", "1.0.0", 'a', "'naga'"),
            ("naga", "30.0.1", ' ', ""),
        ]);
        let resolved = lock(&[
            ("mod_a", "0.0.0", ' ', "'parent'"),
            ("parent", "1.0.0", 'a', "'naga'"),
            ("naga", "30.0.1", ' ', ""),
        ]);
        let error = plan(&game, &[("mod_a", pins)])
            .check(&resolved)
            .unwrap_err();
        assert!(error.contains("parent"), "{error}");
        // The shape the lock generator writes: crates.io `naga`, which the staged resolution
        // replaces with the SDK's patched copy. The recorded edge is not the resolved one.
        let pins = lock(&[
            ("mod_a", "0.0.0", ' ', "'parent'"),
            ("parent", "1.0.0", 'a', "'naga'"),
            ("naga", "30.0.1", 'e', ""),
        ]);
        let error = plan(&game, &[("mod_a", pins)])
            .check(&resolved)
            .unwrap_err();
        assert!(error.contains("parent"), "{error}");
    }

    #[test]
    fn a_mods_lock_counts_only_for_packages_its_own_crate_reaches() {
        let game = lock(&[("parent", "1.0.0", 'a', ""), ("child", "1.0.0", 'b', "")]);
        let mods = [
            (
                "mod_a",
                lock(&[
                    ("mod_a", "0.0.0", ' ', "'parent'"),
                    ("parent", "1.0.0", 'a', ""),
                ]),
            ),
            (
                "mod_b",
                lock(&[
                    ("mod_b", "0.0.0", ' ', "'parent'"),
                    ("parent", "1.0.0", 'a', "'child'"),
                    ("child", "1.0.0", 'b', ""),
                ]),
            ),
        ];
        let resolved = |b: &str| {
            lock(&[
                ("mod_a", "0.0.0", ' ', "'parent'"),
                ("mod_b", "0.0.0", ' ', b),
                ("parent", "1.0.0", 'a', "'child'"),
                ("child", "1.0.0", 'b', ""),
            ])
        };
        // mod_b's record of `parent` counts only when mod_b's own crate reaches `parent`.
        let error = plan(&game, &mods).check(&resolved("")).unwrap_err();
        assert!(error.contains("parent"), "{error}");
        plan(&game, &mods).check(&resolved("'parent'")).unwrap();
    }

    /// Both mods reach `parent` through `gate` in the shared workspace, but only mod_a turns on
    /// `gate`'s dependency on it, and mod_a's lock leaves out `parent -> child`. mod_b's lock
    /// lists that edge on a `parent` record nothing in its own lock leads to.
    #[test]
    fn an_orphan_record_in_one_mods_lock_authorises_nothing() {
        let game = lock(&[
            ("gate", "1.0.0", 'a', "'parent'"),
            ("parent", "1.0.0", 'b', ""),
            ("child", "1.0.0", 'c', ""),
        ]);
        let mods = [
            (
                "mod_a",
                lock(&[
                    ("mod_a", "0.0.0", ' ', "'gate'"),
                    ("gate", "1.0.0", 'a', "'parent'"),
                    ("parent", "1.0.0", 'b', ""),
                ]),
            ),
            (
                "mod_b",
                lock(&[
                    ("mod_b", "0.0.0", ' ', "'gate'"),
                    ("gate", "1.0.0", 'a', ""),
                    ("parent", "1.0.0", 'b', "'child'"),
                    ("child", "1.0.0", 'c', ""),
                ]),
            ),
        ];
        let resolved = lock(&[
            ("mod_a", "0.0.0", ' ', "'gate'"),
            ("mod_b", "0.0.0", ' ', "'gate'"),
            ("gate", "1.0.0", 'a', "'parent'"),
            ("parent", "1.0.0", 'b', "'child'"),
            ("child", "1.0.0", 'c', ""),
        ]);
        let error = plan(&game, &mods).check(&resolved).unwrap_err();
        assert!(error.contains("parent"), "{error}");
    }
    #[test]
    fn sdk_partial_graph_accepts_inactive_edges_but_refuses_new_registry_edges() {
        let original = format!("version=4\n[[package]]\nname='game'\nversion='1.0.0'\ndependencies=['log']\n[[package]]\nname='log'\nversion='0.4.0'\nsource='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='{}'\n", "a".repeat(64));
        let plan = Plan::read(&original, &[], &[]).unwrap();
        let narrower = original.replace("dependencies=['log']", "dependencies=[]");
        plan.allowed
            .check_partial_resolution(&RegistryLock::parse(&narrower).unwrap(), &plan.game)
            .unwrap();
        let added = format!("{original}dependencies=['game']\n");
        assert!(plan
            .allowed
            .check_partial_resolution(&RegistryLock::parse(&added).unwrap(), &plan.game)
            .is_err());
    }

    #[test]
    fn production_plan_preserves_isolated_game_patch_identity() {
        let original = "version=4\n[[package]]\nname='allocator-sys'\nversion='0.1.49'\n";
        let plan = Plan::read(original, &[], &[]).unwrap();
        plan.allowed
            .check_resolution(
                &RegistryLock::parse(original).unwrap(),
                &RegistryLock::default(),
            )
            .unwrap();
        let moved = RegistryLock::parse(&original.replace("0.1.49", "0.1.50")).unwrap();
        // Both fetch and SDK compilation use allowed against resolved, with an empty game argument.
        let error = plan
            .allowed
            .check_resolution(&moved, &RegistryLock::default())
            .unwrap_err();
        assert!(error.contains("allocator-sys"), "{error}");
    }

    #[test]
    fn cargo_resolves_registry_parent_through_sdk_local_patch_without_compiling() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for path in [
            "vendor/allocator/src",
            "sdk/allocator-sys/src",
            "src",
            ".cargo",
        ] {
            std::fs::create_dir_all(root.join(path)).unwrap();
        }
        for (path, text) in [
            ("Cargo.toml", "[package]\nname='fixture'\nversion='1.0.0'\n[workspace]\n[dependencies]\nallocator='1'\n"),
            ("src/lib.rs", ""),
            ("vendor/allocator/Cargo.toml", "[package]\nname='allocator'\nversion='1.0.0'\n[dependencies]\nallocator-sys='0.1'\n"),
            ("vendor/allocator/src/lib.rs", ""),
            ("sdk/allocator-sys/Cargo.toml", "[package]\nname='allocator-sys'\nversion='0.1.49'\n"),
            ("sdk/allocator-sys/src/lib.rs", ""),
            (".cargo/config.toml", "[source.crates-io]\nreplace-with='fixture'\n[source.fixture]\ndirectory='vendor'\n"),
        ] { std::fs::write(root.join(path), text).unwrap(); }
        std::fs::write(
            root.join("vendor/allocator/.cargo-checksum.json"),
            format!(r#"{{"package":"{}","files":{{}}}}"#, "a".repeat(64)),
        )
        .unwrap();
        let patches: toml::Table = format!(
            "[crates-io]\nallocator-sys={{path={:?}}}\n",
            root.join("sdk/allocator-sys").to_string_lossy()
        )
        .parse()
        .unwrap();
        apply_patches(root, &patches).unwrap();
        let output = std::process::Command::new("cargo")
            .args(["generate-lockfile", "--offline"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        let game = format!("version=4\n[[package]]\nname='allocator'\nversion='1.0.0'\nsource='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='{}'\ndependencies=['allocator-sys']\n[[package]]\nname='allocator-sys'\nversion='0.1.49'\n", "a".repeat(64));
        RegistryLock::parse(&game)
            .unwrap()
            .check_resolution(
                &RegistryLock::parse(&text).unwrap(),
                &RegistryLock::default(),
            )
            .unwrap();
        assert!(!root.join("target").exists());
    }

    #[test]
    fn mod_only_registry_edge_can_use_the_same_version_sdk_naga_patch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for path in ["vendor/mod-parent/src", "sdk/naga/src", "src", ".cargo"] {
            std::fs::create_dir_all(root.join(path)).unwrap();
        }
        for (path, text) in [
            ("Cargo.toml", "[package]\nname='fixture'\nversion='1.0.0'\n[workspace]\n[dependencies]\nmod-parent='1'\n"),
            ("src/lib.rs", ""),
            ("vendor/mod-parent/Cargo.toml", "[package]\nname='mod-parent'\nversion='1.0.0'\n[dependencies]\nnaga='30'\n"),
            ("vendor/mod-parent/src/lib.rs", ""),
            ("sdk/naga/Cargo.toml", "[package]\nname='naga'\nversion='30.0.1'\n"),
            ("sdk/naga/src/lib.rs", ""),
            (".cargo/config.toml", "[source.crates-io]\nreplace-with='fixture'\n[source.fixture]\ndirectory='vendor'\n"),
        ] { std::fs::write(root.join(path), text).unwrap(); }
        std::fs::write(
            root.join("vendor/mod-parent/.cargo-checksum.json"),
            format!(r#"{{"package":"{}","files":{{}}}}"#, "a".repeat(64)),
        )
        .unwrap();
        let patches: toml::Table = format!(
            "[crates-io]\nnaga={{path={:?}}}\n",
            root.join("sdk/naga").to_string_lossy()
        )
        .parse()
        .unwrap();
        // First obtain the mod's independent registry lock, without SDK patches.
        std::fs::create_dir_all(root.join("vendor/naga/src")).unwrap();
        std::fs::write(
            root.join("vendor/naga/Cargo.toml"),
            "[package]\nname='naga'\nversion='30.0.1'\n",
        )
        .unwrap();
        std::fs::write(root.join("vendor/naga/src/lib.rs"), "").unwrap();
        std::fs::write(
            root.join("vendor/naga/.cargo-checksum.json"),
            format!(r#"{{"package":"{}","files":{{}}}}"#, "b".repeat(64)),
        )
        .unwrap();
        let original = std::process::Command::new("cargo")
            .args(["generate-lockfile", "--offline"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            original.status.success(),
            "{}",
            String::from_utf8_lossy(&original.stderr)
        );
        let mod_lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        let pins = RegistryLock::parse(&mod_lock).unwrap();
        apply_patches(root, &patches).unwrap();
        let output = std::process::Command::new("cargo")
            .args(["generate-lockfile", "--offline"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        let resolved = RegistryLock::parse(&text).unwrap();
        let game_lock = "version=4\n[[package]]\nname='naga'\nversion='30.0.1'\n";
        let mut plan = Plan::read(game_lock, &[], &[]).unwrap();
        plan.allowed.extend(&pins).unwrap();
        let before = plan
            .allowed
            .check_resolution(&resolved, &RegistryLock::default())
            .unwrap_err();
        assert!(before.contains("mod-parent"), "{before}");
        plan.authorize_game_patches(&patches);
        plan.allowed
            .check_resolution(&resolved, &RegistryLock::default())
            .unwrap();
        // Authorization is directional and version-exact: neither re-sourcing the
        // SDK package nor actually moving its version is permission to change it.
        for bad in [&mod_lock, &text.replace("30.0.1", "30.0.2")] {
            assert!(plan
                .allowed
                .check_resolution(&RegistryLock::parse(bad).unwrap(), &RegistryLock::default())
                .is_err());
        }
        // A game-owned parent's original edges remain strict, too.
        let mut game_parent = Plan::read(&text, &[], &[]).unwrap();
        game_parent.allowed.extend(&pins).unwrap();
        game_parent.authorize_game_patches(&patches);
        assert!(game_parent
            .allowed
            .check_resolution(&pins, &RegistryLock::default())
            .is_err());
        println!("ORIGINAL MOD LOCK:\n{mod_lock}\nSDK PATCHED RESOLUTION:\n{text}\nOLD CHECK REFUSAL: {before}\nAUTHORIZED CHECK: accepted");
        assert!(!root.join("target").exists());
    }

    #[test]
    fn cargo_optional_game_edges_may_shrink_but_not_grow_or_move() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in ["bumpalo", "allocator-api2", "unexpected"] {
            let crate_dir = root.join("vendor").join(name);
            std::fs::create_dir_all(crate_dir.join("src")).unwrap();
            let extra = if name == "bumpalo" {
                "[dependencies]\nallocator-api2={version='1',optional=true}\nunexpected={version='1',optional=true}\n"
            } else {
                ""
            };
            std::fs::write(
                crate_dir.join("Cargo.toml"),
                format!("[package]\nname='{name}'\nversion='1.0.0'\n{extra}"),
            )
            .unwrap();
            std::fs::write(crate_dir.join("src/lib.rs"), "").unwrap();
            std::fs::write(
                crate_dir.join(".cargo-checksum.json"),
                format!(r#"{{"package":"{}","files":{{}}}}"#, "a".repeat(64)),
            )
            .unwrap();
        }
        std::fs::create_dir_all(root.join(".cargo")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        std::fs::write(
            root.join(".cargo/config.toml"),
            "[source.crates-io]\nreplace-with='fixture'\n[source.fixture]\ndirectory='vendor'\n",
        )
        .unwrap();
        let resolve = |features: &str| {
            std::fs::write(root.join("Cargo.toml"), format!("[package]\nname='fixture'\nversion='1.0.0'\n[workspace]\n[dependencies]\nbumpalo={{version='1',default-features=false,features=[{features}]}}\n")).unwrap();
            let output = std::process::Command::new("cargo")
                .args(["generate-lockfile", "--offline"])
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            std::fs::read_to_string(root.join("Cargo.lock")).unwrap()
        };
        let game = resolve("'allocator-api2'");
        let plan = Plan::read(&game, &[], &[]).unwrap();
        let narrower = resolve("");
        let narrower = RegistryLock::parse(&narrower).unwrap();
        plan.allowed
            .check_partial_resolution(&narrower, &plan.game)
            .unwrap();
        // Without game ownership a mod's declared graph remains exact.
        assert!(plan
            .allowed
            .check_partial_resolution(&narrower, &RegistryLock::default())
            .unwrap_err()
            .contains("bumpalo"));
        let added = resolve("'unexpected'");
        assert!(plan
            .allowed
            .check_partial_resolution(&RegistryLock::parse(&added).unwrap(), &plan.game)
            .unwrap_err()
            .contains("bumpalo"));
        // A mod whose own lock records that feature compiles its own bumpalo with it.
        let added = RegistryLock::parse(&added).unwrap();
        let mut with_mod = Plan::read(&game, &[], &[]).unwrap();
        with_mod.allowed.extend(&added).unwrap();
        with_mod
            .owned
            .push(("example".into(), "fixture".into(), added.clone()));
        with_mod.check(&added).unwrap();
        let moved = game.replace(
            "name = \"bumpalo\"\nversion = \"1.0.0\"",
            "name = \"bumpalo\"\nversion = \"1.0.1\"",
        );
        assert_ne!(moved, game);
        assert!(plan
            .allowed
            .check_partial_resolution(&RegistryLock::parse(&moved).unwrap(), &plan.game)
            .unwrap_err()
            .contains("bumpalo"));
        assert!(with_mod
            .check(&RegistryLock::parse(&moved).unwrap())
            .unwrap_err()
            .contains("bumpalo"));
    }

    #[test]
    fn fetch_retains_actual_feature_closure_and_excludes_unreachable_pins() {
        const ACTOR: &str = "HORIZON_REGISTRY_FETCH_TEST_ACTOR";
        if std::env::var_os(ACTOR).is_none() {
            let home = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "native_mods::registry_plan::tests::fetch_retains_actual_feature_closure_and_excludes_unreachable_pins", "--nocapture"])
                .env(ACTOR, "1")
                .env("HORIZON_LAUNCHER_HOME", home.path())
                .output().unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{stdout}\n{stderr}");
            assert!(
                stdout.contains("test result: ok. 1 passed;"),
                "exact-name child did not run one test:\n{stdout}\n{stderr}"
            );
            return;
        }
        assert!(
            !super::super::log_path().exists(),
            "the isolated actor must start with a fresh home"
        );
        // Production build entries call logged()/clear_log(); native-lock also
        // creates root() before resolution. This direct unit call bypasses them.
        super::super::clear_log().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in ["feature-parent", "feature-only"] {
            let folder = root.join("vendor").join(name);
            std::fs::create_dir_all(folder.join("src")).unwrap();
            let extra = if name == "feature-parent" {
                "[dependencies]\nfeature-only={version='1',optional=true}\n[features]\ndefault=['feature-only']\n"
            } else {
                ""
            };
            std::fs::write(
                folder.join("Cargo.toml"),
                format!("[package]\nname='{name}'\nversion='1.0.0'\n{extra}"),
            )
            .unwrap();
            std::fs::write(folder.join("src/lib.rs"), "").unwrap();
            std::fs::write(
                folder.join(".cargo-checksum.json"),
                format!(r#"{{"package":"{}","files":{{}}}}"#, "a".repeat(64)),
            )
            .unwrap();
        }
        std::fs::create_dir_all(root.join(".cargo")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        std::fs::write(
            root.join(".cargo/config.toml"),
            "[source.crates-io]\nreplace-with='fixture'\n[source.fixture]\ndirectory='vendor'\n",
        )
        .unwrap();
        // Alias, explicit feature and default-feature semantics are left intact.
        let manifest = "[package]\nname='fixture'\nversion='1.0.0'\n[workspace]\n[dependencies]\nalias={package='feature-parent',version='1',default-features=false,features=['feature-only']}\n";
        std::fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let cargo = |subcommand: &str| {
            let mut command = std::process::Command::new("cargo");
            command
                .arg(subcommand)
                .arg("--offline")
                .current_dir(root)
                .env("CARGO_HOME", root.join("cold-cargo-home"));
            command
        };
        let generated = cargo("generate-lockfile").output().unwrap();
        assert!(
            generated.status.success(),
            "{}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let game = format!("{}\n[[package]]\nname='outside-closure'\nversion='1.0.0'\nsource='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='{}'\n", std::fs::read_to_string(root.join("Cargo.lock")).unwrap(), "b".repeat(64));
        std::fs::write(root.join("Cargo.lock"), &game).unwrap();
        let plan = Plan::read(&game, &[], &[]).unwrap();
        plan.resolve_and_fetch(root, cargo).unwrap();
        let lock = RegistryLock::parse(&std::fs::read_to_string(root.join("Cargo.lock")).unwrap())
            .unwrap();
        assert!(lock.packages.keys().any(|id| id.name == "feature-only"));
        assert!(!lock.packages.keys().any(|id| id.name == "outside-closure"));
        // No source for outside-closure exists: eagerly fetching the union would fail.
        assert!(!root.join("vendor/outside-closure").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("Cargo.toml")).unwrap(),
            manifest
        );
        let metadata = cargo("metadata")
            .args(["--locked", "--format-version", "1"])
            .output()
            .unwrap();
        assert!(
            metadata.status.success(),
            "{}",
            String::from_utf8_lossy(&metadata.stderr)
        );
        assert!(!root.join("target").exists());
    }

    #[test]
    fn missing_malformed_and_bad_checksum_locks_refuse_before_fetch() {
        let dir = tempfile::tempdir().unwrap();
        super::super::tests::write_bundle(dir.path(), "example", "");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='example'\nversion='1.0.0'\n[dependencies]\nhorizon-mod-api={path='.horizon/sdk/api/crates/horizon-mod-api'}\nitoa='1'\n").unwrap();
        let bundle = super::super::bundle_in(dir.path()).unwrap().unwrap();
        let projects = [Project::read(&bundle).unwrap()];
        let wanted = [Wanted {
            mod_id: "example".into(),
            name: "itoa".into(),
            version: "1".into(),
            requirement: semver::VersionReq::parse("1").unwrap(),
        }];
        let game = "version=4\npackage=[]\n";
        let missing = Plan::read(game, &projects, &wanted).err().unwrap();
        assert!(missing.contains("Cargo.lock"), "{missing}");
        for (text, expected) in [("version=4", "missing packages"), ("version=4\n[[package]]\nname='itoa'\nversion='1.0.18'\nsource='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='bad'\n", "checksum")] {
            std::fs::write(dir.path().join("Cargo.lock"), text).unwrap();
            let error = Plan::read(game, &projects, &wanted).err().unwrap();
            assert!(error.contains(expected), "{error}");
        }
        assert!(!dir.path().join("target").exists());
    }
}
