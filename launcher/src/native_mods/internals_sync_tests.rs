use super::*;
use crate::native_mods::{tests::dir, tests::write_bundle};
use std::cell::Cell;

const STRUCT: &str = "horizon_client::game::ported_level::PortedGame";
const METHOD: &str = "horizon_client::game::ported_level::PortedGame::tick";
const PAYLOAD: &str =
    "pub struct PortedGame; impl PortedGame { pub fn tick(&mut self) -> u32 { 7 } }";

struct Fixture {
    root: PathBuf,
    folder: PathBuf,
    source: PathBuf,
}
impl Fixture {
    fn new(name: &str, modules: bool) -> Self {
        let root = dir(name);
        let folder = root.join("mod");
        write_bundle(&folder, "example", "");
        fs::write(folder.join("Cargo.toml"), "[package]\nname='example'\nversion='0.1.0'\n[dependencies]\nmod_api={package='horizon-mod-api',path='.horizon/sdk/api/crates/horizon-mod-api'}\n").unwrap();
        if modules {
            let manifest = folder.join(horizon_mod_package::manifest::MANIFEST);
            let text = fs::read_to_string(&manifest).unwrap();
            fs::write(manifest, format!("{text}\n[[native.module]]\nid='ported'\nfile='game.rs'\nname='ported_level'\nsource='src/ported.rs'\n")).unwrap();
            fs::write(folder.join("src/ported.rs"), PAYLOAD).unwrap();
        }
        let source = root.join("source");
        for (file, text) in [
            ("Cargo.toml", "[workspace]\nmembers=[]\n"),
            (
                "crates/horizon-client/Cargo.toml",
                "[package]\nname='horizon-client'\n",
            ),
            ("crates/horizon-client/src/lib.rs", "mod game; mod entry;"),
            ("crates/horizon-client/src/entry.rs", ""),
            ("crates/horizon-client/src/game.rs", ""),
            ("crates/horizon-mod-api-gen/internals.txt", ""),
            (
                "crates/horizon-engine-slots/Cargo.toml",
                "[package]\nname='horizon-engine-slots'\n",
            ),
            ("crates/horizon-engine-slots/src/lib.rs", ""),
            (
                "crates/horizon-mod-points/Cargo.toml",
                "[package]\nname='horizon-mod-points'\n",
            ),
        ] {
            let path = source.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        Self {
            root,
            folder,
            source,
        }
    }

    fn catalog(&self, calls: &Cell<usize>) -> Result<PathBuf, Error> {
        catalog_for_mod_with(
            &self.folder,
            &self.root.join("cache"),
            |_| Ok(self.source.clone()),
            |bundle, prepared, input, output| {
                calls.set(calls.get() + 1);
                assert_eq!(bundle.manifest.id, "example");
                // This is the external tool seam only. Production code prepares the real tool
                // input and pristine source, and runs the actual discovery staging afterwards.
                assert_eq!(
                    fs::read_to_string(prepared.join("crates/horizon-client/src/game.rs")).unwrap(),
                    ""
                );
                let payload = fs::read_to_string(input.join("example/src/ported.rs")).unwrap();
                assert!(input.join("example/horizon-mixin.toml").is_file());
                super::super::sync(prepared, output)?;
                fs::write(
                    output.join("crates/horizon-client/src/game.rs"),
                    format!("pub mod ported_level {{ {payload} }}"),
                )
                .map_err(|e| e.to_string())
            },
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn pinned_module_catalog_uses_canonical_paths_and_fresh_source_twice() {
    let fixture = Fixture::new("discovery-own-module", true);
    let calls = Cell::new(0);
    let mut previous = None;
    for _ in 0..2 {
        let game = fixture.catalog(&calls).unwrap();
        let entries = catalog::read(&game.join("internals.json")).unwrap();
        for path in [STRUCT, METHOD] {
            let entry = entries.iter().find(|entry| entry.path == path).unwrap();
            assert!(!entry.hash.is_empty());
        }
        let bytes = fs::read(game.join("internals.json")).unwrap();
        if let Some(previous) = previous {
            assert_eq!(bytes, previous);
        }
        previous = Some(bytes);
    }
    assert_eq!(calls.get(), 2);
    assert_eq!(
        fs::read_to_string(fixture.source.join("crates/horizon-client/src/game.rs")).unwrap(),
        ""
    );
}

#[test]
fn module_free_discovery_reuses_unchanged_catalog_without_running_mixins() {
    let fixture = Fixture::new("discovery-no-modules", false);
    let game = fixture.catalog(&Cell::new(0)).unwrap();
    let bytes = fs::read(game.join("internals.json")).unwrap();
    let entries = catalog::read(&game.join("internals.json")).unwrap();
    assert!(!entries.iter().any(|e| e.path == STRUCT || e.path == METHOD));
    let again = catalog_for_mod_with(
        &fixture.folder,
        &fixture.root.join("cache"),
        |_| panic!("warm discovery must not obtain source"),
        |_, _, _, _| panic!("module-free discovery must not run mixins"),
    )
    .unwrap();
    assert_eq!(again, game);
    assert_eq!(fs::read(again.join("internals.json")).unwrap(), bytes);
}

#[test]
fn module_edit_regenerates_hash_and_apply_failure_names_the_mod() {
    let fixture = Fixture::new("discovery-module-edit", true);
    let calls = Cell::new(0);
    let hash = |path: &Path| {
        catalog::read(&path.join("internals.json"))
            .unwrap()
            .into_iter()
            .find(|e| e.path == METHOD)
            .unwrap()
            .hash
    };
    let old = hash(&fixture.catalog(&calls).unwrap());
    fs::write(
        fixture.folder.join("src/ported.rs"),
        PAYLOAD.replace("u32", "u64"),
    )
    .unwrap();
    assert_ne!(hash(&fixture.catalog(&calls).unwrap()), old);
    let error = catalog_for_mod_with(
        &fixture.folder,
        &fixture.root.join("cache"),
        |_| Ok(fixture.source.clone()),
        |_, _, _, _| Err("E_TARGET_NOT_FOUND: cannot apply ported".into()),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "example: E_TARGET_NOT_FOUND: cannot apply ported"
    );
}

#[test]
fn discovery_error_text_is_unchanged() {
    assert_eq!(Error::SourceUnavailable { revision: "pinned".into(), reason: "offline".into() }.to_string(), "E_INTERNAL_SOURCE_UNAVAILABLE: internals sync needs the source for pinned. It is not cached and could not be downloaded: offline");
    assert_eq!(
        Error::Check("compiler failure".into()).to_string(),
        "compiler failure"
    );
}
