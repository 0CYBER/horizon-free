#![cfg(unix)]

use horizon_mod_package::{Manifest, internals::ListedInternal};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

const METHOD: &str = "horizon_client::game::ported_level::PortedGame::tick";
const HASH: &str = "c53f233eacb5389b";

struct Fixture {
    root: PathBuf,
    launcher: PathBuf,
}
impl Fixture {
    fn new(modules: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "horizon-module-discovery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname='example'\nversion='0.1.0'\nedition='2024'\n[lib]\npath='lib.rs'\n[workspace]\n").unwrap();
        fs::write(root.join("lib.rs"), "").unwrap();
        let mut text = format!(
            "id='example'\nname='Example'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        if modules {
            text.push_str("[[native.module]]\nid='ported'\nfile='game.rs'\nname='ported_level'\nsource='src/ported.rs'\n");
        }
        fs::write(root.join("horizon-mod.toml"), text).unwrap();
        let catalog = root.join(if modules {
            "module-catalog".to_owned()
        } else {
            format!(".horizon/cache/{}/build-data", "a".repeat(40))
        });
        fs::create_dir_all(&catalog).unwrap();
        if !modules {
            fs::write(catalog.parent().unwrap().join("build"), "a".repeat(40)).unwrap();
        }
        let build = serde_json::json!({"commit": "a".repeat(40)});
        fs::write(catalog.join("internals.json"), serde_json::json!({"schema": 1, "build": build, "internals": [{
            "path": METHOD, "kind": "method", "signature": "impl PortedGame { fn tick(&mut self) -> u32 }", "hash": HASH,
        }]}).to_string()).unwrap();
        fs::write(
            catalog.join("points.json"),
            serde_json::json!({"schema": 1, "build": build, "points": [{"target": METHOD}], "sites": []}).to_string(),
        )
        .unwrap();
        let launcher = root.join("launcher-stub");
        // Verify the real command contract and emit a path without shell-interpolating it.
        fs::write(&launcher, "#!/bin/sh\n[ \"$1\" = native-build ] && [ \"$2\" = --catalog ] && [ \"$3\" = --trust-native-code ] && [ \"$6\" = \"$PWD\" ] || exit 19\nmkdir \"$5\"; cp \"$PWD/module-catalog/\"*.json \"$5/\"; printf '%s\\n' \"$5\"\nprintf invoked > \"$PWD/launcher-called\"\n").unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
        Self { root, launcher }
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-horizon-mod"))
            .args(args)
            .current_dir(&self.root)
            .env("HORIZON_LAUNCHER", &self.launcher)
            .env("HORIZON_SDK_BASE", self.root.join("no-sdk"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
    fn manifest(&self) -> Manifest {
        Manifest::parse(&fs::read_to_string(self.root.join("horizon-mod.toml")).unwrap()).unwrap()
    }
    fn catalog_hash(&self) -> String {
        let entries: Vec<ListedInternal> = horizon_mod_package::internals::parse(
            &fs::read_to_string(self.root.join("module-catalog/internals.json")).unwrap(),
        )
        .unwrap();
        entries.into_iter().find(|e| e.path == METHOD).unwrap().hash
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
#[ignore = "daily"]
fn internals_add_uses_launcher_module_catalog_hash() {
    let fixture = Fixture::new(true);
    fixture.run(&["internals", "add", METHOD, "--kind", "method"]);
    let declared = fixture.manifest();
    assert_eq!(declared.internals.len(), 1);
    assert_eq!(declared.internals[0].path, METHOD);
    assert_eq!(declared.internals[0].hash, Some(fixture.catalog_hash()));
    assert!(fixture.root.join("launcher-called").is_file());
}

#[test]
#[ignore = "daily"]
fn inject_add_head_uses_launcher_module_catalog_hash() {
    let fixture = Fixture::new(true);
    fixture.run(&[
        "inject", "add", METHOD, "--kind", "head", "--with", "listen",
    ]);
    let declared = fixture.manifest();
    assert_eq!(declared.inject.len(), 1);
    assert_eq!(declared.inject[0].at, METHOD);
    assert_eq!(declared.inject[0].hash, Some(fixture.catalog_hash()));
    assert!(fixture.root.join("launcher-called").is_file());
}

#[test]
#[ignore = "daily"]
fn module_free_add_commands_keep_prebuilt_output_and_never_run_launcher() {
    for args in [
        vec!["internals", "add", METHOD, "--kind", "method"],
        vec![
            "inject", "add", METHOD, "--kind", "head", "--with", "listen",
        ],
    ] {
        let modules = Fixture::new(true);
        let prebuilt = Fixture::new(false);
        let expected = modules.run(&args);
        let actual = prebuilt.run(&args);
        assert_eq!(actual.stdout, expected.stdout);
        assert_eq!(actual.stderr, expected.stderr);
        assert!(!prebuilt.root.join("launcher-called").exists());
        let mut declared: toml::Value =
            toml::from_str(&fs::read_to_string(modules.root.join("horizon-mod.toml")).unwrap())
                .unwrap();
        declared["native"].as_table_mut().unwrap().remove("module");
        let prebuilt_text: toml::Value =
            toml::from_str(&fs::read_to_string(prebuilt.root.join("horizon-mod.toml")).unwrap())
                .unwrap();
        assert_eq!(declared, prebuilt_text);
    }
}
